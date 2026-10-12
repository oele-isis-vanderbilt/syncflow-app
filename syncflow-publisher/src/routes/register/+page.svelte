<script lang="ts">
    import { goto } from '$app/navigation';
    import { invoke } from '@tauri-apps/api/core';
    import { A, Alert, Button, Heading, Helper, Input, Label, P, Spinner } from 'flowbite-svelte';
    import {
        DesktopPcOutline,
        ExclamationCircleOutline,
        ServerOutline,
        VideoCameraOutline,
    } from 'flowbite-svelte-icons';
    import { onMount } from 'svelte';
    import SectionCard from '#lib/components/ui/SectionCard.svelte';

    let credentials = $state({
        syncflowProjectId: '',
        syncflowApiKey: '',
        syncflowServerUrl: '',
        syncflowApiSecret: '',
        deviceName: '',
        deviceGroup: '',
        rabbitmqHost: '',
        rabbitmqPort: '',
        rabbitmqVhost: '',
        rabbitmqUsername: '',
        rabbitmqPassword: '',
    });

    let submitting = $state(false);
    let errorMessage = $state<string | null>(null);

    async function handleSubmit(event: Event) {
        event.preventDefault();
        submitting = true;
        errorMessage = null;
        try {
            await invoke('register_to_syncflow', {
                credentials: {
                    syncflowProjectId: credentials.syncflowProjectId,
                    syncflowApiKey: credentials.syncflowApiKey,
                    syncflowServerUrl: credentials.syncflowServerUrl,
                    syncflowApiSecret: credentials.syncflowApiSecret,
                    deviceName: credentials.deviceName === '' ? null : credentials.deviceName,
                    deviceGroup: credentials.deviceGroup,
                    rabbitmqHost: credentials.rabbitmqHost,
                    rabbitmqPort: credentials.rabbitmqPort,
                    rabbitmqVhost: credentials.rabbitmqVhost,
                    rabbitmqUsername: credentials.rabbitmqUsername,
                    rabbitmqPassword: credentials.rabbitmqPassword,
                },
            });
            goto('/');
        } catch (err) {
            errorMessage = `Registration failed: ${JSON.stringify(err)}`;
        } finally {
            submitting = false;
        }
    }

    onMount(() => {
        // Check if the device is already registered
        invoke('get_registration')
            .then((registration) => {
                console.log('Current registration:', registration);
                if (registration) {
                    goto('/');
                }
            })
            .catch((err) => {
                console.error('Failed to check registration:', err);
            });
    });
</script>

<section class="flex flex-col items-center gap-2 text-center">
    <img src="/syncflow-logo.svg" alt="SyncFlow logo" class="h-16 w-16" />
    <Heading tag="h1" class="text-2xl">Register this device</Heading>
    <P class="text-gray-500">Connect it to a SyncFlow project to receive sessions.</P>
</section>

<form class="space-y-6" onsubmit={handleSubmit}>
    <SectionCard title="SyncFlow Project" icon={VideoCameraOutline}>
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
                <Label for="syncflow-project-id" class="mb-2">Project ID</Label>
                <Input
                    id="syncflow-project-id"
                    type="text"
                    required
                    bind:value={credentials.syncflowProjectId}
                />
            </div>
            <div>
                <Label for="syncflow-server-url" class="mb-2">Server URL</Label>
                <Input
                    id="syncflow-server-url"
                    type="text"
                    required
                    bind:value={credentials.syncflowServerUrl}
                />
            </div>
            <div>
                <Label for="syncflow-api-key" class="mb-2">API Key</Label>
                <Input
                    id="syncflow-api-key"
                    type="text"
                    required
                    bind:value={credentials.syncflowApiKey}
                />
            </div>
            <div>
                <Label for="syncflow-api-secret" class="mb-2">API Secret</Label>
                <Input
                    id="syncflow-api-secret"
                    type="password"
                    required
                    bind:value={credentials.syncflowApiSecret}
                />
            </div>
        </div>
    </SectionCard>

    <SectionCard title="Device" icon={DesktopPcOutline}>
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
                <Label for="device-name" class="mb-2">Device Name</Label>
                <Input
                    id="device-name"
                    type="text"
                    aria-describedby="device-name-help"
                    bind:value={credentials.deviceName}
                />
                <Helper id="device-name-help" class="mt-1"
                    >Optional. Defaults to hostname-ip-address.</Helper
                >
            </div>
            <div>
                <Label for="device-group" class="mb-2">Device Group</Label>
                <Input
                    id="device-group"
                    type="text"
                    required
                    bind:value={credentials.deviceGroup}
                />
            </div>
        </div>
    </SectionCard>

    <SectionCard title="RabbitMQ" icon={ServerOutline}>
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div>
                <Label for="rabbitmq-host" class="mb-2">Host</Label>
                <Input
                    id="rabbitmq-host"
                    type="text"
                    required
                    bind:value={credentials.rabbitmqHost}
                />
            </div>
            <div>
                <Label for="rabbitmq-port" class="mb-2">Port</Label>
                <Input
                    id="rabbitmq-port"
                    type="number"
                    required
                    bind:value={credentials.rabbitmqPort}
                />
            </div>
            <div>
                <Label for="rabbitmq-vhost" class="mb-2">Virtual Host</Label>
                <Input
                    id="rabbitmq-vhost"
                    type="text"
                    required
                    bind:value={credentials.rabbitmqVhost}
                />
            </div>
            <div>
                <Label for="rabbitmq-username" class="mb-2">Username</Label>
                <Input
                    id="rabbitmq-username"
                    type="text"
                    required
                    bind:value={credentials.rabbitmqUsername}
                />
            </div>
            <div>
                <Label for="rabbitmq-password" class="mb-2">Password</Label>
                <Input
                    id="rabbitmq-password"
                    type="password"
                    required
                    bind:value={credentials.rabbitmqPassword}
                />
            </div>
        </div>
    </SectionCard>

    {#if errorMessage}
        <Alert color="red">
            {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
            {errorMessage}
        </Alert>
    {/if}

    <div class="flex justify-end gap-2">
        <Button color="alternative" onclick={() => goto('/')}>Go Home</Button>
        <Button type="submit" color="primary" disabled={submitting}>
            {#if submitting}<Spinner size="4" color="gray" class="me-2" />{/if}
            Register
        </Button>
    </div>
</form>

<footer class="text-center text-xs text-gray-500">
    &copy; {new Date().getFullYear()} OELE, ISIS Vanderbilt. All rights reserved.
    <br />
    <A
        href="https://teachableagents.org"
        target="_blank"
        rel="noopener noreferrer"
        class="text-primary-700 hover:underline"
    >
        Learn more at teachableagents.org
    </A>
</footer>
