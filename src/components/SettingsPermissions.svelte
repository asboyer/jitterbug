<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type { SessionPermissionActionId, SessionPermissionView } from '../lib/types';
    import SettingSwitch from './SettingSwitch.svelte';
    import Icon from './Icon.svelte';

    let { desktop }: { desktop: boolean } = $props();
    let view: SessionPermissionView | null = $state(null);
    let error = $state('');
    let status = $state('');
    let busy = $state(false);

    async function load() {
        if (!desktop) return;
        try {
            view = await invoke<SessionPermissionView>('get_session_permissions');
            error = '';
        } catch (reason) {
            error = errorMessage(reason);
        }
    }

    async function save() {
        if (!desktop || !view) return;
        busy = true;
        error = '';
        const enabled: Partial<Record<SessionPermissionActionId, boolean>> = {};
        for (const group of view.groups)
            for (const action of group.actions) enabled[action.id] = action.session_enabled;
        try {
            view = await invoke<SessionPermissionView>('save_session_permissions', { enabled });
            status = 'Session rights saved for this PC.';
        } catch (reason) {
            error = errorMessage(reason);
        } finally {
            busy = false;
        }
    }

    onMount(() => {
        void load();
    });
</script>

<section class="preference-card" aria-label="Session optimization rights">
    <div class="preference-heading">
        <span class="preference-icon"><Icon name="settings" size={20} /></span>
        <div>
            <h2>Session rights</h2>
            <p>
                {#if view}This {view.environment.os}
                    {view.environment.version} ({view.environment.arch}).{:else}Rights for this PC.{/if}
                Off until you allow a group. Sessions can only use compiled, reversible actions that exist
                on this OS.
            </p>
        </div>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if status}<p class="success" role="status">{status}</p>{/if}
    {#if view}
        {#each view.groups as group}
            <details class="preference-detail" open={group.id === 'power_and_sleep'}>
                <summary><span class="detail-name">{group.label}</span></summary>
                <div class="detail-body">
                    <p class="muted">{group.detail}</p>
                    {#each group.actions as action}
                        <SettingSwitch
                            title={action.label}
                            description={action.status === 'available'
                                ? action.detail
                                : `${action.status.replaceAll('_', ' ')} — ${action.detail}`}
                            bind:checked={action.session_enabled}
                            disabled={!desktop || busy || action.status !== 'available'}
                        />
                    {/each}
                </div>
            </details>
        {/each}
        <button class="primary" disabled={!desktop || busy} onclick={save}
            >{busy ? 'Saving…' : 'Save session rights'}</button
        >
    {/if}
</section>
