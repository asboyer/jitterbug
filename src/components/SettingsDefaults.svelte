<script lang="ts">
    import { onDestroy, onMount, tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type {
        SystemCapability,
        SystemPreferenceValue,
        SystemDefaultHistory,
        SystemDefaultSnapshot,
        SystemRestorePreview,
        SystemRollbackResult,
    } from '../lib/types';

    const PAGE_SIZE = 20;
    const captureGroups = [
        'Audio',
        'Audio hardware',
        'Displays',
        'Power profiles',
        'Desktop & Dock',
        'Keyboard',
        'Pointer',
        'Finder storage',
    ];
    let selectedGroups = $state([...captureGroups]);

    let { desktop }: { desktop: boolean } = $props();
    let entries: SystemDefaultSnapshot[] = $state([]);
    let currentDefault: SystemDefaultSnapshot | null = $state(null);
    let cursor: string | null = $state(null);
    let viewingLatest = $state(true);
    let interruptedOperations = $state(0);
    let preview: SystemRestorePreview | null = $state(null);
    let result: SystemRollbackResult | null = $state(null);
    let busy = $state(false);
    let status = $state('');
    let error = $state('');
    let historyOpen = $state(false);
    let dialog: HTMLDialogElement;
    let historyTrigger: HTMLButtonElement;
    let restoreId: string | null = null;
    let noticeFading = $state(false);
    let noticeTimers: ReturnType<typeof setTimeout>[] = [];

    function clearNoticeTimers() {
        for (const timer of noticeTimers) clearTimeout(timer);
        noticeTimers = [];
        noticeFading = false;
    }

    function showSavedNotice(message: string) {
        clearNoticeTimers();
        status = message;
        noticeTimers.push(
            setTimeout(() => {
                noticeFading = true;
                noticeTimers.push(
                    setTimeout(() => {
                        status = '';
                        noticeFading = false;
                        noticeTimers = [];
                    }, 1000)
                );
            }, 3000)
        );
    }

    function mountDialog(node: HTMLDialogElement) {
        dialog = node;
        const overflow = document.body.style.overflow;
        document.body.style.overflow = 'hidden';
        node.showModal();
        return {
            destroy() {
                node.close();
                document.body.style.overflow = overflow;
            },
        };
    }

    function dialogKeydown(event: KeyboardEvent) {
        if (event.key !== 'Tab') return;
        const controls = Array.from(
            dialog.querySelectorAll<HTMLElement>('button:not(:disabled), summary, [tabindex="0"]')
        ).filter((element) => element.getClientRects().length > 0);
        const first = controls[0];
        const last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first?.focus();
        }
    }

    function validDate(value: number | null) {
        if (value === null || !Number.isFinite(value)) return null;
        const date = new Date(value);
        return Number.isNaN(date.getTime()) ? null : date;
    }

    function timestamp(value: number | null) {
        const date = validDate(value);
        if (!date) return 'Timestamp unavailable';
        try {
            return new Intl.DateTimeFormat(undefined, {
                dateStyle: 'medium',
                timeStyle: 'medium',
            }).format(date);
        } catch {
            return 'Timestamp unavailable';
        }
    }

    function datetime(value: number | null) {
        const date = validDate(value);
        if (!date) return undefined;
        try {
            return date.toISOString();
        } catch {
            return undefined;
        }
    }

    function volume(value: number | null) {
        return value === null ? 'Not supported' : `${(value * 100).toFixed(1)}%`;
    }

    function muted(value: boolean | null) {
        if (value === null) return 'Not supported';
        return value ? 'Muted' : 'Unmuted';
    }

    async function perform(action: () => Promise<void>) {
        busy = true;
        status = '';
        error = '';
        try {
            await action();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function loadHistory(next: string | null = null) {
        if (!desktop) return;
        await perform(async () => {
            const page = await invoke<SystemDefaultHistory>('get_system_default_history', {
                cursor: next,
                pageSize: PAGE_SIZE,
            });
            entries = page.entries;
            const pageCurrent = page.entries.find((entry) => entry.current);
            if (pageCurrent) currentDefault = pageCurrent;
            cursor = page.next_cursor;
            viewingLatest = next === null;
            interruptedOperations = page.interrupted_operations;
        });
    }

    function preferenceValue(value: SystemPreferenceValue) {
        if (value.kind === 'absent') return 'System default';
        if (value.kind === 'boolean') return value.value ? 'On' : 'Off';
        return String(value.value);
    }

    function hardwareSummary(entry: SystemDefaultSnapshot) {
        const hardware = Object.values(entry.values?.hardware ?? {});
        return hardware.length
            ? ` · ${hardware.length} extra setting${hardware.length === 1 ? '' : 's'}`
            : '';
    }

    async function setDefault() {
        if (!desktop) return;
        await perform(async () => {
            const saved = await invoke<SystemDefaultSnapshot>('set_system_default', {
                groups: selectedGroups,
            });
            currentDefault = saved;
            const previous = entries
                .filter((entry) => entry.id !== saved.id)
                .map((entry) => ({ ...entry, current: false }));
            if (viewingLatest) {
                const combined = [saved, ...previous];
                entries = combined.slice(0, PAGE_SIZE);
                if (combined.length > PAGE_SIZE) cursor = entries[entries.length - 1].id;
            } else {
                entries = previous;
            }
            showSavedNotice('Saved. This is now your current baseline.');
        });
    }

    function focusSnapshot() {
        const button = Array.from(
            dialog.querySelectorAll<HTMLButtonElement>('[data-snapshot-id]')
        ).find((element) => element.dataset.snapshotId === restoreId);
        button?.focus();
    }

    async function startRestore(id: string) {
        if (!desktop) return;
        restoreId = id;
        result = null;
        await perform(async () => {
            preview = await invoke<SystemRestorePreview>('preview_system_restore', { id });
        });
        await tick();
        dialog.querySelector<HTMLButtonElement>('.restore-cancel')?.focus();
    }

    async function cancelRestore() {
        const pending = preview;
        preview = null;
        if (!pending || !desktop) return;
        await perform(async () => {
            await invoke('cancel_system_restore', { token: pending.token });
            status = 'Rollback canceled. No system settings were changed.';
        });
        await tick();
        focusSnapshot();
    }

    async function confirmRestore() {
        const pending = preview;
        if (!pending || !desktop) return;
        preview = null;
        await perform(async () => {
            result = await invoke<SystemRollbackResult>('confirm_system_restore', {
                token: pending.token,
                confirmed: true,
            });
            if (result.status === 'partial_failure') {
                error = result.message;
            } else {
                status = result.message;
            }
        });
        await tick();
        focusSnapshot();
    }

    async function closeHistory() {
        if (busy) return;
        if (preview) await cancelRestore();
        historyOpen = false;
        result = null;
        await tick();
        historyTrigger?.focus();
    }

    onMount(() => {
        void loadHistory();
    });
    onDestroy(() => {
        clearNoticeTimers();
        if (desktop && preview) void invoke('cancel_system_restore', { token: preview.token });
    });
</script>

{#snippet snapshotEntry(entry: SystemDefaultSnapshot, current: boolean)}
    <article class:error-entry={entry.error} class="default-entry">
        <div class="default-entry-heading">
            <div>
                <strong>{current ? 'Current system baseline' : 'Past system baseline'}</strong>
                <time datetime={datetime(entry.created_at_ms)}
                    >{timestamp(entry.created_at_ms)}</time
                >
            </div>
            {#if !entry.error}
                <button
                    disabled={busy || !entry.compatible}
                    onclick={() => startRestore(entry.id)}
                    data-snapshot-id={entry.id}
                    aria-label={`${current ? 'Rollback current' : 'Preview past'} system baseline from ${timestamp(entry.created_at_ms)}`}
                    >{current ? 'Rollback' : 'Preview rollback'}</button
                >
            {/if}
        </div>
        {#if entry.error}
            <p class="error">{entry.error}</p>
        {:else}
            <p>{entry.summary}{hardwareSummary(entry)}</p>
            {#if !entry.compatible}
                <p class="muted">This snapshot doesn’t match this Mac or user.</p>
            {/if}
            {#if entry.values}
                <details>
                    <summary>Saved settings</summary>
                    <dl class="snapshot-values">
                        {#if entry.values.output_volume !== null || entry.values.output_muted !== null}
                            <div>
                                <dt>Volume</dt>
                                <dd>{volume(entry.values.output_volume)}</dd>
                            </div>
                            <div>
                                <dt>Mute</dt>
                                <dd>{muted(entry.values.output_muted)}</dd>
                            </div>
                        {/if}
                        {#each Object.entries(entry.values.preferences ?? {}) as [key, value]}
                            <div>
                                <dt>
                                    {entry.coverage?.find((item) => item.id === key)?.label ??
                                        key.replaceAll('_', ' ')}
                                </dt>
                                <dd>{preferenceValue(value)}</dd>
                            </div>
                        {/each}
                    </dl>
                </details>
            {/if}
        {/if}
    </article>
{/snippet}

{#snippet coverageList(items: SystemCapability[])}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="capability-list" role="region" aria-label="Per-setting coverage" tabindex="0">
        {#each [...new Set(items.map((item) => item.group))] as group}
            <section aria-label={group}>
                <h4>{group}</h4>
                {#each items.filter((item) => item.group === group) as item (item.id)}
                    <article class="capability-item">
                        <div>
                            <strong>{item.label}</strong>
                            <span class="capability-status">{item.status.replaceAll('_', ' ')}</span
                            >
                        </div>
                    </article>
                {/each}
            </section>
        {/each}
    </div>
{/snippet}

<section class="panel settings-panel settings-defaults" aria-label="System settings baseline">
    <div class="section-title">
        <div>
            <h2>System settings baseline</h2>
            <p class="muted">Save how this PC is set up. Rollback puts those settings back.</p>
        </div>
        <div class="header-actions">
            <button
                disabled={!desktop || busy}
                onclick={() => (historyOpen = true)}
                aria-haspopup="dialog"
                bind:this={historyTrigger}>View past defaults</button
            >
            <button
                class="primary"
                disabled={!desktop || busy || selectedGroups.length === 0}
                onclick={setDefault}>Set Default</button
            >
        </div>
    </div>
    {#if !desktop}
        <p class="preview-note">
            Preview only: system baselines are unavailable here; nothing is stored or restored.
        </p>
    {/if}
    {#if currentDefault && !currentDefault.error}
        {@render snapshotEntry(currentDefault, true)}
    {:else if desktop && !busy && !error}
        <p class="empty compact">No baseline saved yet.</p>
    {/if}
    <details class="capability-details">
        <summary>What to save</summary>
        <fieldset class="capture-groups" disabled={!desktop || busy}>
            <legend>Categories</legend>
            {#each captureGroups as group}
                <label
                    ><input type="checkbox" bind:group={selectedGroups} value={group} />
                    {group}</label
                >
            {/each}
        </fieldset>
    </details>
    {#if status && !historyOpen}<p
            class="success saved-notice"
            class:fading={noticeFading}
            role="status"
        >
            {status}
        </p>{/if}
    {#if error && !historyOpen}<p class="error" role="alert">{error}</p>{/if}
</section>

{#if historyOpen}
    <dialog
        class="panel defaults-dialog"
        use:mountDialog
        aria-labelledby="defaults-dialog-title"
        oncancel={(event) => {
            event.preventDefault();
            if (preview && !busy) void cancelRestore();
            else void closeHistory();
        }}
        onkeydown={dialogKeydown}
        onclick={(event) => {
            if (event.target !== dialog) return;
            const rect = dialog.getBoundingClientRect();
            if (
                event.clientX < rect.left ||
                event.clientX > rect.right ||
                event.clientY < rect.top ||
                event.clientY > rect.bottom
            )
                void closeHistory();
        }}
    >
        <div class="section-title">
            <h2 id="defaults-dialog-title">
                {preview ? 'Confirm system settings rollback' : 'System baseline history'}
            </h2>
            <button disabled={busy} onclick={closeHistory} aria-label="Close baseline history"
                >Close</button
            >
        </div>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
            class="defaults-dialog-scroll"
            role="region"
            tabindex="0"
            aria-label="System baseline snapshots"
        >
            {#if preview}
                <p class="eyebrow">One-use preview · expires in five minutes</p>
                <p>
                    Roll back to the exact supported values captured on
                    <strong>{timestamp(preview.target.created_at_ms)}</strong>?
                </p>
                {#if preview.changes.length}
                    {#each [...new Set(preview.changes.map((change) => change.group ?? 'Audio'))] as group}
                        <h3>{group}</h3>
                        <dl class="restore-changes">
                            {#each preview.changes.filter((change) => (change.group ?? 'Audio') === group) as change}
                                <div>
                                    <dt>{change.field}</dt>
                                    <dd>
                                        {change.before} → {change.after}
                                        <p class="muted">
                                            {change.verification ?? 'Live CoreAudio readback'}
                                        </p>
                                    </dd>
                                </div>
                            {/each}
                        </dl>
                    {/each}
                {:else}
                    <p class="muted">Actual supported values already match this baseline.</p>
                {/if}
                <p class="muted">
                    Jitterbug will recheck compatibility and current values before confirmation and
                    before each native write. Any mismatch stops the remaining writes. Saved
                    preference verification does not prove a live effect. No Dock or system process
                    is restarted. Confirmation automatically runs the selected supported changes;
                    there are no commands to copy. Audio format changes can interrupt playback or
                    recording and may require reopening audio applications. Displays may blank or
                    windows move; current modes are verified, but reboot persistence is not
                    verified. Power timers require externally provided authorization and can change
                    future sleep behavior.
                </p>
                {#if preview.coverage?.length}
                    <details>
                        <summary>Current capability and permission state</summary
                        >{@render coverageList(preview.coverage)}
                    </details>
                {/if}
                <div class="form-actions">
                    <button class="restore-cancel" disabled={busy} onclick={cancelRestore}
                        >Cancel</button
                    >
                    <button class="primary" disabled={busy} onclick={confirmRestore}
                        >Roll back system settings</button
                    >
                </div>
            {:else}
                {#if interruptedOperations > 0}
                    <p class="error" role="alert">
                        {interruptedOperations} earlier rollback {interruptedOperations === 1
                            ? 'journal needs'
                            : 'journals need'} review. Jitterbug did not replay any writes.
                    </p>
                {/if}
                {#if currentDefault}{@render snapshotEntry(currentDefault, true)}{/if}
                {#if desktop && !busy && entries.length === 0 && !currentDefault && !error}
                    <p class="empty compact">No system baselines yet.</p>
                {/if}
                {#each entries.filter((entry) => entry.id !== currentDefault?.id) as entry (entry.id)}
                    {@render snapshotEntry(entry, false)}
                {/each}
                <div class="form-actions">
                    {#if !viewingLatest}
                        <button disabled={busy} onclick={() => loadHistory()}
                            >Return to latest defaults</button
                        >
                    {/if}
                    {#if cursor}
                        <button disabled={busy} onclick={() => loadHistory(cursor)}
                            >Load older defaults</button
                        >
                    {/if}
                </div>
            {/if}
            {#if result}
                <dl class="restore-changes">
                    {#each result.outcomes as outcome}
                        <div>
                            <dt>{outcome.field}</dt>
                            <dd>
                                {outcome.status === 'pending'
                                    ? 'Not attempted — stopped after earlier failure'
                                    : outcome.status.replaceAll('_', ' ')}{#if outcome.error}<p
                                        class="error"
                                    >
                                        {outcome.error}
                                    </p>{/if}
                            </dd>
                        </div>
                    {/each}
                </dl>
            {/if}
            {#if status}<p class="success" role="status">{status}</p>{/if}
            {#if error}<p class="error" role="alert">{error}</p>{/if}
        </div>
    </dialog>
{/if}

<style>
    .capture-groups {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem 1.25rem;
        margin: 1rem 0;
        padding: 0.75rem;
        border: 1px solid var(--border, #343941);
        border-radius: 0.5rem;
    }
    .capture-groups label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
    }
    .capture-groups input {
        width: auto;
    }
    .capability-details {
        margin: 0.15rem 0 0.75rem;
    }
    .capability-list {
        max-height: 24rem;
        overflow: auto;
        overscroll-behavior: contain;
        padding-right: 0.5rem;
    }
    .capability-item {
        padding: 0.8rem 0;
        border-bottom: 1px solid var(--border, #343941);
    }
    .capability-status {
        display: inline-block;
        margin-left: 0.5rem;
        font-size: 0.75rem;
        padding: 0.15rem 0.4rem;
        border: 1px solid var(--border, #343941);
        border-radius: 0.35rem;
        text-transform: capitalize;
    }
    .header-actions {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        flex-shrink: 0;
    }
    .saved-notice {
        margin-top: 1rem;
        transition: opacity 1s ease;
    }
    .saved-notice.fading {
        opacity: 0;
    }
</style>
