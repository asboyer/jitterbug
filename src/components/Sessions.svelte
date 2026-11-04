<script lang="ts">
    import { listen } from '@tauri-apps/api/event';
    import { onMount, tick } from 'svelte';
    import type { OptimizedSession, OptimizedSettingState } from '../lib/types';
    import { sessionClient } from '../lib/sessions';
    import Icon from './Icon.svelte';

    let { desktop = false }: { desktop?: boolean } = $props();
    const client = sessionClient(true);
    let sessions = $state<OptimizedSession[]>([]);
    let selectedId = $state('');
    let composerOpen = $state(false);
    let nameInput: HTMLInputElement | undefined = $state();
    $effect(() => {
        if (composerOpen) void tick().then(() => nameInput?.focus());
    });
    let name = $state('');
    let goal = $state('');
    let busy = $state(false);
    let loading = $state(true);
    let error = $state('');
    let notice = $state('');
    let deleteConfirmId = $state('');
    const selected = $derived(sessions.find((session) => session.id === selectedId) ?? sessions[0]);

    onMount(() => {
        void refresh();
        let unlisten: (() => void) | undefined;
        if (desktop)
            listen(
                'telemetry',
                () => selected?.status === 'monitoring' && void refresh(false)
            ).then((stop) => (unlisten = stop));
        return () => unlisten?.();
    });

    async function refresh(showLoading = true) {
        if (!desktop) {
            loading = false;
            error = 'Open the Jitterbug desktop app to use optimization sessions.';
            return;
        }
        if (showLoading) loading = true;
        try {
            sessions = await client.list();
            if (!sessions.some((session) => session.id === selectedId))
                selectedId = sessions[0]?.id ?? '';
            error = '';
        } catch (reason) {
            error = String(reason);
        } finally {
            if (showLoading) loading = false;
        }
    }

    function update(changed: OptimizedSession) {
        sessions = [changed, ...sessions.filter((session) => session.id !== changed.id)];
        selectedId = changed.id;
    }

    async function run(operation: () => Promise<OptimizedSession>, success: string) {
        busy = true;
        error = '';
        try {
            update(await operation());
            notice = success;
        } catch (reason) {
            error = String(reason);
        } finally {
            busy = false;
        }
    }

    async function create() {
        await run(
            () => client.create(name.trim(), goal.trim()),
            'AI profile proposed. Review it, then start the real task to establish its baseline.'
        );
        if (!error) {
            composerOpen = false;
            name = '';
            goal = '';
        }
    }

    async function remove(session: OptimizedSession) {
        if (deleteConfirmId !== session.id) {
            deleteConfirmId = session.id;
            return;
        }
        busy = true;
        error = '';
        try {
            await client.remove(session.id);
            sessions = sessions.filter((item) => item.id !== session.id);
            selectedId = sessions[0]?.id ?? '';
            deleteConfirmId = '';
            notice = 'Session deleted.';
        } catch (reason) {
            error = String(reason);
        } finally {
            busy = false;
        }
    }

    function settingName(setting: OptimizedSettingState['setting']) {
        switch (setting) {
            case 'keep_awake':
                return 'Keep this PC awake';
            case 'stage_manager':
                return 'Stage Manager';
            case 'dock_autohide':
                return 'Dock auto-hide';
            case 'dock_magnification':
                return 'Dock magnification';
            case 'dock_animate':
                return 'Animate opening applications';
            case 'dock_minimize_to_application':
                return 'Minimize into application icon';
            case 'dock_show_recents':
                return 'Show recent applications';
            case 'press_and_hold':
                return 'Press-and-hold accent menu';
            case 'natural_scroll':
                return 'Natural scrolling';
            case 'automatic_spelling':
                return 'Automatic spelling correction';
            case 'automatic_capitalization':
                return 'Automatic capitalization';
            case 'automatic_period':
                return 'Automatic period substitution';
            case 'automatic_dash':
                return 'Smart dashes';
            case 'automatic_quote':
                return 'Smart quotes';
            case 'ds_store_network':
                return 'Avoid .DS_Store on network volumes';
            case 'ds_store_usb':
                return 'Avoid .DS_Store on USB volumes';
            case 'mouse_linear':
                return 'Mouse linear motion';
            case 'icloud_optimize_storage':
                return 'iCloud Optimize Mac Storage';
            default:
                return setting;
        }
    }
    function settingValue(value: boolean) {
        return value ? 'On' : 'Off';
    }
    function settingRows(session: OptimizedSession) {
        const keys = [
            ...session.original_configuration,
            ...session.best_configuration,
            ...session.applied_configuration,
        ].map((item) => item.setting);
        const unique = [...new Set(keys)];
        return unique.map((setting) => {
            const original = session.original_configuration.find(
                (item) => item.setting === setting
            );
            const saved = session.best_configuration.find((item) => item.setting === setting);
            const applied = session.applied_configuration.find((item) => item.setting === setting);
            return {
                setting,
                original: original ? settingValue(original.value) : '—',
                saved: saved ? settingValue(saved.value) : 'Not saved',
                applied: applied ? settingValue(applied.value) : 'Not applied',
            };
        });
    }
    function proposed(session: OptimizedSession) {
        return Boolean(session.proposal?.provider && session.proposal?.rationale);
    }
    function phase(session: OptimizedSession) {
        if (session.status !== 'monitoring')
            return session.best_configuration.length
                ? 'Optimized profile ready'
                : session.status === 'paused'
                  ? 'Paused'
                  : 'Not learned yet';
        return session.phase === 'baseline'
            ? 'Learning baseline'
            : session.phase === 'measuring_change'
              ? 'Verifying configuration'
              : 'Optimized session active';
    }
    function formatDate(at: number) {
        return new Date(at * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
    }
</script>

<svelte:window
    onkeydown={(event) => {
        if (event.key === 'Escape' && !busy) composerOpen = false;
    }}
/>

<div class="sessions-page sessions-simple">
    {#if sessions.length}<header class="sessions-heading">
            <span></span>
            <button
                class="primary sessions-new"
                onclick={() => (composerOpen = true)}
                disabled={!desktop}>＋ New session</button
            >
        </header>{/if}

    {#if notice}<div class="sessions-notice" role="status">
            <span>{notice}</span><button
                class="icon-button"
                aria-label="Dismiss message"
                onclick={() => (notice = '')}>×</button
            >
        </div>{/if}
    {#if error}<div class="sessions-notice session-error" role="alert">
            <span>{error}</span><button
                class="icon-button"
                aria-label="Dismiss error"
                onclick={() => (error = '')}>×</button
            >
        </div>{/if}

    {#if loading}<div class="panel sessions-empty">Loading optimization profiles…</div>
    {:else if !sessions.length}
        <section class="session-first-use">
            <Icon name="activity" size={24} />
            <h2>No saved sessions</h2>
            <p>
                Describe work you repeat, such as a long coding session. Jitterbug proposes a
                resource setup for it, and nothing changes until you review it.
            </p>
            <button class="primary" onclick={() => (composerOpen = true)} disabled={!desktop}
                >New session</button
            >
        </section>
    {:else}
        <div class="session-profile-layout">
            <aside class="panel session-profile-library">
                <div class="sessions-library-header">
                    <div>
                        <h2>Saved profiles</h2>
                        <p>{sessions.length} recurring tasks</p>
                    </div>
                </div>
                <div class="session-list">
                    {#each sessions as session}<button
                            class="session-row"
                            class:selected={session.id === selected?.id}
                            onclick={() => {
                                selectedId = session.id;
                                notice = '';
                            }}
                            ><span class="session-row-icon"
                                ><Icon
                                    name={session.best_configuration.length ? 'bulb' : 'activity'}
                                    size={18}
                                /></span
                            ><span class="session-row-copy"
                                ><strong>{session.name}</strong><small>{phase(session)}</small
                                ></span
                            >{#if session.status === 'monitoring'}<span class="session-active-dot"
                                ></span>{/if}</button
                        >{:else}<div class="session-library-empty">
                            <strong>No profiles yet</strong><span
                                >Create one for a task you repeat.</span
                            >
                        </div>{/each}
                </div>
            </aside>

            <main class="session-profile-detail panel">
                {#if selected}
                    <div class="session-profile-head">
                        <div>
                            <span
                                class:active={selected.status === 'monitoring'}
                                class="session-state">{phase(selected)}</span
                            >
                            <h2>{selected.name}</h2>
                            <p>{selected.goal}</p>
                        </div>
                        <div class="session-profile-actions">
                            {#if selected.status === 'monitoring'}<button
                                    class="primary"
                                    disabled={busy}
                                    onclick={() =>
                                        run(
                                            () => client.pause(selected.id),
                                            'Session paused. Jitterbug restored any setting it owned.'
                                        )}>Stop & restore</button
                                >{:else if selected.best_configuration.length}<button
                                    class="primary"
                                    disabled={busy ||
                                        selected.stale ||
                                        Boolean(selected.pending_transaction)}
                                    onclick={() =>
                                        run(
                                            () => client.reapply(selected.id),
                                            'Cached configuration reapplied and verified.'
                                        )}>Apply cached profile</button
                                ><button
                                    disabled={busy}
                                    onclick={() =>
                                        run(
                                            () => client.start(selected.id),
                                            'Fresh validation started for this profile.'
                                        )}>Relearn</button
                                >{:else}<button
                                    class="primary"
                                    disabled={busy}
                                    onclick={() =>
                                        run(
                                            () => client.start(selected.id),
                                            'Learning started. Jitterbug is collecting a baseline before any supported change.'
                                        )}>Learn this session</button
                                >{/if}
                            <button
                                class:session-delete-confirm={deleteConfirmId === selected.id}
                                disabled={busy ||
                                    selected.status === 'monitoring' ||
                                    Boolean(selected.pending_transaction) ||
                                    selected.applied_configuration.length > 0}
                                onclick={() => remove(selected)}
                                >{deleteConfirmId === selected.id
                                    ? 'Confirm delete'
                                    : 'Delete'}</button
                            >
                        </div>
                    </div>

                    <section class="session-config-card">
                        <div>
                            <p class="eyebrow">AI-PROPOSED PROFILE</p>
                            <h3>
                                {selected.proposal?.capability === 'keep_awake'
                                    ? 'Keep Mac awake'
                                    : 'Measure and monitor'}
                            </h3>
                        </div>
                        {#if proposed(selected)}
                            <p>{selected.proposal.rationale}</p>
                            <dl class="session-settings-list">
                                <div>
                                    <dt>Capability</dt>
                                    <dd>
                                        {selected.proposal.capability === 'keep_awake'
                                            ? 'Keep this Mac awake while the session is active'
                                            : 'Watch the task only. No system setting will be changed.'}
                                    </dd>
                                </div>
                                <div>
                                    <dt>Provider</dt>
                                    <dd>
                                        {selected.proposal.provider} · {selected.proposal.model}
                                    </dd>
                                </div>
                            </dl>
                            <small
                                >No system change occurs until you choose Learn this session. AI
                                cannot pick settings outside Settings → System rights.</small
                            >
                            {#if selected.status !== 'monitoring'}
                                <button
                                    disabled={busy}
                                    onclick={() =>
                                        run(
                                            () => client.propose(selected.id),
                                            'AI updated this profile. Review it before learning.'
                                        )}>Ask AI again</button
                                >
                            {/if}
                        {:else}
                            <p>
                                This profile has no AI proposal yet. Ask your configured intelligent
                                provider to classify the task.
                            </p>
                            <button
                                class="primary"
                                disabled={busy || selected.status === 'monitoring'}
                                onclick={() =>
                                    run(
                                        () => client.propose(selected.id),
                                        'AI proposed a profile. Review it, then learn during the real task.'
                                    )}>Ask AI to propose</button
                            >
                        {/if}
                    </section>

                    {#if selected.stale || selected.pending_transaction}<div
                            class="session-profile-warning"
                        >
                            <strong
                                >{selected.pending_transaction
                                    ? 'Recovery required'
                                    : selected.best_configuration.length
                                      ? 'Cached profile is stale'
                                      : 'Learning interrupted'}</strong
                            ><span
                                >{selected.stale_reason ??
                                    'Jitterbug retained a durable recovery journal and will not overlap another native action.'}</span
                            >{#if selected.status !== 'monitoring' && (selected.pending_transaction || selected.applied_configuration.length)}<button
                                    disabled={busy}
                                    onclick={() =>
                                        run(
                                            () => client.restore(selected.id),
                                            'Original state restored and verified.'
                                        )}>Restore original state</button
                                >{/if}
                        </div>{/if}

                    <section class="session-config-card">
                        <div>
                            <p class="eyebrow">SAVED CONFIGURATION</p>
                            <h3>
                                {selected.best_configuration.length
                                    ? 'Verified settings for this task'
                                    : selected.applied_configuration.length
                                      ? 'Settings currently applied'
                                      : selected.original_configuration.length
                                        ? 'Original settings captured'
                                        : 'No settings saved yet'}
                            </h3>
                        </div>
                        {#if settingRows(selected).length}
                            <dl class="session-settings-list">
                                {#each settingRows(selected) as row}
                                    <div>
                                        <dt>{settingName(row.setting)}</dt>
                                        <dd>
                                            Original {row.original} → Saved {row.saved} → Applied {row.applied}
                                        </dd>
                                    </div>
                                {/each}
                            </dl>
                        {:else}
                            <p>
                                {selected.setting_support ||
                                    'Learn this session during the real task. Jitterbug will list every setting it is allowed to change.'}
                            </p>
                        {/if}
                        {#if selected.fingerprint}<small
                                >Bound to this OS and {selected.fingerprint.applications.length} detected
                                app versions. A changed environment invalidates the cache.</small
                            >{/if}
                    </section>

                    <section class="session-recent-use">
                        <p class="eyebrow">RECENT USE</p>
                        {#if selected.runs.length}{#each selected.runs.slice(0, 4) as run}<div
                                    class="session-run"
                                >
                                    <strong>{run.outcome.replaceAll('_', ' ')}</strong><span
                                        >{formatDate(run.completed_at)}</span
                                    >
                                </div>{/each}{:else}<p class="muted">
                                No completed optimization runs yet.
                            </p>{/if}
                    </section>

                    <details class="session-log">
                        <summary>Safety and activity log</summary>{#each selected.events as event}<p
                            >
                                <time>{formatDate(event.at)}</time> · {event.message}
                            </p>{/each}
                    </details>
                {:else}<div class="sessions-empty">
                        <Icon name="activity" size={24} />
                        <h2>Create an optimization profile</h2>
                        <p>
                            Use it during a recurring task so Jitterbug can learn a compatible,
                            reversible configuration.
                        </p>
                        <button class="primary" onclick={() => (composerOpen = true)}
                            >Create profile</button
                        >
                    </div>{/if}
            </main>
        </div>
    {/if}
</div>

{#if composerOpen}<div class="modal-backdrop sessions-modal-backdrop">
        <div
            class="sessions-composer panel"
            role="dialog"
            aria-modal="true"
            aria-labelledby="session-composer-title"
        >
            <div class="session-card-heading">
                <div>
                    <h2 id="session-composer-title">What task do you repeat?</h2>
                </div>
                <button
                    class="icon-button"
                    aria-label="Close"
                    onclick={() => (composerOpen = false)}>×</button
                >
            </div>
            <p class="composer-intro">
                Describe the work and what you want to improve. Jitterbug suggests settings you can
                review.
            </p>
            <label
                >Profile name<input
                    bind:this={nameInput}
                    bind:value={name}
                    maxlength="80"
                    placeholder="Parallel coding session"
                /></label
            ><label
                >What should improve?<textarea
                    bind:value={goal}
                    maxlength="480"
                    rows="5"
                    placeholder="Keep the computer responsive while my coding agents run tests."
                ></textarea></label
            >
            <div class="composer-actions">
                <button onclick={() => (composerOpen = false)} disabled={busy}>Cancel</button
                ><button
                    class="primary"
                    onclick={create}
                    disabled={busy || !name.trim() || !goal.trim()}
                    >{busy ? 'Asking AI…' : 'Create profile'}</button
                >
            </div>
        </div>
    </div>{/if}
