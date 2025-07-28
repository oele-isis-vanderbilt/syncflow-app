import type { PublishOptions } from "./components/types"

export let selectedDeviceStore: {
    addDevice: (device: PublishOptions) => void;
    removeDevice: (deviceId: string) => void;
    getSelectedDevices: () => PublishOptions[];
} | null = null;


export function initialize() {
    let selectedDevices: Record<string, PublishOptions> = $state({});

    selectedDeviceStore = {
        addDevice: (device: PublishOptions) => {
            if (device.kind === 'Video' || device.kind === 'Audio') {
                selectedDevices[device.deviceId] = device;
            } else if (device.kind === 'Screen') {
                selectedDevices[device.screenIdOrName] = device;
            }
            selectedDevices = { ...selectedDevices }; // Trigger reactivity
            console.log("Device added:", device, selectedDevices);
        },
        removeDevice: (deviceId: string) => {
            delete selectedDevices[deviceId];
        },
        getSelectedDevices: () => Object.values(selectedDevices)
    }

}