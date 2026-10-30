<script lang="ts">
    import { onDestroy, onMount, tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type {
        SystemDefaultHistory,
        SystemDefaultSnapshot,
        SystemRestorePreview,
        SystemRollbackResult,
    } from '../lib/types';

    const PAGE_SIZE = 20;

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

    async function setDefault() {
        if (!desktop) return;
        await perform(async () => {
            const saved = await invoke<SystemDefaultSnapshot>('set_system_default');
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
            status = 'Actual system audio settings captured as the current baseline.';
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

    onMount(() => void loadHistory());
    onDestroy(() => {
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
            <p>{entry.summary}</p>
            {#if !entry.compatible}
                <p class="muted">
                    Unavailable for the current OS, user, machine, or output device.
                </p>
            {/if}
            {#if entry.values}
                <details>
                    <summary>Inspect exact values</summary>
                    <dl class="snapshot-values">
                        <div>
                            <dt>Output volume</dt>
                            <dd>{volume(entry.values.output_volume)}</dd>
                        </div>
                        <div>
                            <dt>Output mute</dt>
                            <dd>{muted(entry.values.output_muted)}</dd>
                        </div>
                    </dl>
                </details>
            {/if}
        {/if}
    </article>
{/snippet}

<section class="panel settings-panel settings-defaults" aria-label="System settings baseline">
    <div class="section-title">
        <div>
            <h2>System settings baseline</h2>
            <p class="muted">Capture actual macOS audio settings before an optimization.</p>
        </div>
        <button class="primary" disabled={!desktop || busy} onclick={setDefault}>Set Default</button
        >
    </div>
    {#if !desktop}
        <p class="preview-note">
            Preview only: system baselines are unavailable here; nothing is stored or restored.
        </p>
    {/if}
    <div class="default-scope">
        <div>
            <h3>Included</h3>
            <p>
                The current default output device’s settable master volume and mute values, when
                CoreAudio reports those controls as supported.
            </p>
        </div>
        <div>
            <h3>Never included</h3>
            <p>
                Provider configuration, credentials, privacy/security/network controls, processes,
                software or OS versions, device selection, and arbitrary commands.
            </p>
        </div>
    </div>
    <p class="muted">
        Each Set Default creates a new immutable local snapshot. Rollback works only on the same
        macOS version, user, machine, and default output device, and verifies CoreAudio readback.
        Verification confirms the setting value—not a performance improvement or full-system backup.
    </p>
    <button
        disabled={!desktop || busy}
        onclick={() => (historyOpen = true)}
        aria-haspopup="dialog"
        bind:this={historyTrigger}>View past defaults</button
    >
    {#if status && !historyOpen}<p class="success" role="status">{status}</p>{/if}
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
                    <dl class="restore-changes">
                        {#each preview.changes as change}
                            <div>
                                <dt>{change.field}</dt>
                                <dd>{change.before} → {change.after}</dd>
                            </div>
                        {/each}
                    </dl>
                {:else}
                    <p class="muted">Actual supported values already match this baseline.</p>
                {/if}
                <p class="muted">
                    Jitterbug will recheck compatibility and current values before confirmation and
                    before each CoreAudio write. Any mismatch stops the remaining writes.
                </p>
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
                            <dd>{outcome.status.replaceAll('_', ' ')}</dd>
                        </div>
                    {/each}
                </dl>
            {/if}
            {#if status}<p class="success" role="status">{status}</p>{/if}
            {#if error}<p class="error" role="alert">{error}</p>{/if}
        </div>
    </dialog>
{/if}
