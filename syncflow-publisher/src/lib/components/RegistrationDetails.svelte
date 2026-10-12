<script lang="ts">
    import { InfoCircleOutline } from 'flowbite-svelte-icons';
    import Clipboard from '#lib/components/Clipboard.svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';
    import type { RegistrationResponse } from '#lib/components/types.js';

    let {
        registrationDetails,
    }: {
        registrationDetails: RegistrationResponse;
    } = $props();

    let fields = $derived([
        { label: 'Device ID', value: registrationDetails.deviceId, mono: true },
        { label: 'Device Name', value: registrationDetails.deviceName, mono: false },
        { label: 'Device Group', value: registrationDetails.deviceGroup, mono: false },
        { label: 'Project Name', value: registrationDetails.projectName, mono: false },
        { label: 'Project ID', value: registrationDetails.projectId, mono: true },
        { label: 'Project Comments', value: registrationDetails.projectComments, mono: false },
        { label: 'LiveKit Server URL', value: registrationDetails.lkServerUrl, mono: true },
        { label: 'S3 Bucket Name', value: registrationDetails.s3BucketName, mono: false },
        { label: 'S3 Endpoint', value: registrationDetails.s3Endpoint, mono: true },
    ]);
</script>

<SectionCard title="Device & Project Details" icon={InfoCircleOutline} collapsible collapsed>
    <dl class="grid grid-cols-1 gap-4 md:grid-cols-2">
        {#each fields as field (field.label)}
            <div class="min-w-0">
                <dt class="text-sm text-gray-500">{field.label}</dt>
                <dd class="flex items-center gap-2 text-gray-900">
                    <span class="break-all {field.mono ? 'font-mono text-sm' : ''}">
                        {field.value}
                    </span>
                    <Clipboard contents={field.value} label={field.label} />
                </dd>
            </div>
        {/each}
    </dl>
</SectionCard>
