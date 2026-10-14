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

/// A readable recording folder name, e.g. `2026-10-14_11-38_Lecture-3_8f3c1a`: the start
/// time to the minute, the session name, and a short id (`local` for local recordings).
pub fn recording_folder_name(
    started_at: chrono::DateTime<chrono::Local>,
    session_name: &str,
    short_id: &str,
) -> String {
    let name = livekit_gstreamer::composite::naming::slug(session_name, SESSION_SLUG_LEN);
    let name = if name.is_empty() { "session" } else { &name };
    format!("{}_{name}_{short_id}", started_at.format("%Y-%m-%d_%H-%M"))
}

/// The first six letters and digits of a session id.
pub fn short_session_id(session_id: &str) -> String {
    session_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(6)
        .collect()
}

/// `parent/folder`, or `parent/folder-2`, `-3`, … if it already exists, so a recording
/// never lands in another recording's folder.
pub fn unique_recording_dir(parent: &Path, folder: &str) -> PathBuf {
    let candidate = parent.join(folder);
    if !candidate.exists() {
        return candidate;
    }
    (2..)
        .map(|n| parent.join(format!("{folder}-{n}")))
        .find(|dir| !dir.exists())
        .unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn folder_names_are_readable() {
        let at = chrono::Local
            .with_ymd_and_hms(2026, 10, 14, 11, 38, 5)
            .unwrap();
        assert_eq!(
            recording_folder_name(at, "Lecture 3", "8f3c1a"),
            "2026-10-14_11-38_Lecture-3_8f3c1a"
        );
        assert_eq!(
            recording_folder_name(at, "Lab test: Oct. 9 (group B)", "local"),
            "2026-10-14_11-38_Lab-test-Oct-9-group-B_local"
        );
        assert_eq!(
            recording_folder_name(at, "***", "local"),
            "2026-10-14_11-38_session_local"
        );
        let long = recording_folder_name(at, &"a very long session name ".repeat(5), "local");
        assert!(long.len() <= "2026-10-14_11-38__local".len() + SESSION_SLUG_LEN);
    }

    #[test]
    fn short_ids() {
        assert_eq!(short_session_id("8f3c1a2b-77de-4c1e"), "8f3c1a");
        assert_eq!(short_session_id("local-1760459882"), "local1");
    }

    #[test]
    fn existing_folders_get_a_suffix() {
        let parent = std::env::temp_dir().join(format!("folder-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(parent.join("rec")).unwrap();
        std::fs::create_dir_all(parent.join("rec-2")).unwrap();
        assert_eq!(unique_recording_dir(&parent, "rec"), parent.join("rec-3"));
        assert_eq!(unique_recording_dir(&parent, "other"), parent.join("other"));
        std::fs::remove_dir_all(parent).unwrap();
    }
}
