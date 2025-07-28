<script lang="ts">
    import type { MediaDeviceInfo, PublishOptions } from "./types";
    import { Select } from "flowbite-svelte";

    let { device, addDevice, removeDevice }: { 
        device: MediaDeviceInfo,
        addDevice: (option: PublishOptions) => void,
        removeDevice: (id: string) => void,
    } = $props();

    let publishOptions: PublishOptions[] = $derived.by(() => {
        if (device.deviceClass === "Video/Source") {
            return device.capabilities
                .filter((capability) => capability.kind === "Video")
                .flatMap((capability) =>
                    capability.framerates.map((framerate) => ({
                        kind: "Video",
                        codec: capability.codec,
                        device_id: device.devicePath,
                        width: capability.width,
                        height: capability.height,
                        framerate: framerate,
                    })),
                );
        } else if (device.deviceClass === "Audio/Source") {
            let options = device.capabilities
                .filter((capability) => capability.kind === "Audio")
                .flatMap((capability) =>
                    capability.framerates.map((framerate) => ({
                        kind: "Audio",
                        codec: capability.codec,
                        device_id: device.devicePath,
                        channels: capability.channels,
                        framerate: framerate,
                    })),
                );

            let maxFramerate = Math.max(...options.map((opt) => opt.framerate));
            let capsWithMaxFramerate = options.find(
                (opt) => opt.framerate === maxFramerate,
            );
            if (maxFramerate > 48000) {
                options.push({
                    kind: "Audio",
                    codec: "audio/x-raw",
                    device_id: device.devicePath,
                    channels: capsWithMaxFramerate?.channels || 1,
                    framerate: 48000,
                });
                options.push({
                    kind: "Audio",
                    codec: "audio/x-raw",
                    device_id: device.devicePath,
                    channels: capsWithMaxFramerate?.channels || 1,
                    framerate: 44100,
                });
            }

            return options;
        } else if (device.deviceClass === "Screen/Source") {
            return device.capabilities
                .filter((capability) => capability.kind === "Screen")
                .flatMap((capability) => {
                    return capability.framerates.map((framerate) => ({
                        kind: "Screen",
                        device_id: device.devicePath,
                        width: capability.width,
                        height: capability.height,
                        framerate: framerate,
                    }));
                });
        }
    });

    let selectedOption: PublishOptions | null = $state(null);

    function optionLabel(option: PublishOptions): string {
        if (option.kind === "Video") {
            return `${option.codec} ${option.width}x${option.height} @ ${option.framerate}fps`;
        }
        if (option.kind === "Audio") {
            return `${option.codec} ${option.channels}ch @ ${option.framerate}fps`;
        }
        if (option.kind === "Screen") {
            return `Screen ${option.width}x${option.height} @ ${option.framerate}fps`;
        }
        return "Unknown Option";
    }

    let selectOptions = $derived.by(() => {
        return publishOptions.map((option) => ({
            value: option,
            name: optionLabel(option),
        }));
    });
</script>

<div class="bg-white rounded-2xl shadow-xl p-8 mt-2 border border-purple-100">
    <div class="flex justify-between items-center gap-2">
        <h3 class="text-lg font-semibold text-blue-700">
            {device.displayName} ({device.deviceClass})
        </h3>
        <Select
            bind:value={selectedOption}
            class="m-w-64 flex-1"
            items={selectOptions}
            placeholder="Select Publish Option"
            clearable
            onClear={() => {
                console.log("Selection cleared", selectedOption);
            }}
        />
    </div>
</div>
