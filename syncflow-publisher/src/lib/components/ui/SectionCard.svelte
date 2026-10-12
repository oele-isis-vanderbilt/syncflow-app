<script lang="ts">
    import type { Component, Snippet } from 'svelte';
    import { Button, Card, Heading } from 'flowbite-svelte';
    import { ChevronDownOutline, ChevronUpOutline } from 'flowbite-svelte-icons';

    // Flowbite 1.x Card defaults to max-w-sm with no padding; this is the full-width,
    // padded section used on every screen.
    interface Props {
        title: string;
        icon?: Component;
        /** Shows a Show/Hide toggle for the body. */
        collapsible?: boolean;
        /** Initial state when collapsible. */
        collapsed?: boolean;
        actions?: Snippet;
        children: Snippet;
    }

    let {
        title,
        icon: Icon,
        collapsible = false,
        collapsed = false,
        actions,
        children,
    }: Props = $props();

    // svelte-ignore state_referenced_locally
    let isCollapsed = $state(collapsed);
    const bodyId = $props.id();
</script>

<Card size="xl" class="p-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <Heading tag="h2" class="flex w-auto items-center gap-2 text-xl">
            {#if Icon}<Icon class="h-6 w-6 shrink-0 text-primary-700" />{/if}
            {title}
        </Heading>
        <div class="flex items-center gap-2">
            {@render actions?.()}
            {#if collapsible}
                <Button
                    color="alternative"
                    size="xs"
                    aria-expanded={!isCollapsed}
                    aria-controls={bodyId}
                    onclick={() => (isCollapsed = !isCollapsed)}
                >
                    {#if isCollapsed}
                        <ChevronDownOutline class="me-1 h-4 w-4" />Show
                    {:else}
                        <ChevronUpOutline class="me-1 h-4 w-4" />Hide
                    {/if}
                </Button>
            {/if}
        </div>
    </div>
    {#if !isCollapsed}
        <div id={bodyId} class="mt-4">
            {@render children()}
        </div>
    {/if}
</Card>
