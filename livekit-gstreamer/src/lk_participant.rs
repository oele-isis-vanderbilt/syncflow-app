use crate::composite::{CompositeStream, SinkId, SinkKind};
use crate::media_device::GStreamerError;
use gstreamer::Buffer;
use livekit::options::{TrackPublishOptions, VideoCodec};
use livekit::track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{
    AudioFrame, I420Buffer, RtcAudioSource, RtcVideoSource, VideoFrame, VideoResolution,
    VideoRotation,
};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::{DataPacket, Room, RoomError};
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

/// Publishes a composite stream's appsinks to a LiveKit room, and data messages.
pub struct LKParticipant {
    room: Arc<Room>,
    /// Published tracks, keyed by sink id (e.g. `av-mix-video`).
    published_tracks: HashMap<String, TrackHandle>,
}

struct TrackHandle {
    track: LocalTrack,
    task: tokio::task::JoinHandle<()>,
}

impl LKParticipant {
    pub fn new(room: Arc<Room>) -> Self {
        Self {
            room,
            published_tracks: HashMap::new(),
        }
    }

    /// Publishes every appsink of `stream` as a track named `<participant>-<sink-id>`:
    /// cameras and the AV mix video as Camera, screens as Screenshare, and mono audio as
    /// Microphone. A track unpublishes itself when its branch closes (on stop, or when the
    /// devices feeding it fail). Returns the sinks published.
    pub async fn publish_composite(
        &mut self,
        stream: &CompositeStream,
    ) -> Result<Vec<SinkId>, LKParticipantError> {
        let participant = self.room.local_participant().name();
        let mut published = Vec::new();
        for info in stream.sinks() {
            let Some(subscription) = stream.subscribe(&info.id) else {
                continue;
            };
            let track_name = format!("{participant}-{}", info.id);
            let (track, options, task): (LocalTrack, TrackPublishOptions, _) = match info.kind {
                SinkKind::Video { width, height } => {
                    let rtc_source = NativeVideoSource::new(VideoResolution { width, height });
                    let track = LocalVideoTrack::create_video_track(
                        &track_name,
                        RtcVideoSource::Native(rtc_source.clone()),
                    );
                    let source = match info.id {
                        SinkId::Screen(_) => TrackSource::Screenshare,
                        _ => TrackSource::Camera,
                    };
                    let options = TrackPublishOptions {
                        source,
                        simulcast: false,
                        video_codec: VideoCodec::VP9,
                        ..Default::default()
                    };
                    let task = Self::video_track_task(
                        subscription.closed,
                        subscription.frames,
                        rtc_source,
                    );
                    (
                        LocalTrack::Video(track),
                        options,
                        futures::future::Either::Left(task),
                    )
                }
                SinkKind::Audio { rate, channels } => {
                    let rtc_source =
                        NativeAudioSource::new(Default::default(), rate, channels, 2000);
                    let track = LocalAudioTrack::create_audio_track(
                        &track_name,
                        RtcAudioSource::Native(rtc_source.clone()),
                    );
                    let options = TrackPublishOptions {
                        source: TrackSource::Microphone,
                        ..Default::default()
                    };
                    let task = Self::audio_track_task(
                        subscription.closed,
                        subscription.frames,
                        rtc_source,
                    );
                    (
                        LocalTrack::Audio(track),
                        options,
                        futures::future::Either::Right(task),
                    )
                }
            };
            self.room
                .local_participant()
                .publish_track(track.clone(), options)
                .await?;
            let room = self.room.clone();
            let sid = track.sid();
            let task = tokio::spawn(async move {
                task.await;
                let _ = room.local_participant().unpublish_track(&sid).await;
            });
            self.published_tracks
                .insert(info.id.to_string(), TrackHandle { track, task });
            published.push(info.id);
        }
        Ok(published)
    }

    /// Publishes a data message to the room under `topic`.
    pub async fn publish_data(
        &self,
        topic: &str,
        payload: Vec<u8>,
        reliable: bool,
    ) -> Result<(), LKParticipantError> {
        self.room
            .local_participant()
            .publish_data(DataPacket {
                payload,
                topic: Some(topic.to_string()),
                reliable,
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn unpublish_track(&mut self, track_sid: &str) -> Result<(), LKParticipantError> {
        if let Some(handle) = self.published_tracks.remove(track_sid) {
            handle.task.abort();
            self.room
                .local_participant()
                .unpublish_track(&handle.track.sid())
                .await?;
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
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    {
                        let Ok(map) = frame.map_readable() else { continue };
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
                        let frame = match frame {
                            Ok(frame) => frame,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        };
                        {
                            let Ok(map) = frame.map_readable() else { continue };
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
                            if rtc_source.capture_frame(&audio_frame).await.is_err() {
                                break;
                            }
                    }
                }
            }
        }
    }
}
