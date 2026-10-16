<script lang="ts">
    import { goto } from '$app/navigation';
    import { invoke } from '@tauri-apps/api/core';
    import { Alert, Button, Heading, Helper, Radio, Spinner } from 'flowbite-svelte';
    import {
        ArrowRightToBracketOutline,
        CogOutline,
        ExclamationCircleOutline,
        InfoCircleOutline,
        TrashBinOutline,
    } from 'flowbite-svelte-icons';
    import ConfirmModal from '#lib/components/ui/ConfirmModal.svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import type { PageProps } from './$types';
    import RegistrationDetails from '#lib/components/RegistrationDetails.svelte';
    import CompositeSetup from '#lib/components/CompositeSetup.svelte';
    import { refreshDevices, setup } from '#lib/store.svelte.js';
    import { counts, notices, toConfig, validate } from '#lib/compositeConfig.js';
    import type { RecordingMode } from '#lib/components/types.js';

    let { data }: PageProps = $props();

    let registrationDetails = $derived.by(() => data.registration);
    // svelte-ignore state_referenced_locally
    setup.devices = data.devices;

    let confirmDeregister = $state(false);

    async function deregister() {
        try {
            await invoke('delete_registration');
        } catch (err) {
            throw `Deregistration failed: ${JSON.stringify(err)}`;
        }
        goto('/register');
    }

    let errorMessage = $state<string | null>(null);
    let starting = $state(false);
    let recordingMode = $state<RecordingMode>('sessionMode');

    // Load saved recording mode on page initialization
    invoke<RecordingMode>('get_recording_mode')
        .then((savedMode) => {
            recordingMode = savedMode;
        })
        .catch((error) => {
            console.warn('Failed to load saved recording mode:', error);
            // Default to sessionMode if loading fails
            recordingMode = 'sessionMode';
        });

    // Save recording mode whenever it changes
    $effect(() => {
        invoke('set_recording_mode', { recordingMode }).catch((error) => {
            console.error('Failed to save recording mode:', error);
            errorMessage = `Failed to save recording mode: ${JSON.stringify(error)}`;
        });
    });

    let issues = $derived(validate(setup.draft, setup.devices));
    let setupNotices = $derived(notices(setup.draft, recordingMode, data.capabilities));
    let summary = $derived(counts(setup.draft));
    let refreshing = $state(false);

    async function refresh() {
        refreshing = true;
        try {
            await refreshDevices();
        } catch (err) {
            errorMessage = `Failed to refresh devices: ${JSON.stringify(err)}`;
        } finally {
            refreshing = false;
        }
    }

    async function start() {
        starting = true;
        try {
            await invoke('set_streaming_config', {
                config: toConfig(setup.draft, recordingMode),
            });
            errorMessage = null;
            goto('/stream');
        } catch (err) {
            errorMessage = `Failed to save the recording setup: ${JSON.stringify(err)}`;
        } finally {
            starting = false;
        }
    }
</script>

<div class="flex flex-wrap items-center justify-between gap-4">
    <div>
        <Heading tag="h1" class="text-2xl">SyncFlow Publisher</Heading>
        {#if registrationDetails?.deviceName}
            <p class="text-sm text-gray-500">{registrationDetails.deviceName}</p>
        {/if}
    </div>
    {#if registrationDetails}
        <Button color="red" outline onclick={() => (confirmDeregister = true)}>
            <TrashBinOutline class="me-2 h-4 w-4" />Delete Registration
        </Button>
    {:else}
        <Button color="alternative" onclick={() => goto('/register')}>
            <ArrowRightToBracketOutline class="me-2 h-4 w-4" />Register Your Device
        </Button>
    {/if}
</div>

{#if registrationDetails}
    <RegistrationDetails {registrationDetails} />
{:else}
    <Alert color="blue">
        {#snippet icon()}<InfoCircleOutline class="h-5 w-5" />{/snippet}
        This device isn't registered, so only offline (local recording) mode is available.
    </Alert>
{/if}

<SectionCard title="Recording Mode" icon={CogOutline}>
    <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
        <div>
            <Radio
                bind:group={recordingMode}
                value="sessionMode"
                aria-describedby="session-mode-help"
            >
                Session Mode: listen for remote sessions and auto-join
            </Radio>
            <Helper id="session-mode-help" class="ps-6">
                Requires an internet connection and registration
            </Helper>
        </div>
        <div>
            <Radio bind:group={recordingMode} value="localMode" aria-describedby="local-mode-help">
                Local Recording Mode: record manually with local controls
            </Radio>
            <Helper id="local-mode-help" class="ps-6">
                Works offline; you start and stop each recording
            </Helper>
        </div>
    </div>
</SectionCard>

<CompositeSetup
    draft={setup.draft}
    devices={setup.devices}
    {issues}
    notices={setupNotices}
    capabilities={data.capabilities}
    {refreshing}
    onrefresh={refresh}
/>

{#if errorMessage}
    <Alert color="red">
        {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
        {errorMessage}
    </Alert>
{/if}

<div class="flex items-center justify-end gap-4">
    <span class="text-sm text-gray-500">
        {summary.files} file{summary.files === 1 ? '' : 's'} · {summary.streams} stream{summary.streams ===
        1
            ? ''
            : 's'}
    </span>
    <Button color="primary" disabled={issues.length > 0 || starting} onclick={start}>
        {#if starting}<Spinner size="4" color="gray" class="me-2" />{/if}
        {recordingMode === 'sessionMode' ? 'Start Session Listener' : 'Set Up Local Recording'}
    </Button>
</div>

<ConfirmModal
    bind:open={confirmDeregister}
    title="Delete registration?"
    message="This removes the device from the SyncFlow project and deletes its saved credentials. Local recordings are kept."
    confirmLabel="Delete Registration"
    onConfirm={deregister}
/>
