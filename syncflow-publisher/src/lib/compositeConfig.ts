// The recording setup being edited (a draft that may have empty slots), its checks, and
// the conversion to the `CompositeConfig` the backend saves.
import type {
    AudioPublishOptions,
    CompositeConfig,
    MediaDeviceInfo,
    PipelineCapabilities,
    RecordingMode,
    ScreenPublishOptions,
    VideoPublishOptions,
} from '#lib/components/types.js';
import { channelsLabel, deviceIdOf, deviceLabel, type DeviceKind } from '#lib/publishOptions.js';

/** The voice-activity threshold of a new setup (matches `DEFAULT_VAD_THRESHOLD` in Rust). */
export const DEFAULT_VAD_THRESHOLD = 0.2;

export interface DraftOutput<T> {
    options: T | null;
    stream: boolean;
}

export interface Draft {
    avMix: {
        include: boolean;
        camera: VideoPublishOptions | null;
        mic1: AudioPublishOptions | null;
        /** Whether a mic R row is shown. */
        useMic2: boolean;
        mic2: AudioPublishOptions | null;
        stream: boolean;
    };
    screen: { include: boolean } & DraftOutput<ScreenPublishOptions>;
    cameras: DraftOutput<VideoPublishOptions>[];
    mics: DraftOutput<AudioPublishOptions>[];
    /** Streamed audio is muted while the chance of speech is below this (0 to 1). */
    vadThreshold: number;
}

/** Where a device sits in the setup. */
export type SlotId =
    | 'avMix.camera'
    | 'avMix.mic1'
    | 'avMix.mic2'
    | 'screen'
    | `camera.${number}`
    | `mic.${number}`;

export interface Slot {
    id: SlotId;
    /** e.g. `Camera`, `Mic L`, `camera-2`, `mic-3` */
    label: string;
    kind: DeviceKind;
    options: VideoPublishOptions | AudioPublishOptions | ScreenPublishOptions | null;
    stream: boolean;
}

export interface Issue {
    /** The row the issue belongs to; `null` for the setup as a whole. */
    slot: SlotId | null;
    message: string;
}

export function emptyDraft(): Draft {
    return {
        avMix: {
            include: true,
            camera: null,
            mic1: null,
            useMic2: false,
            mic2: null,
            stream: true,
        },
        screen: { include: false, options: null, stream: false },
        cameras: [],
        mics: [],
        vadThreshold: DEFAULT_VAD_THRESHOLD,
    };
}

export function draftFromConfig(config: CompositeConfig): Draft {
    const mix = config.avMix;
    return {
        avMix: mix
            ? {
                  include: true,
                  camera: mix.camera,
                  mic1: mix.mic1,
                  useMic2: mix.mic2 !== null,
                  mic2: mix.mic2,
                  stream: mix.stream,
              }
            : { ...emptyDraft().avMix, include: false },
        screen: config.screen
            ? { include: true, ...config.screen }
            : { include: false, options: null, stream: false },
        cameras: config.cameras.map((c) => ({ ...c })),
        mics: config.mics.map((m) => ({ ...m })),
        vadThreshold: config.vadThreshold ?? DEFAULT_VAD_THRESHOLD,
    };
}

/** How many mics the AV mix has: extra mics are numbered after them, as on disk. */
function mixMicCount(draft: Draft): number {
    if (!draft.avMix.include) return 0;
    return draft.avMix.useMic2 ? 2 : 1;
}

/** Output label of an additional camera, matching the file name prefix (`camera-2`). */
export function cameraLabel(draft: Draft, index: number): string {
    return `camera-${(draft.avMix.include ? 1 : 0) + index + 1}`;
}

/** Output label of an additional mic, matching the file name prefix (`mic-3`). */
export function micLabel(draft: Draft, index: number): string {
    return `mic-${mixMicCount(draft) + index + 1}`;
}

/** Every row of the setup that takes a device, in the order the backend numbers them. */
export function slots(draft: Draft): Slot[] {
    const result: Slot[] = [];
    if (draft.avMix.include) {
        const stream = draft.avMix.stream;
        result.push({
            id: 'avMix.camera',
            label: 'Camera',
            kind: 'camera',
            options: draft.avMix.camera,
            stream,
        });
        result.push({
            id: 'avMix.mic1',
            label: 'Mic L',
            kind: 'mic',
            options: draft.avMix.mic1,
            stream,
        });
        if (draft.avMix.useMic2) {
            result.push({
                id: 'avMix.mic2',
                label: 'Mic R',
                kind: 'mic',
                options: draft.avMix.mic2,
                stream,
            });
        }
    }
    if (draft.screen.include) {
        result.push({
            id: 'screen',
            label: 'Screen',
            kind: 'screen',
            options: draft.screen.options,
            stream: draft.screen.stream,
        });
    }
    draft.cameras.forEach((c, i) =>
        result.push({
            id: `camera.${i}`,
            label: cameraLabel(draft, i),
            kind: 'camera',
            options: c.options,
            stream: c.stream,
        })
    );
    draft.mics.forEach((m, i) =>
        result.push({
            id: `mic.${i}`,
            label: micLabel(draft, i),
            kind: 'mic',
            options: m.options,
            stream: m.stream,
        })
    );
    return result;
}

function channelOf(slot: Slot): number | null {
    return slot.options && 'selectedChannel' in slot.options
        ? (slot.options.selectedChannel ?? null)
        : null;
}

/** Whether two slots read the same device in a way that can't be shared. */
function clash(a: Slot, b: Slot): boolean {
    if (!a.options || !b.options || a.kind !== b.kind) return false;
    if (deviceIdOf(a.options) !== deviceIdOf(b.options)) return false;
    const [ca, cb] = [channelOf(a), channelOf(b)];
    // Only distinct, explicitly picked channels of one interface can be shared.
    return ca === null || cb === null || ca === cb;
}

/** Another slot already using `deviceId` so that `slot` can't use it, if any. */
export function deviceTakenBy(draft: Draft, slotId: SlotId, deviceId: string): Slot | null {
    const all = slots(draft);
    const self = all.find((s) => s.id === slotId);
    if (!self) return null;
    return (
        all.find(
            (other) =>
                other.id !== slotId &&
                other.kind === self.kind &&
                other.options !== null &&
                deviceIdOf(other.options) === deviceId &&
                (channelOf(other) === null || channelOf(self) === null)
        ) ?? null
    );
}

/**
 * The format an earlier slot already opened `deviceId` with: every channel of a shared
 * interface is read at one format, set by the first slot using it.
 */
export function sharedAudioFormat(
    draft: Draft,
    slotId: SlotId,
    deviceId: string
): { by: string; options: AudioPublishOptions } | null {
    for (const other of slots(draft)) {
        if (other.id === slotId) break;
        if (other.kind === 'mic' && other.options && deviceIdOf(other.options) === deviceId) {
            return { by: other.label, options: other.options as AudioPublishOptions };
        }
    }
    return null;
}

/** Channels of `deviceId` already picked by other mic slots. */
export function channelsInUse(draft: Draft, slotId: SlotId, deviceId: string): Map<number, string> {
    const used = new Map<number, string>();
    for (const other of slots(draft)) {
        const channel = channelOf(other);
        if (
            other.id !== slotId &&
            other.options &&
            channel !== null &&
            deviceIdOf(other.options) === deviceId
        ) {
            used.set(channel, other.label);
        }
    }
    return used;
}

function deviceName(deviceId: string, devices: MediaDeviceInfo[]): string {
    const device = devices.find((d) => d.devicePath === deviceId);
    return device ? deviceLabel(device, devices) : deviceId;
}

const MISSING: Record<DeviceKind, string> = {
    camera: 'Choose a camera.',
    mic: 'Choose a microphone.',
    screen: 'Choose a display.',
};

/** Problems that block saving the setup. */
export function validate(draft: Draft, devices: MediaDeviceInfo[]): Issue[] {
    const issues: Issue[] = [];
    const all = slots(draft);
    if (all.length === 0) {
        issues.push({ slot: null, message: 'Add at least one device.' });
    }
    if (!(draft.vadThreshold >= 0 && draft.vadThreshold <= 1)) {
        issues.push({
            slot: null,
            message: 'The voice activity threshold must be between 0 and 1.',
        });
    }
    all.forEach((slot, index) => {
        if (!slot.options) {
            issues.push({ slot: slot.id, message: MISSING[slot.kind] });
            return;
        }
        const id = deviceIdOf(slot.options);
        const name = deviceName(id, devices);
        if (!devices.some((d) => d.devicePath === id)) {
            issues.push({ slot: slot.id, message: `${name} is no longer connected.` });
            return;
        }
        const earlier = all.slice(0, index);
        const taken = earlier.find((other) => clash(other, slot));
        if (taken) {
            const channel = channelOf(slot);
            const what =
                channel !== null && channelOf(taken) === channel
                    ? `${name} channel ${channel}`
                    : name;
            issues.push({ slot: slot.id, message: `${what} is already used as ${taken.label}.` });
            return;
        }
        if (slot.kind === 'mic') {
            const options = slot.options as AudioPublishOptions;
            const shared = earlier.find(
                (other) => other.kind === 'mic' && other.options && deviceIdOf(other.options) === id
            );
            const sharedOptions = shared?.options as AudioPublishOptions | undefined;
            if (
                sharedOptions &&
                (sharedOptions.framerate !== options.framerate ||
                    sharedOptions.channels !== options.channels ||
                    sharedOptions.codec !== options.codec)
            ) {
                issues.push({
                    slot: slot.id,
                    message: `${name} is already set to ${sharedOptions.framerate / 1000} kHz · ${channelsLabel(sharedOptions.channels)} by ${shared!.label}.`,
                });
            }
        }
    });
    return issues;
}

/** Things worth knowing that don't block saving. */
export function notices(
    draft: Draft,
    mode: RecordingMode,
    capabilities: PipelineCapabilities | null
): string[] {
    const result: string[] = [];
    const all = slots(draft);
    const streamed = all.filter((s) => s.stream);
    if (mode === 'sessionMode' && all.length > 0 && streamed.length === 0) {
        result.push('Nothing will be streamed; this machine records only.');
    }
    if (capabilities && !capabilities.noiseSuppression && all.some((s) => s.kind === 'mic')) {
        result.push("Audio won't be noise suppressed: audiornnoise is not installed.");
    }
    return result;
}

/** How many files the recording writes and how many tracks it streams. */
export function counts(draft: Draft): { files: number; streams: number } {
    let files = 0;
    let streams = 0;
    if (draft.avMix.include) {
        files += 1 + mixMicCount(draft); // av-mix.mp4 plus one .m4a per mic
        if (draft.avMix.stream) streams += 2; // video and mono audio
    }
    const singles = [
        ...(draft.screen.include ? [draft.screen] : []),
        ...draft.cameras,
        ...draft.mics,
    ];
    files += singles.length;
    streams += singles.filter((o) => o.stream).length;
    return { files, streams };
}

/** The saved config. Call only when `validate` found no issues. */
export function toConfig(draft: Draft, recordingMode: RecordingMode): CompositeConfig {
    const mix = draft.avMix;
    const output = <T>(o: DraftOutput<T>) => ({ options: o.options!, stream: o.stream });
    return {
        avMix: mix.include
            ? {
                  camera: mix.camera!,
                  mic1: mix.mic1!,
                  mic2: mix.useMic2 ? mix.mic2 : null,
                  stream: mix.stream,
              }
            : null,
        screen: draft.screen.include ? output(draft.screen) : null,
        cameras: draft.cameras.map(output),
        mics: draft.mics.map(output),
        recordingMode,
        vadThreshold: draft.vadThreshold,
    };
}
