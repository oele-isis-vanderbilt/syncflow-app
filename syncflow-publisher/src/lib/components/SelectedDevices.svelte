<script lang="ts">
    import { Button, Toggle, Select } from 'flowbite-svelte';
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

    let minimized = $state(false);

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

    $inspect(selectedDevicesFn());
</script>

<div
    class="bg-white rounded-2xl shadow-xl p-8 mt-2 border border-purple-100 w-full h-full flex flex-col"
>
    <div class="flex justify-between items-center mb-6 w-full h-full">
        <h2 class="text-xl font-bold text-blue-700 flex items-center gap-2">
            <svg
                class="w-32 h-32 text-purple-400"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                viewBox="0 0 24 24"
            >
                <rect x="3" y="7" width="13" height="10" rx="2" />
                <path d="M16 10l4 2-4 2v-4z" />
                <circle cx="8" cy="12" r="1.5" />
            </svg>
            Selected Devices and Publish Options
        </h2>
        <Button color="purple" outline class="ml-2" onclick={() => (minimized = !minimized)}>
            {minimized ? 'Show' : 'Minimize'}
        </Button>
    </div>
    {#if !minimized}
        <!-- AV Mix Mode Toggle -->
        {#if canEnableAvMix}
            <div class="mb-6 p-4 bg-blue-50 rounded-lg border border-blue-200">
                <div class="flex items-center justify-between mb-2">
                    <label for="av-mix-toggle" class="text-sm font-medium text-blue-800">
                        AV Mix Mode
                    </label>
                    <Toggle
                        id="av-mix-toggle"
                        bind:checked={avMixMode}
                        onchange={() => onAvMixModeChange(avMixMode)}
                        disabled={readonly}
                    />
                </div>
                <p class="text-xs text-blue-600">
                    Combine video and audio into a single synchronized stream
                </p>
            </div>
        {/if}

        <div class="space-y-4 flex-1 overflow-y-auto">
            {#each selectedDevicesFn() as device, index}
                {@const deviceName =
                    device.kind == 'Screen'
                        ? getDeviceName(device.screenIdOrName)
                        : getDeviceName(device.deviceId)}
                <div class="border border-purple-200 rounded-lg p-4 bg-purple-50">
                    <div class="flex items-center justify-between gap-2 mb-2">
                        <div class="flex items-center gap-2">
                            <span class="font-semibold text-purple-700">{device.kind}</span>
                            <span class="text-gray-600">({deviceName})</span>
                        </div>
                        {#if showDeleteButton}
                            <Button
                                color="red"
                                outline
                                size="xs"
                                onclick={() => {
                                    if (device.kind === 'Screen') {
                                        onRemoveDevice(device.screenIdOrName);
                                    } else {
                                        onRemoveDevice(device.deviceId);
                                    }
                                }}
                            >
                                Delete
                            </Button>
                        {/if}
                    </div>
                    <div class="text-sm text-gray-700">
                        <div>
                            <span class="font-medium">Codec:</span>
                            <span>{device.codec ?? 'N/A'}</span>
                        </div>
                        <div>
                            <span class="font-medium">Framerate:</span>
                            <span>
                                {#if device.framerate}
                                    {device.framerate} fps
                                {:else}
                                    N/A
                                {/if}
                            </span>
                        </div>
                        {#if device.kind !== 'Audio' && 'width' in device && 'height' in device && device.width && device.height}
                            <div>
                                <span class="font-medium">Resolution:</span>
                                <span>{device.width}x{device.height}</span>
                            </div>
                        {/if}
                        {#if device.kind === 'Audio' && 'channels' in device && device.channels}
                            <div>
                                <span class="font-medium">Channels:</span>
                                <span>{device.channels} channels</span>
                            </div>
                        {/if}
                        <div>
                            <span class="font-medium">Streaming Enabled:</span>
                            <span
                                >{streamingEnabled[
                                    device.kind === 'Screen'
                                        ? device.screenIdOrName
                                        : device.deviceId
                                ]
                                    ? 'Yes'
                                    : 'No'}</span
                            >
                        </div>
                        {#if avMixMode}
                            {@const deviceId =
                                device.kind === 'Screen' ? device.screenIdOrName : device.deviceId}
                            <div class="mt-3 pt-3 border-t border-purple-200">
                                <label class="block text-sm font-medium text-purple-700 mb-2">
                                    AV Mix Role:
                                </label>
                                <Select
                                    value={avMixRoles[deviceId] || ''}
                                    placeholder="Select role..."
                                    items={avMixRoleOptions.filter((option) => {
                                        // Filter roles based on device type
                                        if (device.kind === 'Video' || device.kind === 'Screen') {
                                            return option.value === 'primary';
                                        } else if (device.kind === 'Audio') {
                                            return (
                                                option.value === 'mic1' || option.value === 'mic2'
                                            );
                                        }
                                        return false;
                                    })}
                                    onchange={(e) => {
                                        const target = e.target as HTMLSelectElement;
                                        console.log('Select change event:', deviceId, target.value);
                                        onAvMixRoleChange(deviceId, target.value as AvMixMode);
                                    }}
                                    disabled={readonly}
                                />
                            </div>
                        {/if}
                    </div>
                </div>
            {/each}
        </div>
    {/if}
</div>
