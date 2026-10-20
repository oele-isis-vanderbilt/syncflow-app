<script lang="ts">
    import { Alert, Button, Progressbar, Spinner } from 'flowbite-svelte';
    import { ExclamationCircleOutline, RefreshOutline } from 'flowbite-svelte-icons';
    import type { FileUpload } from '#lib/components/types.js';

    // One join's upload: an overall bar, one row per file, and a retry for failed files.
    interface Props {
        join: number;
        files: Record<string, FileUpload>;
        /** An upload of this join is running (files waiting for it aren't enough to tell). */
        uploading: boolean;
        onRetry: () => Promise<void>;
    }

    let { join, files, uploading, onRetry }: Props = $props();

    const LABEL = {
        pending: 'Waiting',
        uploading: 'Uploading',
        done: 'Uploaded',
        failed: 'Failed',
    };

    let entries = $derived(Object.entries(files));
    let totals = $derived(
        entries.reduce(
            (sum, [, f]) => ({
                uploaded: sum.uploaded + f.uploadedBytes,
                total: sum.total + f.totalBytes,
            }),
            { uploaded: 0, total: 0 }
        )
    );
    let failed = $derived(entries.filter(([, f]) => f.status === 'failed'));
    // Also true after a crash mid-upload, when files are left waiting and none failed.
    let incomplete = $derived(entries.some(([, f]) => f.status !== 'done'));
    let retrying = $state(false);
    let busy = $derived(uploading || retrying);

    function percent(uploaded: number, total: number): number {
        return total === 0 ? 100 : Math.floor((uploaded / total) * 100);
    }

    async function retry() {
        retrying = true;
        try {
            await onRetry();
        } finally {
            retrying = false;
        }
    }
</script>

<div class="space-y-3 rounded-lg border border-gray-200 p-3">
    <Progressbar
        progress={percent(totals.uploaded, totals.total)}
        labelOutside="Upload progress · join {join}"
        color="primary"
        size="h-4"
    />
    <ul class="space-y-2">
        {#each entries as [name, file] (name)}
            <li class="space-y-1">
                <div class="flex items-center justify-between gap-2 text-xs">
                    <span class="min-w-0 truncate font-mono text-gray-900">{name}</span>
                    <span class="shrink-0 text-gray-500">
                        {LABEL[file.status]}
                        {#if file.status === 'uploading'}
                            {percent(file.uploadedBytes, file.totalBytes)}%
                        {/if}
                    </span>
                </div>
                <Progressbar
                    progress={percent(file.uploadedBytes, file.totalBytes)}
                    color="primary"
                    size="h-2"
                />
            </li>
        {/each}
    </ul>
    {#if failed.length > 0}
        <Alert color="red">
            {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
            <span class="font-medium">
                {failed.length} file{failed.length === 1 ? '' : 's'} failed to upload.
            </span>
            The recording is kept on this device.
            <ul class="mt-1 list-inside list-disc">
                {#each failed as [name, file] (name)}
                    <li><span class="font-mono">{name}</span>: {file.error}</li>
                {/each}
            </ul>
        </Alert>
    {/if}
    {#if failed.length > 0 || (incomplete && !uploading)}
        <Button color="alternative" size="xs" disabled={busy} onclick={retry}>
            {#if busy}
                <Spinner size="4" color="gray" class="me-1" />
            {:else}
                <RefreshOutline class="me-1 h-4 w-4" />
            {/if}
            Retry upload
        </Button>
    {/if}
</div>
