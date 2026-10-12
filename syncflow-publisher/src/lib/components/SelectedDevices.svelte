<script lang="ts">
    import { Badge, Button, Helper, Label, P, Select, Toggle } from 'flowbite-svelte';
    import { DesktopPcOutline, TrashBinOutline } from 'flowbite-svelte-icons';
    import SessionStatus from './SessionStatus.svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import type { MediaDeviceInfo, PublishOptions, AvMixMode } from './types';

    let {
        selectedDevicesFn,
        streamingConfigFn,
        allDevices,
        onRemoveDevice = () => {},
        showDeleteButton = true,
        avMixMode = false,
        avMixRoles = {},
        onAvMixModeChange = () => {},
        onAvMixRoleChange = () => {},
        readonly = false,
    }: {
        selectedDevicesFn: () => PublishOptions[];
        allDevices: MediaDeviceInfo[];
        onRemoveDevice?: (deviceId: string) => void;
        streamingConfigFn: () => Record<string, boolean>;
        showDeleteButton?: boolean;
        avMixMode?: boolean;
        avMixRoles?: Record<string, AvMixMode>;
        onAvMixModeChange?: (enabled: boolean) => void;
        onAvMixRoleChange?: (deviceId: string, role: AvMixMode | undefined) => void;
        readonly?: boolean;
    } = $props();

    // Check if AV mix mode is valid (1 video + 1-2 audio devices)
    let canEnableAvMix = $derived.by(() => {
        const devices = selectedDevicesFn();
        const videoDevices = devices.filter((d) => d.kind === 'Video');
        const audioDevices = devices.filter((d) => d.kind === 'Audio');
        return videoDevices.length === 1 && audioDevices.length >= 1 && audioDevices.length <= 2;
    });

    let avMixRoleOptions = [
        { value: 'primary', name: 'Primary Camera' },
        { value: 'mic1', name: 'Microphone 1' },
        { value: 'mic2', name: 'Microphone 2' },
    ];

    function getDeviceName(devicePath: string) {
        const device = allDevices.find((d) => d.devicePath === devicePath);
        console.log('Device for path', devicePath, 'is', device);
        return device ? device.displayName : 'Unknown Device';
    }

    let streamingEnabled = $derived.by(() => {
        const streamingConfigs = streamingConfigFn();
        return streamingConfigs;
    });

    function deviceKey(device: PublishOptions): string {
        return device.kind === 'Screen' ? device.screenIdOrName : device.deviceId;
    }

    const idPrefix = $props.id();
</script>

<SectionCard title="Selected Devices" icon={DesktopPcOutline} collapsible>
    <div class="space-y-4">
        {#if canEnableAvMix}
            <div class="rounded-lg border border-gray-200 bg-gray-50 p-3">
                <Toggle
                    bind:checked={avMixMode}
                    onchange={() => onAvMixModeChange(avMixMode)}
                    disabled={readonly}
                    size="small"
                >
                    <span class="text-sm font-medium text-gray-900">AV Mix Mode</span>
                </Toggle>
                <Helper class="mt-1">
                    Combine video and audio into a single synchronized stream
                </Helper>
            </div>
        {/if}

        {#each selectedDevicesFn() as device (deviceKey(device))}
            {@const deviceId = deviceKey(device)}
            {@const deviceName = getDeviceName(deviceId)}
            <div class="space-y-2 rounded-lg border border-gray-200 bg-gray-50 p-3">
                <div class="flex items-center justify-between gap-2">
                    <div class="flex min-w-0 items-center gap-2">
                        <Badge color="primary">{device.kind}</Badge>
                        <span class="truncate text-gray-900">{deviceName}</span>
                    </div>
                    <div class="flex items-center gap-2">
                        <SessionStatus
                            status={streamingEnabled[deviceId] ? 'streaming' : 'recordOnly'}
                        />
                        {#if showDeleteButton}
                            <Button
                                color="red"
                                outline
                                size="xs"
                                aria-label="Remove {deviceName}"
                                onclick={() => onRemoveDevice(deviceId)}
                            >
                                <TrashBinOutline class="h-4 w-4" />
                            </Button>
                        {/if}
                    </div>
                </div>
                <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
                    <dt class="text-gray-500">Codec</dt>
                    <dd class="text-gray-900">{device.codec ?? 'N/A'}</dd>
                    <dt class="text-gray-500">Framerate</dt>
                    <dd class="text-gray-900">
                        {device.framerate
                            ? `${device.framerate} ${device.kind === 'Audio' ? 'Hz' : 'fps'}`
                            : 'N/A'}
                    </dd>
                    {#if device.kind !== 'Audio' && 'width' in device && 'height' in device && device.width && device.height}
                        <dt class="text-gray-500">Resolution</dt>
                        <dd class="text-gray-900">{device.width}x{device.height}</dd>
                    {/if}
                    {#if device.kind === 'Audio' && 'channels' in device && device.channels}
                        <dt class="text-gray-500">Channels</dt>
                        <dd class="text-gray-900">{device.channels}</dd>
                    {/if}
                </dl>
                {#if avMixMode}
                    {@const roleSelectId = `${idPrefix}-role-${deviceId}`}
                    <div class="border-t border-gray-200 pt-2">
                        <Label for={roleSelectId} class="mb-2">AV Mix Role</Label>
                        <Select
                            id={roleSelectId}
                            size="sm"
                            value={avMixRoles[deviceId] || ''}
                            placeholder="Select role..."
                            items={avMixRoleOptions.filter((option) => {
                                // Filter roles based on device type
                                if (device.kind === 'Video' || device.kind === 'Screen') {
                                    return option.value === 'primary';
                                } else if (device.kind === 'Audio') {
                                    return option.value === 'mic1' || option.value === 'mic2';
                                }
                                return false;
                            })}
                            onchange={(e) => {
                                const target = e.target as HTMLSelectElement;
                                onAvMixRoleChange(deviceId, target.value as AvMixMode);
                            }}
                            disabled={readonly}
                        />
                    </div>
                {/if}
            </div>
        {:else}
            <P class="text-sm text-gray-500">No devices selected yet.</P>
        {/each}
    </div>
</SectionCard>
