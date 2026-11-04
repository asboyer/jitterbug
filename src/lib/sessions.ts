import { invoke } from '@tauri-apps/api/core';
import type { OptimizedSession } from './types';

export function sessionClient(desktop: boolean) {
    function requireDesktop() {
        if (!desktop) throw new Error('Optimization sessions require the Jitterbug desktop app.');
    }
    async function command(name: string, arguments_: Record<string, unknown>) {
        requireDesktop();
        return invoke<OptimizedSession>(name, arguments_);
    }
    return {
        async list() {
            requireDesktop();
            return invoke<OptimizedSession[]>('list_sessions');
        },
        create: (name: string, goal: string) => command('create_session', { name, goal }),
        propose: (id: string) => command('propose_session', { id }),
        start: (id: string) => command('activate_session', { id, explicitOptIn: true }),
        pause: (id: string) => command('deactivate_session', { id }),
        reapply: (id: string) => command('reapply_session', { id, explicitOptIn: true }),
        restore: (id: string) => command('restore_session', { id }),
        remove: (id: string) => {
            requireDesktop();
            return invoke<void>('delete_session', { id });
        },
    };
}
