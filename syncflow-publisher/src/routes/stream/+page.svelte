<script lang="ts">
    import SelectedDevices from '$lib/components/SelectedDevices.svelte';
    import { Accordion, AccordionItem, Button, Progressbar } from 'flowbite-svelte';
    import type { PageProps } from './$types';
    import { invoke } from '@tauri-apps/api/core';
    import { goto } from '$app/navigation';
    import { listen } from '@tauri-apps/api/event';
    import type {
        NewSessionMessage,
        PublicationNotification,
        PublicationNotificationFailure,
        PublicationNotificationStreamingSuccess,
    } from '$lib/components/types';

    let { data }: PageProps = $props();

    let sessionMessages = $state<NewSessionMessage[]>([]);
    let publicationNotifications = $state<PublicationNotification[]>([]);

    listen<NewSessionMessage>('new-session', (event) => {
        const message = event.payload as NewSessionMessage;
        sessionMessages.push(message);
        sessionMessages = [...sessionMessages];
    });

    listen<PublicationNotification>('publication-notification', (event) => {
        const notification = event.payload as PublicationNotification;
        publicationNotifications.push(notification);
        publicationNotifications = [...publicationNotifications];
    });

    let failures: Record<string, PublicationNotificationFailure[]> = $derived.by(() => {
        return Object.fromEntries(
            publicationNotifications
                .filter((notification) => notification.kind === 'failure')
                .map((notification) => [
                    notification.sessionId,
                    publicationNotifications.filter(
                        (n) => n.kind === 'failure' && n.sessionId === notification.sessionId
                    ) as PublicationNotificationFailure[],
                ])
        );
    });

    let successes: Record<string, PublicationNotificationStreamingSuccess[]> = $derived.by(() => {
        return Object.fromEntries(
            publicationNotifications
                .filter((notification) => notification.kind === 'streamingSuccess')
                .map((notification) => [
                    notification.sessionId,
                    publicationNotifications.filter(
                        (n) =>
                            n.kind === 'streamingSuccess' && n.sessionId === notification.sessionId
                    ) as PublicationNotificationStreamingSuccess[],
                ])
        );
    });

    let uploadProgress: Record<string, number> = $derived.by(() => {
        const progressMap: Record<string, number> = {};
        publicationNotifications
            .filter((notification) => notification.kind === 'uploadProgress')
            .forEach((notification) => {
                progressMap[notification.sessionId] = notification.progress;
            });
        return progressMap;
    });

    let endedSessions: Set<string> = $derived.by(() => {
        const endedSet: Set<string> = new Set();
        publicationNotifications
            .filter((notification) => notification.kind === 'sessionEnded')
            .forEach((notification) => {
                endedSet.add(notification.sessionId);
            });
        return endedSet;
    });

    $inspect({
        successes,
        uploadProgress,
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
        class="mt-4 w-64 self-center"
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
                    <span class="text-sm font-medium text-gray-600">Local Recorded Devices:</span>
                    <p class="text-gray-800">
                        {data.streamingConfigs.length} device(s)
                    </p>
                </div>

                <div>
                    <span class="text-sm font-medium text-gray-600">Streaming Devices:</span>
                    <p class="text-gray-800">
                        {data.streamingConfigs.filter((data) => data.enableStreaming).length} device(s)
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
                        {#if endedSessions.has(message.sessionId)}
                            <div class="w-2 h-2 bg-red-500 rounded-full mt-2"></div>
                        {:else}
                            <div class="w-2 h-2 bg-green-500 animate-pulse rounded-full mt-2"></div>
                        {/if}
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
    <div class="bg-white rounded-lg shadow-md p-6 border border-gray-200">
        <h2 class="text-xl font-semibold text-gray-800 mb-4">Session Messages</h2>
        <Accordion class="w-full">
            {#each sessionMessages as message, index (message.sessionId)}
                <AccordionItem open={index === sessionMessages.length - 1}>
                    {#snippet header()}
                        <div class="flex items-center justify-between w-full">
                            <h2 class="text-lg font-medium text-gray-900">
                                {message.sessionName}({message.sessionId})
                            </h2>
                            <div class="flex-shrink-0">
                                {#if endedSessions.has(message.sessionId)}
                                    <div class="w-5 h-5 bg-red-500 rounded-full mt-2 mr-10"></div>
                                {:else}
                                    <div
                                        class="w-5 h-5 bg-green-500 animate-pulse rounded-full mt-2 mr-10"
                                    ></div>
                                {/if}
                            </div>
                        </div>
                    {/snippet}
                    <div class="space-y-4 mt-4">
                        {#if successes[message.sessionId]}
                            <div class="bg-green-50 p-4 rounded-lg border border-green-200">
                                <h3 class="text-md font-semibold text-green-800 mb-2">Messeges</h3>
                                {#each successes[message.sessionId] as success}
                                    <pre
                                        class="text-sm text-green-900 bg-green-100 p-2 rounded">{JSON.stringify(
                                            success,
                                            null,
                                            2
                                        )}</pre>
                                {/each}
                            </div>
                        {/if}
                        {#if failures[message.sessionId]}
                            <div class="bg-red-50 p-4 rounded-lg border border-red-200">
                                <h3 class="text-md font-semibold text-red-800 mb-2">Failures</h3>
                                {#each failures[message.sessionId] as failure}
                                    <pre
                                        class="text-sm text-red-900 bg-red-100 p-2 rounded">{JSON.stringify(
                                            failure,
                                            null,
                                            2
                                        )}</pre>
                                {/each}
                            </div>
                        {/if}
                        {#if uploadProgress[message.sessionId] !== undefined}
                            <div class="space-y-2 mb-10">
                                <h3 class="text-md font-semibold text-gray-800">Upload Progress</h3>
                                <Progressbar
                                    progress={uploadProgress[message.sessionId]}
                                    labelInside
                                    class="h-6"
                                    color="green"
                                    size="h-6"
                                >
                                    <span class="text-sm font-medium text-gray-700">
                                        {uploadProgress[message.sessionId]}%
                                    </span>
                                </Progressbar>
                            </div>
                        {/if}
                    </div>
                </AccordionItem>
            {/each}
        </Accordion>
    </div>
</main>
