<script lang="ts">
    import type { MediaDeviceInfo, PublishOptions } from './types';
    import { Button, Select, Toggle } from 'flowbite-svelte';

    let {
        device,
        addDevice,
    }: {
        device: MediaDeviceInfo;
        addDevice: (option: PublishOptions, enableStreaming?: boolean) => void;
        removeDevice: (id: string) => void;
    } = $props();

    let publishOptions: PublishOptions[] = $derived.by(() => {
        if (device.deviceClass === 'Video/Source') {
            return device.capabilities
                .filter((capability) => capability.kind === 'Video')
                .flatMap((capability) =>
                    capability.framerates.map((framerate) => ({
                        kind: 'Video',
                        codec: capability.codec,
                        deviceId: device.devicePath,
                        width: capability.width,
                        height: capability.height,
                        framerate: framerate,
                    }))
                );
        } else if (device.deviceClass === 'Audio/Source') {
            let options = device.capabilities
                .filter((capability) => capability.kind === 'Audio')
                .flatMap((capability) =>
                    capability.framerates.flatMap((framerate) => {
                        if (capability.channels <= 2) {
                            return [
                                {
                                    kind: 'Audio',
                                    codec: capability.codec,
                                    deviceId: device.devicePath,
                                    channels: capability.channels,
                                    framerate: framerate,
                                },
                            ];
                        } else {
                            // For channels ranging from 1...numof channels as well as all channels
                            let deviceOptions = [
                                {
                                    kind: 'Audio',
                                    codec: capability.codec,
                                    deviceId: device.devicePath,
                                    channels: capability.channels,
                                    framerate: framerate,
                                },
                            ];
                            let channelsArray = Array.from(
                                { length: capability.channels },
                                (_, index) => ({
                                    kind: 'Audio',
                                    codec: capability.codec,
                                    deviceId: device.devicePath,
                                    channels: capability.channels,
                                    framerate: framerate,
                                    selectedChannel: index + 1,
                                })
                            );
                            deviceOptions.push(...channelsArray);
                            return deviceOptions;
                        }
                    })
                );

            let maxFramerate = Math.max(...options.map((opt) => opt.framerate));
            let capsWithMaxFramerate = options.find((opt) => opt.framerate === maxFramerate);
            if (maxFramerate > 96100) {
                options.push({
                    kind: 'Audio',
                    codec: 'audio/x-raw',
                    deviceId: device.devicePath,
                    channels: capsWithMaxFramerate?.channels || 1,
                    framerate: 48000,
                });
                options.push({
                    kind: 'Audio',
                    codec: 'audio/x-raw',
                    deviceId: device.devicePath,
                    channels: capsWithMaxFramerate?.channels || 1,
                    framerate: 44100,
                });
            }

            return options;
        } else if (device.deviceClass === 'Screen/Source') {
            return device.capabilities
                .filter((capability) => capability.kind === 'Screen')
                .flatMap((capability) => {
                    return capability.framerates.map((framerate) => ({
                        kind: 'Screen',
                        codec: capability.codec,
                        screenIdOrName: device.devicePath,
                        width: capability.width,
                        height: capability.height,
                        framerate: framerate,
                    }));
                });
        }
    });

    let selectedOption: PublishOptions | null = $state(null);

    function optionLabel(option: PublishOptions): string {
        if (option.kind === 'Video') {
            return `${option.codec} ${option.width}x${option.height} @ ${option.framerate}fps`;
        }
        if (option.kind === 'Audio') {
            return option.selectedChannel
                ? `${option.codec} ${option.channels}ch @ ${option.framerate}Hz @channel${option.selectedChannel}Only`
                : `${option.codec} ${option.channels}ch @ ${option.framerate}Hz`;
        }
        if (option.kind === 'Screen') {
            return `Screen ${option.width}x${option.height} @ ${option.framerate}fps`;
        }
        return 'Unknown Option';
    }

    let selectOptions = $derived.by(() => {
        return publishOptions.map((option) => ({
            value: option,
            name: optionLabel(option),
        }));
    });

    let streamingDisabled = $state(false);
</script>

<div class="bg-white rounded-2xl shadow-xl p-8 mt-2 border border-purple-100">
    <div class="flex flex-col justify-between gap-2">
        <div class="flex items-center justify-between mb-4">
            <h3 class="text-md font-semibold text-blue-700">
                {device.displayName} ({device.deviceClass})
            </h3>
            <Toggle bind:checked={streamingDisabled}>
                <span class="text-xs">Disbale Streaming</span>
            </Toggle>
        </div>
        {#if device.deviceClass == 'Video/Source'}
            <Select
                items={selectOptions}
                placeholder="Select Resolution and Codecs"
                bind:value={selectedOption}
            />
            {#if selectedOption}
                <Button
                    color="blue"
                    class="mt-2"
                    onclick={() => addDevice(selectedOption!, !streamingDisabled)}
                >
                    Add Video Device
                </Button>
            {/if}
        {/if}
        {#if device.deviceClass == 'Audio/Source'}
            <Select
                items={selectOptions}
                placeholder="Select Audio Options"
                bind:value={selectedOption}
            />
            {#if selectedOption}
                <Button color="blue" class="mt-2" onclick={() => addDevice(selectedOption!)}>
                    Add Audio Device
                </Button>
            {/if}
        {/if}
        {#if device.deviceClass == 'Screen/Source'}
            <Select
                items={selectOptions}
                placeholder="Select Screen Options"
                bind:value={selectedOption}
            />
            {#if selectedOption}
                <Button color="blue" class="mt-2" onclick={() => addDevice(selectedOption!)}>
                    Add Screen Device
                </Button>
            {/if}
        {/if}
    </div>
</div>
