import { expect, test, type Page } from '@playwright/test';
import { demoSnapshot, demoDetails, demoBootstrap } from '../../src/lib/demo';

async function mockDesktop(page: Page, fail = false) {
    const snapshot = demoSnapshot();
    const detail = demoDetails(snapshot, 'demo-browser');
    snapshot.source = 'live';
    detail.application = {
        name: 'exampled',
        executable_name: 'exampled',
        component: 'ExampleAnalysis.framework',
        component_kind: 'framework',
        installation: 'macOS system component',
        identifier: 'example.analysis',
        version: '1',
        identity_source: 'Component bundle metadata and executable location',
    };
    detail.profile = {
        text: 'Application or operating-system process. Purpose is not established from its name alone.',
        source: 'inference',
        provider: null,
        model: null,
        generated_at: null,
        cached: false,
    };
    detail.can_generate_profile = true;
    detail.profile_key = 'synthetic-component';
    await page.addInitScript(
        ({ snapshot, detail, bootstrap, fail }) => {
            let requests = 0;
            let callbackId = 0;
            const callbacks = new Map();
            const listeners = new Map();
            Object.defineProperty(window, 'isTauri', { value: true });
            Object.defineProperty(window, '__TAURI_INTERNALS__', {
                value: {
                    transformCallback: (callback: unknown) => {
                        callbacks.set(++callbackId, callback);
                        return callbackId;
                    },
                    unregisterCallback: (id: number) => callbacks.delete(id),
                    invoke: async (command: string, args: Record<string, unknown>) => {
                        if (command === 'plugin:event|listen') {
                            listeners.set(args.event, args.handler);
                            return listeners.size;
                        }
                        if (command === 'plugin:event|unlisten') return;
                        if (command === 'get_snapshot') return snapshot;
                        if (command === 'search_workloads') return [detail.workload];
                        if (command === 'get_bootstrap') return bootstrap;
                        if (command === 'get_workload_icon') return null;
                        if (command === 'get_workload_details') return structuredClone(detail);
                        if (command === 'ask') {
                            document.documentElement.dataset.insightTarget = String(
                                args.workloadId
                            );
                            document.documentElement.dataset.insightQuestion = JSON.stringify(
                                args.messages
                            );
                            return {
                                text: 'Activity is light. The selected app has available capacity. Keep it running.',
                                insight: {
                                    headline: 'Activity is light',
                                    finding: 'The selected app has available capacity.',
                                    next_step: 'Keep it running.',
                                },
                                source: 'model',
                                tier: 'intelligent',
                                provider: 'meta',
                                model: 'synthetic-model',
                                route: ['Selected workload resolved locally.'],
                                estimated_cost_usd: 0,
                                investigation: {
                                    target: {
                                        workload_id: snapshot.workloads[0].id,
                                        label: 'Example Browser',
                                        pid: null,
                                    },
                                    captured_at: snapshot.captured_at,
                                    cpu_percent: 2,
                                    memory_bytes: 104857600,
                                    process_count: 1,
                                    processes: [],
                                    history: [],
                                    limits: 'Synthetic measurements only.',
                                },
                                choices: [],
                            };
                        }
                        if (command === 'explain_application') {
                            document.documentElement.dataset.explanationRequests = String(
                                ++requests
                            );
                            await new Promise((resolve) => setTimeout(resolve, 500));
                            // Rust command errors cross the IPC boundary as strings.
                            if (fail) throw 'The intelligent provider is excluded by your mode.';
                            detail.profile = {
                                text: 'This system component analyzes media for search. It can work in the background while media is indexed.',
                                source: 'model',
                                provider: 'meta',
                                model: 'synthetic-model',
                                generated_at: 1,
                                cached: true,
                            };
                            return detail.profile;
                        }
                        throw new Error(`Unexpected test command: ${command}`);
                    },
                },
            });
            window.addEventListener('fixture-refresh', () => {
                snapshot.captured_at += 5;
                callbacks.get(listeners.get('telemetry'))?.({ payload: snapshot });
            });
        },
        { snapshot, detail, bootstrap: demoBootstrap, fail }
    );
    await page.goto('/');
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('button', { name: /Example Browser/ }).click();
}

test('opening a process explains it automatically once and keeps origins visible', async ({
    page,
}) => {
    await mockDesktop(page);
    const explanation = page.getByRole('region', { name: 'Process explanation' });
    await expect(explanation.getByRole('status')).toContainText('Identifying');
    await expect(explanation).toContainText('This system component analyzes media');
    await expect(explanation).toContainText('ExampleAnalysis.framework');
    for (let i = 0; i < 4; i++)
        await page.evaluate(() => window.dispatchEvent(new Event('fixture-refresh')));
    await expect(page.locator('html')).toHaveAttribute('data-explanation-requests', '1');
    await expect(explanation).not.toContainText('Purpose is not established');
    await explanation.getByRole('button', { name: 'Show origin evidence' }).click();
    await expect(page.getByRole('region', { name: 'Process origins' })).toBeFocused();
    await page.getByRole('button', { name: 'Back', exact: false }).click();
    await page.getByRole('button', { name: /Example Browser/ }).click();
    await expect(explanation).toContainText('AI · saved on this device');
    await expect(page.locator('html')).toHaveAttribute('data-explanation-requests', '1');
});

test('blocked explanations show a reason and require an explicit retry', async ({ page }) => {
    await mockDesktop(page, true);
    const explanation = page.getByRole('region', { name: 'Process explanation' });
    await expect(explanation).toContainText('The explanation couldn’t be completed');
    await explanation.getByRole('button', { name: 'Why the explanation is unavailable' }).click();
    await expect(explanation).toContainText('excluded by your mode');
    for (let i = 0; i < 4; i++)
        await page.evaluate(() => window.dispatchEvent(new Event('fixture-refresh')));
    await expect(page.locator('html')).toHaveAttribute('data-explanation-requests', '1');
    await explanation.getByRole('button', { name: 'Retry explanation' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-explanation-requests', '2');
});

test('process insights stay on the page and use the selection without another picker', async ({
    page,
}) => {
    await mockDesktop(page);
    await expect(page.getByRole('region', { name: 'Process explanation' })).toContainText(
        'This system component'
    );
    const insight = page.getByRole('region', { name: 'Workload activity insight' });
    await insight.getByRole('button', { name: 'Check activity', exact: true }).click();
    await expect(insight.getByRole('heading', { name: 'Activity is light' })).toBeVisible();
    await expect(insight).toContainText('Keep it running.');
    await expect(page.locator('html')).toHaveAttribute('data-insight-target', 'demo-browser');
    await expect(insight.locator('details[open]')).toHaveCount(0);
    await expect(page.getByRole('complementary', { name: 'Contextual assistant' })).toHaveCount(0);
    await insight.getByRole('button', { name: 'Ask a follow-up' }).click();
    const assistant = page.getByRole('complementary', { name: 'Contextual assistant' });
    await expect(assistant.getByRole('textbox', { name: 'Ask Jitterbug' })).toHaveValue('');
    await expect(assistant).not.toContainText('Why is Spotify');
    await expect(assistant.getByRole('heading', { name: 'Activity is light' })).toBeVisible();
    await assistant
        .getByRole('textbox', { name: 'Ask Jitterbug' })
        .fill('Should I keep it running?');
    await assistant.getByRole('button', { name: 'Ask ↑', exact: true }).click();
    await expect(assistant.getByRole('heading', { name: 'Activity is light' })).toHaveCount(2);
    await expect(page.locator('html')).toHaveAttribute('data-insight-target', 'demo-browser');
});

test('origin headings have deliberate spacing and the compact page fits a small window', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 650 });
    await mockDesktop(page);
    await page.getByRole('button', { name: 'Show origin evidence' }).click();
    const origin = page.getByRole('region', { name: 'Process origins' });
    await expect(origin).toBeFocused();
    const heading = origin.locator('.ancestor-heading').last();
    const name = await heading.locator('strong').boundingBox();
    const pid = await heading.locator('.pid').boundingBox();
    expect(pid!.x - name!.x - name!.width).toBeGreaterThanOrEqual(10);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});

test('search can open a workload outside the dashboard list', async ({ page }) => {
    await mockDesktop(page);
    await page.getByRole('button', { name: 'Back', exact: false }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('quiet-helper');
    // The fixture returns a full-inventory match that client-side name filtering cannot find.
    await page.getByRole('button', { name: /Example Browser/ }).click();
    await expect(page.getByRole('heading', { name: 'Example Browser', exact: true })).toBeVisible();
});
