<script lang="ts">
    import CompositeSummary from '#lib/components/CompositeSummary.svelte';
    import SessionStatus, { type Status } from '#lib/components/SessionStatus.svelte';
    import ConfirmModal from '#lib/components/ui/ConfirmModal.svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import {
        Accordion,
        AccordionItem,
        Alert,
        Button,
        Heading,
        P,
        Progressbar,
        Spinner,
    } from 'flowbite-svelte';
    import {
        CogOutline,
        ExclamationCircleOutline,
        ListOutline,
        PlayOutline,
        RefreshOutline,
        StopOutline,
        VideoCameraOutline,
    } from 'flowbite-svelte-icons';
    import type { PageProps } from './$types';
    import { invoke } from '@tauri-apps/api/core';
    import { goto } from '$app/navigation';
    import { listen } from '@tauri-apps/api/event';
    import type {
        NewSessionMessage,
        PublicationNotification,
        PublicationNotificationStreamingSuccess,
    } from '#lib/components/types.js';
    import { counts, draftFromConfig } from '#lib/compositeConfig.js';
    import { describeProblem, type Problem } from '#lib/notifications.js';

    let { data }: PageProps = $props();

    // svelte-ignore state_referenced_locally
    const recordingMode = data.recordingMode;
    let setupCounts = $derived(counts(draftFromConfig(data.config)));

    let sessionMessages = $state<NewSessionMessage[]>([]);
    let publicationNotifications = $state<PublicationNotification[]>([]);
    let allSessions = $state<Array<[string, string, boolean]>>([]);
    let currentlyJoinedSession = $state<string | null>(null);

    // Local recording mode state
    let isLocalRecording = $state<boolean>(false);
    let activeLocalSessionId = $state<string | null>(null);

    // Generate automatic session name with local time
    function generateLocalSessionName(): string {
        const now = new Date();
        const year = now.getFullYear();
        const month = String(now.getMonth() + 1).padStart(2, '0');
        const day = String(now.getDate()).padStart(2, '0');
        const hours = String(now.getHours()).padStart(2, '0');
        const minutes = String(now.getMinutes()).padStart(2, '0');
        const seconds = String(now.getSeconds()).padStart(2, '0');
        return `recording-${year}-${month}-${day}-${hours}-${minutes}-${seconds}`;
    }

    // Load all sessions and currently joined session on page load
    Promise.all([
        invoke<Array<[string, string, boolean]>>('get_all_sessions'),
        invoke<string | null>('get_currently_joined_session'),
    ]).then(([sessions, joinedSession]) => {
        allSessions = sessions;
        currentlyJoinedSession = joinedSession;
        // Convert to session messages format for unified display
        sessionMessages = sessions.map(([id, name, _active]) => ({
            sessionId: id,
            sessionName: name || id,
        }));
    });

    // Listen for session updates instead of individual new-session events
    listen('session-update', () => {
        // Refresh all sessions and currently joined session when any session changes
        Promise.all([
            invoke<Array<[string, string, boolean]>>('get_all_sessions'),
            invoke<string | null>('get_currently_joined_session'),
        ])
            .then(([sessions, joinedSession]) => {
                allSessions = sessions;
                currentlyJoinedSession = joinedSession;
                // Update session messages to include all current sessions
                const existingSessionIds = new Set(sessionMessages.map((msg) => msg.sessionId));

                // Add any new sessions that aren't already in sessionMessages
                sessions.forEach(([id, name, _active]) => {
                    if (!existingSessionIds.has(id)) {
                        sessionMessages.push({ sessionId: id, sessionName: name || id });
                    }
                });

                sessionMessages = [...sessionMessages];
            })
            .catch((error) => {
                console.error('Failed to update sessions:', error);
            });
    });

    listen<PublicationNotification>('publication-notification', (event) => {
        const notification = event.payload as PublicationNotification;
        publicationNotifications.push(notification);
        publicationNotifications = [...publicationNotifications];

        // Add local recording sessions to sessionMessages when they start
        if (notification.kind === 'streamingSuccess') {
            const existingSession = sessionMessages.find(
                (msg) => msg.sessionId === notification.sessionId
            );
            if (!existingSession) {
                sessionMessages.push({
                    sessionId: notification.sessionId,
                    sessionName: notification.sessionName,
                });
                sessionMessages = [...sessionMessages];
            }

            // Track active local session for recording indicator
            if (notification.sessionId.startsWith('local-')) {
                activeLocalSessionId = notification.sessionId;
                isLocalRecording = true;
            }
        }

        // Handle session ended notifications
        if (notification.kind === 'sessionEnded') {
            // Clear local recording state if it's the active local session
            if (notification.sessionId === activeLocalSessionId) {
                isLocalRecording = false;
                activeLocalSessionId = null;
            }

            // Refresh server sessions
            Promise.all([
                invoke<Array<[string, string, boolean]>>('get_all_sessions'),
                invoke<string | null>('get_currently_joined_session'),
            ]).then(([sessions, joinedSession]) => {
                allSessions = sessions;
                currentlyJoinedSession = joinedSession;
            });
        }
    });

    let problems: Record<string, Problem[]> = $derived.by(() => {
        const bySession: Record<string, Problem[]> = {};
        for (const notification of publicationNotifications) {
            const problem = describeProblem(notification);
            if (problem) (bySession[notification.sessionId] ??= []).push(problem);
        }
        return bySession;
    });

    // The recording in progress, or the most recent one.
    let currentSessionId = $derived(
        currentlyJoinedSession ??
            activeLocalSessionId ??
            publicationNotifications.findLast((n) => n.kind === 'streamingSuccess')?.sessionId ??
            null
    );
    let currentNotifications = $derived(
        publicationNotifications.filter((n) => n.sessionId === currentSessionId)
    );
    let currentProblems = $derived(currentSessionId ? (problems[currentSessionId] ?? []) : []);

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

    // Errors from user actions, shown in an Alert at the top of the page
    let actionError = $state<string | null>(null);
    let joiningSessionId = $state<string | null>(null);
    let startingRecording = $state(false);
    let confirmReconfigure = $state(false);
    let exitTarget = $state<NewSessionMessage | null>(null);
    let confirmExit = $state(false);

    function describe(err: unknown): string {
        return typeof err === 'string' ? err : JSON.stringify(err);
    }

    async function exitSession(sessionId: string) {
        try {
            await invoke('exit_session', { sessionId });
        } catch (error) {
            console.error('Failed to exit session:', error);
            throw `Failed to exit session: ${describe(error)}`;
        }
    }

    async function rejoinSession(sessionId: string, sessionName: string) {
        actionError = null;
        joiningSessionId = sessionId;
        try {
            await invoke('rejoin_session', { sessionId, sessionName });
        } catch (error) {
            console.error('Failed to rejoin session:', error);
            actionError = `Failed to join session: ${describe(error)}`;
        } finally {
            joiningSessionId = null;
        }
    }

    async function startLocalRecording() {
        actionError = null;
        startingRecording = true;
        try {
            const sessionName = generateLocalSessionName();
            await invoke('start_local_recording', { sessionName });
        } catch (error) {
            console.error('Failed to start local recording:', error);
            actionError = `Failed to start local recording: ${describe(error)}`;
        } finally {
            startingRecording = false;
        }
    }

    // No confirmation: on constrained devices any extra step delays ending the recording.
    async function stopLocalRecording() {
        actionError = null;
        try {
            await invoke('stop_local_recording');
        } catch (error) {
            console.error('Failed to stop local recording:', error);
            actionError = `Failed to stop local recording: ${describe(error)}`;
        }
    }

    async function reconfigureDevices() {
        try {
            await invoke('delete_streaming_config');
        } catch (error) {
            throw `Failed to reset device configuration: ${describe(error)}`;
        }
        goto('/');
    }

    function sessionStatus(sessionId: string): Status {
        if (endedSessions.has(sessionId)) return 'ended';
        if (currentlyJoinedSession === sessionId) return 'joined';
        if (isSessionAvailableOnServer(sessionId)) return 'available';
        return 'unavailable';
    }

    let localSessionMessages = $derived(
        sessionMessages.filter((msg) => msg.sessionId.startsWith('local-'))
    );
    function isSessionAvailableOnServer(sessionId: string): boolean {
        return allSessions.some(([id, _name, _active]) => id === sessionId);
    }

    async function refreshSessions() {
        try {
            const [sessions, joinedSession] = await Promise.all([
                invoke<Array<[string, string, boolean]>>('get_all_sessions'),
                invoke<string | null>('get_currently_joined_session'),
            ]);
            allSessions = sessions;
            currentlyJoinedSession = joinedSession;
            // Update session messages to include all current sessions
            const existingSessionIds = new Set(sessionMessages.map((msg) => msg.sessionId));

            // Add any new sessions that aren't already in sessionMessages
            sessions.forEach(([id, name, _active]) => {
                if (!existingSessionIds.has(id)) {
                    sessionMessages.push({ sessionId: id, sessionName: name || id });
                }
            });

            sessionMessages = [...sessionMessages];
        } catch (error) {
            console.error('Failed to refresh sessions:', error);
            actionError = `Failed to refresh sessions: ${describe(error)}`;
        }
    }
</script>

{#snippet problemAlert(problem: Problem)}
    <Alert color={problem.color}>
        {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
        <span class="font-medium">{problem.title}:</span>
        {problem.detail}
    </Alert>
{/snippet}

{#snippet sessionDetails(sessionId: string, detailsTitle: string)}
    <div class="space-y-4">
        {#each successes[sessionId] ?? [] as success}
            <div class="rounded-lg border border-gray-200 p-3">
                <Heading tag="h5" class="mb-2 text-lg">{detailsTitle}</Heading>
                <dl class="grid grid-cols-1 gap-x-4 gap-y-1 text-sm sm:grid-cols-4">
                    <dt class="text-gray-500">Started</dt>
                    <dd class="text-gray-900 sm:col-span-3">
                        {new Date(Number(BigInt(success.startedAt) / 1_000_000n)).toLocaleString()}
                    </dd>
                    <dt class="text-gray-500">Recording</dt>
                    <dd class="text-gray-900 sm:col-span-3">{success.devices.join(', ')}</dd>
                </dl>
            </div>
        {/each}
        {#each problems[sessionId] ?? [] as problem}
            {@render problemAlert(problem)}
        {/each}
        {#if uploadProgress[sessionId] !== undefined}
            <Progressbar
                progress={uploadProgress[sessionId]}
                labelOutside="Upload progress"
                color="primary"
                size="h-4"
            />
        {/if}
        {#if !successes[sessionId] && !problems[sessionId] && uploadProgress[sessionId] === undefined}
            <P class="text-sm text-gray-500">No messages yet.</P>
        {/if}
    </div>
{/snippet}

<div class="flex flex-wrap items-center justify-between gap-4">
    <Heading tag="h1" class="text-2xl">
        {recordingMode === 'sessionMode' ? 'Session Listener' : 'Local Recording'}
    </Heading>
    <Button color="red" outline onclick={() => (confirmReconfigure = true)}>
        <CogOutline class="me-2 h-4 w-4" />Reconfigure Devices
    </Button>
</div>

{#if actionError}
    <Alert color="red" dismissable onclose={() => (actionError = null)}>
        {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
        {actionError}
    </Alert>
{/if}

<CompositeSummary
    config={data.config}
    devices={data.devices}
    notifications={currentNotifications}
/>

{#each currentProblems as problem}
    {@render problemAlert(problem)}
{/each}

{#if recordingMode === 'sessionMode'}
    <SectionCard title="Stream Status" icon={VideoCameraOutline}>
        {#snippet actions()}<SessionStatus status="listening" />{/snippet}
        <dl class="grid grid-cols-2 gap-4">
            <div>
                <dt class="text-sm text-gray-500">Files per recording</dt>
                <dd class="text-gray-900">{setupCounts.files}</dd>
            </div>
            <div>
                <dt class="text-sm text-gray-500">Streams</dt>
                <dd class="text-gray-900">{setupCounts.streams}</dd>
            </div>
        </dl>
    </SectionCard>

    <SectionCard title="Sessions" icon={ListOutline}>
        {#snippet actions()}
            <Button color="alternative" size="xs" onclick={refreshSessions}>
                <RefreshOutline class="me-1 h-4 w-4" />Refresh
            </Button>
        {/snippet}
        {#if sessionMessages.length === 0}
            <P class="text-sm text-gray-500"
                >No sessions yet. New sessions appear here when they start.</P
            >
        {:else}
            <Accordion class="w-full">
                {#each sessionMessages as message, index (message.sessionId)}
                    {@const status = sessionStatus(message.sessionId)}
                    <AccordionItem open={index === sessionMessages.length - 1}>
                        {#snippet header()}
                            <div class="flex w-full items-center justify-between gap-2 pe-2">
                                <span class="min-w-0 truncate text-gray-900">
                                    {message.sessionName}
                                    <span class="font-mono text-xs text-gray-500">
                                        {message.sessionId}
                                    </span>
                                </span>
                                <div class="flex shrink-0 items-center gap-2">
                                    <SessionStatus {status} />
                                    {#if status === 'joined'}
                                        <Button
                                            color="red"
                                            size="xs"
                                            onclick={(e: Event) => {
                                                e.stopPropagation();
                                                exitTarget = message;
                                                confirmExit = true;
                                            }}
                                        >
                                            Exit
                                        </Button>
                                    {:else if status === 'available'}
                                        <Button
                                            color="primary"
                                            size="xs"
                                            disabled={joiningSessionId !== null}
                                            onclick={(e: Event) => {
                                                e.stopPropagation();
                                                rejoinSession(
                                                    message.sessionId,
                                                    message.sessionName
                                                );
                                            }}
                                        >
                                            {#if joiningSessionId === message.sessionId}
                                                <Spinner size="4" color="gray" class="me-1" />
                                            {/if}
                                            Join
                                        </Button>
                                    {/if}
                                </div>
                            </div>
                        {/snippet}
                        {@render sessionDetails(message.sessionId, 'Recording')}
                    </AccordionItem>
                {/each}
            </Accordion>
        {/if}
    </SectionCard>
{:else}
    <SectionCard title="Recording Controls" icon={VideoCameraOutline}>
        <div class="flex flex-wrap items-center justify-between gap-4">
            <dl class="grid grid-cols-2 gap-x-8 gap-y-1">
                <dt class="text-sm text-gray-500">Files per recording</dt>
                <dd class="text-gray-900">{setupCounts.files}</dd>
                <dt class="text-sm text-gray-500">Upload enabled</dt>
                <dd class="text-gray-900">{setupCounts.streams > 0 ? 'Yes' : 'No'}</dd>
            </dl>
            <div class="flex items-center gap-2">
                {#if !isLocalRecording}
                    <Button
                        color="primary"
                        disabled={startingRecording}
                        onclick={startLocalRecording}
                    >
                        {#if startingRecording}
                            <Spinner size="4" color="gray" class="me-2" />
                        {:else}
                            <PlayOutline class="me-2 h-4 w-4" />
                        {/if}
                        Start Recording
                    </Button>
                {:else}
                    <SessionStatus status="recording" />
                    <Button color="red" onclick={stopLocalRecording}>
                        <StopOutline class="me-2 h-4 w-4" />Stop Recording
                    </Button>
                {/if}
            </div>
        </div>
    </SectionCard>

    {#if localSessionMessages.length > 0}
        <SectionCard title="Local Recording Sessions" icon={ListOutline}>
            <Accordion class="w-full">
                {#each localSessionMessages as message, index (message.sessionId)}
                    <AccordionItem open={index === localSessionMessages.length - 1}>
                        {#snippet header()}
                            <div class="flex w-full items-center justify-between gap-2 pe-2">
                                <span class="min-w-0 truncate text-gray-900">
                                    {message.sessionName}
                                    <span class="font-mono text-xs text-gray-500">
                                        {message.sessionId}
                                    </span>
                                </span>
                                <SessionStatus
                                    status={endedSessions.has(message.sessionId)
                                        ? 'completed'
                                        : 'recording'}
                                />
                            </div>
                        {/snippet}
                        {@render sessionDetails(message.sessionId, 'Recording details')}
                    </AccordionItem>
                {/each}
            </Accordion>
        </SectionCard>
    {/if}
{/if}

<ConfirmModal
    bind:open={confirmReconfigure}
    title="Reconfigure devices?"
    message="This stops listening with the current device setup and returns to device selection. Existing recordings are kept."
    confirmLabel="Reconfigure"
    onConfirm={reconfigureDevices}
/>
<ConfirmModal
    bind:open={confirmExit}
    title="Exit session?"
    message="Exit {exitTarget?.sessionName ??
        'this session'}? This device stops recording and streaming for it."
    confirmLabel="Exit Session"
    onConfirm={() => exitSession(exitTarget!.sessionId)}
/>
