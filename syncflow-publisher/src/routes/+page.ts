import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import { invoke } from '@tauri-apps/api/core';
import type { RegistrationResponse, MediaDeviceInfo } from '$lib/components/types';
import { goto } from '$app/navigation';

export const load: PageLoad = async () => {
    try {
        const _ = await invoke('get_streaming_config');
        goto('/stream');
    } catch (error) {
        console.log('No streaming config found');
    }
    try {
        const registration: RegistrationResponse = await invoke('get_registration');
        const devices: MediaDeviceInfo[] = await invoke('get_devices');
        return {
            registration,
            devices,
        };
    } catch (error) {
        console.log('No registration found, redirecting to register page.', error);
        redirect(302, '/register');
    }
};
