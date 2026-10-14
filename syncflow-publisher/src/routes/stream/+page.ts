import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import { invoke } from '@tauri-apps/api/core';
import type { CompositeConfig, MediaDeviceInfo } from '#lib/components/types.js';

export const load: PageLoad = async () => {
    try {
        const config: CompositeConfig = await invoke('get_streaming_config');
        const devices: MediaDeviceInfo[] = await invoke('get_devices');
        return { config, devices, recordingMode: config.recordingMode };
    } catch (error) {
        console.log('No recording setup found, returning to setup.', error);
        throw redirect(302, '/');
    }
};
