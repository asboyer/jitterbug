import { expect, test, type Page } from '@playwright/test';
import { demoSnapshot, demoDetails, demoBootstrap, demoIncidents } from '../../src/lib/demo';

async function mockDesktop(page: Page, fail = false, unavailable = false, launch = false) {
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
    if (launch) {
        detail.can_generate_profile = false;
        detail.launch = {
            program: 'Hermes',
            task: 'Background gateway',
            summary:
                'Hermes is running its background gateway to handle messaging connections and agent requests.',
            launcher: 'Hermes · Gateway log wrapper',
            evidence: ['Running command: hermes gateway run'],
            chain: [{ pid: detail.process.pid, role: 'Background gateway' }],
            limitation:
                'This synthetic fixture identifies a command, not a particular user request.',
            reference_url: 'https://hermes-agent.nousresearch.com/docs/user-guide/messaging',
        };
    }
    await page.addInitScript(
        ({ snapshot, detail, bootstrap, fail, unavailable }) => {
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
                            if (unavailable)
                                return {
                                    text: 'The slowdown needs a closer look.',
                                    insight: {
                                        headline: 'The slowdown needs a closer look',
                                        finding: 'Low CPU does not rule out a frozen app.',
                                        next_step: 'Is the whole window frozen, or one page?',
                                    },
                                    unavailable:
                                        'The $2.00 cloud budget cannot cover this request. Review the cloud budget in Settings.',
                                    research: {
                                        title: 'Apple Safari troubleshooting',
                                        url: 'https://support.apple.com/en-us/102564',
                                        summary:
                                            'Test extensions or try the affected page in a private window.',
                                        checked_at: 1,
                                        status: 'cached',
                                    },
                                    source: 'rules',
                                    tier: 'rules',
                                    provider: null,
                                    model: null,
                                    route: ['Synthetic unavailable-provider fixture.'],
                                    estimated_cost_usd: 0,
                                    choices: [],
                                    investigation: null,
                                };

                            document.documentElement.dataset.insightTarget = String(
                                args.workloadId
                            );
                            document.documentElement.dataset.insightQuestion = JSON.stringify(
                                args.messages
                            );
                            return {
                                text: 'Activity is light. The selected app has available capacity. Keep it running.',
                                follow_ups: [
                                    'The whole window is stuck',
                                    'Just one tab is stuck',
                                    'It recovered now',
                                ],
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
            window.addEventListener('fixture-incident', (event) => {
                const incident = (event as CustomEvent).detail;
                bootstrap.automation_cases = [incident];
                callbacks.get(listeners.get('automation-case'))?.({ payload: incident });
            });
        },
        { snapshot, detail, bootstrap: demoBootstrap, fail, unavailable }
    );
    await page.goto('/');
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('button', { name: /Example Browser/ }).click();
}

test('runtime details lead with the running job and do not request an interpreter biography', async ({
    page,
}) => {
    await mockDesktop(page, false, false, true);
    await expect(page.getByRole('heading', { name: 'Why it’s running' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Hermes · Background gateway' })).toBeVisible();
    await expect(page.getByLabel('Origin overview')).toContainText('Hermes · Gateway log wrapper');
    await expect(page.locator('html')).not.toHaveAttribute('data-explanation-requests');
    await page.getByRole('button', { name: /Show origin evidence/ }).click();
    await expect(page.getByText('Running command: hermes gateway run')).toBeVisible();
    await page.evaluate(() => window.dispatchEvent(new Event('fixture-refresh')));
    await expect(page.getByText('Running command: hermes gateway run')).toBeVisible();
    await expect(page.locator('html')).not.toHaveAttribute('data-explanation-requests');
});

test('a memory prompt closes after recovery while its findings stay in Inbox', async ({ page }) => {
    await mockDesktop(page);
    await expect(page.getByRole('region', { name: 'Process explanation' })).toContainText(
        'This system component analyzes media'
    );
    const incident = demoIncidents()[0];
    incident.source = 'live';
    incident.stage = 'detected';
    incident.trigger.kind = 'memory_pressure';
    incident.trigger.key = 'memory-pressure';
    incident.proposal = null;
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await expect(prompt).toContainText('your computer is short on memory');
    incident.stage = 'completed';
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    await expect(prompt).toContainText('No automatic change was applied');
    incident.stage = 'resolved';
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    await expect(prompt).toHaveCount(0);
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await expect(page.getByText('Memory pressure detected', { exact: true })).toBeVisible();
    await expect(page.getByText('Trigger cleared', { exact: true })).toBeVisible();
});

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
    await insight.getByRole('button', { name: 'Investigate', exact: true }).click();
    await expect(insight.getByRole('heading', { name: 'Activity is light' })).toBeVisible();
    await expect(insight.getByText('Keep it running.', { exact: true })).toBeInViewport();
    await expect(insight.getByRole('button', { name: 'Check again' })).toBeInViewport();
    const purposeBox = await page
        .getByRole('region', { name: 'Process explanation' })
        .boundingBox();
    const insightBox = await insight.boundingBox();
    expect(Math.abs(purposeBox!.y - insightBox!.y)).toBeLessThan(2);
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
    await expect(assistant.getByRole('heading', { name: 'Activity is light' })).toHaveCount(1);
    await expect(assistant.locator('.conversation-history')).toBeVisible();
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

test('unavailable AI is visible instead of masquerading as a resource diagnosis', async ({
    page,
}) => {
    await mockDesktop(page, false, true);
    await expect(page.getByRole('region', { name: 'Process explanation' })).toContainText(
        'This system component'
    );
    const insight = page.getByRole('region', { name: 'Workload activity insight' });
    await insight.getByRole('button', { name: 'Investigate', exact: true }).click();
    await expect(insight.getByRole('status')).toContainText(
        'cloud budget cannot cover this request'
    );
    await expect(insight).toContainText('Local checks · AI unavailable');
    await expect(insight).toContainText('Low CPU does not rule out a frozen app');
    await expect(insight).not.toContainText('Current observations');
    await insight.getByText('Apple guidance · cached', { exact: true }).click();
    await expect(
        insight.getByRole('link', { name: 'Read Apple’s troubleshooting guide ↗' })
    ).toHaveAttribute('href', 'https://support.apple.com/en-us/102564');
});

test('the investigation keeps its latest answer readable and preserves keyboard-scrollable history', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.getByRole('button', { name: 'Ask a follow-up' }).click();
    const assistant = page.getByRole('complementary', { name: 'Contextual assistant' });
    const question = assistant.getByRole('textbox', { name: 'Ask Jitterbug' });
    await question.fill('Why is this app lagging?');
    await assistant.getByRole('button', { name: 'Ask ↑', exact: true }).click();
    await assistant.getByRole('button', { name: 'The whole window is stuck', exact: true }).click();
    await expect(assistant.locator('.insight-card')).toHaveCount(1);
    const history = assistant.locator('.conversation-history');
    await expect(history).not.toHaveAttribute('open');
    await history.locator('summary').click();
    await expect(history).toContainText('Why is this app lagging?');
    const scroll = assistant.getByRole('log', { name: 'Investigation conversation' });
    await expect(scroll).toHaveCSS('scrollbar-width', 'none');
    await scroll.focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('End');
    await expect.poll(() => scroll.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await expect(assistant.getByRole('textbox')).toBeVisible();
});
