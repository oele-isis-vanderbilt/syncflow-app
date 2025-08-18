<script lang="ts">
    import { goto } from '$app/navigation';
    import { error } from '@sveltejs/kit';
    import { invoke } from '@tauri-apps/api/core';
    import { Input, Label, Button } from 'flowbite-svelte';
    import { onMount } from 'svelte';

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

    async function handleSubmit(event: Event) {
        event.preventDefault();
        try {
            const result = await invoke('register_to_syncflow', {
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
            error(500, {
                message: `Registration failed: ${JSON.stringify(err)}`,
            });
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

<main
    class="container mx-auto flex flex-col w-full justify-start p-2 gap-8 min-h-screen bg-gradient-to-br from-blue-50 via-white to-purple-100"
>
    <section class="flex flex-col items-center gap-2 mt-8">
        <img src="/syncflow-logo.svg" alt="SyncFlow Logo" class="w-20 h-20 mb-2 drop-shadow-lg" />
        <h1 class="text-4xl font-extrabold text-blue-700 drop-shadow-sm">Welcome to SyncFlow!</h1>
        <h2 class="text-xl font-semibold text-gray-700">Register your device to get started</h2>
    </section>
    <form
        class="flex flex-col gap-6 w-full max-w-2xl mx-auto bg-white/80 p-8 rounded-2xl shadow-xl border border-blue-100 backdrop-blur"
    >
        <div class="grid grid-cols-1 gap-5">
            <div>
                <Label
                    for="syncflow-project-id"
                    class="block text-sm font-medium text-blue-900 mb-1"
                >
                    Syncflow Project ID
                </Label>
                <Input
                    id="syncflow-project-id"
                    type="text"
                    required
                    bind:value={credentials.syncflowProjectId}
                    placeholder="Enter your Syncflow Project ID"
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label
                    for="syncflow-server-url"
                    class="block text-sm font-medium text-blue-900 mb-1"
                >
                    Syncflow Server URL
                </Label>
                <Input
                    id="syncflow-server-url"
                    type="text"
                    bind:value={credentials.syncflowServerUrl}
                    placeholder="Enter your Syncflow Server URL"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="syncflow-api-key" class="block text-sm font-medium text-blue-900 mb-1">
                    Syncflow API Key
                </Label>
                <Input
                    id="syncflow-api-key"
                    type="text"
                    bind:value={credentials.syncflowApiKey}
                    placeholder="Enter your Syncflow API Key"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label
                    for="syncflow-api-secret"
                    class="block text-sm font-medium text-blue-900 mb-1"
                >
                    Syncflow API Secret
                </Label>
                <Input
                    id="syncflow-api-secret"
                    type="text"
                    bind:value={credentials.syncflowApiSecret}
                    placeholder="Enter your Syncflow API Secret"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="device-name" class="block text-sm font-medium text-blue-900 mb-1">
                    Device Name
                </Label>
                <Input
                    id="device-name"
                    type="text"
                    bind:value={credentials.deviceName}
                    placeholder="Enter Device Name (Optional, defaults to hostname-ip-address)"
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="device-group" class="block text-sm font-medium text-blue-900 mb-1">
                    Device Group
                </Label>
                <Input
                    id="device-group"
                    type="text"
                    bind:value={credentials.deviceGroup}
                    placeholder="Enter Device Group"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="rabbitmq-username" class="block text-sm font-medium text-blue-900 mb-1">
                    RabbitMQ Username
                </Label>
                <Input
                    id="rabbitmq-username"
                    type="text"
                    bind:value={credentials.rabbitmqUsername}
                    placeholder="Enter RabbitMQ Username"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="rabbitmq-host" class="block text-sm font-medium text-blue-900 mb-1">
                    RabbitMQ Host
                </Label>
                <Input
                    id="rabbitmq-host"
                    type="text"
                    bind:value={credentials.rabbitmqHost}
                    placeholder="Enter RabbitMQ Host"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="rabbitmq-port" class="block text-sm font-medium text-blue-900 mb-1">
                    RabbitMQ Port
                </Label>
                <Input
                    id="rabbitmq-port"
                    type="number"
                    bind:value={credentials.rabbitmqPort}
                    placeholder="Enter RabbitMQ Port"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="rabbitmq-vhost" class="block text-sm font-medium text-blue-900 mb-1">
                    RabbitMQ Virtual Host
                </Label>
                <Input
                    id="rabbitmq-vhost"
                    type="text"
                    bind:value={credentials.rabbitmqVhost}
                    placeholder="Enter RabbitMQ Virtual Host"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
            <div>
                <Label for="rabbitmq-password" class="block text-sm font-medium text-blue-900 mb-1">
                    RabbitMQ Password
                </Label>
                <Input
                    id="rabbitmq-password"
                    type="password"
                    bind:value={credentials.rabbitmqPassword}
                    placeholder="Enter RabbitMQ Password"
                    required
                    class="focus:ring-blue-500 focus:border-blue-500"
                />
            </div>
        </div>
        <Button
            onclick={handleSubmit}
            class="bg-gradient-to-r from-blue-500 to-purple-500 text-white font-bold py-2 px-4 rounded-lg shadow hover:scale-105 transition-transform duration-150"
        >
            Register
        </Button>
    </form>
    <footer class="text-center text-xs text-gray-400 mt-12">
        &copy; {new Date().getFullYear()} OELE, ISIS Vanderbilt. All rights reserved.
        <br />
        <a
            href="https://teachableagents.org"
            target="_blank"
            rel="noopener noreferrer"
            class="text-blue-500 hover:underline"
        >
            Learn more at teachableagents.org
        </a>
    </footer>
</main>
