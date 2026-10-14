pub mod composite;
pub mod devices;
pub mod lk_participant;
pub mod media_device;
pub mod publish_options;
pub mod runtime;
pub mod utils;

pub use devices::*;
pub use lk_participant::*;
pub use media_device::*;
pub use publish_options::*;

/// Initializes GStreamer, first pointing it at the runtime this process
/// linked (see [`runtime::configure`]). Use instead of `gstreamer::init()`,
/// and call it before starting other threads.
pub fn initialize_gstreamer() {
    runtime::configure();
    gstreamer::init().expect("Failed to initialize GStreamer");
}
