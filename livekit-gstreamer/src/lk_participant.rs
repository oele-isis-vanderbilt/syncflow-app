use crate::media_device::GStreamerError;
use crate::media_stream::{GstMediaStream, PublishOptions};
use crate::utils::random_string;
use crate::{AudioPublishOptions, AvMixStream, VideoPublishOptions};
use gstreamer::prelude::ClockExt;
use gstreamer::Buffer;
use livekit::options::{TrackPublishOptions, VideoCodec};
use livekit::track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{
    AudioFrame, I420Buffer, RtcAudioSource, RtcVideoSource, VideoFrame, VideoResolution,
    VideoRotation,
};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::{Room, RoomError};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Error, Debug)]
pub enum LKParticipantError {
    #[error("GStreamer error: {0}")]
    GStreamerError(#[from] GStreamerError),
    #[error("Livekit error: {0}")]
    LivekitError(#[from] Box<RoomError>),
    #[error("Streaming error: {0}")]
    StreamingError(String),
}

impl From<RoomError> for LKParticipantError {
    fn from(err: RoomError) -> Self {
        LKParticipantError::LivekitError(Box::new(err))
    }
}

pub struct LKParticipant {
    room: Arc<Room>,
    published_tracks: HashMap<String, TrackHandle>,
    master_clock: gstreamer::Clock,  // shared clock
    base_time: gstreamer::ClockTime, // shared base time
}

struct TrackHandle {
    track: LocalTrack,
    task: tokio::task::JoinHandle<()>,
}

pub struct StreamingHandlingConfig {
    pub gst_media_stream: GstMediaStream,
    pub publish_to_livekit: bool,
}

pub struct AvMixStreamingConfig {
    pub camera: VideoPublishOptions,
    pub mic1: AudioPublishOptions,
    pub mic2: Option<AudioPublishOptions>,
    pub publish_audio_to_livekit: bool,
}

impl LKParticipant {
    pub fn new(room: Arc<Room>) -> Self {
        let master_clock = gstreamer::SystemClock::obtain();
        let base_time = master_clock.time(); // snapshot once at construction
        Self {
            room,
            published_tracks: HashMap::new(),
            master_clock,
            base_time,
        }
    }

    pub async fn start_stream(
        &mut self,
        stream: &mut GstMediaStream,
    ) -> Result<(), LKParticipantError> {
        if !stream.has_started() {
            stream
                .start_with_clock(self.master_clock.clone(), self.base_time)
                .await?;
        }
        Ok(())
    }

    pub async fn handle_av_stream(
        &mut self,
        config: AvMixStreamingConfig,
    ) -> Result<(), LKParticipantError> {
        let camera = config.camera.clone();
        let mic1 = config.mic1.clone();
        let mic2 = config.mic2.clone();
        let mut av_mix_stream = AvMixStream::new(camera, mic1, mic2);
        av_mix_stream.build_pipeline().await?;
        av_mix_stream.preroll_pipeline().await?;
        av_mix_stream.set_pipeline_clock(&self.master_clock, self.base_time)?;

        if config.publish_audio_to_livekit {
            self.publish_avmix_stream_audio(&mut av_mix_stream, None)
                .await?;
        }

        if config.publish_audio_to_livekit {
            self.publish_avmix_stream_video(&mut av_mix_stream, None)
                .await?;
        }

        Ok(())
    }

    pub async fn publish_avmix_stream_audio(
        &mut self,
        stream: &mut AvMixStream,
        track_name: Option<String>,
    ) -> Result<String, LKParticipantError> {
        if !stream.has_started() {
            return Err(LKParticipantError::GStreamerError(
                GStreamerError::PipelineError("Stream has not started".into()),
            ));
        }
        let (frames_rx, close_rx) = stream.subscribe_audio().unwrap();
        let audio_options = stream.get_audio_publish_options();
        let track_name = format!(
            "{}-{}",
            self.room.local_participant().name(),
            "AV Mix Audio"
        );

        let rtc_source = NativeAudioSource::new(
            Default::default(),
            audio_options.framerate as u32,
            audio_options.channels as u32,
            2000,
        );

        let track = LocalAudioTrack::create_audio_track(
            &track_name,
            RtcAudioSource::Native(rtc_source.clone()),
        );

        let track_sid = random_string("audio-track");

        let task = tokio::spawn(Self::audio_track_task(
            close_rx,
            frames_rx,
            rtc_source.clone(),
        ));

        self.room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                },
            )
            .await?;

        self.published_tracks.insert(
            track_sid.clone(),
            TrackHandle {
                track: LocalTrack::Audio(track),
                task,
            },
        );

        Ok(track_sid)
    }

    pub async fn publish_avmix_stream_video(
        &mut self,
        stream: &mut AvMixStream,
        track_name: Option<String>,
    ) -> Result<String, LKParticipantError> {
        if !stream.has_started() {
            return Err(LKParticipantError::GStreamerError(
                GStreamerError::PipelineError("Stream has not started".into()),
            ));
        }
        let (frames_rx, close_rx) = stream.subscribe_video().unwrap();
        let video_options = stream.get_video_publish_options();
        let track_name = format!(
            "{}-{}",
            self.room.local_participant().name(),
            "AV Mix Video"
        );

        let rtc_source = NativeVideoSource::new(VideoResolution {
            width: 640,  // video_options.width as u32,
            height: 480, // video_options.height as u32,
        });

        let track = LocalVideoTrack::create_video_track(
            &track_name,
            RtcVideoSource::Native(rtc_source.clone()),
        );

        let track_sid = random_string("video-track");

        let task = tokio::spawn(Self::video_track_task(
            close_rx,
            frames_rx,
            rtc_source.clone(),
        ));

        self.room
            .local_participant()
            .publish_track(
                LocalTrack::Video(track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    simulcast: false,
                    video_codec: VideoCodec::VP9,
                    ..Default::default()
                },
            )
            .await?;

        self.published_tracks.insert(
            track_sid.clone(),
            TrackHandle {
                track: LocalTrack::Video(track),
                task,
            },
        );

        Ok(track_sid)
    }

    pub async fn handle_streams(
        &mut self,
        configs: &mut [StreamingHandlingConfig],
    ) -> Result<(), LKParticipantError> {
        let mut recording_metadatas = vec![];

        for config in configs.iter_mut() {
            let (_, metadata) = config.gst_media_stream.build_pipeline().await?;
            recording_metadatas.push(metadata);
        }

        let preroll_futures: Vec<_> = configs
            .iter_mut()
            .map(|config| config.gst_media_stream.preroll_pipeline())
            .collect();

        futures::future::try_join_all(preroll_futures).await?;

        for (config, metadata) in configs.iter_mut().zip(recording_metadatas.iter_mut()) {
            config.gst_media_stream.set_pipeline_clock(
                &self.master_clock,
                self.base_time,
                metadata.as_mut(),
            )?;
        }

        for (config, metadata) in configs.iter_mut().zip(recording_metadatas.iter_mut()) {
            config.gst_media_stream.play_pipeline(metadata.as_mut())?;
        }

        let bus_loop_futures: Vec<_> = configs
            .iter_mut()
            .zip(recording_metadatas.iter_mut())
            .map(|(config, metadata)| config.gst_media_stream.run_bus_loop(metadata.as_mut()))
            .collect();

        futures::future::try_join_all(bus_loop_futures).await?;

        for config in configs.iter_mut().filter(|c| c.publish_to_livekit) {
            self.publish_stream(&mut config.gst_media_stream, None)
                .await?;
        }

        Ok(())
    }

    pub async fn publish_stream(
        &mut self,
        stream: &mut GstMediaStream,
        track_name: Option<String>,
    ) -> Result<String, LKParticipantError> {
        if !stream.has_started() {
            stream
                .start_with_clock(self.master_clock.clone(), self.base_time)
                .await?;
        }
        // This unwrap is safe because we know the stream has started
        let (frames_rx, close_rx) = stream.subscribe().unwrap();
        let details = stream.details().unwrap();
        let device_name = stream
            .get_device_name()
            .unwrap_or("Unknown Device".to_string());
        let track_name = format!("{}-{}", self.room.local_participant().name(), device_name);

        match details {
            PublishOptions::Video(details) => {
                // let rtc_source = NativeVideoSource::new(VideoResolution {
                //     width: details.width as u32,
                //     height: details.height as u32,
                // });
                let rtc_source = NativeVideoSource::new(VideoResolution {
                    width: 640,
                    height: 480,
                });

                let track = LocalVideoTrack::create_video_track(
                    &track_name,
                    RtcVideoSource::Native(rtc_source.clone()),
                );

                let track_sid = random_string("video-track");

                let task = tokio::spawn(Self::video_track_task(
                    close_rx,
                    frames_rx,
                    rtc_source.clone(),
                ));

                self.room
                    .local_participant()
                    .publish_track(
                        LocalTrack::Video(track.clone()),
                        TrackPublishOptions {
                            source: TrackSource::Camera,
                            simulcast: false,
                            video_codec: VideoCodec::VP9,
                            ..Default::default()
                        },
                    )
                    .await?;

                self.published_tracks.insert(
                    track_sid.clone(),
                    TrackHandle {
                        track: LocalTrack::Video(track),
                        task,
                    },
                );

                Ok(track_sid)
            }
            PublishOptions::Audio(details) => {
                let rtc_source = match details.selected_channel {
                    Some(_) => NativeAudioSource::new(
                        Default::default(),
                        details.framerate as u32,
                        1,
                        2000,
                    ),
                    None => NativeAudioSource::new(
                        Default::default(),
                        details.framerate as u32,
                        details.channels as u32,
                        2000,
                    ),
                };

                let track = LocalAudioTrack::create_audio_track(
                    &track_name,
                    RtcAudioSource::Native(rtc_source.clone()),
                );

                let track_sid = random_string("audio-track");

                let task = tokio::spawn(Self::audio_track_task(
                    close_rx,
                    frames_rx,
                    rtc_source.clone(),
                ));

                self.room
                    .local_participant()
                    .publish_track(
                        LocalTrack::Audio(track.clone()),
                        TrackPublishOptions {
                            source: TrackSource::Microphone,
                            ..Default::default()
                        },
                    )
                    .await?;

                self.published_tracks.insert(
                    track_sid.clone(),
                    TrackHandle {
                        track: LocalTrack::Audio(track),
                        task,
                    },
                );

                Ok(track_sid)
            }
            PublishOptions::Screen(details) => {
                // let rtc_source = NativeVideoSource::new(VideoResolution {
                //     width: details.width as u32,
                //     height: details.height as u32,
                // });

                let rtc_source = NativeVideoSource::new(VideoResolution {
                    width: 640,
                    height: 480,
                });

                let track = LocalVideoTrack::create_video_track(
                    &track_name,
                    RtcVideoSource::Native(rtc_source.clone()),
                );

                let track_sid = random_string("screen-track");

                let task = tokio::spawn(Self::video_track_task(
                    close_rx,
                    frames_rx,
                    rtc_source.clone(),
                ));

                self.room
                    .local_participant()
                    .publish_track(
                        LocalTrack::Video(track.clone()),
                        TrackPublishOptions {
                            source: TrackSource::Screenshare,
                            simulcast: false,
                            video_codec: VideoCodec::VP9,
                            ..Default::default()
                        },
                    )
                    .await?;

                self.published_tracks.insert(
                    track_sid.clone(),
                    TrackHandle {
                        track: LocalTrack::Video(track),
                        task,
                    },
                );

                Ok(track_sid)
            }
        }
    }

    pub async fn unpublish_track(&mut self, track_sid: &str) -> Result<(), LKParticipantError> {
        if let Some(handle) = self.published_tracks.get(track_sid) {
            self.room
                .local_participant()
                .unpublish_track(&handle.track.sid())
                .await?;
            handle.task.abort();
        }
        Ok(())
    }

    async fn video_track_task(
        mut close_rx: broadcast::Receiver<()>,
        mut frames_rx: broadcast::Receiver<Arc<Buffer>>,
        rtc_source: NativeVideoSource,
    ) {
        loop {
            tokio::select! {
                _ = close_rx.recv() => {
                    break;
                }
                frame = frames_rx.recv() => {
                    if let Ok(frame) = frame {
                        let map = frame.map_readable().unwrap();
                        let data = map.as_slice();
                        let timestamp_us = frame.pts().unwrap_or_default().useconds() as i64;
                        let res = rtc_source.video_resolution();
                        let width = res.width;
                        let height = res.height;
                        let mut wrtc_video_buffer = I420Buffer::new(width, height);
                        let (data_y, data_u, data_v) = wrtc_video_buffer.data_mut();

                        let y_plane_size = (width * height) as usize;
                        let uv_plane_size = (width * height / 4) as usize;

                        data_y.copy_from_slice(&data[0..y_plane_size]);
                        data_u.copy_from_slice(&data[y_plane_size..y_plane_size + uv_plane_size]);
                        data_v.copy_from_slice(
                            &data[y_plane_size + uv_plane_size..y_plane_size + 2 * uv_plane_size],
                        );

                        let video_frame = VideoFrame {
                            buffer: wrtc_video_buffer,
                            rotation: VideoRotation::VideoRotation0,
                            timestamp_us,
                        };
                        rtc_source.capture_frame(&video_frame);
                    }
                }
            }
        }
    }

    async fn audio_track_task(
        mut close_rx: broadcast::Receiver<()>,
        mut frames_rx: broadcast::Receiver<Arc<Buffer>>,
        rtc_source: NativeAudioSource,
    ) {
        loop {
            tokio::select! {
                    _ = close_rx.recv() => {
                        break;
                    }
                    frame = frames_rx.recv() => {
                        if let Ok(frame) = frame {
                            let map = frame.map_readable().unwrap();
                            let audio_data: &[i16] = unsafe {
                                std::slice::from_raw_parts(map.as_ptr() as *const i16, map.size() / 2)
                            };
                            let samples_per_channel = audio_data.len() as u32 / rtc_source.num_channels();
                            let audio_frame = AudioFrame {
                                data: Cow::Borrowed(audio_data),
                                sample_rate: rtc_source.sample_rate(),
                                num_channels: rtc_source.num_channels(),
                                samples_per_channel,
                            };
                            rtc_source.capture_frame(&audio_frame).await.unwrap();
                    }
                }
            }
        }
    }
}
