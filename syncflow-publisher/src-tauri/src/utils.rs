use std::path::{Path, PathBuf};

use crate::errors::SyncFlowPublisherError;

pub fn host_name() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

pub fn get_ip_address() -> Option<String> {
    local_ip_address::local_ip().ok().map(|ip| ip.to_string())
}

pub fn save_json(data: &impl serde::Serialize, path: &Path) -> Result<(), SyncFlowPublisherError> {
    let file = std::fs::File::create(path)?;
    serde_json::to_writer(file, data)?;
    Ok(())
}

pub fn load_json<T: serde::de::DeserializeOwned>(
    file_loc: &Path,
) -> Result<T, SyncFlowPublisherError> {
    let file = std::fs::File::open(file_loc)?;
    Ok(serde_json::from_reader(file)?)
}

/// Session names in folder names are cut to this many characters.
const SESSION_SLUG_LEN: usize = 40;

/// The folder that holds every join of one session, e.g.
/// `Lecture-3_8f3c1a2b-77de-4c1e-9d1a-0b6c2f1e4a55`: the session name and the full
/// session id, so every join of a session finds the same folder and no two sessions share one.
pub fn session_folder_name(session_name: &str, session_id: &str) -> String {
    let name = livekit_gstreamer::composite::naming::slug(session_name, SESSION_SLUG_LEN);
    let name = if name.is_empty() { "session" } else { &name };
    let id: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{name}_{id}")
}

/// Creates the next numbered join folder (`1`, `2`, …) in `session_dir` and returns its
/// number and path. Uses `create_dir`, so two joins never claim the same number.
pub fn claim_join_dir(session_dir: &Path) -> std::io::Result<(u32, PathBuf)> {
    std::fs::create_dir_all(session_dir)?;
    let mut n = std::fs::read_dir(session_dir)?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    loop {
        let dir = session_dir.join(n.to_string());
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((n, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_folder_names() {
        assert_eq!(
            session_folder_name("Lecture 3", "8f3c1a2b-77de-4c1e"),
            "Lecture-3_8f3c1a2b-77de-4c1e"
        );
        assert_eq!(
            session_folder_name("Lab test: Oct. 9 (group B)", "local-1760459882"),
            "Lab-test-Oct-9-group-B_local-1760459882"
        );
        assert_eq!(session_folder_name("***", "a/b"), "session_a-b");
        let long = session_folder_name(&"a very long session name ".repeat(5), "id");
        assert!(long.len() <= SESSION_SLUG_LEN + "_id".len());
    }

    #[test]
    fn join_folders_are_numbered() {
        let parent = std::env::temp_dir().join(format!("join-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let session = parent.join("Lecture_abc");
        assert_eq!(claim_join_dir(&session).unwrap(), (1, session.join("1")));
        assert_eq!(claim_join_dir(&session).unwrap(), (2, session.join("2")));
        // Gaps and stray entries don't matter: the next number is after the highest.
        std::fs::create_dir(session.join("7")).unwrap();
        std::fs::write(session.join("notes.txt"), "").unwrap();
        assert_eq!(claim_join_dir(&session).unwrap().0, 8);
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn concurrent_claims_get_distinct_numbers() {
        let parent = std::env::temp_dir().join(format!("join-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let session = parent.join("s");
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let session = session.clone();
                std::thread::spawn(move || claim_join_dir(&session).unwrap().0)
            })
            .collect();
        let mut numbers: Vec<u32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        numbers.sort();
        assert_eq!(numbers, (1..=8).collect::<Vec<_>>());
        std::fs::remove_dir_all(parent).unwrap();
    }
}
