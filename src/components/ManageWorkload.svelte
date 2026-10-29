<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory } from '../lib/format';
    import type { Snapshot } from '../lib/types';
    import type {
        ManagedProfile,
        ManagementCandidate,
        ManagementDiscovery,
        ManagementPreview,
        ManagementSelection,
    } from '../lib/manage';

    let {
        selection,
        snapshot,
        onclose,
    }: { selection: ManagementSelection; snapshot: Snapshot; onclose: () => void } = $props();
    let dialog: HTMLDialogElement;
    let body: HTMLDivElement | undefined = $state();
    let page = $state<'choose' | 'edit' | 'review' | 'managed'>('choose');
    let discovery: ManagementDiscovery | null = $state(null);
    let profiles: ManagedProfile[] = $state([]);
    let preview: ManagementPreview | null = $state(null);
    let project = $state('');
    let name = $state('');
    let executable = $state('');
    let args = $state('');
    let memoryMib = $state(2048);
    let evidence = $state('Enter a trusted command that finishes, such as a build or test.');
    let clients: string[] = $state([]);
    let approved = $state(false);
    let busy = $state(false);
    let error = $state('');
    let notice = $state('');
    let saved = $state('');
    let revoke = $state('');
    let disposed = false;
    const orderedProfiles = $derived(
        [...profiles].sort(
            (a, b) => Number(b.profile.name === saved) - Number(a.profile.name === saved)
        )
    );
    const basename = (path: string) => path.split(/[/\\]/).filter(Boolean).at(-1) ?? path;
    const clientNames: Record<string, string> = {
        claude: 'Claude Code',
        codex: 'Codex',
        meta: 'Muse Code',
    };

    onMount(() => {
        dialog.showModal();
        void discover(selection);
        return () => {
            disposed = true;
            if (preview) void invoke('discard_managed_preview', { token: preview.token });
        };
    });

    $effect(() => {
        snapshot.captured_at;
        if (page === 'managed') void refresh();
    });

    $effect(() => {
        page;
        body?.scrollTo({ top: 0 });
    });

    async function refresh() {
        try {
            const value = await invoke<ManagedProfile[]>('get_managed_workloads');
            if (!disposed) profiles = value;
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        }
    }

    async function discover(source: ManagementSelection) {
        if (busy) return;
        busy = true;
        error = '';
        try {
            const value = await invoke<ManagementDiscovery>('discover_managed_workload', {
                selection: source,
            });
            if (disposed) return;
            discovery = value;
            project = value.worktree ?? project;
            await refresh();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    function choose(candidate: ManagementCandidate) {
        project = candidate.worktree;
        executable = candidate.command[0] ?? '';
        args = candidate.command.slice(1).join('\n');
        const folder = project.split(/[/\\]/).filter(Boolean).at(-1) ?? 'project';
        name = `${folder}-${candidate.command.at(-1) ?? 'validation'}`
            .replace(/[^a-zA-Z0-9_.-]+/g, '-')
            .slice(0, 96);
        memoryMib = candidate.memory_mib;
        evidence = candidate.evidence;
        clients = [];
        error = '';
        notice = '';
        page = 'edit';
    }

    function edit(profile: ManagedProfile) {
        choose({
            label: profile.profile.name,
            worktree: profile.profile.worktree,
            command: profile.profile.command,
            memory_mib: profile.profile.memory_bytes / 1024 / 1024,
            evidence: 'Already approved. Review to add or refresh client connections.',
            script: null,
        });
        name = profile.profile.name;
        clients = profile.connections.filter((c) => c.configured).map((c) => c.client);
    }

    async function review() {
        if (busy) return;
        busy = true;
        error = '';
        try {
            if (preview) await invoke('discard_managed_preview', { token: preview.token });
            preview = await invoke<ManagementPreview>('preview_managed_workload', {
                request: {
                    name: name.trim(),
                    worktree: project.trim(),
                    command: [executable.trim(), ...args.split('\n').filter((a) => a.length)],
                    memory_mib: memoryMib,
                    clients,
                },
            });
            if (disposed) {
                void invoke('discard_managed_preview', { token: preview.token });
                return;
            }
            approved = false;
            page = 'review';
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function approve() {
        if (!preview || !approved || busy) return;
        busy = true;
        error = '';
        try {
            saved = await invoke<string>('approve_managed_workload', { token: preview.token });
            notice = 'Profile approved. Future submitted runs share the queue.';
            await refresh();
            page = 'managed';
        } catch (cause) {
            error = errorMessage(cause);
            preview = null;
            page = 'edit';
        } finally {
            busy = false;
        }
    }

    async function run(profile: string) {
        if (busy) return;
        busy = true;
        error = '';
        try {
            await invoke('run_managed_workload', { name: profile });
            notice = 'Validation submitted. Keep Jitterbug open to schedule it.';
            await refresh();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function remove(profile: string) {
        if (busy) return;
        busy = true;
        error = '';
        try {
            await invoke('revoke_managed_workload', { name: profile });
            revoke = '';
            notice = 'Approval revoked. Waiting jobs were cancelled; running work continues.';
            await refresh();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<dialog
    bind:this={dialog}
    {onclose}
    oncancel={(event) => {
        if (busy) event.preventDefault();
    }}
    aria-labelledby="manage-title"
>
    <header>
        <div>
            <span class="eyebrow">Shared validation</span>
            <h2 id="manage-title">Manage this workload</h2>
        </div>
        <button
            class="text-button"
            aria-label="Close workload setup"
            disabled={busy}
            onclick={onclose}>✕</button
        >
    </header>
    <nav aria-label="Workload setup">
        <button
            class:selected={page !== 'managed'}
            disabled={busy}
            onclick={() => {
                page = 'choose';
                error = '';
            }}>Set up</button
        >
        <button
            class:selected={page === 'managed'}
            disabled={busy}
            onclick={() => {
                page = 'managed';
                error = '';
            }}>Managed · {profiles.length}</button
        >
    </nav>
    <div class="body" bind:this={body}>
        {#if error}<p class="banner error" role="alert">{error}</p>{/if}
        {#if notice && page === 'managed'}<p class="notice" role="status">{notice}</p>{/if}
        {#if page === 'choose'}
            <div class="intro">
                <span class="state">Observed</span>
                <h3>Choose a repeatable check</h3>
                <p>{discovery?.message ?? 'Reading local command evidence…'}</p>
            </div>
            {#if busy}<p class="muted" role="status">Looking for commands…</p>{/if}
            <div class="candidates">
                {#each discovery?.candidates ?? [] as candidate}
                    <button class="candidate" disabled={busy} onclick={() => choose(candidate)}>
                        <span
                            ><strong>{candidate.label}</strong><code
                                >{candidate.command.join(' ')}</code
                            ><small>{candidate.worktree}</small></span
                        ><span aria-hidden="true">›</span>
                    </button>
                {/each}
            </div>
            <form
                onsubmit={(event) => {
                    event.preventDefault();
                    void discover({ worktree: project });
                }}
            >
                <label for="manage-folder">Project folder</label>
                <div class="inline">
                    <input
                        id="manage-folder"
                        placeholder="Absolute path to a project"
                        bind:value={project}
                        required
                    /><button disabled={busy || !project.trim()}>Find checks</button>
                </div>
            </form>
            <button
                class="text-button"
                disabled={busy}
                onclick={() => {
                    name = '';
                    executable = '';
                    args = '';
                    clients = [];
                    page = 'edit';
                }}>Enter a command manually</button
            >
            <p class="footnote">
                Apps and background services stay observed. This queue is for commands that finish.
                Existing processes keep running normally.
            </p>
        {:else if page === 'edit'}
            <h3>Approve future runs</h3>
            <p class="muted">{evidence}</p>
            <form
                onsubmit={(event) => {
                    event.preventDefault();
                    void review();
                }}
            >
                <label for="profile-name">Profile name</label><input
                    id="profile-name"
                    bind:value={name}
                    pattern="[a-zA-Z0-9_.\-]+"
                    maxlength="96"
                    required
                    placeholder="project-tests"
                />
                <label for="profile-folder">Working folder</label><input
                    id="profile-folder"
                    bind:value={project}
                    required
                />
                <label for="profile-executable">Executable</label><input
                    id="profile-executable"
                    bind:value={executable}
                    required
                    placeholder="npm or an absolute executable path"
                />
                <label for="profile-args">Arguments <span class="muted">· one per line</span></label
                ><textarea
                    id="profile-args"
                    rows="3"
                    bind:value={args}
                    spellcheck="false"
                    placeholder={'run\ncheck'}></textarea>
                <div class="memory-row">
                    <label for="profile-memory">Additional memory estimate</label><input
                        id="profile-memory"
                        type="number"
                        min="128"
                        max="131072"
                        step="128"
                        bind:value={memoryMib}
                        required
                    /><span>MiB</span>
                </div>
                <small class="muted"
                    >Admission guidance; running work is not capped or killed if it grows.</small
                >
                <fieldset>
                    <legend>Add or refresh connections</legend>
                    <div class="clients">
                        {#each Object.entries(clientNames) as [id, label]}<label
                                ><input
                                    type="checkbox"
                                    value={id}
                                    bind:group={clients}
                                />{label}</label
                            >{/each}
                    </div>
                    <p class="footnote">
                        Optional. Adds a private MCP entry for your user account. The client
                        receives only approved profiles; normal tool approvals stay in place. You
                        can also run directly from Jitterbug.
                    </p>
                </fieldset>
                <div class="actions">
                    <button
                        type="button"
                        disabled={busy}
                        onclick={() => {
                            page = 'choose';
                        }}>Back</button
                    ><button class="primary" disabled={busy}
                        >{busy ? 'Preparing preview…' : 'Review changes'}</button
                    >
                </div>
            </form>
        {:else if page === 'review' && preview}
            <h3>Review {preview.profile.name}</h3>
            <div class="review-card">
                <span class="eyebrow">Fixed command</span><code
                    >{preview.profile.command.map((arg) => JSON.stringify(arg)).join(' ')}</code
                ><span class="eyebrow">Working folder</span><code>{preview.profile.worktree}</code>
                <p>{memory(preview.profile.memory_bytes)} estimated additional memory</p>
            </div>
            {#if preview.project_scripts}<details>
                    <summary>Project scripts this command may invoke</summary>
                    <pre>{preview.project_scripts}</pre>
                </details>{/if}
            {#if preview.changes.length}<h4>Client configuration changes</h4>{/if}
            {#each preview.changes as change}<details>
                    <summary>{change.path}</summary>
                    <pre>{change.description}</pre>
                </details>{/each}
            <p class="footnote">
                Stored locally. Client connections expose the approved command, working folder, and
                requested output to that harness and potentially its model provider. Restart clients
                after connecting. MCP instructions ask agents to queue matching checks; commands
                launched elsewhere remain observed.
            </p>
            <label class="approval"
                ><input type="checkbox" bind:checked={approved} /><span
                    >I trust this project and approve this fixed command to run as my OS user,
                    outside the requesting agent’s sandbox. It finishes without leaving background
                    services.</span
                ></label
            >
            <div class="actions">
                <button
                    disabled={busy}
                    onclick={() => {
                        page = 'edit';
                    }}>Back</button
                ><button class="primary" disabled={!approved || busy} onclick={approve}
                    >{busy
                        ? 'Saving…'
                        : preview.existing
                          ? 'Approve connections'
                          : 'Approve profile'}</button
                >
            </div>
        {:else if page === 'managed'}
            <div class="intro">
                <h3>Ready when you need them</h3>
                <p>
                    One shared slot across connected sessions. “Managed” means approved for future
                    submitted runs.
                </p>
            </div>
            {#each orderedProfiles as item (item.profile.name)}
                <article class="managed-card" class:highlight={item.profile.name === saved}>
                    <div class="profile-heading">
                        <strong>{item.profile.name}</strong><span
                            class="state"
                            class:working={['running', 'waiting'].includes(item.state)}
                            >{item.state.replaceAll('_', ' ')}</span
                        >
                    </div>
                    <p class="profile-caption">
                        {basename(item.profile.worktree)} · {memory(item.profile.memory_bytes)} estimate{item.last_result
                            ? ` · Last check: ${item.last_result.split(' · ')[0]}`
                            : ''}
                    </p>
                    <div class="connections">
                        {#each item.connections.filter((c) => c.configured) as connection}<span
                                title={connection.message}
                                class:connected={connection.configured}
                                >{clientNames[connection.client]} · {connection.configured
                                    ? connection.last_seen
                                        ? 'contact observed'
                                        : 'restart to connect'
                                    : 'not connected'}</span
                            >{:else}<span>Desktop / terminal</span>{/each}
                    </div>
                    <details>
                        <summary>Command & connection details</summary>
                        <code
                            >{item.profile.command
                                .map((arg) => JSON.stringify(arg))
                                .join(' ')}</code
                        >
                        <small class="path">{item.profile.worktree}</small>
                        {#if item.last_result}<p class="muted">{item.last_result}</p>{/if}
                        {#each item.connections as connection}<p class="muted">
                                {clientNames[connection.client]}: {connection.message}{connection.last_seen
                                    ? ` Last contact ${new Date(connection.last_seen * 1000).toLocaleString()}.`
                                    : ''}
                            </p>{/each}
                    </details>
                    <div class="actions">
                        <button
                            disabled={busy ||
                                ['running', 'waiting', 'needs_review'].includes(item.state)}
                            onclick={() => run(item.profile.name)}>Run validation</button
                        ><button disabled={busy} onclick={() => edit(item)}>Connections</button
                        ><button
                            class="text-button"
                            disabled={busy}
                            onclick={() => {
                                revoke = item.profile.name;
                            }}>Revoke</button
                        >
                    </div>
                    {#if revoke === item.profile.name}<div class="revoke">
                            <p>Revoke future runs and cancel waiting jobs for this profile?</p>
                            <button disabled={busy} onclick={() => remove(item.profile.name)}
                                >Revoke approval</button
                            ><button
                                class="text-button"
                                onclick={() => {
                                    revoke = '';
                                }}>Keep profile</button
                            >
                        </div>{/if}
                </article>
            {:else}<p class="muted">
                    No approved profiles yet. Choose a workload or a project to get started.
                </p>{/each}
        {/if}
    </div>
</dialog>

<style>
    dialog {
        width: min(720px, calc(100vw - 48px));
        max-height: calc(100vh - 48px);
        padding: 0;
        border: 1px solid var(--border);
        border-radius: 16px;
        color: var(--text);
        background: var(--panel, #242426);
        box-shadow: 0 24px 80px #0005;
    }
    dialog::backdrop {
        background: #0006;
        backdrop-filter: blur(4px);
    }
    header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 20px 24px 12px;
    }
    h2 {
        margin: 4px 0 0;
        font-size: 21px;
    }
    h3 {
        font-size: 17px;
        margin: 0 0 8px;
    }
    h4 {
        margin: 18px 0 10px;
    }
    p {
        line-height: 1.5;
    }
    nav {
        display: flex;
        gap: 4px;
        margin: 0 24px 16px;
        padding: 3px;
        background: var(--surface, #0002);
        border-radius: 9px;
    }
    nav button {
        flex: 1;
        border: 0;
        background: transparent;
    }
    nav button.selected {
        background: var(--hover, #8882);
        box-shadow: 0 1px 4px #0002;
    }
    .body {
        padding: 4px 24px 24px;
        max-height: calc(100vh - 200px);
        overflow-y: auto;
    }
    .eyebrow {
        color: var(--muted);
        font-size: 10px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
    }
    .intro {
        margin-bottom: 18px;
    }
    .intro p {
        color: var(--muted);
        margin: 6px 0;
    }
    .intro .state {
        display: inline-block;
        margin-bottom: 12px;
    }
    .state {
        font-size: 11px;
        padding: 4px 8px;
        background: #8882;
        border-radius: 6px;
        text-transform: capitalize;
    }
    .state.working {
        color: var(--accent);
        background: #438bed18;
    }
    .candidate {
        display: flex;
        width: 100%;
        justify-content: space-between;
        align-items: center;
        text-align: left;
        padding: 12px;
        margin-bottom: 6px;
    }
    .candidate span:first-child {
        min-width: 0;
    }
    code,
    .path,
    .candidate small {
        display: block;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
        margin-top: 5px;
        font-size: 11px;
        color: var(--muted);
    }
    form > label {
        display: block;
        margin: 14px 0 6px;
        font-size: 12px;
    }
    form > input,
    textarea {
        width: 100%;
        box-sizing: border-box;
    }
    textarea {
        resize: vertical;
        padding: 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--surface, #0002);
        color: var(--text);
        font-family: monospace;
    }
    .inline,
    .memory-row {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .inline input {
        min-width: 0;
        flex: 1;
    }
    .memory-row {
        margin: 16px 0 8px;
    }
    .memory-row label {
        flex: 1;
    }
    .memory-row input {
        width: 96px;
    }
    fieldset {
        border: 0;
        padding: 0;
        margin: 22px 0 0;
    }
    legend {
        font-size: 12px;
        margin-bottom: 10px;
    }
    .clients {
        display: flex;
        flex-wrap: wrap;
        gap: 16px;
    }
    .clients label,
    .approval {
        display: flex;
        align-items: flex-start;
        gap: 8px;
    }
    input[type='checkbox'] {
        flex-shrink: 0;
    }
    .footnote {
        font-size: 11px;
        line-height: 1.5;
        color: var(--muted);
        margin: 12px 0;
    }
    .actions {
        display: flex;
        justify-content: flex-end;
        gap: 8px;
        margin-top: 18px;
        flex-wrap: wrap;
    }
    .review-card,
    .managed-card {
        padding: 16px;
        border: 1px solid var(--border);
        border-radius: 10px;
        margin-bottom: 12px;
    }
    .review-card .eyebrow {
        display: block;
        margin-top: 10px;
    }
    .review-card .eyebrow:first-child {
        margin-top: 0;
    }
    .review-card code {
        color: var(--text);
    }
    details {
        margin: 12px 0;
        font-size: 12px;
    }
    summary {
        cursor: pointer;
        overflow-wrap: anywhere;
    }
    pre {
        font-size: 11px;
        line-height: 1.5;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        background: #8881;
        border-radius: 6px;
        padding: 12px;
        max-height: 220px;
        overflow-y: auto;
    }
    .approval {
        line-height: 1.5;
        font-size: 12px;
        margin-top: 20px;
    }
    .profile-heading {
        display: flex;
        gap: 8px;
        align-items: center;
        justify-content: space-between;
    }
    .profile-caption {
        color: var(--muted);
        margin: 8px 0;
    }
    .connections {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-top: 12px;
    }
    .connections span {
        font-size: 10px;
        color: var(--muted);
        background: #8881;
        border-radius: 5px;
        padding: 5px 7px;
    }
    .connections .connected {
        color: var(--text);
    }
    .managed-card.highlight {
        border-color: var(--accent);
    }
    .managed-card p {
        font-size: 11px;
    }
    .notice {
        font-size: 12px;
        border-left: 3px solid var(--accent);
        padding-left: 12px;
    }
    .revoke {
        border-top: 1px solid var(--border);
        margin-top: 12px;
        padding-top: 6px;
    }
</style>
