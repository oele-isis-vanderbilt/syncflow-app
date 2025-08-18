import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import { invoke } from '@tauri-apps/api/core';
import type { RegistrationResponse, MediaDeviceInfo } from '$lib/components/types';

export const load: PageLoad = async () => {
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
