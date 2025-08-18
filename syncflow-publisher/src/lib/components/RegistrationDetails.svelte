<script lang="ts">
    import { Button } from 'flowbite-svelte';
    import Clipboard from '$lib/components/Clipboard.svelte';
    import type { RegistrationResponse } from '$lib/components/types';

    let {
        registrationDetails,
    }: {
        registrationDetails: RegistrationResponse;
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
                ><path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M13 16h-1v-4h-1m1-4h.01M12 20a8 8 0 100-16 8 8 0 000 16z"
                /></svg
            >
            Device & Project Details
        </h2>
        <Button color="purple" outline class="ml-2" onclick={() => (minimized = !minimized)}>
            {minimized ? 'Show' : 'Minimize'}
        </Button>
    </div>
    {#if !minimized}
        <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
                <div class="font-medium text-gray-700">Device ID:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900 font-mono">{registrationDetails.deviceId}</span>
                    <Clipboard contents={registrationDetails.deviceId} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
                <div class="font-medium text-gray-700">Device Name:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900">{registrationDetails.deviceName}</span>
                    <Clipboard contents={registrationDetails.deviceName} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
                <div class="font-medium text-gray-700">Device Group:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900">{registrationDetails.deviceGroup}</span>
                    <Clipboard contents={registrationDetails.deviceGroup} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
                <div class="font-medium text-gray-700">Project Name:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900">{registrationDetails.projectName}</span>
                    <Clipboard contents={registrationDetails.projectName} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
                <div class="font-medium text-gray-700">Project ID:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900 font-mono">{registrationDetails.projectId}</span>
                    <Clipboard contents={registrationDetails.projectId} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
                <div class="font-medium text-gray-700">Project Comments:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900">{registrationDetails.projectComments}</span>
                    <Clipboard contents={registrationDetails.projectComments} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
                <div class="font-medium text-gray-700">LiveKit Server URL:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900 font-mono">{registrationDetails.lkServerUrl}</span>
                    <Clipboard contents={registrationDetails.lkServerUrl} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
                <div class="font-medium text-gray-700">S3 Bucket Name:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900">{registrationDetails.s3BucketName}</span>
                    <Clipboard contents={registrationDetails.s3BucketName} />
                </div>
            </div>
            <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
                <div class="font-medium text-gray-700">S3 Endpoint:</div>
                <div class="flex items-center gap-2">
                    <span class="text-gray-900 font-mono">{registrationDetails.s3Endpoint}</span>
                    <Clipboard contents={registrationDetails.s3Endpoint} />
                </div>
            </div>
        </div>
    {/if}
</div>
