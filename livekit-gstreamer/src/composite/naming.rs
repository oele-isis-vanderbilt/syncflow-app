//! Human-readable file names for composite recordings, e.g. `av-mix.mp4`,
//! `mic-3_UMC1820-ch3.m4a` or `camera-2_Logitech-C920.mp4`.
//!
//! The role prefix keeps names unique within a recording folder, so there are no
//! random suffixes or timestamps; the folder carries the time.

use super::ids::OutputId;

const MAX_SLUG_LEN: usize = 32;

/// Reduces `name` to ASCII letters, digits and single dashes, at most `max_len`
/// characters, with no leading or trailing dash. May be empty.
pub fn slug(name: &str, max_len: usize) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    slug[..slug.len().min(max_len)]
        .trim_end_matches('-')
        .to_string()
}

/// Reduces a device name to ASCII letters, digits and single dashes, at most 32 characters.
pub fn device_slug(name: &str) -> String {
    let slug = slug(name, MAX_SLUG_LEN);
    if slug.is_empty() {
        "device".to_string()
    } else {
        slug
    }
}

/// File name for `output`. `device_name` is ignored for the AV mix.
pub fn output_file_name(
    output: OutputId,
    device_name: &str,
    selected_channel: Option<u8>,
) -> String {
    let slug = device_slug(device_name);
    match output {
        OutputId::AvMix => "av-mix.mp4".to_string(),
        OutputId::Camera(_) | OutputId::Screen(_) => format!("{output}_{slug}.mp4"),
        OutputId::Mic(_) => match selected_channel {
            Some(channel) => format!("{output}_{slug}-ch{channel}.m4a"),
            None => format!("{output}_{slug}.m4a"),
        },
    }
}

/// Name of the metadata file written next to `file_name`, e.g. `av-mix.json`.
pub fn metadata_file_name(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    format!("{stem}.json")
}

pub const MANIFEST_FILE_NAME: &str = "manifest.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(device_slug("FaceTime HD Camera"), "FaceTime-HD-Camera");
        assert_eq!(device_slug("  USB  Mic (2)  "), "USB-Mic-2");
        assert_eq!(device_slug("Micro (Réalité)"), "Micro-R-alit");
        assert_eq!(device_slug("***"), "device");
        let long = device_slug("An Extremely Long Device Name That Goes On And On");
        assert!(long.len() <= MAX_SLUG_LEN);
        assert!(!long.ends_with('-'));
    }

    #[test]
    fn file_names() {
        assert_eq!(
            output_file_name(OutputId::AvMix, "ignored", None),
            "av-mix.mp4"
        );
        assert_eq!(
            output_file_name(OutputId::Camera(2), "Logitech C920", None),
            "camera-2_Logitech-C920.mp4"
        );
        assert_eq!(
            output_file_name(OutputId::Mic(3), "UMC1820", Some(3)),
            "mic-3_UMC1820-ch3.m4a"
        );
        assert_eq!(
            output_file_name(OutputId::Mic(1), "USB Mic A", None),
            "mic-1_USB-Mic-A.m4a"
        );
        assert_eq!(
            output_file_name(OutputId::Screen(1), "Display 1", None),
            "screen-1_Display-1.mp4"
        );
    }

    #[test]
    fn metadata_names() {
        assert_eq!(metadata_file_name("av-mix.mp4"), "av-mix.json");
        assert_eq!(
            metadata_file_name("mic-3_UMC1820-ch3.m4a"),
            "mic-3_UMC1820-ch3.json"
        );
    }
}
