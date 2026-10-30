<script lang="ts">
    import { onDestroy, onMount, tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type {
        Bootstrap,
        Settings,
        SettingsDefaultHistory,
        SettingsDefaultSnapshot,
        SettingsRestorePreview,
        SupportedSettings,
    } from '../lib/types';

    const PAGE_SIZE = 20;

    let {
        desktop,
        onrestored,
    }: { desktop: boolean; onrestored: (settings: Settings) => Promise<void> } = $props();
    let entries: SettingsDefaultSnapshot[] = $state([]);
    let currentDefault: SettingsDefaultSnapshot | null = $state(null);
    let cursor: string | null = $state(null);
    let viewingLatest = $state(true);
    let preview: SettingsRestorePreview | null = $state(null);
    let busy = $state(false);
    let status = $state('');
    let error = $state('');
    let historyOpen = $state(false);
    let dialog: HTMLDialogElement;
    let historyTrigger: HTMLButtonElement;
    let restoreId: string | null = null;

    function focusSnapshot() {
        const button = Array.from(
            dialog.querySelectorAll<HTMLButtonElement>('[data-snapshot-id]')
        ).find((element) => element.dataset.snapshotId === restoreId);
        button?.focus();
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

    async function closeHistory() {
        if (busy) return;
        if (preview) await cancelRestore();
        historyOpen = false;
        await tick();
        historyTrigger?.focus();
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

    function tier(value: SupportedSettings['decision']) {
        return `${value.provider} / ${value.model}`;
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
            const page = await invoke<SettingsDefaultHistory>('get_settings_default_history', {
                cursor: next,
                pageSize: PAGE_SIZE,
            });
            entries = page.entries;
            const pageCurrent = page.entries.find((entry) => entry.current);
            if (pageCurrent) currentDefault = pageCurrent;
            cursor = page.next_cursor;
            viewingLatest = next === null;
        });
    }

    async function setDefault() {
        if (!desktop) return;
        await perform(async () => {
            const saved = await invoke<SettingsDefaultSnapshot>('set_settings_default');
            currentDefault = saved;
            const previous = entries
                .filter((entry) => entry.id !== saved.id)
                .map((entry) => ({ ...entry, current: false }));
            if (viewingLatest) {
                const combined = [saved, ...previous];
                entries = combined.slice(0, PAGE_SIZE);
                if (combined.length > PAGE_SIZE) {
                    // Resume after the visible page so displaced snapshots are not skipped.
                    cursor = entries[entries.length - 1].id;
                }
            } else {
                entries = previous;
            }
            status = 'Saved settings captured as the current default.';
        });
    }

    async function startRestore(id: string) {
        if (!desktop) return;
        restoreId = id;
        await perform(async () => {
            preview = await invoke<SettingsRestorePreview>('preview_settings_restore', { id });
        });
        await tick();
        dialog.querySelector<HTMLButtonElement>('.restore-cancel')?.focus();
    }

    async function cancelRestore() {
        const pending = preview;
        preview = null;
        if (!pending || !desktop) return;
        await perform(async () => {
            await invoke('cancel_settings_restore', { token: pending.token });
            status = 'Restore canceled. No settings were changed.';
        });
        await tick();
        focusSnapshot();
    }

    async function confirmRestore() {
        const pending = preview;
        if (!pending || !desktop) return;
        preview = null;
        await perform(async () => {
            let effective: Settings;
            try {
                effective = await invoke<Settings>('confirm_settings_restore', {
                    token: pending.token,
                    confirmed: true,
                });
            } catch (cause) {
                try {
                    const reconciled = await invoke<Bootstrap>('get_bootstrap');
                    await onrestored(reconciled.settings);
                } catch {
                    // Preserve the restore error even if a best-effort refresh also fails.
                }
                throw cause;
            }
            await onrestored(effective);
            status = 'Settings restored and read back from disk.';
        });
        await tick();
        focusSnapshot();
    }

    onMount(() => void loadHistory());
    onDestroy(() => {
        if (desktop && preview) {
            void invoke('cancel_settings_restore', { token: preview.token });
        }
    });
</script>

{#snippet snapshotEntry(entry: SettingsDefaultSnapshot, current: boolean)}
    <article class:error-entry={entry.error} class="default-entry">
        <div class="default-entry-heading">
            <div>
                <strong>{current ? 'Current saved default' : 'Historic default'}</strong>
                <time datetime={datetime(entry.created_at_ms)}
                    >{timestamp(entry.created_at_ms)}</time
                >
            </div>
            {#if !entry.error}
                <button
                    disabled={busy}
                    onclick={() => startRestore(entry.id)}
                    data-snapshot-id={entry.id}
                    aria-label={current
                        ? `Rollback current saved default from ${timestamp(entry.created_at_ms)}`
                        : `Preview historic restore from ${timestamp(entry.created_at_ms)}`}
                    >{current ? 'Rollback' : 'Preview restore'}</button
                >
            {/if}
        </div>
        {#if entry.error}
            <p class="error">{entry.error}</p>
        {:else}
            <p>{entry.summary}</p>
            {#if entry.settings}
                <details>
                    <summary>Inspect snapshot</summary>
                    <dl class="snapshot-values">
                        <div>
                            <dt>Privacy mode</dt>
                            <dd>{entry.settings.mode}</dd>
                        </div>
                        <div>
                            <dt>Decision tier</dt>
                            <dd>{tier(entry.settings.decision)}</dd>
                        </div>
                        <div>
                            <dt>Cheap tier</dt>
                            <dd>{tier(entry.settings.cheap)}</dd>
                        </div>
                        <div>
                            <dt>Intelligent tier</dt>
                            <dd>{tier(entry.settings.intelligent)}</dd>
                        </div>
                        <div>
                            <dt>Decision model</dt>
                            <dd>{entry.settings.decision_enabled ? 'Enabled' : 'Disabled'}</dd>
                        </div>
                        <div>
                            <dt>Budget ceiling</dt>
                            <dd>${entry.settings.budget_usd.toFixed(2)}</dd>
                        </div>
                    </dl>
                </details>
            {/if}
        {/if}
    </article>
{/snippet}

<section class="panel settings-panel settings-defaults" aria-label="Saved settings defaults">
    <div class="section-title">
        <div>
            <h2>Saved settings defaults</h2>
            <p class="muted">Create a rollback point from settings already saved in Jitterbug.</p>
        </div>
        <button
            class="primary"
            disabled={!desktop || busy}
            onclick={setDefault}
            aria-label="Set current saved settings as default">Set Default</button
        >
    </div>
    {#if !desktop}
        <p class="preview-note">
            Preview only: saved-default history is nonpersistent here; nothing is stored or
            restored.
        </p>
    {/if}
    <div class="default-scope">
        <div>
            <h3>Included</h3>
            <p>
                Provider privacy mode; provider and model choices for all three tiers; optional
                decision model enabled state; and the cloud budget ceiling.
            </p>
        </div>
        <div>
            <h3>Never included</h3>
            <p>
                Automation policy and permissions; credentials; spent or reserved budget
                consumption; ledger and telemetry data; process priorities or other OS settings;
                queue state; and command authorization.
            </p>
        </div>
    </div>
    <p class="muted">
        This is Jitterbug configuration history, not an operating-system backup. Set Default reads
        the backend’s currently saved configuration, not unsaved edits in this form. Every prior
        default remains as an individual snapshot unless storage is removed outside Jitterbug.
    </p>

    <button
        disabled={!desktop || busy}
        onclick={() => (historyOpen = true)}
        aria-haspopup="dialog"
        bind:this={historyTrigger}
    >
        View past defaults
    </button>
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
            if (preview) {
                if (!busy) void cancelRestore();
            } else void closeHistory();
        }}
        onkeydown={dialogKeydown}
        onclick={(event) => {
            if (event.target === dialog) {
                const rect = dialog.getBoundingClientRect();
                if (
                    event.clientX < rect.left ||
                    event.clientX > rect.right ||
                    event.clientY < rect.top ||
                    event.clientY > rect.bottom
                )
                    void closeHistory();
            }
        }}
    >
        <div class="section-title">
            <h2 id="defaults-dialog-title">
                {preview ? 'Confirm settings restore' : 'Saved default history'}
            </h2>
            <button disabled={busy} onclick={closeHistory} aria-label="Close default history"
                >Close</button
            >
        </div>
        <!-- Keyboard users must be able to scroll the bounded history region itself. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
            class="defaults-dialog-scroll"
            role="region"
            tabindex="0"
            aria-label="Default snapshots"
        >
            {#if preview}
                <p class="eyebrow">One-time preview · expires in five minutes</p>
                <p>
                    Restore the supported fields from
                    <strong>{timestamp(preview.target.created_at_ms)}</strong>? Excluded state
                    remains untouched.
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
                    <p class="muted">The supported values already match this snapshot.</p>
                {/if}
                <p class="muted">
                    Automation, permissions, credentials, spending consumption, telemetry,
                    processes, queue state, and command authorization will not be restored.
                </p>
                <div class="form-actions">
                    <button class="restore-cancel" disabled={busy} onclick={cancelRestore}
                        >Cancel</button
                    >
                    <button class="primary" disabled={busy} onclick={confirmRestore}
                        >Restore saved settings</button
                    >
                </div>
            {:else}
                {#if currentDefault}
                    {@render snapshotEntry(currentDefault, true)}
                {/if}
                {#if desktop && !busy && entries.length === 0 && !currentDefault && !error}
                    <p class="empty compact">No saved defaults yet.</p>
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
            {#if status}<p class="success" role="status">{status}</p>{/if}
            {#if error}<p class="error" role="alert">{error}</p>{/if}
        </div>
    </dialog>
{/if}
