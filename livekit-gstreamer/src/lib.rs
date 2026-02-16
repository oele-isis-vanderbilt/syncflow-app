pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod media_stream;
pub mod single_container_pipeline;
pub mod utils;

pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use media_stream::*;
pub use single_container_pipeline::*;

pub fn initialize_gstreamer() {
    gstreamer::init().expect("Failed to initialize GStreamer");
}
