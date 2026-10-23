export interface VideoCapability {
    width: number;
    height: number;
    framerates: number[];
    codec: string;
}

export interface AudioCapability {
    channels: number;
    framerates: [number, number];
    codec: string;
    /** Only offered in exclusive mode (Windows): the device's own format. */
    exclusive?: boolean;
}

export interface ScreenCapability {
    width: number;
    height: number;
    framerates: number[];
    codec: string;
    startx: number;
    starty: number;
    endx: number;
    endy: number;
}

export interface MediaDeviceInfo {
    devicePath: string;
    displayName: string;
    capabilities: MediaCapability[];
    deviceClass: string;
}

export type MediaCapability =
    | ({ kind: 'Video' } & VideoCapability)
    | ({ kind: 'Audio' } & AudioCapability)
    | ({ kind: 'Screen' } & ScreenCapability);

export interface RegistrationResponse {
    deviceId: string;
    deviceName: string;
    deviceGroup: string;
    projectName: string;
    projectId: string;
    projectComments: string;
    lkServerUrl: string;
    s3BucketName: string;
    s3Endpoint: string;
}

export interface VideoPublishOptions {
    codec: string;
    deviceId: string;
    width: number;
    height: number;
    framerate: number;
}

export interface AudioPublishOptions {
    codec: string;
    deviceId: string;
    framerate: number;
    channels: number;
    selectedChannel?: number;
    /** Open the device in exclusive mode (Windows); other apps can't use it meanwhile. */
    exclusive?: boolean;
}

export interface ScreenPublishOptions {
    codec: string;
    screenIdOrName: string;
    width: number;
    height: number;
    framerate: number;
}

export type PublishOptions =
    | ({ kind: 'Video' } & VideoPublishOptions)
    | ({ kind: 'Audio' } & AudioPublishOptions)
    | ({ kind: 'Screen' } & ScreenPublishOptions);

/** One file and its optional stream. */
export interface OutputConfig<T> {
    options: T;
    stream: boolean;
}

/** Camera plus one or two mics, recorded to av-mix.mp4 with mic 1 on L and mic 2 on R. */
export interface AvMixConfig {
    camera: VideoPublishOptions;
    mic1: AudioPublishOptions;
    mic2: AudioPublishOptions | null;
    stream: boolean;
}

/** What one recording contains; saved by `set_streaming_config`. */
export interface CompositeConfig {
    avMix: AvMixConfig | null;
    screen: OutputConfig<ScreenPublishOptions> | null;
    cameras: OutputConfig<VideoPublishOptions>[];
    mics: OutputConfig<AudioPublishOptions>[];
    recordingMode: RecordingMode;
    /** Voice-activity threshold for streamed audio (0 turns gating off); recordings are unaffected. */
    vadThreshold: number;
}

export interface PipelineCapabilities {
    noiseSuppression: boolean;
}

export type RecordingMode = 'sessionMode' | 'localMode';

export interface NewSessionMessage {
    sessionId: string;
    sessionName: string;
}

export type PublicationNotification =
    | PublicationNotificationFailure
    | PublicationNotificationStreamingSuccess
    | FileUploadProgress
    | UploadFinished
    | SessionEnded
    | DeviceFailure
    | OutputFailure
    | StreamFailure
    | PipelineWarning;

/** The device behind a failure. */
export interface DeviceDetails {
    /** e.g. `mic-3` */
    role: string;
    kind: 'camera' | 'mic' | 'screen';
    displayName: string;
    deviceId: string;
    codec: string;
    selectedChannel: number | null;
}

/** A device failed; the files and streams it fed were finalized or closed. */
export interface DeviceFailure {
    kind: 'deviceFailure';
    sessionId: string;
    device: DeviceDetails;
    roles: string[];
    /** Outputs it fed, e.g. `av-mix`, `mic-3` */
    outputs: string[];
    reason: string;
    phase: 'start' | 'running';
    recordingContinues: boolean;
}

/** One file failed (encoder, muxer or disk); the device keeps feeding the rest. */
export interface OutputFailure {
    kind: 'outputFailure';
    sessionId: string;
    output: string;
    reason: string;
    recordingContinues: boolean;
}

/** One stream failed; files are unaffected. */
export interface StreamFailure {
    kind: 'streamFailure';
    sessionId: string;
    sink: string;
    reason: string;
}

export interface PipelineWarning {
    kind: 'pipelineWarning';
    sessionId: string;
    scope: string | null;
    message: string;
}

export interface PublicationNotificationFailure {
    kind: 'failure';
    sessionId: string;
    reason: string;
}

export interface PublicationNotificationStreamingSuccess {
    kind: 'streamingSuccess';
    sessionId: string;
    sessionName: string;
    startedAt: string;
    devices: string[];
}

export type FileUploadStatus = 'pending' | 'uploading' | 'done' | 'failed';

/** One file of a join's upload changed status or sent a part. */
export interface FileUploadProgress {
    kind: 'fileUploadProgress';
    sessionId: string;
    /** 1 for the first join of the session, 2 for the next, … */
    join: number;
    file: string;
    uploadedBytes: number;
    totalBytes: number;
    status: FileUploadStatus;
    error: string | null;
}

/** A join's upload stopped: everything is in S3 (`complete`), or `failed` files remain. */
export interface UploadFinished {
    kind: 'uploadFinished';
    sessionId: string;
    join: number;
    complete: boolean;
    failed: string[];
}

/** A join with an unfinished upload, from `get_pending_uploads` (`upload-state.json`). */
export interface PendingUpload {
    sessionId: string;
    sessionName: string;
    join: number;
    bucket: string;
    prefix: string;
    files: Record<string, { size: number; status: FileUploadStatus; error: string | null }>;
}

/** What the UI knows about one file of a join's upload. */
export interface FileUpload {
    uploadedBytes: number;
    totalBytes: number;
    status: FileUploadStatus;
    error: string | null;
}

/** Uploads by session id, then join number, then file name. */
export type Uploads = Record<string, Record<number, Record<string, FileUpload>>>;

export interface SessionEnded {
    kind: 'sessionEnded';
    sessionId: string;
}
