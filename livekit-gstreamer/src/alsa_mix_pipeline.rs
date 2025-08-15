use std::path::PathBuf;

pub struct UMC1820MixStream {
    device_id: String,
    framerate: u32,
    total_channels: usize,
    mix_channels: Vec<usize>,
    opdir: PathBuf
}

impl UMC1820MixStream {
    pub fn new(device_id: &str, framerate: u32, total_channels: usize, mix_channels: Vec<usize>, output_dir: &str) -> Self {
        Self {
            device_id: device_id.to_string(),
            framerate,
            total_channels,
            mix_channels,
            opdir: PathBuf::from(output_dir),
        }
    }

    pub fn pipeline(&self) -> String {
        
    }


}