// Shared state of the setup page: the devices this machine reports and the recording
// setup being edited.
import { invoke } from '@tauri-apps/api/core';
import type { MediaDeviceInfo } from '#lib/components/types.js';
import { emptyDraft, type Draft } from '#lib/compositeConfig.js';

export const setup = $state<{ devices: MediaDeviceInfo[]; draft: Draft }>({
    devices: [],
    draft: emptyDraft(),
});

/** Reloads the device list. Slots keep their devices; missing ones are flagged by `validate`. */
export async function refreshDevices(): Promise<void> {
    setup.devices = await invoke<MediaDeviceInfo[]>('get_devices');
}
