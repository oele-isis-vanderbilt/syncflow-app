import type { PageLoad } from './$types';
import { invoke } from '@tauri-apps/api/core';
import type {
    MediaDeviceInfo,
    PipelineCapabilities,
    RegistrationResponse,
} from '#lib/components/types.js';
import { goto } from '$app/navigation';

export const load: PageLoad = async () => {
    try {
        await invoke('get_streaming_config');
        goto('/stream');
    } catch {
        // No saved setup: configure one here.
    }

    // Devices are needed in local mode too, so they load whether or not this machine is
    // registered.
    const [registration, devices, capabilities] = await Promise.all([
        invoke<RegistrationResponse>('get_registration').catch(() => null),
        invoke<MediaDeviceInfo[]>('get_devices').catch(() => [] as MediaDeviceInfo[]),
        invoke<PipelineCapabilities>('get_pipeline_capabilities').catch(() => null),
    ]);
    return { registration, devices, capabilities };
};
