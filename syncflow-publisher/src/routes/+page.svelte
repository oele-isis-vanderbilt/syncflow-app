<script lang="ts">
    import { goto } from '$app/navigation';
    import { error } from '@sveltejs/kit';
    import { invoke } from '@tauri-apps/api/core';
    import { Button } from 'flowbite-svelte';
    import type { PageProps } from './$types';
    import RegistrationDetails from '$lib/components/RegistrationDetails.svelte';
    import Devices from '$lib/components/Devices.svelte';
    import { devicesStore } from '$lib/store.svelte';
    import SelectedDevices from '$lib/components/SelectedDevices.svelte';
    import type { DeviceRecordingAndStreamingConfig } from '$lib/components/types';

    let { data }: PageProps = $props();

    let registrationDetails = $derived.by(() => data.registration);
    let mediaDevices = $derived.by(() => data.devices || []);

    let {
        getSelectedDevicesFn,
        addDevice,
        removeDevice,
        getRemainingDevicesFn,
        getStreamingConfigFn,
        refreshAvailableDevices,
    } = devicesStore!;

    let availableDevicesToSelect = $derived.by(() => {
        const remainingDevicesFn = getRemainingDevicesFn();
        return remainingDevicesFn();
    });

    async function deregister() {
        try {
            await invoke('delete_registration');
            goto('/register');
        } catch (err) {
            error(500, { message: `Deregistration failed: ${JSON.stringify(err)}` });
        }
    }

    let errorMessage = $state<string | null>(null);
</script>

<main
    class="container mx-auto flex flex-col w-full justify-start p-4 gap-6 bg-gradient-to-br from-blue-50 via-white to-purple-100 min-h-screen"
>
    <div class="flex justify-between items-center bg-white rounded-xl shadow-lg px-6 py-4 mb-2">
        <h1 class="text-2xl font-extrabold text-purple-700 flex-1">
            Welcome to <span class="text-blue-600">SyncFlow Publisher</span>! {registrationDetails?.deviceName &&
                `(${registrationDetails.deviceName})`}
        </h1>
        <Button
            color="red"
            class="ml-4 shadow hover:scale-105 transition-transform"
            onclick={deregister}>Delete Registration</Button
        >
    </div>
    {#if registrationDetails}
        <RegistrationDetails {registrationDetails} />
    {:else}
        <div class="flex flex-col items-center justify-center mt-12">
            <svg
                class="w-16 h-16 text-gray-300 mb-4"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                viewBox="0 0 24 24"
                ><path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M12 8v4l3 3m6 0a9 9 0 11-18 0 9 9 0 0118 0z"
                /></svg
            >
            <p class="text-lg text-gray-500">No registration details found.</p>
        </div>
    {/if}
    <div class="flex items-center justify-between flex-col md:flex-row w-full h-full">
        <div class="p-5 flex-1">
            <Devices
                devices={availableDevicesToSelect}
                onAddDevice={addDevice}
                onRemoveDevice={removeDevice}
                onRefresh={refreshAvailableDevices}
            />
        </div>

        <div class="p-5 flex-1 h-full">
            <SelectedDevices
                allDevices={mediaDevices}
                selectedDevicesFn={getSelectedDevicesFn()}
                onRemoveDevice={removeDevice}
                streamingConfigFn={getStreamingConfigFn()}
            />
        </div>
    </div>

    <div>
        {#if errorMessage}
            <div class="bg-red-100 text-red-700 p-4 rounded-lg mb-4">
                <p class="font-semibold">Error:</p>
                <p>{errorMessage}</p>
            </div>
        {/if}
        <Button
            class="w-full mb-20"
            color="blue"
            disabled={!getSelectedDevicesFn()().length}
            onclick={async () => {
                const selectedDevicesFn = getSelectedDevicesFn();
                const streamingConfigFn = getStreamingConfigFn();
                const selectedDevices = selectedDevicesFn();
                const streamingConfigs = streamingConfigFn();
                const recordingAndStreamingConfig: DeviceRecordingAndStreamingConfig[] =
                    selectedDevices.map((option) => {
                        return {
                            enableStreaming:
                                streamingConfigs[
                                    option.kind === 'Screen'
                                        ? option.screenIdOrName
                                        : option.deviceId
                                ],
                            publishOptions: option,
                        };
                    });

                try {
                    await invoke('set_streaming_config', {
                        configs: recordingAndStreamingConfig,
                    });
                    errorMessage = null;
                    goto('/stream');
                } catch (err) {
                    errorMessage = `Failed to set streaming config: ${JSON.stringify(err)}`;
                }
            }}
        >
            {getSelectedDevicesFn()().length > 0
                ? 'Start Publishing'
                : 'Please select at least one device to publish'}
        </Button>
    </div>
</main>
