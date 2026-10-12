<script lang="ts">
    import '@fontsource-variable/inter';
    import '../app.css';

    let { children } = $props();
</script>

<main class="mx-auto w-full max-w-6xl space-y-6 px-4 py-6">
    {@render children()}
</main>
