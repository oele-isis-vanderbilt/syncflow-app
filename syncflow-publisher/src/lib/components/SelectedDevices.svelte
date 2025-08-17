<script lang="ts">
    import { Button } from "flowbite-svelte";
    import type { MediaDeviceInfo, PublishOptions } from "./types";

    let {
        selectedDevicesFn,
        allDevices,
    }: {
        selectedDevicesFn: () => PublishOptions[],
        allDevices: MediaDeviceInfo[],
    } = $props();

    let minimized = $state(true);

    function getDeviceName(
        devicePath: string,
    ) {
        const device = allDevices.find((d) => d.devicePath === devicePath);
        return device ? device.displayName: "Unknown Device";
    }

    $inspect({
        selectedDevicesFn,
        allDevices,
    });

</script>

<div class="bg-white rounded-2xl shadow-xl p-8 mt-2 border border-purple-100">
    <div class="flex justify-between items-center mb-6">
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
        <Button
            color="purple"
            outline
            class="ml-2"
            onclick={() => (minimized = !minimized)}
        >
            {minimized ? "Show" : "Minimize"}
        </Button>
    </div>
    {#if !minimized}
        <div class="space-y-4">
            {#each selectedDevicesFn() as device, index}
                {@const deviceName = device.kind == "Screen" ? getDeviceName(device.screenIdOrName) : getDeviceName(device.deviceId)}
                <div class="border border-purple-200 rounded-lg p-4 bg-purple-50">
                    <div class="flex items-center justify-between gap-2 mb-2">
                        <div class="flex items-center gap-2">
                            <span class="font-semibold text-purple-700">{device.kind}</span>
                            <span class="text-gray-600">({deviceName})</span>
                        </div>
                        <Button
                            color="red"
                            outline
                            size="xs"
                            onclick={() => {
                                // Add your delete logic here
                                console.log('Delete device at index:', index);
                            }}
                        >
                            Delete
                        </Button>
                    </div>
                    <div class="text-sm text-gray-700">
                        <div>
                            <span class="font-medium">Codec:</span>
                            <span>{device.codec ?? "N/A"}</span>
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
                        {#if device.kind !== "Audio" && 'width' in device && 'height' in device && device.width && device.height}
                        <div>
                            <span class="font-medium">Resolution:</span>
                            <span>{device.width}x{device.height}</span>
                        </div>
                        {/if}
                        {#if device.kind === "Audio" && 'channels' in device
                            && device.channels}
                        <div>
                            <span class="font-medium">Channels:</span>
                            <span>{device.channels} channels</span>
                        </div>
                        {/if}
                    </div>
                </div>
            {/each}
        </div>
    {/if}
</div>