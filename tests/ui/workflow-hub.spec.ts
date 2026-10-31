import { expect, test } from '@playwright/test';

// These interactions use the explicit demo adapter. Native scheduling/acknowledgment
// is independently exercised by the validation_queue integration tests.
test('workflow hub holds checks during pressure and resumes them when room returns', async ({
    page,
}) => {
    const external: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith('http://127.0.0.1:1420')) external.push(request.url());
    });
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    await expect(hub.getByRole('heading', { name: 'Room to work.' })).toBeVisible();
    const search = hub.getByRole('article', { name: 'Check search changes' });
    const checkout = hub.getByRole('article', { name: 'Validate checkout' });
    await hub.getByRole('button', { name: 'Resource spike', exact: true }).click();
    await expect(hub.getByRole('meter', { name: 'Total CPU usage' })).toHaveAttribute(
        'aria-valuenow',
        '88'
    );
    const notice = page.getByRole('region', { name: 'Make room for your apps', exact: true });
    await notice.getByRole('button', { name: 'Not now' }).click();
    await expect(search).toContainText('Waiting');
    await hub.getByRole('button', { name: 'Next checkpoint', exact: true }).click();
    await hub.getByRole('button', { name: /Recent ·/ }).click();
    await expect(checkout).toContainText('Completed');
    await expect(search).toContainText('Waiting');
    await hub.getByRole('button', { name: 'Clear spike', exact: true }).click();
    await expect(search).toContainText('Running');
    expect(external).toEqual([]);
});

test('checkpoint pause distinguishes request from acknowledgment and preserves disclosure', async ({
    page,
}) => {
    await page.clock.install();
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    const task = hub.getByRole('article', { name: 'Improve sign-in' });
    await task.getByRole('button', { name: 'Pause at checkpoint: Improve sign-in' }).click();
    await expect(task).toContainText('Pause requested');
    await task.getByRole('button', { name: /Improve sign-in.*Meta/ }).click();
    await expect(task).toContainText('existing processes and memory remain');
    await hub.getByRole('button', { name: 'Next checkpoint', exact: true }).click();
    await expect(task).toContainText('Paused at checkpoint');
    await page.clock.fastForward(6_000);
    await expect(task).toContainText('Pause requested');
    await expect(task).toContainText('existing processes and memory remain');
    await task.getByRole('button', { name: 'Resume agent: Improve sign-in' }).click();
    await expect(task).toContainText('Checked in');
    const observed = hub.getByRole('article', { name: 'Add search and keyboard navigation' });
    await expect(observed.getByRole('button', { name: /Pause/ })).toHaveCount(0);
    await expect(observed.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
});

test('plan is explicitly simulated and protection waits for apply', async ({ page }) => {
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    await hub.getByRole('button', { name: 'Find a plan', exact: true }).click();
    await expect(hub.getByLabel('Scheduling plan')).toContainText('No model called');
    await expect(hub.getByText('Making room for your apps', { exact: true })).toHaveCount(0);
    await hub.getByRole('button', { name: 'Apply plan', exact: true }).click();
    await expect(hub.getByText('Making room for your apps', { exact: true })).toBeVisible();
    await hub.getByRole('button', { name: 'Next checkpoint', exact: true }).click();
    await expect(hub.getByRole('article', { name: 'Check search changes' })).toContainText('Held');
    await hub.getByRole('button', { name: 'End protection', exact: true }).click();
    await expect(hub.getByRole('article', { name: 'Check search changes' })).toContainText(
        'Running'
    );
});

test('controls fit the smallest window and remain keyboard operable', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    const hold = hub.getByRole('button', { name: 'Hold check: Check search changes' });
    await hold.scrollIntoViewIfNeeded();
    await expect(hold).toBeInViewport();
    await hold.focus();
    await page.keyboard.press('Enter');
    await expect(hub.getByRole('article', { name: 'Check search changes' })).toContainText('Held');
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});

test('desktop bridge sends exact controls and keeps a rejected AI plan reviewable', async ({
    page,
}) => {
    const { demoSnapshot, demoBootstrap } = await import('../../src/lib/demo');
    const { hubDemo } = await import('../../src/lib/workflow-hub');
    const snapshot = demoSnapshot();
    snapshot.source = 'live';
    await page.addInitScript(
        ({ snapshot, bootstrap, queue }) => {
            Object.defineProperty(window, 'isTauri', { value: true });
            let callbackId = 0;
            Object.defineProperty(window, '__TAURI_INTERNALS__', {
                value: {
                    transformCallback: () => ++callbackId,
                    unregisterCallback: () => {},
                    invoke: async (command: string, args: Record<string, unknown>) => {
                        if (command.startsWith('plugin:event|')) return 1;
                        if (command === 'get_snapshot') return snapshot;
                        if (command === 'get_bootstrap') return bootstrap;
                        if (command === 'get_validation_queue') return structuredClone(queue);
                        if (command === 'get_workload_icon') return null;
                        if (command === 'control_validation_queue') {
                            document.documentElement.dataset.control = JSON.stringify(args);
                            if (args.action === 'protect') throw 'Queue unavailable. Try again.';
                            if (args.action === 'hold_job') queue.jobs[1].held = true;
                            return;
                        }
                        if (command === 'plan_workflow_queue')
                            return {
                                action: 'protect',
                                job_id: null,
                                reason: 'Hold upcoming checks.',
                                provider: 'Test provider',
                                model: 'Fixture',
                            };
                        throw new Error(`Unexpected command: ${command}`);
                    },
                },
            });
        },
        { snapshot, bootstrap: demoBootstrap, queue: hubDemo() }
    );
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    await hub.getByRole('button', { name: 'Hold check: Check search changes' }).click();
    await expect(page.locator('html')).toHaveAttribute(
        'data-control',
        JSON.stringify({ action: 'hold_job', jobId: 'demo-2' })
    );
    await expect(hub.getByRole('article', { name: 'Check search changes' })).toContainText('Held');
    await hub.getByRole('button', { name: 'Find a plan', exact: true }).click();
    await expect(hub.getByLabel('Scheduling plan')).toContainText('Test provider');
    await hub.getByRole('button', { name: 'Apply plan', exact: true }).click();
    await expect(hub.getByRole('alert')).toContainText('Queue unavailable');
    await expect(hub.getByLabel('Scheduling plan')).toBeVisible();
    await expect(hub.getByText('Making room for your apps', { exact: true })).toHaveCount(0);
});

test('capacity sharing explains parallel work without a chat or a model call', async ({ page }) => {
    await page.goto('/');
    const hub = page.getByRole('region', { name: 'Workflow hub' });
    const capacity = hub.getByRole('region', { name: 'Hardware capacity' });
    await expect(capacity).toContainText('Learning this check');
    await expect(capacity).toContainText('1 running');
    await hub.getByRole('button', { name: 'Share spare capacity', exact: true }).click();
    await expect(capacity).toContainText('Putting spare capacity to work');
    await expect(capacity).toContainText('2 running');
    await expect(hub.getByRole('article', { name: 'Validate checkout' })).toContainText('8.0%');
    await expect(hub.getByRole('article', { name: 'Check search changes' })).toContainText('5.0%');
    await expect(capacity.getByRole('meter', { name: 'Total CPU usage' })).toHaveAttribute(
        'aria-valuenow',
        '36'
    );
    await capacity.getByRole('button', { name: 'How capacity sharing works' }).click();
    await expect(capacity).toContainText('Estimates include headroom');
    await expect(capacity).toContainText('2 profiles learned');
    await expect(hub).toContainText('SIMULATED');
    await page.keyboard.press('Escape');
    await hub.getByRole('button', { name: 'Make room · 20 min' }).click();
    await expect(capacity).toContainText('Keeping room for your apps');
    // Protection affects admission, never silently suspends the two running checks.
    await expect(capacity).toContainText('2 running');
    await hub.getByRole('button', { name: 'Next checkpoint', exact: true }).click();
    await expect(capacity).toContainText('0 running');
});

test('hardware summary fits a small window with readable values and details on demand', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    const capacity = page.getByRole('region', { name: 'Hardware capacity' });
    await expect(capacity).toBeInViewport();
    await expect(capacity.getByRole('meter', { name: 'Total memory usage' })).toBeVisible();
    await expect(capacity).not.toContainText('Familiar checks can overlap');
    const details = capacity.getByRole('button', { name: 'How capacity sharing works' });
    await details.focus();
    await page.keyboard.press('Enter');
    await expect(capacity).toContainText('Familiar checks can overlap');
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});

test('live queue readings expire instead of masquerading as current usage', async () => {
    const { demoSnapshot } = await import('../../src/lib/demo');
    const { hubDemo, workflowRows } = await import('../../src/lib/workflow-hub');
    const snapshot = demoSnapshot();
    snapshot.source = 'live';
    const queue = hubDemo();
    queue.resources.captured_at -= 30;
    queue.jobs[0].last_sample!.captured_at = queue.resources.captured_at;
    expect(workflowRows(queue, snapshot).find((row) => row.id === 'demo-1')?.cpu).toBeNull();
    snapshot.source = 'demo';
    expect(workflowRows(queue, snapshot).find((row) => row.id === 'demo-1')?.cpu).toBe(8);
});
