pub mod av_mix_stream;
pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod media_stream;
pub mod utils;

pub use av_mix_stream::*;
pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use media_stream::*;

pub fn initialize_gstreamer() {
    gstreamer::init().expect("Failed to initialize GStreamer");
}
