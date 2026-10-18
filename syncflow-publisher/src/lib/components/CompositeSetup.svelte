<script lang="ts">
    import { Alert, Button, Heading, Helper, Label, Range, Toggle } from 'flowbite-svelte';
    import {
        ExclamationCircleOutline,
        InfoCircleOutline,
        PlusOutline,
        RefreshOutline,
        VideoCameraOutline,
    } from 'flowbite-svelte-icons';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import DeviceRow from './DeviceRow.svelte';
    import type { MediaDeviceInfo, PipelineCapabilities } from './types';
    import {
        cameraLabel,
        micLabel,
        type Draft,
        type Issue,
        type SlotId,
    } from '#lib/compositeConfig.js';

    // The recording setup: the AV mix, an optional screen share, and additional cameras
    // and microphones, each with its own file and stream toggle.
    interface Props {
        draft: Draft;
        devices: MediaDeviceInfo[];
        issues: Issue[];
        notices: string[];
        capabilities: PipelineCapabilities | null;
        refreshing: boolean;
        onrefresh: () => void;
    }

    let { draft, devices, issues, notices, capabilities, refreshing, onrefresh }: Props = $props();

    function errorFor(slot: SlotId): string | null {
        return issues.find((i) => i.slot === slot)?.message ?? null;
    }

    let setupIssues = $derived(issues.filter((i) => i.slot === null));
    let micFiles = $derived(draft.avMix.useMic2 ? 'mic-1, mic-2' : 'mic-1');
    // Without audiornnoise there is nothing to gate with; unknown capabilities leave it enabled.
    let vadDisabled = $derived(capabilities?.noiseSuppression === false);
</script>

<SectionCard title="Recording setup" icon={VideoCameraOutline}>
    {#snippet actions()}
        <Button color="alternative" size="xs" disabled={refreshing} onclick={onrefresh}>
            <RefreshOutline class="me-1 h-4 w-4" />Refresh devices
        </Button>
    {/snippet}
    <div class="space-y-4">
        <div class="space-y-3 rounded-lg border border-gray-200 p-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <Heading tag="h5" class="text-lg">AV mix</Heading>
                    <p class="text-sm text-gray-500">
                        One file with the camera and both mics (av-mix.mp4), plus {micFiles}.m4a
                    </p>
                </div>
                <div class="flex items-center gap-4">
                    <Toggle size="small" bind:checked={draft.avMix.include}>
                        <span class="text-sm">Include</span>
                    </Toggle>
                    <Toggle
                        size="small"
                        bind:checked={draft.avMix.stream}
                        disabled={!draft.avMix.include}
                    >
                        <span class="text-sm">Stream</span>
                    </Toggle>
                </div>
            </div>
            {#if draft.avMix.include}
                <DeviceRow
                    label="Camera"
                    kind="camera"
                    slotId="avMix.camera"
                    {draft}
                    {devices}
                    bind:options={draft.avMix.camera}
                    error={errorFor('avMix.camera')}
                />
                <DeviceRow
                    label="Mic L"
                    kind="mic"
                    slotId="avMix.mic1"
                    {draft}
                    {devices}
                    bind:options={draft.avMix.mic1}
                    error={errorFor('avMix.mic1')}
                />
                {#if draft.avMix.useMic2}
                    <DeviceRow
                        label="Mic R"
                        kind="mic"
                        slotId="avMix.mic2"
                        {draft}
                        {devices}
                        bind:options={draft.avMix.mic2}
                        error={errorFor('avMix.mic2')}
                        onremove={() => {
                            draft.avMix.useMic2 = false;
                            draft.avMix.mic2 = null;
                        }}
                    />
                {:else}
                    <Button
                        color="alternative"
                        size="xs"
                        onclick={() => (draft.avMix.useMic2 = true)}
                    >
                        <PlusOutline class="me-1 h-4 w-4" />Add mic R
                    </Button>
                {/if}
                <Helper>
                    Mic L and Mic R are noise suppressed and recorded on the left and right
                    channels. The stream gets the camera and one mono track of both mics.
                </Helper>
            {/if}
        </div>

        <div class="space-y-3 rounded-lg border border-gray-200 p-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <Heading tag="h5" class="text-lg">Screen share</Heading>
                    <p class="text-sm text-gray-500">Its own file (screen-1)</p>
                </div>
                <div class="flex items-center gap-4">
                    <Toggle size="small" bind:checked={draft.screen.include}>
                        <span class="text-sm">Include</span>
                    </Toggle>
                    <Toggle
                        size="small"
                        bind:checked={draft.screen.stream}
                        disabled={!draft.screen.include}
                    >
                        <span class="text-sm">Stream</span>
                    </Toggle>
                </div>
            </div>
            {#if draft.screen.include}
                <DeviceRow
                    label="Display"
                    kind="screen"
                    slotId="screen"
                    {draft}
                    {devices}
                    bind:options={draft.screen.options}
                    error={errorFor('screen')}
                />
            {/if}
        </div>

        <div class="space-y-3 rounded-lg border border-gray-200 p-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <Heading tag="h5" class="text-lg">Additional cameras</Heading>
                    <p class="text-sm text-gray-500">Each records its own file</p>
                </div>
                <Button
                    color="alternative"
                    size="xs"
                    onclick={() => draft.cameras.push({ options: null, stream: false })}
                >
                    <PlusOutline class="me-1 h-4 w-4" />Add camera
                </Button>
            </div>
            {#each draft.cameras as camera, i (i)}
                <DeviceRow
                    label={cameraLabel(draft, i)}
                    kind="camera"
                    slotId={`camera.${i}`}
                    {draft}
                    {devices}
                    bind:options={camera.options}
                    bind:stream={camera.stream}
                    error={errorFor(`camera.${i}`)}
                    onremove={() => draft.cameras.splice(i, 1)}
                />
            {/each}
        </div>

        <div class="space-y-3 rounded-lg border border-gray-200 p-3">
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <Heading tag="h5" class="text-lg">Additional microphones</Heading>
                    <p class="text-sm text-gray-500">
                        Each records its own file; a multichannel interface can feed several rows,
                        one channel each
                    </p>
                </div>
                <Button
                    color="alternative"
                    size="xs"
                    onclick={() => draft.mics.push({ options: null, stream: false })}
                >
                    <PlusOutline class="me-1 h-4 w-4" />Add microphone
                </Button>
            </div>
            {#each draft.mics as mic, i (i)}
                <DeviceRow
                    label={micLabel(draft, i)}
                    kind="mic"
                    slotId={`mic.${i}`}
                    {draft}
                    {devices}
                    bind:options={mic.options}
                    bind:stream={mic.stream}
                    error={errorFor(`mic.${i}`)}
                    onremove={() => draft.mics.splice(i, 1)}
                />
            {/each}
        </div>

        <div class="space-y-3 rounded-lg border border-gray-200 p-3">
            <Heading tag="h5" class="text-lg">Audio</Heading>
            <div class="space-y-2">
                <div class="flex items-center justify-between gap-2">
                    <Label for="vad-threshold">Voice activity threshold</Label>
                    <span class="text-sm text-gray-500">{draft.vadThreshold.toFixed(2)}</span>
                </div>
                <Range
                    id="vad-threshold"
                    color="gray"
                    min={0}
                    max={1}
                    step={0.05}
                    disabled={vadDisabled}
                    bind:value={draft.vadThreshold}
                />
                <Helper>
                    Mutes recorded and streamed audio while the chance of speech is below this
                    value. 0 turns gating off.
                </Helper>
            </div>
        </div>

        {#each setupIssues as issue}
            <Alert color="red">
                {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
                {issue.message}
            </Alert>
        {/each}
        {#each notices as notice}
            <Alert color="blue">
                {#snippet icon()}<InfoCircleOutline class="h-5 w-5" />{/snippet}
                {notice}
            </Alert>
        {/each}
    </div>
</SectionCard>
