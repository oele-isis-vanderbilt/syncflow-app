<script lang="ts">
    import SelectedDevices from '$lib/components/SelectedDevices.svelte';
    import { Accordion, AccordionItem, Button, Progressbar } from 'flowbite-svelte';
    import type { PageProps } from './$types';
    import { invoke } from '@tauri-apps/api/core';
    import { goto } from '$app/navigation';
    import { listen } from '@tauri-apps/api/event';
    import { devicesStore, initialize } from '$lib/store.svelte';
    import type {
        NewSessionMessage,
        PublicationNotification,
        PublicationNotificationFailure,
        PublicationNotificationStreamingSuccess,
    } from '$lib/components/types';

    let { data }: PageProps = $props();

    // Initialize store and restore state from backend config
    initialize(data.devices);

    // Add selected devices to store
    data.streamingConfigs.forEach((config) => {
        const enableStreaming = config.enableStreaming;
        devicesStore!.addDevice(config.publishOptions, enableStreaming);
    });

    // Restore avmix state from backend config
    const hasAvMixDevices = data.streamingConfigs.some((config) => config.avMixMode);
    if (hasAvMixDevices) {
        devicesStore!.setAvMixMode(true);

        // Restore avmix roles
        data.streamingConfigs.forEach((config) => {
            if (config.avMixMode) {
                const deviceId =
                    config.publishOptions.kind === 'Screen'
                        ? config.publishOptions.screenIdOrName
                        : config.publishOptions.deviceId;
                devicesStore!.setAvMixRole(deviceId, config.avMixMode as any);
            }
        });
    }

    let sessionMessages = $state<NewSessionMessage[]>([]);
    let publicationNotifications = $state<PublicationNotification[]>([]);
    let allSessions = $state<Array<[string, string, boolean]>>([]);
    let currentlyJoinedSession = $state<string | null>(null);

    // Load all sessions and currently joined session on page load
    Promise.all([
        invoke<Array<[string, string, boolean]>>('get_all_sessions'),
        invoke<string | null>('get_currently_joined_session')
    ]).then(([sessions, joinedSession]) => {
        allSessions = sessions;
        currentlyJoinedSession = joinedSession;
        // Convert to session messages format for unified display
        sessionMessages = sessions.map(([id, name, _active]) => ({
            sessionId: id,
            sessionName: name || id
        }));
    });

    // Listen for session updates instead of individual new-session events
    listen('session-update', () => {
        // Refresh all sessions and currently joined session when any session changes
        Promise.all([
            invoke<Array<[string, string, boolean]>>('get_all_sessions'),
            invoke<string | null>('get_currently_joined_session')
        ]).then(([sessions, joinedSession]) => {
            allSessions = sessions;
            currentlyJoinedSession = joinedSession;
            // Update session messages to include all current sessions
            const newSessionIds = new Set(sessions.map(([id, _, _active]) => id));
            const existingSessionIds = new Set(sessionMessages.map(msg => msg.sessionId));
            
            // Add any new sessions that aren't already in sessionMessages
            sessions.forEach(([id, name, _active]) => {
                if (!existingSessionIds.has(id)) {
                    sessionMessages.push({ sessionId: id, sessionName: name || id });
                }
            });
            
            sessionMessages = [...sessionMessages];
        }).catch(error => {
            console.error('Failed to update sessions:', error);
        });
    });

    listen<PublicationNotification>('publication-notification', (event) => {
        const notification = event.payload as PublicationNotification;
        publicationNotifications.push(notification);
        publicationNotifications = [...publicationNotifications];
        
        // Refresh all sessions when sessions end
        if (notification.kind === 'sessionEnded') {
            Promise.all([
                invoke<Array<[string, string, boolean]>>('get_all_sessions'),
                invoke<string | null>('get_currently_joined_session')
            ]).then(([sessions, joinedSession]) => {
                allSessions = sessions;
                currentlyJoinedSession = joinedSession;
            });
        }
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
        // Only mark sessions as ended if they're not available on the server anymore
        // This prevents locally exited sessions from being marked as ended
        const serverSessionIds = new Set(allSessions.map(([id, _, __]) => id));
        publicationNotifications
            .filter((notification) => notification.kind === 'sessionEnded')
            .forEach((notification) => {
                // Only mark as ended if it's not available on server
                if (!serverSessionIds.has(notification.sessionId)) {
                    endedSet.add(notification.sessionId);
                }
            });
        return endedSet;
    });

    async function exitSession(sessionId: string) {
        try {
            await invoke('exit_session', { sessionId });
        } catch (error) {
            console.error('Failed to exit session:', error);
        }
    }

    async function rejoinSession(sessionId: string, sessionName: string) {
        try {
            await invoke('rejoin_session', { sessionId, sessionName });
        } catch (error) {
            console.error('Failed to rejoin session:', error);
        }
    }
    
    function isSessionActive(sessionId: string): boolean {
        return allSessions.some(([id, _name, active]) => id === sessionId && active);
    }
    
    function isSessionAvailableOnServer(sessionId: string): boolean {
        return allSessions.some(([id, _name, _active]) => id === sessionId);
    }
    
    async function refreshSessions() {
        try {
            const [sessions, joinedSession] = await Promise.all([
                invoke<Array<[string, string, boolean]>>('get_all_sessions'),
                invoke<string | null>('get_currently_joined_session')
            ]);
            allSessions = sessions;
            currentlyJoinedSession = joinedSession;
            // Update session messages to include all current sessions
            const newSessionIds = new Set(sessions.map(([id, _, _active]) => id));
            const existingSessionIds = new Set(sessionMessages.map(msg => msg.sessionId));
            
            // Add any new sessions that aren't already in sessionMessages
            sessions.forEach(([id, name, _active]) => {
                if (!existingSessionIds.has(id)) {
                    sessionMessages.push({ sessionId: id, sessionName: name || id });
                }
            });
            
            sessionMessages = [...sessionMessages];
        } catch (error) {
            console.error('Failed to refresh sessions:', error);
        }
    }

    $inspect({
        successes,
        uploadProgress,
        allSessions,
    });
</script>

<main
    class="container mx-auto flex flex-col w-full justify-start p-4 gap-6 bg-gradient-to-br from-blue-50 via-white to-purple-100 min-h-screen"
>
    <SelectedDevices
        allDevices={data.devices}
        selectedDevicesFn={devicesStore?.getSelectedDevicesFn() || (() => [])}
        showDeleteButton={false}
        streamingConfigFn={devicesStore?.getStreamingConfigFn() || (() => ({}))}
        avMixMode={devicesStore?.getAvMixMode() || false}
        avMixRoles={devicesStore?.getAvMixRoles() || {}}
        onAvMixModeChange={devicesStore?.setAvMixMode || (() => {})}
        onAvMixRoleChange={devicesStore?.setAvMixRole || (() => {})}
        readonly={true}
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
        <div class="flex justify-between items-center mb-4">
            <div>
                <h2 class="text-xl font-semibold text-gray-800 mb-2">Session Messages</h2>
                <div class="flex items-center gap-4 text-sm text-gray-600">
                    <div class="flex items-center gap-1">
                        <div class="w-3 h-3 bg-green-500 animate-pulse rounded-full"></div>
                        <span>Currently joined</span>
                    </div>
                    <div class="flex items-center gap-1">
                        <div class="w-3 h-3 bg-blue-500 rounded-full"></div>
                        <span>Available to join</span>
                    </div>
                    <div class="flex items-center gap-1">
                        <div class="w-3 h-3 bg-red-500 rounded-full"></div>
                        <span>Ended</span>
                    </div>
                    <div class="flex items-center gap-1">
                        <div class="w-3 h-3 bg-gray-500 rounded-full"></div>
                        <span>Unavailable</span>
                    </div>
                </div>
            </div>
            <Button
                color="blue"
                size="sm"
                onclick={refreshSessions}
            >
                Refresh
            </Button>
        </div>
        <Accordion class="w-full">
            {#each sessionMessages as message, index (message.sessionId)}
                <AccordionItem open={index === sessionMessages.length - 1}>
                    {#snippet header()}
                        <div class="flex items-center justify-between w-full">
                            <h2 class="text-lg font-medium text-gray-900">
                                {message.sessionName}({message.sessionId})
                            </h2>
                            <div class="flex items-center gap-2">
                                <div class="flex-shrink-0">
                                    {#if endedSessions.has(message.sessionId)}
                                        <div class="w-5 h-5 bg-red-500 rounded-full" title="Session ended"></div>
                                    {:else if currentlyJoinedSession === message.sessionId}
                                        <div class="w-5 h-5 bg-green-500 animate-pulse rounded-full" title="Currently joined"></div>
                                    {:else if isSessionAvailableOnServer(message.sessionId)}
                                        <div class="w-5 h-5 bg-blue-500 rounded-full" title="Available to join"></div>
                                    {:else}
                                        <div class="w-5 h-5 bg-gray-500 rounded-full" title="Unavailable"></div>
                                    {/if}
                                </div>
                                <div class="flex gap-2 mr-2">
                                    {#if currentlyJoinedSession === message.sessionId}
                                        <Button
                                            color="red"
                                            size="xs"
                                            onclick={(e: Event) => {
                                                e.stopPropagation();
                                                exitSession(message.sessionId);
                                            }}
                                        >
                                            Exit
                                        </Button>
                                    {:else if isSessionAvailableOnServer(message.sessionId) && !endedSessions.has(message.sessionId)}
                                        <Button
                                            color="green"
                                            size="xs"
                                            onclick={(e: Event) => {
                                                e.stopPropagation();
                                                rejoinSession(message.sessionId, message.sessionName);
                                            }}
                                        >
                                            Join
                                        </Button>
                                    {:else if !endedSessions.has(message.sessionId)}
                                        <Button
                                            color="gray"
                                            size="xs"
                                            disabled
                                        >
                                            Unavailable
                                        </Button>
                                    {:else}
                                        <span class="text-xs text-gray-500">Ended</span>
                                    {/if}
                                </div>
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
