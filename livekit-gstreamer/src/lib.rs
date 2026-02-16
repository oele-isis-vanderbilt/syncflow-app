pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod media_stream;
pub mod utils;
// Only enable this for windows for now
#[cfg(target_os = "windows")]
pub mod single_container_pipeline;

pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use media_stream::*;
#[cfg(target_os = "windows")]
pub use single_container_pipeline::*;

pub fn initialize_gstreamer() {
    gstreamer::init().expect("Failed to initialize GStreamer");
}
