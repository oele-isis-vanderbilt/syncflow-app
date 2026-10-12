<script lang="ts">
    import { Tooltip, Clipboard } from 'flowbite-svelte';
    import { CheckOutline, ClipboardCleanSolid } from 'flowbite-svelte-icons';

    let { contents, label = 'value' }: { contents: string; label?: string } = $props();
</script>

<Clipboard value={contents} color="secondary" size="xs" aria-label="Copy {label}">
    {#snippet children(success)}
        <Tooltip isOpen={success}>{success ? 'Copied' : 'Copy to clipboard'}</Tooltip>
        {#if success}<CheckOutline />{:else}<ClipboardCleanSolid />{/if}
    {/snippet}
</Clipboard>
