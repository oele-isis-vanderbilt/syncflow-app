import type { MediaDeviceInfo } from '#lib/components/types.js';
import type { ClientInit } from '@sveltejs/kit/hooks';
import { invoke } from '@tauri-apps/api/core';

export const init: ClientInit = async () => {
    const { initialize } = await import('#lib/store.svelte.js');
    console.log('Initializing selectedDeviceStore');
    const devices: MediaDeviceInfo[] = await invoke('get_devices');
    initialize(devices);
};
