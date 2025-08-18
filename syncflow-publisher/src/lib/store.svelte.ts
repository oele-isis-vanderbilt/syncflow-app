import type { MediaDeviceInfo, PublishOptions } from './components/types';

export let devicesStore: {
    addDevice: (device: PublishOptions, alsoStream?: boolean) => void;
    removeDevice: (deviceId: string) => void;
    getSelectedDevices: () => PublishOptions[];
    getRemainingDevicesFn: () => () => MediaDeviceInfo[];
    getSelectedDevicesFn: () => () => PublishOptions[];
    getStreamingConfigFn: () => () => Record<string, boolean>;
    getFn: () => () => Record<string, PublishOptions>;
} | null = null;

export function initialize(avaliableDevices: MediaDeviceInfo[]) {
    let selectedDevicesStore: Record<string, PublishOptions> = $state({});
    let availableDevicesStore = $state(
        JSON.parse(JSON.stringify(avaliableDevices)) as MediaDeviceInfo[]
    );
    let streamingConfigStore = $state({}) as Record<string, boolean>;

    devicesStore = {
        addDevice: (device: PublishOptions, alsoStream = true) => {
            if (device.kind === 'Video' || device.kind === 'Audio') {
                availableDevicesStore = availableDevicesStore.filter(
                    (d) => d.devicePath !== device.deviceId
                );
                selectedDevicesStore[device.deviceId] = device;
                streamingConfigStore[device.deviceId] = alsoStream;
            } else if (device.kind === 'Screen') {
                availableDevicesStore = availableDevicesStore.filter(
                    (d) => d.devicePath !== device.screenIdOrName
                );
                selectedDevicesStore[device.screenIdOrName] = device;
                streamingConfigStore[device.screenIdOrName] = alsoStream;
            }
            selectedDevicesStore = { ...selectedDevicesStore }; // Trigger reactivity
            availableDevicesStore = [...availableDevicesStore]; // Trigger reactivity
            streamingConfigStore = { ...streamingConfigStore }; // Trigger reactivity
        },
        removeDevice: (deviceId: string) => {
            delete selectedDevicesStore[deviceId];
            availableDevicesStore.push(avaliableDevices.find((d) => d.devicePath === deviceId)!);
            selectedDevicesStore = { ...selectedDevicesStore }; // Trigger reactivity
            availableDevicesStore = [...availableDevicesStore]; // Trigger reactivity
            delete streamingConfigStore[deviceId];
            streamingConfigStore = { ...streamingConfigStore }; // Trigger reactivity
        },
        getRemainingDevicesFn: () => {
            return () => availableDevicesStore;
        },
        getSelectedDevices: () => Object.values(selectedDevicesStore),
        getSelectedDevicesFn: () => {
            return () => Object.values(selectedDevicesStore);
        },
        getStreamingConfigFn: () => {
            return () => streamingConfigStore;
        },
        getFn: () => {
            return () => selectedDevicesStore;
        },
    };
}
