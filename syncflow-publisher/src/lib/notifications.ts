// Readable text for the failures and warnings the backend reports during a recording.
import type { PublicationNotification } from '#lib/components/types.js';

export interface Problem {
    color: 'red' | 'yellow';
    title: string;
    detail: string;
}

/** What to tell the user about `notification`, or `null` if it isn't a problem. */
export function describeProblem(notification: PublicationNotification): Problem | null {
    switch (notification.kind) {
        case 'failure':
            return { color: 'red', title: 'Recording problem', detail: notification.reason };
        case 'deviceFailure': {
            const { device } = notification;
            const channel = device.selectedChannel ? ` · ch ${device.selectedChannel}` : '';
            const when = notification.phase === 'start' ? 'could not be opened' : 'stopped';
            return {
                color: 'red',
                title: `${device.role} (${device.displayName}${channel}) ${when}`,
                detail: `${notification.reason} ${
                    notification.recordingContinues
                        ? 'Other devices are still recording.'
                        : 'Nothing is being recorded anymore.'
                }`,
            };
        }
        case 'outputFailure':
            return {
                color: 'red',
                title: `${notification.output} file failed`,
                detail: `${notification.reason} ${
                    notification.recordingContinues ? 'Other files are still recording.' : ''
                }`.trim(),
            };
        case 'streamFailure':
            return {
                color: 'red',
                title: `${notification.sink} stream stopped`,
                detail: `${notification.reason} Its file is still recording.`,
            };
        case 'pipelineWarning':
            return {
                color: 'yellow',
                title: notification.scope ? `Warning (${notification.scope})` : 'Warning',
                detail: notification.message,
            };
        default:
            return null;
    }
}

/** Outputs that stopped recording: every device feeding them failed, or the file failed. */
export function failedOutputs(notifications: PublicationNotification[]): Set<string> {
    const failed = new Set<string>();
    for (const n of notifications) {
        if (n.kind === 'outputFailure') failed.add(n.output);
        // The AV mix keeps recording while any of its devices works; single-device outputs
        // end with their device.
        if (n.kind === 'deviceFailure')
            n.outputs.filter((o) => o !== 'av-mix').forEach((o) => failed.add(o));
    }
    return failed;
}

/** Streams that stopped. */
export function failedStreams(notifications: PublicationNotification[]): Set<string> {
    return new Set(notifications.flatMap((n) => (n.kind === 'streamFailure' ? [n.sink] : [])));
}
