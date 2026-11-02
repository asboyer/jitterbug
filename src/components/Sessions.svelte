<script lang="ts">
    import Icon from './Icon.svelte';

    type SessionKind = 'gaming' | 'development' | 'creative';
    type SessionStatus = 'ready' | 'active';

    interface SessionPreview {
        id: string;
        name: string;
        description: string;
        kind: SessionKind;
        status: SessionStatus;
        icon: 'code' | 'app' | 'activity';
        lastUsed: string;
        duration: string;
        focus: string[];
        metrics: {
            cpu: number;
            memory: number;
            pressure: string;
        };
        activity: number[];
    }

    const examples: SessionPreview[] = [
        {
            id: 'claude-worktrees',
            name: 'Claude worktrees',
            description:
                'Keep parallel coding agents responsive while builds and test suites share memory.',
            kind: 'development',
            status: 'active',
            icon: 'code',
            lastUsed: 'Active now',
            duration: '42 min',
            focus: ['Memory headroom', 'Agent fairness', 'Build coordination'],
            metrics: { cpu: 64, memory: 71, pressure: 'Normal' },
            activity: [34, 41, 38, 52, 47, 69, 63, 72, 66, 64],
        },
        {
            id: 'cs2',
            name: 'CS2 evening session',
            description:
                'Prioritize a smooth game session and keep background workloads from competing.',
            kind: 'gaming',
            status: 'ready',
            icon: 'app',
            lastUsed: 'Yesterday',
            duration: '1 hr 18 min',
            focus: ['Foreground responsiveness', 'Thermal awareness', 'Background activity'],
            metrics: { cpu: 48, memory: 56, pressure: 'Normal' },
            activity: [29, 33, 46, 51, 58, 54, 49, 44, 50, 48],
        },
        {
            id: 'video-edit',
            name: 'Video export',
            description:
                'Watch a long render while keeping the desktop usable for lightweight work.',
            kind: 'creative',
            status: 'ready',
            icon: 'activity',
            lastUsed: '4 days ago',
            duration: '58 min',
            focus: ['Sustained throughput', 'Memory pressure', 'Keep awake'],
            metrics: { cpu: 82, memory: 67, pressure: 'Elevated' },
            activity: [55, 62, 70, 77, 84, 89, 81, 86, 83, 82],
        },
    ];

    const suggestions = [
        {
            title: 'Game without background competition',
            description:
                'For games, launchers, voice chat, and predictable foreground performance.',
            icon: 'app' as const,
            prompt: 'I play CS2 most evenings and want background workloads to stay out of the way.',
        },
        {
            title: 'Parallel agent development',
            description:
                'For multiple worktrees, coding agents, builds, and tests running together.',
            icon: 'code' as const,
            prompt: 'I run several Claude worktrees in parallel and need to avoid memory slowdowns.',
        },
        {
            title: 'Long-running creative work',
            description:
                'For exports, renders, or other sustained jobs that should remain observable.',
            icon: 'activity' as const,
            prompt: 'I export large videos and want the render monitored while I continue light work.',
        },
    ];

    let sessions = $state(examples);
    let selectedId = $state(examples[0].id);
    let composerOpen = $state(false);
    let description = $state('');
    let sessionName = $state('');
    let notice = $state('');

    const selected = $derived(sessions.find((session) => session.id === selectedId) ?? sessions[0]);

    function openComposer(prompt = '') {
        description = prompt;
        sessionName = '';
        composerOpen = true;
        notice = '';
    }

    function savePreview() {
        const name = sessionName.trim() || 'New custom session';
        const session: SessionPreview = {
            id: `preview-${Date.now()}`,
            name,
            description: description.trim(),
            kind: 'development',
            status: 'ready',
            icon: 'activity',
            lastUsed: 'Not started',
            duration: '—',
            focus: ['Live monitoring', 'Memory headroom', 'Workload balance'],
            metrics: { cpu: 0, memory: 0, pressure: 'Waiting' },
            activity: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        sessions = [session, ...sessions];
        selectedId = session.id;
        composerOpen = false;
        notice =
            'Session preview created. It will not persist until the Sessions backend is connected.';
    }

    function togglePreview() {
        sessions = sessions.map((session) => ({
            ...session,
            status:
                session.id === selected.id
                    ? session.status === 'active'
                        ? 'ready'
                        : 'active'
                    : session.status === 'active'
                      ? 'ready'
                      : session.status,
            lastUsed: session.id === selected.id ? 'Active now' : session.lastUsed,
        }));
        notice =
            selected.status === 'active'
                ? 'Preview stopped. No system settings were changed.'
                : 'Preview activated. This is visual only—no monitoring or optimization is running.';
    }
</script>

<div class="sessions-page">
    <header class="sessions-heading">
        <div>
            <p class="eyebrow">REPEATABLE WORKSPACES</p>
            <h1>Sessions</h1>
            <p class="muted">
                Describe the work you do often, then return to one place to prepare, monitor, and
                review it.
            </p>
        </div>
        <button class="primary sessions-new" onclick={() => openComposer()}>
            <span aria-hidden="true">＋</span> New session
        </button>
    </header>

    <div class="sessions-preview-note" role="status">
        <Icon name="info" size={16} />
        <div>
            <strong>Frontend preview</strong>
            <span>
                Session controls and readings are sample data. Nothing here monitors or changes your
                computer yet.
            </span>
        </div>
    </div>

    {#if notice}
        <div class="sessions-notice" role="status">
            <span>{notice}</span>
            <button class="icon-button" aria-label="Dismiss message" onclick={() => (notice = '')}
                >×</button
            >
        </div>
    {/if}

    <div class="sessions-layout">
        <aside class="sessions-library panel" aria-label="Saved sessions">
            <div class="sessions-library-header">
                <div>
                    <h2>Your sessions</h2>
                    <p>{sessions.length} saved previews</p>
                </div>
                <button
                    class="icon-button"
                    aria-label="Create session"
                    onclick={() => openComposer()}>＋</button
                >
            </div>

            <div class="session-list">
                {#each sessions as session}
                    <button
                        class="session-row"
                        class:selected={session.id === selected.id}
                        aria-current={session.id === selected.id ? 'true' : undefined}
                        onclick={() => {
                            selectedId = session.id;
                            notice = '';
                        }}
                    >
                        <span class="session-row-icon"><Icon name={session.icon} size={18} /></span>
                        <span class="session-row-copy">
                            <strong>{session.name}</strong>
                            <small>{session.lastUsed}</small>
                        </span>
                        {#if session.status === 'active'}
                            <span class="session-active-dot" title="Active preview"></span>
                        {/if}
                    </button>
                {/each}
            </div>

            <div class="session-library-foot">
                <Icon name="bulb" size={16} />
                <span>Sessions will learn from measured runs once the backend is connected.</span>
            </div>
        </aside>

        <section class="session-workspace">
            <article class="session-hero panel">
                <div class="session-hero-main">
                    <span class="session-hero-icon"><Icon name={selected.icon} size={23} /></span>
                    <div>
                        <div class="session-title-line">
                            <h2>{selected.name}</h2>
                            <span class:active={selected.status === 'active'} class="session-state">
                                {selected.status === 'active' ? 'Active preview' : 'Ready'}
                            </span>
                        </div>
                        <p>{selected.description}</p>
                    </div>
                </div>
                <div class="session-hero-actions">
                    <button>•••</button>
                    <button class="primary" onclick={togglePreview}>
                        <Icon name={selected.status === 'active' ? 'pause' : 'play'} size={15} />
                        {selected.status === 'active' ? 'Stop preview' : 'Activate preview'}
                    </button>
                </div>
            </article>

            <div class="session-metrics">
                <article class="session-metric panel">
                    <div class="session-metric-label">
                        <span>CPU</span><Icon name="cpu" size={15} />
                    </div>
                    <strong>{selected.metrics.cpu}%</strong>
                    <div class="session-meter">
                        <i style:width={`${selected.metrics.cpu}%`}></i>
                    </div>
                    <small>Sample session average</small>
                </article>
                <article class="session-metric panel">
                    <div class="session-metric-label">
                        <span>Memory</span><Icon name="memory" size={15} />
                    </div>
                    <strong>{selected.metrics.memory}%</strong>
                    <div class="session-meter">
                        <i style:width={`${selected.metrics.memory}%`}></i>
                    </div>
                    <small>Sample memory in use</small>
                </article>
                <article class="session-metric panel">
                    <div class="session-metric-label">
                        <span>Pressure</span><Icon name="activity" size={15} />
                    </div>
                    <strong class="session-pressure">{selected.metrics.pressure}</strong>
                    <div class="session-pressure-line">
                        <span class:raised={selected.metrics.pressure === 'Elevated'}></span>
                        Preview health
                    </div>
                    <small>Would use live OS readings</small>
                </article>
            </div>

            <div class="session-detail-grid">
                <article class="session-activity panel">
                    <div class="session-card-heading">
                        <div>
                            <p class="eyebrow">CURRENT RUN</p>
                            <h2>Live session activity</h2>
                        </div>
                        <span>{selected.duration}</span>
                    </div>
                    <div class="session-chart" aria-label="Sample activity chart">
                        {#each selected.activity as value}
                            <i style:height={`${Math.max(value, 4)}%`}></i>
                        {/each}
                    </div>
                    <div class="session-chart-legend">
                        <span><i></i> Resource activity</span>
                        <span>Sample data</span>
                    </div>
                </article>

                <article class="session-focus panel">
                    <div class="session-card-heading">
                        <div>
                            <p class="eyebrow">SESSION INTENT</p>
                            <h2>What Jitterbug will watch</h2>
                        </div>
                    </div>
                    <div class="focus-list">
                        {#each selected.focus as item, index}
                            <div>
                                <span>{index + 1}</span>
                                <div>
                                    <strong>{item}</strong>
                                    <small>
                                        {index === 0
                                            ? 'Primary signal for this workflow.'
                                            : 'Supporting context for measured decisions.'}
                                    </small>
                                </div>
                            </div>
                        {/each}
                    </div>
                </article>
            </div>

            <article class="session-history panel">
                <div class="session-card-heading">
                    <div>
                        <p class="eyebrow">RECENT RUNS</p>
                        <h2>Session history</h2>
                    </div>
                    <button class="text-button">View all</button>
                </div>
                <div class="session-history-row">
                    <span class="history-status"><i></i></span>
                    <div>
                        <strong>{selected.lastUsed}</strong>
                        <small>{selected.duration} · Sample run completed normally</small>
                    </div>
                    <div class="history-result">
                        <strong>{selected.metrics.memory}% memory</strong>
                        <small>Preview measurement</small>
                    </div>
                </div>
            </article>
        </section>
    </div>

    <section class="session-templates">
        <div class="session-card-heading">
            <div>
                <p class="eyebrow">STARTING POINTS</p>
                <h2>Build from a familiar workflow</h2>
            </div>
        </div>
        <div class="session-template-grid">
            {#each suggestions as suggestion}
                <button
                    class="session-template panel"
                    onclick={() => openComposer(suggestion.prompt)}
                >
                    <span><Icon name={suggestion.icon} size={19} /></span>
                    <strong>{suggestion.title}</strong>
                    <small>{suggestion.description}</small>
                    <em>Use template →</em>
                </button>
            {/each}
        </div>
    </section>
</div>

{#if composerOpen}
    <div class="modal-backdrop sessions-modal-backdrop">
        <div
            class="sessions-composer panel"
            role="dialog"
            aria-modal="true"
            aria-labelledby="session-composer-title"
            tabindex="-1"
        >
            <div class="session-card-heading">
                <div>
                    <p class="eyebrow">NEW SESSION</p>
                    <h2 id="session-composer-title">What do you do often?</h2>
                </div>
                <button
                    class="icon-button"
                    aria-label="Close"
                    onclick={() => (composerOpen = false)}>×</button
                >
            </div>
            <p class="composer-intro">
                Describe the apps, tasks, and outcome in your own words. This preview creates the
                interface only and will not start monitoring.
            </p>
            <label>
                Session name <span>Optional</span>
                <input bind:value={sessionName} maxlength="80" placeholder="Claude worktrees" />
            </label>
            <label>
                Describe your workflow
                <textarea
                    bind:value={description}
                    maxlength="500"
                    rows="6"
                    placeholder="I run several Claude worktrees, builds, and tests in parallel. Keep enough memory available so the computer stays responsive."
                ></textarea>
            </label>
            <div class="composer-guidance">
                <Icon name="bulb" size={16} />
                <span
                    >Include the apps you use, what usually slows down, and what matters most.</span
                >
            </div>
            <div class="form-actions">
                <button onclick={() => (composerOpen = false)}>Cancel</button>
                <button class="primary" disabled={!description.trim()} onclick={savePreview}
                    >Create session preview</button
                >
            </div>
        </div>
    </div>
{/if}
