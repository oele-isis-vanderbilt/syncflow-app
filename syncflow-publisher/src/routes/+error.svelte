<script lang="ts">
    import { page } from '$app/state';
    import { Alert, Heading, P } from 'flowbite-svelte';
    import { ExclamationCircleOutline } from 'flowbite-svelte-icons';
</script>

<Heading tag="h1" class="text-2xl">Error</Heading>
<Alert color="red">
    {#snippet icon()}<ExclamationCircleOutline class="h-5 w-5" />{/snippet}
    An error occurred: {page.data.error?.message ?? page.error?.message}
</Alert>
<P class="text-gray-500">Please try again later or contact support.</P>
