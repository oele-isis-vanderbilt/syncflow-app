import type { ClientInit } from '@sveltejs/kit';

export const init: ClientInit = async () => {
    const { initialize } = await import('$lib/store.svelte');
    console.log("Initializing selectedDeviceStore");
    initialize();   
}