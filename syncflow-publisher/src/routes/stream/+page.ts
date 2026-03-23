import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import { invoke } from '@tauri-apps/api/core';
import type { DeviceRecordingAndStreamingConfig, MediaDeviceInfo } from '$lib/components/types';

export const load: PageLoad = async () => {
    try {
        const existingConfigs: DeviceRecordingAndStreamingConfig[] =
            await invoke('get_streaming_config');
        const devices: MediaDeviceInfo[] = await invoke('get_devices');

        // Determine the recording mode from the configs
        const recordingMode =
            existingConfigs.length > 0 ? existingConfigs[0].recordingMode : 'sessionMode';

        return {
            streamingConfigs: existingConfigs,
            devices: devices,
            recordingMode: recordingMode,
        };
    } catch (error) {
        console.log('No streaming config found, redirecting to register page.', error);
        throw redirect(302, '/');
    }
};
