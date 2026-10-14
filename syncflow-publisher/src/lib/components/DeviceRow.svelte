<script
    lang="ts"
    generics="T extends VideoPublishOptions | AudioPublishOptions | ScreenPublishOptions"
>
    import { Button, Helper, Label, Select, Toggle } from 'flowbite-svelte';
    import { CloseOutline } from 'flowbite-svelte-icons';
    import type {
        AudioPublishOptions,
        MediaDeviceInfo,
        ScreenPublishOptions,
        VideoPublishOptions,
    } from './types';
    import {
        channelsInUse,
        deviceTakenBy,
        sharedAudioFormat,
        type Draft,
        type SlotId,
    } from '#lib/compositeConfig.js';
    import {
        audioFormats,
        defaultAudio,
        defaultScreen,
        defaultVideo,
        deviceChannels,
        deviceIdOf,
        deviceLabel,
        devicesOfKind,
        formatKey,
        formatLabel,
        screenFormats,
        videoFormats,
        type DeviceKind,
    } from '#lib/publishOptions.js';

    // One device of the recording setup: device, channel (multichannel interfaces) and
    // format, plus an optional stream toggle and remove button.
    interface Props {
        label: string;
        kind: DeviceKind;
        slotId: SlotId;
        draft: Draft;
        devices: MediaDeviceInfo[];
        options: T | null;
        /** Shown as a Stream toggle when set. */
        stream?: boolean;
        error?: string | null;
        onremove?: () => void;
    }

    let {
        label,
        kind,
        slotId,
        draft,
        devices,
        options = $bindable(),
        stream = $bindable(),
        error = null,
        onremove,
    }: Props = $props();

    const id = $props.id();
    let candidates = $derived(devicesOfKind(devices, kind));
    let deviceId = $derived(options ? deviceIdOf(options) : '');
    let device = $derived(candidates.find((d) => d.devicePath === deviceId));
    let interfaceChannels = $derived(kind === 'mic' && device ? deviceChannels(device) : 0);
    let shared = $derived(
        kind === 'mic' && deviceId ? sharedAudioFormat(draft, slotId, deviceId) : null
    );

    let deviceItems = $derived.by(() => {
        const items = candidates.map((d) => {
            const multichannel = kind === 'mic' && deviceChannels(d) > 2;
            const taken = multichannel ? null : deviceTakenBy(draft, slotId, d.devicePath);
            const name = deviceLabel(d, candidates);
            return {
                value: d.devicePath,
                name:
                    taken && d.devicePath !== deviceId ? `${name} (in use: ${taken.label})` : name,
                disabled: taken !== null && d.devicePath !== deviceId,
            };
        });
        if (deviceId && !device) {
            items.push({ value: deviceId, name: `${deviceId} (disconnected)`, disabled: true });
        }
        return items;
    });

    function formatsOf(d: MediaDeviceInfo): T[] {
        if (kind === 'camera') return videoFormats(d) as T[];
        if (kind === 'screen') return screenFormats(d) as T[];
        return audioFormats(d) as T[];
    }

    let formatItems = $derived(
        device ? formatsOf(device).map((f) => ({ value: formatKey(f), name: formatLabel(f) })) : []
    );

    let channelItems = $derived.by(() => {
        if (interfaceChannels <= 2) return [];
        const used = channelsInUse(draft, slotId, deviceId);
        return [
            { value: '', name: 'All channels' },
            ...Array.from({ length: interfaceChannels }, (_, i) => {
                const channel = i + 1;
                const by = used.get(channel);
                return {
                    value: String(channel),
                    name: by ? `Channel ${channel} (in use: ${by})` : `Channel ${channel}`,
                    disabled: by !== undefined,
                };
            }),
        ];
    });

    function chooseDevice(path: string) {
        const next = candidates.find((d) => d.devicePath === path);
        if (!next) return;
        if (kind === 'camera') {
            options = defaultVideo(next) as T | null;
        } else if (kind === 'screen') {
            options = defaultScreen(next) as T | null;
        } else {
            // A shared interface is read at the format its first user set.
            const first = sharedAudioFormat(draft, slotId, path);
            const base = first ? { ...first.options } : defaultAudio(next);
            if (base && deviceChannels(next) > 2) {
                const used = channelsInUse(draft, slotId, path);
                const free = Array.from({ length: deviceChannels(next) }, (_, i) => i + 1).find(
                    (c) => !used.has(c)
                );
                base.selectedChannel = free;
            } else if (base) {
                base.selectedChannel = undefined;
            }
            options = base as T | null;
        }
    }

    function chooseFormat(key: string) {
        if (!device) return;
        const format = formatsOf(device).find((f) => formatKey(f) === key);
        if (!format) return;
        const selectedChannel =
            options && 'selectedChannel' in options ? options.selectedChannel : undefined;
        options = (kind === 'mic' ? { ...format, selectedChannel } : { ...format }) as T;
    }

    function chooseChannel(value: string) {
        if (options && kind === 'mic') {
            options = { ...options, selectedChannel: value ? Number(value) : undefined } as T;
        }
    }

    let selectedChannel = $derived(
        options && 'selectedChannel' in options && options.selectedChannel
            ? String(options.selectedChannel)
            : ''
    );
</script>

<div class="space-y-1">
    <div class="flex flex-wrap items-center gap-2">
        <span class="w-20 shrink-0 text-sm font-medium text-gray-900">{label}</span>
        <div class="min-w-0 flex-1">
            <Label for="{id}-device" class="sr-only">{label} device</Label>
            <Select
                id="{id}-device"
                size="sm"
                items={deviceItems}
                value={deviceId}
                placeholder="Choose a device"
                onchange={(e: Event) => chooseDevice((e.target as HTMLSelectElement).value)}
            />
        </div>
        {#if channelItems.length}
            <div class="w-40">
                <Label for="{id}-channel" class="sr-only">{label} channel</Label>
                <Select
                    id="{id}-channel"
                    size="sm"
                    items={channelItems}
                    value={selectedChannel}
                    onchange={(e: Event) => chooseChannel((e.target as HTMLSelectElement).value)}
                />
            </div>
        {/if}
        <div class="w-56">
            <Label for="{id}-format" class="sr-only">{label} format</Label>
            <Select
                id="{id}-format"
                size="sm"
                items={formatItems}
                value={options && device ? formatKey(options) : ''}
                placeholder="Format"
                disabled={!device || shared !== null}
                onchange={(e: Event) => chooseFormat((e.target as HTMLSelectElement).value)}
            />
        </div>
        {#if stream !== undefined}
            <Toggle size="small" bind:checked={stream}>
                <span class="text-sm">Stream</span>
            </Toggle>
        {/if}
        {#if onremove}
            <Button color="alternative" size="xs" aria-label="Remove {label}" onclick={onremove}>
                <CloseOutline class="h-4 w-4" />
            </Button>
        {/if}
    </div>
    {#if shared && !error}
        <Helper>Format set by {shared.by}, which uses the same interface.</Helper>
    {/if}
    {#if error}
        <Helper color="red">{error}</Helper>
    {/if}
</div>
