<script lang="ts">
    import { Button } from "flowbite-svelte";
    import DeviceSelector from "./DeviceSelector.svelte";
    import type { MediaDeviceInfo, PublishOptions } from "./types";

    let { devices, onAddDevice, onRemoveDevice }: {
        devices: MediaDeviceInfo[],
        onAddDevice: (option: PublishOptions) => void,
        onRemoveDevice: (id: string) => void,
    } = $props();
    let minimized = $state(true);
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
            Media Devices and Codecs
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
        {#each devices as device, index (index)}
            <DeviceSelector device={device} addDevice={onAddDevice} removeDevice={onRemoveDevice}/>
        {/each}
    {/if}
</div>
