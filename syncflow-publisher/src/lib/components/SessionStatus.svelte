<script lang="ts" module>
    export type Status =
        | 'listening'
        | 'joined'
        | 'available'
        | 'ended'
        | 'unavailable'
        | 'recording'
        | 'completed'
        | 'streaming'
        | 'recordOnly'
        | 'failed';
</script>

<script lang="ts">
    import { Badge, Indicator } from 'flowbite-svelte';

    // The only place status colors and labels are chosen; see docs/design-rules.md.
    const STATUS: Record<
        Status,
        { color: 'green' | 'blue' | 'gray' | 'yellow' | 'red'; label: string; live?: boolean }
    > = {
        listening: { color: 'green', label: 'Listening', live: true },
        joined: { color: 'green', label: 'Joined', live: true },
        available: { color: 'blue', label: 'Available' },
        ended: { color: 'gray', label: 'Ended' },
        unavailable: { color: 'yellow', label: 'Unavailable' },
        recording: { color: 'red', label: 'Recording', live: true },
        completed: { color: 'green', label: 'Completed' },
        streaming: { color: 'green', label: 'Streaming' },
        recordOnly: { color: 'gray', label: 'Record only' },
        failed: { color: 'red', label: 'Failed' },
    };

    let { status }: { status: Status } = $props();
    let config = $derived(STATUS[status]);
</script>

<Badge color={config.color} class="gap-1.5">
    {#if config.live}
        <Indicator color={config.color} size="xs" class="animate-pulse" />
    {/if}
    {config.label}
</Badge>
