<script lang="ts">
    import { Tooltip, Clipboard } from "flowbite-svelte";
    import { CheckOutline, ClipboardCleanSolid } from "flowbite-svelte-icons";

    let { contents }: { contents: string } = $props();
</script>

<Clipboard value={contents} color="secondary" size="xs">
    {#snippet children(success)}
        <Tooltip isOpen={success}>{success ? "Copied" : "Copy to clipboard"}</Tooltip>
        {#if success}<CheckOutline />{:else}<ClipboardCleanSolid />{/if}
    {/snippet}
</Clipboard>
