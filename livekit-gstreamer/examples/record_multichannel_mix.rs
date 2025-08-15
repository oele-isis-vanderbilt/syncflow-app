use livekit_gstreamer::alsa_mix_pipeline::UMC1820MixStream;
use livekit_gstreamer::initialize_gstreamer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    initialize_gstreamer();

    let stream: UMC1820MixStream = UMC1820MixStream::new(
        "hw:1",
        96000,
        10,
        vec![0, 1, 2, 3, 4, 5, 6, 7],
        "recordings",
    );

    stream.start()?;

    Ok(())
}
