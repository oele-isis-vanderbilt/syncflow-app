pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod media_stream;
pub mod utils;

#[cfg(target_os = "linux")]
pub mod alsa_stream;
pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use media_stream::*;

#[cfg(target_os = "linux")]
pub use alsa_stream::*;

pub fn initialize_gstreamer() {
    gstreamer::init().expect("Failed to initialize GStreamer");
}
