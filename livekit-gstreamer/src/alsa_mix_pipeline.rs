use std::path::PathBuf;

use crate::GStreamerError;

pub struct UMC1820MixStream {
    device_id: String,
    framerate: u32,
    total_channels: usize,
    mix_channels: Vec<usize>,
    opdir: PathBuf,
}

fn get_pipeline_head(device_id: &str, sample_rate: u32, total_channels: usize) -> String {
    format!(
        "alsasrc device={device_id} buffer-time=200000 latency-time=10000 ! \
         audio/x-raw,format=S32LE,rate={sample_rate},channels={total_channels},layout=interleaved ! \
         deinterleave name=di \\",
        device_id = device_id,
        sample_rate = sample_rate,
        total_channels = total_channels
    )
}

fn get_channel_and_mix(
    channel: usize,
    sample_rate: u32,
    amplification: f64,
    channel_prefix: usize,
    outdir: &PathBuf,
    mixer_name: &str,
    is_first_mixer: bool,
) -> String {
    format!(
        "di.src_{channel} ! tee name=t{channel} \\
         t{channel}. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate={sample_rate},channels=1 \\
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location={outdir}/ch{channel_prefix}.m4a \\
         t{channel}. ! queue ! audioconvert ! audioresample ! audioamplify amplification={amplification} ! {mixer_head} \\",
         mixer_head = if is_first_mixer {
            format!("{}.", mixer_name)
         } else {
            format!("{}.", mixer_name)
         },
        channel = channel,
        sample_rate = sample_rate,
        amplification = amplification,
        channel_prefix = channel_prefix,
        outdir = outdir.to_str().unwrap(),
    )
}

fn appsink_str(sample_rate: u32, mixer_name: &str, appsink_name: &str, op_dir: &str) -> String {
    format!(
        " {mixer_name}. ! tee name=appsink_tee \\
          appsink_tee. ! queue ! \\
          audioconvert ! audioresample ! audio/x-raw,channels=1,rate={sample_rate},format=S16LE ! \\
          ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location={op_dir}/mix.m4a \\
          appsink_tee. ! queue ! audioconvert ! audioresample ! audio/x-raw,channels=1,rate={sample_rate},format=S16LE ! interleave name=il \\
          il.   ! queue ! audio/x-raw,channels=1,rate={sample_rate},format=S16LE ! appsink name={appsink_name} emit-signals=true sync=false",
        sample_rate = sample_rate,
        mixer_name = mixer_name,
        appsink_name = appsink_name
    )
}

impl UMC1820MixStream {
    pub fn new(
        device_id: &str,
        framerate: u32,
        total_channels: usize,
        mix_channels: Vec<usize>,
        output_dir: &str,
    ) -> Self {
        Self {
            device_id: device_id.to_string(),
            framerate,
            total_channels,
            mix_channels,
            opdir: PathBuf::from(output_dir),
        }
    }

    pub fn start(&self) -> Result<(), GStreamerError> {
        let pipeline_head = get_pipeline_head(&self.device_id, self.framerate, self.total_channels);

        let mixer_heads = self
            .mix_channels
            .iter()
            .enumerate()
            .map(|(index, &channel)| {
                get_channel_and_mix(
                    channel,
                    self.framerate,
                    0.125,
                    channel + 1,
                    &self.opdir,
                    "mix",
                    index == 0,
                )
            })
            .collect::<Vec<String>>();

        let appsink = appsink_str(
            self.framerate,
            "mix",
            "broadcast_appsink",
            self.opdir.to_str().unwrap(),
        );

        let pipeline_str = format!(
            "gst-launch-1.0 {}\n{}\n{}",
            pipeline_head,
            mixer_heads.join("\n"),
            appsink
        );

        println!("{}", pipeline_str);

        Ok(())
    }

    pub fn stop(&self) -> Result<(), GStreamerError> {
        Ok(())
    }
}
