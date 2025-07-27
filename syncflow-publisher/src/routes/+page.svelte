<script lang="ts">
    import { goto } from "$app/navigation";
    import { error } from "@sveltejs/kit";
    import { invoke } from "@tauri-apps/api/core";
    import { Button } from "flowbite-svelte";
    import type { PageProps } from './$types';

	  let { data }: PageProps = $props();

    let registrationDetails = $derived.by(() => data.registration);
 

    async function deregister() {
      try {
        await invoke("delete_registration");
        console.log("Deregistration successful, redirecting to register page.");
        goto("/register");
      } catch (err) {
        error(500, { message: `Deregistration failed: ${JSON.stringify(err)}` });
      }
    }


</script>

<main class="container mx-auto flex flex-col w-full justify-start p-4 gap-6 bg-gradient-to-br from-blue-50 via-white to-purple-100 min-h-screen">
  <div class="flex justify-between items-center bg-white rounded-xl shadow-lg px-6 py-4 mb-2">
    <h1 class="text-2xl font-extrabold text-purple-700 flex-1">
      Welcome to <span class="text-blue-600">SyncFlow Publisher</span>! {registrationDetails?.deviceName && `(${registrationDetails.deviceName})`}
    </h1>
    <Button
      color="red"
      class="ml-4 shadow hover:scale-105 transition-transform"
      onclick={deregister}
      >Delete Registration</Button>
  </div>
  {#if registrationDetails}
    <div class="bg-white rounded-2xl shadow-xl p-8 mt-2 border border-purple-100">
      <h2 class="text-xl font-bold mb-6 text-blue-700 flex items-center gap-2">
        <svg class="w-32 h-32 text-purple-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M13 16h-1v-4h-1m1-4h.01M12 20a8 8 0 100-16 8 8 0 000 16z"/></svg>
        Device & Project Details
      </h2>
      <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
          <div class="font-medium text-gray-700">Device ID:</div>
          <div class="text-gray-900 font-mono">{registrationDetails.deviceId}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
          <div class="font-medium text-gray-700">Device Name:</div>
          <div class="text-gray-900">{registrationDetails.deviceName}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
          <div class="font-medium text-gray-700">Device Group:</div>
          <div class="text-gray-900">{registrationDetails.deviceGroup}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
          <div class="font-medium text-gray-700">Project Name:</div>
          <div class="text-gray-900">{registrationDetails.projectName}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
          <div class="font-medium text-gray-700">Project ID:</div>
          <div class="text-gray-900 font-mono">{registrationDetails.projectId}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
          <div class="font-medium text-gray-700">Project Comments:</div>
          <div class="text-gray-900">{registrationDetails.projectComments}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
          <div class="font-medium text-gray-700">LiveKit Server URL:</div>
          <div class="text-gray-900 font-mono">{registrationDetails.lkServerUrl}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-blue-50 shadow-sm">
          <div class="font-medium text-gray-700">S3 Bucket Name:</div>
          <div class="text-gray-900">{registrationDetails.s3BucketName}</div>
        </div>
        <div class="flex flex-col gap-1 p-4 rounded-lg bg-purple-50 shadow-sm">
          <div class="font-medium text-gray-700">S3 Endpoint:</div>
          <div class="text-gray-900 font-mono">{registrationDetails.s3Endpoint}</div>
        </div>
      </div>
    </div>
  {:else}
    <div class="flex flex-col items-center justify-center mt-12">
      <svg class="w-16 h-16 text-gray-300 mb-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M12 8v4l3 3m6 0a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>
      <p class="text-lg text-gray-500">No registration details found.</p>
    </div>
  {/if}
</main>
