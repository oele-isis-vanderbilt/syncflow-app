pub mod av_mix_stream;
pub mod composite;
pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod media_stream;
pub mod runtime;
pub mod utils;

pub use av_mix_stream::*;
pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use media_stream::*;

/// Initializes GStreamer, first pointing it at the runtime this process
/// linked (see [`runtime::configure`]). Use instead of `gstreamer::init()`,
/// and call it before starting other threads.
pub fn initialize_gstreamer() {
    runtime::configure();
    gstreamer::init().expect("Failed to initialize GStreamer");
}
