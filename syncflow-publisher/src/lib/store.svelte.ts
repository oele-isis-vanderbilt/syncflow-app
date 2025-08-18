import type { MediaDeviceInfo, PublishOptions } from './components/types';

export let devicesStore: {
    addDevice: (device: PublishOptions) => void;
    removeDevice: (deviceId: string) => void;
    getSelectedDevices: () => PublishOptions[];
    getRemainingDevicesFn: () => () => MediaDeviceInfo[];
    getSelectedDevicesFn: () => () => PublishOptions[];
    getFn: () => () => Record<string, PublishOptions>;
} | null = null;

export function initialize(avaliableDevices: MediaDeviceInfo[]) {
    let selectedDevicesStore: Record<string, PublishOptions> = $state({});
    let availableDevicesStore = $state(
        JSON.parse(JSON.stringify(avaliableDevices)) as MediaDeviceInfo[]
    );

    devicesStore = {
        addDevice: (device: PublishOptions) => {
            if (device.kind === 'Video' || device.kind === 'Audio') {
                availableDevicesStore = availableDevicesStore.filter(
                    (d) => d.devicePath !== device.deviceId
                );
                selectedDevicesStore[device.deviceId] = device;
            } else if (device.kind === 'Screen') {
                availableDevicesStore = availableDevicesStore.filter(
                    (d) => d.devicePath !== device.screenIdOrName
                );
                selectedDevicesStore[device.screenIdOrName] = device;
            }
            selectedDevicesStore = { ...selectedDevicesStore }; // Trigger reactivity
            availableDevicesStore = [...availableDevicesStore]; // Trigger reactivity
        },
        removeDevice: (deviceId: string) => {
            delete selectedDevicesStore[deviceId];
            availableDevicesStore.push(avaliableDevices.find((d) => d.devicePath === deviceId)!);
            selectedDevicesStore = { ...selectedDevicesStore }; // Trigger reactivity
            availableDevicesStore = [...availableDevicesStore]; // Trigger reactivity
        },
        getRemainingDevicesFn: () => {
            return () => availableDevicesStore;
        },
        getSelectedDevices: () => Object.values(selectedDevicesStore),
        getSelectedDevicesFn: () => {
            return () => Object.values(selectedDevicesStore);
        },
        getFn: () => {
            return () => selectedDevicesStore;
        },
    };
}
