import type { Page } from '@playwright/test';
import { demoSnapshot, demoDetails, demoBootstrap } from '../../../src/lib/demo';
import { demoInventory } from '../../../src/lib/monitor';

export async function desktop(
    page: Page,
    options: { unavailable?: boolean; delay?: number; launch?: boolean } = {}
) {
    const sample = demoSnapshot();
    const inventory = demoInventory(sample, {
        search: '',
        sort: 'impact',
        view: 'processes',
        filter: 'all',
        offset: 0,
    });
    const details = sample.workloads.map((w) => demoDetails(sample, w.id));
    if (options.launch) {
        const detail = details.find((d) => d.workload.id === 'demo-browser')!;
        detail.launch = {
            program: 'Hermes',
            task: 'Background gateway',
            summary: 'Hermes is running its background gateway to handle messaging connections.',
            launcher: 'Hermes · Gateway log wrapper',
            evidence: ['Running command: hermes gateway run'],
            chain: [{ pid: detail.process.pid, role: 'Background gateway' }],
            limitation: 'This synthetic fixture identifies a command, not a particular request.',
            reference_url: null,
        };
    }
    const hidden = structuredClone(details[0]);
    hidden.workload = {
        ...hidden.workload,
        id: 'quiet-group',
        label: 'Quiet service',
        history: [],
    };
    hidden.process = {
        ...hidden.process,
        pid: 9900,
        name: 'quiet-worker',
        workload_id: 'quiet-group',
    };
    hidden.members = [hidden.process];
    hidden.member_count = 1;
    details.push(hidden);
    inventory.rows.push({
        workload: hidden.workload,
        process: hidden.process,
        identity: hidden.process,
    });
    sample.source = 'live';
    await page.addInitScript(
        ({ sample, inventory, details, bootstrap, options }) => {
            let callbackId = 0;
            const callbacks = new Map();
            const listeners = new Map();
            const asks: Record<string, unknown>[] = [];
            let summaries = 0;
            let batches = 0;
            const result = (pid?: number) => ({
                text: pid ? `Specific finding for PID ${pid}` : 'Room for your next task',
                insight: {
                    headline: pid ? `Activity of PID ${pid}` : 'Room for your next task',
                    finding: pid
                        ? 'The selected instance is processing a synthetic workload.'
                        : 'There is available CPU and memory capacity.',
                    next_step: 'Keep it running and watch for a sustained change.',
                },
                follow_ups: ['The whole window is stuck', 'It recovered now'],
                source: 'model',
                tier: 'intelligent',
                provider: 'meta',
                model: 'synthetic-model',
                route: ['Synthetic test fixture.'],
                estimated_cost_usd: 0,
                investigation: null,
                choices: [],
            });
            Object.defineProperty(window, 'isTauri', { value: true });
            Object.defineProperty(window, '__TAURI_INTERNALS__', {
                value: {
                    transformCallback: (callback: unknown) => {
                        callbacks.set(++callbackId, callback);
                        return callbackId;
                    },
                    unregisterCallback: (id: number) => callbacks.delete(id),
                    invoke: async (command: string, args: Record<string, any>) => {
                        if (command === 'plugin:event|listen') {
                            listeners.set(args.event, args.handler);
                            return listeners.size;
                        }
                        if (command === 'plugin:event|unlisten') return;
                        if (command === 'get_snapshot') return structuredClone(sample);
                        if (command === 'get_bootstrap') return bootstrap;
                        if (command === 'get_validation_queue') return null;
                        if (command === 'get_workload_icon') return null;
                        if (command === 'get_inside_view')
                            return {
                                workload_id: args.workloadId,
                                status: 'unavailable',
                                units: [],
                                labels: [],
                                limitations: [],
                            };
                        if (command === 'get_monitor_inventory') {
                            const q = args.query;
                            let rows = inventory.rows.filter(
                                (r) =>
                                    !q.search ||
                                    `${r.workload.label} ${r.process?.name} ${r.process?.pid}`
                                        .toLowerCase()
                                        .includes(q.search.toLowerCase())
                            );
                            if (q.view === 'apps')
                                rows = rows
                                    .filter(
                                        (r, i) =>
                                            rows.findIndex(
                                                (other) => other.workload.id === r.workload.id
                                            ) === i
                                    )
                                    .map((r) => ({ ...r, process: null }));
                            return {
                                ...inventory,
                                rows,
                                total: rows.length,
                                captured_at: sample.captured_at,
                            };
                        }
                        if (command === 'get_workload_details') {
                            const detail = structuredClone(
                                details.find((d) => d.workload.id === args.workloadId)!
                            );
                            detail.captured_at = sample.captured_at;
                            detail.profile = {
                                ...detail.profile,
                                source: 'reference',
                                text: 'A reusable application description.',
                            };
                            if (args.pid) {
                                const process = detail.members.find(
                                    (p) => p.pid === args.pid && p.start_time === args.startTime
                                );
                                if (!process)
                                    throw 'The selected process has exited or its PID was reused.';
                                detail.process = process;
                            }
                            return detail;
                        }
                        if (command === 'summarize_monitor') {
                            document.documentElement.dataset.summaryCalls = String(++summaries);
                            if (options.unavailable)
                                throw 'The configured cloud budget cannot cover this request.';
                            return result();
                        }
                        if (command === 'explain_monitor_profiles') {
                            if (!args.generate) return [];
                            document.documentElement.dataset.batchCalls = String(++batches);
                            return args.identities.map((identity: unknown) => ({
                                identity,
                                profile: {
                                    text: 'Helps this application draw its interface.',
                                    source: 'model',
                                    cached: false,
                                    provider: 'meta',
                                    model: 'synthetic-model',
                                    generated_at: sample.captured_at,
                                },
                            }));
                        }
                        if (command === 'explain_application') {
                            document.documentElement.dataset.explanationRequests = '1';
                            throw 'The fixture should investigate the selected process directly.';
                        }
                        if (command === 'ask') {
                            asks.push(structuredClone(args));
                            document.documentElement.dataset.askCalls = JSON.stringify(asks);
                            if (options.delay)
                                await new Promise((resolve) => setTimeout(resolve, options.delay));
                            if (options.unavailable)
                                return {
                                    ...result(args.pid),
                                    source: 'rules',
                                    unavailable:
                                        'The configured cloud budget cannot cover this request.',
                                };
                            return result(args.pid);
                        }
                        throw new Error(`Unexpected test command: ${command}`);
                    },
                },
            });
            window.addEventListener('fixture-incident', (event) => {
                const incident = (event as CustomEvent).detail;
                bootstrap.automation_cases = [incident];
                callbacks.get(listeners.get('automation-case'))?.({ payload: incident });
            });
            window.addEventListener('fixture-sample', (event) => {
                const change = (event as CustomEvent).detail ?? {};
                sample.captured_at += change.seconds ?? 5;
                if (change.cpu !== undefined) sample.cpu_percent = change.cpu;
                if (change.pressure) sample.pressure.level = change.pressure;
                if (change.pid) {
                    for (const detail of details)
                        for (const p of detail.members) {
                            if (p.pid === change.pid) {
                                if (change.processCpu !== undefined)
                                    p.cpu_percent = change.processCpu;
                                if (change.startTime !== undefined) p.start_time = change.startTime;
                            }
                        }
                }
                callbacks.get(listeners.get('telemetry'))?.({ payload: structuredClone(sample) });
            });
        },
        { sample, inventory, details, bootstrap: demoBootstrap, options }
    );
    await page.goto('/');
}
