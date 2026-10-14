// Formats a device offers, built from the capabilities it reports, with readable labels
// and sensible defaults.
import type {
    AudioPublishOptions,
    MediaDeviceInfo,
    ScreenPublishOptions,
    VideoPublishOptions,
} from '#lib/components/types.js';

export type DeviceKind = 'camera' | 'mic' | 'screen';

const DEVICE_CLASS: Record<DeviceKind, string> = {
    camera: 'Video/Source',
    mic: 'Audio/Source',
    screen: 'Screen/Source',
};

/** Rates offered for mics, when the device supports them; 48 kHz is the default. */
const AUDIO_RATES = [48000, 44100, 32000, 24000, 16000];
/** Camera codecs in order of preference: MJPEG keeps USB bandwidth low. */
const CODEC_PREFERENCE = ['image/jpeg', 'video/x-raw', 'video/x-h264'];

export function devicesOfKind(devices: MediaDeviceInfo[], kind: DeviceKind): MediaDeviceInfo[] {
    return devices.filter((d) => d.deviceClass === DEVICE_CLASS[kind]);
}

function unique<T>(items: T[], key: (item: T) => string): T[] {
    const seen = new Set<string>();
    return items.filter((item) => {
        const k = key(item);
        if (seen.has(k)) return false;
        seen.add(k);
        return true;
    });
}

export function videoFormats(device: MediaDeviceInfo): VideoPublishOptions[] {
    const formats = device.capabilities.flatMap((c) =>
        c.kind === 'Video'
            ? c.framerates.map((framerate) => ({
                  codec: c.codec,
                  deviceId: device.devicePath,
                  width: c.width,
                  height: c.height,
                  framerate,
              }))
            : []
    );
    return unique(formats, formatKey).sort(
        (a, b) => b.width * b.height - a.width * a.height || b.framerate - a.framerate
    );
}

export function audioFormats(device: MediaDeviceInfo): AudioPublishOptions[] {
    const formats = device.capabilities.flatMap((c) => {
        if (c.kind !== 'Audio') return [];
        const [min, max] = c.framerates;
        const rates = AUDIO_RATES.filter((rate) => rate >= min && rate <= max);
        return (rates.length ? rates : [max]).map((framerate) => ({
            codec: c.codec,
            deviceId: device.devicePath,
            framerate,
            channels: c.channels,
        }));
    });
    return unique(formats, formatKey).sort(
        (a, b) => b.framerate - a.framerate || a.channels - b.channels
    );
}

export function screenFormats(device: MediaDeviceInfo): ScreenPublishOptions[] {
    const formats = device.capabilities.flatMap((c) =>
        c.kind === 'Screen'
            ? c.framerates.map((framerate) => ({
                  codec: c.codec,
                  screenIdOrName: device.devicePath,
                  width: c.width,
                  height: c.height,
                  framerate,
              }))
            : []
    );
    return unique(formats, formatKey).sort((a, b) => a.framerate - b.framerate);
}

/** 1280×720 at 30 fps if offered (MJPEG first), else the first format up to 1080p. */
export function defaultVideo(device: MediaDeviceInfo): VideoPublishOptions | null {
    const formats = videoFormats(device);
    const byCodec = (a: VideoPublishOptions, b: VideoPublishOptions) =>
        CODEC_PREFERENCE.indexOf(a.codec) - CODEC_PREFERENCE.indexOf(b.codec);
    const hd = formats
        .filter((f) => f.width === 1280 && f.height === 720 && f.framerate === 30)
        .sort(byCodec);
    return hd[0] ?? formats.find((f) => f.height <= 1080) ?? formats[0] ?? null;
}

/**
 * 48 kHz, mono for ordinary mics; every channel for multichannel interfaces, so a
 * channel can be picked.
 */
export function defaultAudio(device: MediaDeviceInfo): AudioPublishOptions | null {
    const formats = audioFormats(device);
    const maxChannels = Math.max(0, ...formats.map((f) => f.channels));
    const channels = maxChannels > 2 ? maxChannels : Math.min(...formats.map((f) => f.channels));
    const candidates = formats.filter((f) => f.channels === channels);
    return candidates.find((f) => f.framerate === 48000) ?? candidates[0] ?? null;
}

/** The display's own size, at 15 fps if offered. */
export function defaultScreen(device: MediaDeviceInfo): ScreenPublishOptions | null {
    const formats = screenFormats(device);
    return formats.find((f) => f.framerate === 15) ?? formats[0] ?? null;
}

type AnyOptions = VideoPublishOptions | AudioPublishOptions | ScreenPublishOptions;

export function deviceIdOf(options: AnyOptions): string {
    return 'screenIdOrName' in options ? options.screenIdOrName : options.deviceId;
}

/** Identifies a format of one device (ignores the selected channel). */
export function formatKey(options: AnyOptions): string {
    if ('screenIdOrName' in options)
        return `${options.width}x${options.height}@${options.framerate}`;
    if ('width' in options)
        return `${options.codec}|${options.width}x${options.height}@${options.framerate}`;
    return `${options.codec}|${options.framerate}|${options.channels}`;
}

const CODEC_LABEL: Record<string, string> = {
    'image/jpeg': 'MJPEG',
    'video/x-raw': 'raw',
    'video/x-h264': 'H.264',
};

export function channelsLabel(channels: number): string {
    if (channels === 1) return 'mono';
    if (channels === 2) return 'stereo';
    return `${channels} ch`;
}

/** e.g. `1280×720 · 30 fps · MJPEG`, `48 kHz · mono`, `1512×982 · 15 fps`. */
export function formatLabel(options: AnyOptions): string {
    if ('screenIdOrName' in options)
        return `${options.width}×${options.height} · ${options.framerate} fps`;
    if ('width' in options) {
        const codec = CODEC_LABEL[options.codec] ?? options.codec;
        return `${options.width}×${options.height} · ${options.framerate} fps · ${codec}`;
    }
    const rate = options.framerate / 1000;
    return `${rate} kHz · ${channelsLabel(options.channels)}`;
}

/**
 * The device's name, followed by a short id when another device has the same name
 * (two identical USB mics, for example).
 */
export function deviceLabel(device: MediaDeviceInfo, devices: MediaDeviceInfo[]): string {
    const twins = devices.filter((d) => d.displayName === device.displayName);
    if (twins.length < 2) return device.displayName;
    const path = device.devicePath;
    const suffix = path.includes(':') ? path.split(':').slice(-2).join(':') : path.slice(-8);
    return `${device.displayName} · ${suffix}`;
}

/** Channels of a device, from its largest audio capability. */
export function deviceChannels(device: MediaDeviceInfo): number {
    return Math.max(0, ...audioFormats(device).map((f) => f.channels));
}
