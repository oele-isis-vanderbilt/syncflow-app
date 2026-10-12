<script lang="ts">
    import { Alert, Button, Modal, P, Spinner } from 'flowbite-svelte';
    import { ExclamationCircleOutline } from 'flowbite-svelte-icons';

    // Confirmation step for destructive actions. Keeps the dialog open with a
    // spinner while `onConfirm` runs, and shows its error inline if it throws.
    interface Props {
        open: boolean;
        title: string;
        message: string;
        confirmLabel: string;
        onConfirm: () => Promise<void> | void;
    }

    let { open = $bindable(), title, message, confirmLabel, onConfirm }: Props = $props();

    let pending = $state(false);
    let errorMessage = $state<string | null>(null);

    async function confirm() {
        pending = true;
        errorMessage = null;
        try {
            await onConfirm();
            open = false;
        } catch (err) {
            errorMessage = typeof err === 'string' ? err : JSON.stringify(err);
        } finally {
            pending = false;
        }
    }

    $effect(() => {
        if (!open) errorMessage = null;
    });
</script>

<Modal bind:open {title} size="sm" dismissable={!pending}>
    <P>{message}</P>
    {#if errorMessage}
        <Alert color="red" class="mt-4">
            {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
            {errorMessage}
        </Alert>
    {/if}
    {#snippet footer()}
        <Button color="red" disabled={pending} onclick={confirm}>
            {#if pending}<Spinner size="4" color="gray" class="me-2" />{/if}
            {confirmLabel}
        </Button>
        <Button color="alternative" disabled={pending} onclick={() => (open = false)}>
            Cancel
        </Button>
    {/snippet}
</Modal>
