<script lang="ts">
    import { Button, P } from 'flowbite-svelte';
    import { RefreshOutline, VideoCameraOutline } from 'flowbite-svelte-icons';
    import DeviceSelector from './DeviceSelector.svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import type { MediaDeviceInfo, PublishOptions } from './types';

    let {
        devices,
        onAddDevice,
        onRemoveDevice,
        onRefresh,
    }: {
        devices: MediaDeviceInfo[];
        onAddDevice: (option: PublishOptions) => void;
        onRemoveDevice: (id: string) => void;
        onRefresh: () => void;
    } = $props();
</script>

<SectionCard title="Available Devices" icon={VideoCameraOutline} collapsible>
    {#snippet actions()}
        <Button color="alternative" size="xs" onclick={() => onRefresh()}>
            <RefreshOutline class="me-1 h-4 w-4" />Refresh
        </Button>
    {/snippet}
    <div class="space-y-4">
        {#each devices as device (device.devicePath)}
            <DeviceSelector {device} addDevice={onAddDevice} removeDevice={onRemoveDevice} />
        {:else}
            <P class="text-sm text-gray-500">No more devices available to add.</P>
        {/each}
    </div>
</SectionCard>
