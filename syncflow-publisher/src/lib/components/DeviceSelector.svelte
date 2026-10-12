<script lang="ts">
    import type { MediaDeviceInfo, PublishOptions } from './types';
    import { Badge, Button, Heading, Label, Select, Toggle } from 'flowbite-svelte';
    import { PlusOutline } from 'flowbite-svelte-icons';

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

    const KIND_LABELS: Record<string, { kind: string; placeholder: string }> = {
        'Video/Source': { kind: 'Video', placeholder: 'Select resolution and codec' },
        'Audio/Source': { kind: 'Audio', placeholder: 'Select audio options' },
        'Screen/Source': { kind: 'Screen', placeholder: 'Select screen options' },
    };
    let kindLabel = $derived(KIND_LABELS[device.deviceClass]);
    const selectId = $props.id();
</script>

<div class="space-y-3 rounded-lg border border-gray-200 p-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex min-w-0 items-center gap-2">
            <Heading tag="h5" class="w-auto truncate text-lg">{device.displayName}</Heading>
            <Badge color="gray">{device.deviceClass}</Badge>
        </div>
        <Toggle bind:checked={streamingDisabled} size="small">
            <span class="text-sm">Record only (no streaming)</span>
        </Toggle>
    </div>
    {#if kindLabel}
        <div>
            <Label for={selectId} class="mb-2">Format</Label>
            <Select
                id={selectId}
                items={selectOptions}
                placeholder={kindLabel.placeholder}
                bind:value={selectedOption}
            />
        </div>
        {#if selectedOption}
            <Button
                color="alternative"
                size="sm"
                onclick={() => addDevice(selectedOption!, !streamingDisabled)}
            >
                <PlusOutline class="me-1 h-4 w-4" />Add {kindLabel.kind} Device
            </Button>
        {/if}
    {/if}
</div>
