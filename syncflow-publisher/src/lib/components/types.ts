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

export interface LocalFileSaveOptions {
  outputDir: string; // Note: no camelCase conversion since it's not in the struct
}

export interface LocalSaveFileMetadata {
  fileName: string;
  codec: string;
  startedAt: string;
}

export interface VideoPublishOptions {
  codec: string;
  deviceId: string;
  width: number;
  height: number;
  framerate: number;
  localFileSaveOptions?: LocalFileSaveOptions;
}

export interface AudioPublishOptions {
  codec: string;
  deviceId: string;
  framerate: number;
  channels: number;
  selectedChannel?: number;
  localFileSaveOptions?: LocalFileSaveOptions;
}

export interface ScreenPublishOptions {
  codec: string;
  screenIdOrName: string;
  width: number;
  height: number;
  framerate: number;
  localFileSaveOptions?: LocalFileSaveOptions;
}

export type PublishOptions = 
  | ({ kind: 'Video' } & VideoPublishOptions)
  | ({ kind: 'Audio' } & AudioPublishOptions)
  | ({ kind: 'Screen' } & ScreenPublishOptions);