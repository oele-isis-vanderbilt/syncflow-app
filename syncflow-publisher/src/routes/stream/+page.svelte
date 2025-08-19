<script lang="ts">
    import SelectedDevices from '$lib/components/SelectedDevices.svelte';
    import { Button } from 'flowbite-svelte';
    import type { PageProps } from './$types';
    import { invoke } from '@tauri-apps/api/core';
    import { goto } from '$app/navigation';
    import { listen } from '@tauri-apps/api/event';
    import type { NewSessionMessage } from '$lib/components/types';

    let { data }: PageProps = $props();

    let sessionMessages = $state<NewSessionMessage[]>([]);

    listen<NewSessionMessage>('new-session', (event) => {
        const message = event.payload as NewSessionMessage;
        console.log('New session message received:', message);
        sessionMessages.push(message);
        sessionMessages = [...sessionMessages];
    });
</script>

<main
    class="container mx-auto flex flex-col w-full justify-start p-4 gap-6 bg-gradient-to-br from-blue-50 via-white to-purple-100 min-h-screen"
>
    <SelectedDevices
        allDevices={data.devices}
        selectedDevicesFn={() => data.streamingConfigs.map((config) => config.publishOptions)}
        showDeleteButton={false}
        streamingConfigFn={() =>
            Object.fromEntries(
                data.streamingConfigs.map((config) => [
                    config.publishOptions.kind === 'Screen'
                        ? config.publishOptions.screenIdOrName
                        : config.publishOptions.deviceId,
                    config.enableStreaming,
                ])
            )}
    />
    <Button
        color="red"
        class="mt-4"
        onclick={async () => {
            await invoke('delete_streaming_config');
            goto('/');
        }}
    >
        Reconfigure Devices
    </Button>
    <div class="bg-white rounded-lg shadow-md p-6 border border-gray-200">
        <h2 class="text-xl font-semibold text-gray-800 mb-4">Stream Status</h2>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="space-y-3">
                <div>
                    <span class="text-sm font-medium text-gray-600">Session Name:</span>
                    <p class="text-gray-800">{data.sessionName || 'Not Set'}</p>
                </div>

                <div>
                    <span class="text-sm font-medium text-gray-600">Local Recorded Devices:</span>
                    <p class="text-gray-800">
                        {data.streamingConfigs.filter((config) => config.enableStreaming).length} device(s)
                    </p>
                </div>

                <div>
                    <span class="text-sm font-medium text-gray-600">Streaming Devices:</span>
                    <p class="text-gray-800">
                        {data.streamingConfigs.length} device(s)
                    </p>
                </div>
            </div>

            <div class="flex items-center justify-center">
                <div class="text-center">
                    <div
                        class="inline-flex items-center px-3 py-1 rounded-full text-sm font-medium bg-green-100 text-green-800"
                    >
                        <div class="w-2 h-2 bg-green-500 rounded-full mr-2 animate-pulse"></div>
                        Listening
                    </div>
                </div>
            </div>
        </div>
    </div>
    <div class="bg-white rounded-lg shadow-md p-6 border border-gray-200">
        <h2 class="text-xl font-semibold text-gray-800 mb-4">Session Messages</h2>

        <div class="space-y-2 max-h-64 overflow-y-auto">
            {#each sessionMessages as message}
                <div class="flex items-start space-x-3 p-3 bg-gray-50 rounded-lg border">
                    <div class="flex-shrink-0">
                        <div class="w-2 h-2 bg-blue-500 rounded-full mt-2"></div>
                    </div>
                    <div class="flex-1 min-w-0">
                        <div class="text-sm font-medium text-gray-900">
                            {message.sessionName || 'New Session'}
                        </div>
                        <div class="text-sm text-gray-600">
                            {message.sessionId || 'Session started'}
                        </div>
                        <div class="text-xs text-gray-400 mt-1">
                            {new Date().toLocaleTimeString()}
                        </div>
                    </div>
                </div>
            {:else}
                <div class="text-center text-gray-500 py-4">No session messages yet</div>
            {/each}
        </div>
    </div>
</main>
