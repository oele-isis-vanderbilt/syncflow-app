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
    import Devices from '#lib/components/Devices.svelte';
    import { devicesStore } from '#lib/store.svelte.js';
    import SelectedDevices from '#lib/components/SelectedDevices.svelte';
    import type {
        DeviceRecordingAndStreamingConfig,
        RecordingMode,
    } from '#lib/components/types.js';

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
        getAvMixMode,
        setAvMixMode,
        getAvMixRoles,
        setAvMixRole,
    } = devicesStore!;

    let availableDevicesToSelect = $derived.by(() => {
        const remainingDevicesFn = getRemainingDevicesFn();
        return remainingDevicesFn();
    });

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

    async function start() {
        const selectedDevicesFn = getSelectedDevicesFn();
        const streamingConfigFn = getStreamingConfigFn();
        const selectedDevices = selectedDevicesFn();
        const streamingConfigs = streamingConfigFn();
        const avMixMode = getAvMixMode();
        const avMixRoles = getAvMixRoles();

        const recordingAndStreamingConfig: DeviceRecordingAndStreamingConfig[] =
            selectedDevices.map((option) => {
                const deviceId = option.kind === 'Screen' ? option.screenIdOrName : option.deviceId;

                return {
                    enableStreaming: streamingConfigs[deviceId],
                    publishOptions: option,
                    avMixMode: avMixMode ? avMixRoles[deviceId] : undefined,
                    recordingMode: recordingMode,
                };
            });

        starting = true;
        try {
            await invoke('set_streaming_config', {
                configs: recordingAndStreamingConfig,
            });
            errorMessage = null;
            goto('/stream');
        } catch (err) {
            errorMessage = `Failed to set streaming config: ${JSON.stringify(err)}`;
        } finally {
            starting = false;
        }
    }

    let selectedCount = $derived(getSelectedDevicesFn()().length);
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

<div class="grid grid-cols-1 items-start gap-6 md:grid-cols-2">
    <Devices
        devices={availableDevicesToSelect}
        onAddDevice={addDevice}
        onRemoveDevice={removeDevice}
        onRefresh={refreshAvailableDevices}
    />
    <SelectedDevices
        allDevices={mediaDevices}
        selectedDevicesFn={getSelectedDevicesFn()}
        onRemoveDevice={removeDevice}
        streamingConfigFn={getStreamingConfigFn()}
        avMixMode={getAvMixMode()}
        avMixRoles={getAvMixRoles()}
        onAvMixModeChange={setAvMixMode}
        onAvMixRoleChange={setAvMixRole}
    />
</div>

{#if errorMessage}
    <Alert color="red">
        {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
        {errorMessage}
    </Alert>
{/if}

<div class="flex items-center justify-end gap-4">
    {#if !selectedCount}
        <span class="text-sm text-gray-500">Select at least one device to continue</span>
    {/if}
    <Button color="primary" disabled={!selectedCount || starting} onclick={start}>
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
