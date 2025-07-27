import { goto } from "$app/navigation";
import { redirect } from "@sveltejs/kit";
import type { PageLoad } from "./$types";
import { invoke } from "@tauri-apps/api/core";

export const load: PageLoad = async () => {
    try {
        const registration = await invoke("get_registration");
        return {
            registration    
        };
    } catch (error) {
        console.log("No registration found, redirecting to register page.", error);
        redirect(302, "/register");
    }

}