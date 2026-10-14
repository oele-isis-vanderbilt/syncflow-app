<script lang="ts">
    import { DesktopPcOutline } from 'flowbite-svelte-icons';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import SessionStatus, { type Status } from './SessionStatus.svelte';
    import type {
        AudioPublishOptions,
        CompositeConfig,
        MediaDeviceInfo,
        PublicationNotification,
    } from './types';
    import { cameraLabel, draftFromConfig, micLabel } from '#lib/compositeConfig.js';
    import { deviceIdOf, deviceLabel, formatLabel } from '#lib/publishOptions.js';
    import { failedOutputs, failedStreams } from '#lib/notifications.js';

    // The saved recording setup, one row per output, with what happened to it in the
    // current recording.
    interface Props {
        config: CompositeConfig;
        devices: MediaDeviceInfo[];
        /** Notifications of the current or most recent recording. */
        notifications: PublicationNotification[];
    }

    let { config, devices, notifications }: Props = $props();

    interface Row {
        output: string;
        files: string;
        source: string;
        status: Status;
    }

    function name(options: { deviceId: string } | { screenIdOrName: string }): string {
        const id = deviceIdOf(options as never);
        const device = devices.find((d) => d.devicePath === id);
        return device ? deviceLabel(device, devices) : id;
    }

    function micName(options: AudioPublishOptions): string {
        return options.selectedChannel
            ? `${name(options)} · ch ${options.selectedChannel}`
            : name(options);
    }

    let rows = $derived.by(() => {
        const failed = failedOutputs(notifications);
        const stopped = failedStreams(notifications);
        const draft = draftFromConfig(config);
        const status = (output: string, streams: string[], stream: boolean): Status => {
            if (failed.has(output)) return 'failed';
            return stream && !streams.every((s) => stopped.has(s)) ? 'streaming' : 'recordOnly';
        };
        const result: Row[] = [];
        if (config.avMix) {
            const mix = config.avMix;
            const mics = [mix.mic1, ...(mix.mic2 ? [mix.mic2] : [])];
            result.push({
                output: 'av-mix',
                files: `av-mix.mp4 · ${mics.map((_, i) => `mic-${i + 1}`).join(', ')}.m4a`,
                source: [name(mix.camera), ...mics.map(micName)].join(' · '),
                status: status('av-mix', ['av-mix-video', 'av-mix-audio'], mix.stream),
            });
        }
        if (config.screen) {
            result.push({
                output: 'screen-1',
                files: 'screen-1.mp4',
                source: `${name(config.screen.options)} · ${formatLabel(config.screen.options)}`,
                status: status('screen-1', ['screen-1'], config.screen.stream),
            });
        }
        config.cameras.forEach((camera, i) => {
            const label = cameraLabel(draft, i);
            result.push({
                output: label,
                files: `${label}.mp4`,
                source: `${name(camera.options)} · ${formatLabel(camera.options)}`,
                status: status(label, [label], camera.stream),
            });
        });
        config.mics.forEach((mic, i) => {
            const label = micLabel(draft, i);
            result.push({
                output: label,
                files: `${label}.m4a`,
                source: micName(mic.options),
                status: status(label, [label], mic.stream),
            });
        });
        return result;
    });
</script>

<SectionCard title="Recording setup" icon={DesktopPcOutline} collapsible>
    <ul class="space-y-2">
        {#each rows as row (row.output)}
            <li
                class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-gray-200 p-3"
            >
                <div class="min-w-0">
                    <p class="font-medium text-gray-900">{row.files}</p>
                    <p class="truncate text-sm text-gray-500">{row.source}</p>
                </div>
                <SessionStatus status={row.status} />
            </li>
        {/each}
    </ul>
</SectionCard>
