import { expect, test, type Page } from '@playwright/test';
import { desktop } from './fixtures/monitor';

const insight = (page: Page) => page.getByRole('region', { name: 'Process activity insight' });
const inspector = (page: Page) => page.getByRole('dialog', { name: 'Process inspector' });
async function selectRenderer(page: Page, pid = 9101) {
    await page.getByRole('button', { name: 'Processes', exact: true }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill(String(pid));
    await page
        .locator('.inspect')
        .filter({ hasText: `PID ${pid} ·` })
        .click();
}

test('one Monitor replaces three tabs and keeps synthetic AI offline', async ({ page }) => {
    const requests: string[] = [];
    page.on('request', (r) => {
        if (!r.url().startsWith(test.info().project.use.baseURL!)) requests.push(r.url());
    });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Main navigation' });
    await expect(nav.getByRole('button', { name: 'Monitor', exact: true })).toBeVisible();
    for (const name of ['Activity', 'Workloads', 'Workflows'])
        await expect(nav.getByRole('button', { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'Computer explanation' })).toContainText(
        'Demo insight'
    );
    await page.getByRole('button', { name: 'Show processes for Example Browser' }).click();
    await page
        .getByRole('group', { name: 'Example Browser processes', exact: true })
        .getByRole('button')
        .first()
        .click();
    await expect(insight(page)).toContainText('PID 9101');
    await expect(insight(page)).toContainText('Demo insight');
    await expect(inspector(page).getByRole('button', { name: 'Manage workload' })).toHaveCount(0);
    expect(requests).toEqual([]);
});

test('selected PID is investigated automatically, reused on reopen, and invalidated by changed activity', async ({
    page,
}) => {
    await desktop(page);
    await selectRenderer(page);
    await expect(insight(page)).toContainText('Activity of PID 9101');
    const calls = async () =>
        JSON.parse((await page.locator('html').getAttribute('data-ask-calls')) ?? '[]');
    expect((await calls())[0]).toMatchObject({
        workloadId: 'demo-browser',
        pid: 9101,
        tier: 'auto',
    });
    expect((await calls())[0].startTime).toBeGreaterThan(0);
    await inspector(page).getByRole('button', { name: 'Close inspector ×' }).click();
    await page.locator('.inspect').click();
    await expect(insight(page)).toContainText('Reused recent finding');
    expect((await calls()).length).toBe(1);
    await page.evaluate(() =>
        window.dispatchEvent(
            new CustomEvent('fixture-sample', { detail: { pid: 9101, processCpu: 61 } })
        )
    );
    await expect(insight(page)).toContainText('Activity has changed');
    await insight(page).getByRole('button', { name: 'Investigate again' }).click();
    await expect.poll(async () => (await calls()).length).toBe(2);
    await expect(insight(page)).not.toContainText('Activity has changed');
    await insight(page).getByRole('button', { name: 'Ask a follow-up' }).click();
    const chat = page.getByRole('complementary', { name: 'Contextual assistant' });
    await chat.getByRole('textbox', { name: 'Ask Jitterbug' }).fill('What should I watch for?');
    await chat.getByRole('button', { name: 'Ask ↑', exact: true }).click();
    await expect.poll(async () => (await calls()).length).toBe(3);
    expect((await calls())[2]).toMatchObject({ pid: 9101, workloadId: 'demo-browser' });
});

test('a reused PID blocks the old selection and never gets an automatic action', async ({
    page,
}) => {
    await desktop(page);
    await selectRenderer(page);
    await expect(insight(page)).toContainText('Activity of PID 9101');
    await page.evaluate(() =>
        window.dispatchEvent(
            new CustomEvent('fixture-sample', { detail: { pid: 9101, startTime: 123456 } })
        )
    );
    await expect(inspector(page).getByRole('alert')).toContainText('PID was reused');
    await expect(insight(page).getByRole('button', { name: 'Investigate again' })).toBeDisabled();
});

test('stale in-flight responses cannot replace a newly selected process', async ({ page }) => {
    await desktop(page, { unavailable: false, delay: 700 });
    await selectRenderer(page);
    await expect(page.locator('html')).toHaveAttribute('data-ask-calls', /9101/);
    await inspector(page).getByRole('button', { name: 'Close inspector ×' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('9102');
    await page.locator('.inspect').filter({ hasText: 'PID 9102 ·' }).click();
    await expect(insight(page)).toContainText('Activity of PID 9102');
    await expect(insight(page)).not.toContainText('Activity of PID 9101');
});

test('overview is event-driven and visible descriptions are batched', async ({ page }) => {
    await desktop(page);
    await expect(page.locator('html')).toHaveAttribute('data-summary-calls', '1');
    await expect(page.getByRole('region', { name: 'Computer explanation' })).toContainText(
        'Room for your next task'
    );
    await expect(page.locator('.purpose').first()).toContainText('draw its interface');
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', '1');
    for (let i = 0; i < 4; i++)
        await page.evaluate(() => window.dispatchEvent(new CustomEvent('fixture-sample')));
    await expect(page.locator('html')).toHaveAttribute('data-summary-calls', '1');
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', '1');
    await page.clock.install();
    await page.clock.fastForward(61_000);
    await page.evaluate(() =>
        window.dispatchEvent(
            new CustomEvent('fixture-sample', { detail: { cpu: 90, pressure: 'high' } })
        )
    );
    await page.clock.fastForward(1000);
    await expect(page.locator('html')).toHaveAttribute('data-summary-calls', '2');
});

test('native inventory can inspect a process outside the dashboard payload', async ({ page }) => {
    await desktop(page);
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('quiet-worker');
    await page.locator('.inspect').filter({ hasText: 'Quiet service' }).click();
    await expect(inspector(page)).toContainText('Quiet service');
    await expect(insight(page)).toContainText('Activity of PID 9900');
});

test('unavailable Meta is explicit while telemetry remains usable', async ({ page }) => {
    await desktop(page, { unavailable: true, delay: 0 });
    await expect(page.getByRole('region', { name: 'Computer explanation' })).toContainText(
        'AI overview unavailable'
    );
    await selectRenderer(page);
    await expect(insight(page)).toContainText('Local checks · AI unavailable');
    await expect(insight(page)).toContainText('cloud budget cannot cover this request');
    await expect(insight(page)).toContainText('PID 9101');
});

test('inspector is keyboard reachable and fits a small desktop window', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 650 });
    await page.goto('/');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).focus();
    await page.keyboard.press('Enter');
    await expect(inspector(page)).toBeFocused();
    await expect(
        inspector(page).getByRole('button', { name: 'Close inspector ×' })
    ).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
    expect(await inspector(page).evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
    await page.keyboard.press('Escape');
    await expect(inspector(page)).toHaveCount(0);
    await expect(page.locator('.inspect').filter({ hasText: 'Example Browser' })).toBeFocused();
});
