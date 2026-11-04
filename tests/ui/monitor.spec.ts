import { expect, test, type Page } from '@playwright/test';
import { desktop } from './fixtures/monitor';

const inspector = (page: Page) => page.getByRole('dialog', { name: 'Process inspector' });
const conversation = (page: Page) => inspector(page).getByRole('log');
const calls = async (page: Page) =>
    JSON.parse((await page.locator('html').getAttribute('data-ask-calls')) ?? '[]');
async function selectRenderer(page: Page, pid = 9101) {
    await page.getByRole('button', { name: 'Processes', exact: true }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill(String(pid));
    await page
        .locator('.inspect')
        .filter({ hasText: `PID ${pid}` })
        .click();
}
async function send(page: Page, text = 'What is this process doing?') {
    await inspector(page).getByRole('textbox', { name: 'Message Jitterbug' }).fill(text);
    await inspector(page).getByRole('button', { name: 'Send message' }).click();
}

test('one compact Monitor keeps synthetic exploration offline and selection does not investigate', async ({
    page,
}) => {
    const requests: string[] = [];
    page.on('request', (r) => {
        if (!r.url().startsWith(test.info().project.use.baseURL!)) requests.push(r.url());
    });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Main navigation' });
    await expect(nav.getByRole('button', { name: 'Monitor', exact: true })).toBeVisible();
    for (const name of ['Activity', 'Workloads', 'Workflows'])
        await expect(nav.getByRole('button', { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'System resources' })).toContainText('Normal');
    await expect(page.getByRole('region', { name: 'Computer explanation' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Show processes for Example Browser' }).click();
    await page
        .getByRole('group', { name: 'Example Browser processes', exact: true })
        .getByRole('button')
        .first()
        .click();
    await expect(inspector(page)).toContainText('PID 9101');
    await expect(conversation(page).locator('.message')).toHaveCount(0);
    await send(page);
    await expect(conversation(page)).toContainText('Jitterbug · demo');
    expect(requests).toEqual([]);
});

test('native app artwork is rendered and retained across telemetry updates', async ({ page }) => {
    await desktop(page);
    const icon = page.locator('.inspect .app-icon img').first();
    await expect(icon).toBeVisible();
    await expect
        .poll(() => icon.evaluate((img: HTMLImageElement) => img.naturalWidth))
        .toBeGreaterThan(0);
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('fixture-sample')));
    await expect(icon).toBeVisible();
});

test('a question interrupts slow background descriptions without waiting for their timeout', async ({
    page,
}) => {
    await desktop(page, { profileDelay: 30_000 });
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', '1');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    await send(page, 'Why is it using this much memory?');
    await expect(conversation(page)).toContainText('The app’s helpers', { timeout: 2500 });
    await expect(page.locator('html')).toHaveAttribute('data-prioritized', 'true');
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', '1');
});

test('explicit conversation targets the selected instance and survives refreshes and reopening', async ({
    page,
}) => {
    await desktop(page);
    await selectRenderer(page);
    await expect(inspector(page).getByRole('textbox')).toBeVisible();
    expect(await calls(page)).toHaveLength(0);
    await send(page);
    await expect(conversation(page)).toContainText('Specific finding for PID 9101');
    expect((await calls(page))[0]).toMatchObject({ workloadId: 'demo-browser', pid: 9101 });
    expect((await calls(page))[0].startTime).toBeGreaterThan(0);
    await page.evaluate(() =>
        window.dispatchEvent(
            new CustomEvent('fixture-sample', { detail: { pid: 9101, processCpu: 61 } })
        )
    );
    await expect(inspector(page).getByRole('region', { name: 'App summary' })).toContainText(
        '61.0%'
    );
    expect(await calls(page)).toHaveLength(1);
    await inspector(page).getByRole('button', { name: 'Close inspector' }).click();
    await page.locator('.inspect').click();
    await expect(conversation(page)).toContainText('Specific finding for PID 9101');
    expect(await calls(page)).toHaveLength(1);
    await send(page, 'What should I watch for?');
    await expect.poll(async () => (await calls(page)).length).toBe(2);
    expect((await calls(page))[1].messages).toHaveLength(3);
    await expect(page.getByRole('complementary', { name: 'Contextual assistant' })).toHaveCount(0);
});

test('browser answers link local tab evidence to the exact process without sending tab titles', async ({
    page,
}) => {
    await desktop(page, { browserEvidence: true });
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    await send(page, 'Why is it using this much memory?');
    const contributor = conversation(page).getByRole('button', {
        name: /Example documentation tab/,
    });
    await expect(contributor).toBeVisible();
    await expect(
        conversation(page).getByRole('link', { name: /Mozilla: memory use/ })
    ).toBeVisible();
    await contributor.click();
    await expect(inspector(page)).toContainText('PID 9101');
    await send(page, 'What about this process?');
    await expect.poll(async () => (await calls(page)).length).toBe(2);
    const requests = await calls(page);
    expect(requests[1]).toMatchObject({ pid: 9101, workloadId: 'demo-browser' });
    expect(JSON.stringify(requests)).not.toContain('Example documentation tab');
});

test('a reused PID disables the old conversation and reviewed action', async ({ page }) => {
    await desktop(page, { action: true });
    await selectRenderer(page);
    await send(page, 'Can I reduce this process’s impact?');
    await expect(
        inspector(page).getByRole('button', { name: /Review background mode/ })
    ).toBeEnabled();
    await page.evaluate(() =>
        window.dispatchEvent(
            new CustomEvent('fixture-sample', { detail: { pid: 9101, startTime: 123456 } })
        )
    );
    await expect(inspector(page).getByRole('status')).toContainText('PID was reused');
    await expect(inspector(page).getByRole('textbox')).toBeDisabled();
    await expect(
        inspector(page).getByRole('button', { name: /Review background mode/ })
    ).toBeDisabled();
    await expect(page.locator('html')).not.toHaveAttribute('data-action-calls');
});

test('a chat proposal opens a concrete review before any native action', async ({ page }) => {
    await desktop(page, { action: true });
    await selectRenderer(page);
    await send(page, 'Can I reduce this process’s impact?');
    await inspector(page)
        .getByRole('button', { name: /Review background mode/ })
        .click();
    const review = page.getByRole('dialog', { name: 'Lower this process’s CPU priority?' });
    await expect(review).toContainText('PID 9101');
    await expect(review).toContainText('does not free memory');
    await expect(page.locator('html')).not.toHaveAttribute('data-action-calls');
    await review.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(review).toHaveCount(0);
    await expect(inspector(page).getByRole('textbox')).toBeInViewport();
});

test('stale in-flight answers cannot appear in a newly selected process', async ({ page }) => {
    await desktop(page, { delay: 700 });
    await selectRenderer(page);
    await send(page);
    await expect(page.locator('html')).toHaveAttribute('data-ask-calls', /9101/);
    await inspector(page).getByRole('button', { name: 'Close inspector' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('9102');
    await page.locator('.inspect').filter({ hasText: 'PID 9102' }).click();
    await send(page);
    await expect(conversation(page)).toContainText('Specific finding for PID 9102');
    await expect(conversation(page)).not.toContainText('Specific finding for PID 9101');
});

test('switching to a subprocess does not show the previous app’s pending investigation', async ({
    page,
}) => {
    await desktop(page, { delay: 900 });
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    await send(page, 'Why is this app using memory?');
    await expect(page.locator('html')).toHaveAttribute('data-ask-calls', /demo-browser/);
    await inspector(page)
        .getByRole('button', { name: /^Processes / })
        .click();
    await inspector(page).locator('.member').filter({ hasText: 'PID 9101' }).click();
    await expect(inspector(page)).toContainText('PID 9101');
    await expect(inspector(page).getByText('Thinking…', { exact: true })).toHaveCount(0);
    await expect(conversation(page).locator('.message')).toHaveCount(0);
    await send(page);
    await expect(conversation(page)).toContainText('Specific finding for PID 9101');
    expect(await calls(page)).toHaveLength(2);
});

test('changing telemetry keeps list order, focus, view and AI descriptions stable', async ({
    page,
}) => {
    await desktop(page, { profileDelay: 500 });
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', /[1-9]/);
    const first = page.locator('.inspect').first();
    const initial = await page.locator('.inspect strong').allTextContents();
    await first.focus();
    for (let i = 0; i < 4; i++)
        await page.evaluate(() =>
            window.dispatchEvent(new CustomEvent('fixture-sample', { detail: { reverse: true } }))
        );
    await expect(page.locator('.purpose').first()).toContainText('draw its interface');
    expect(await page.locator('.inspect strong').allTextContents()).toEqual(initial);
    await expect(first).toBeFocused();
    await expect(page.getByRole('button', { name: 'Apps', exact: true })).toHaveAttribute(
        'aria-pressed',
        'true'
    );
    await expect(first).toBeEnabled();
    const count = await page.locator('html').getAttribute('data-batch-calls');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('fixture-sample')));
    await expect(page.locator('html')).toHaveAttribute('data-batch-calls', count!);
    await expect(page.locator('html')).not.toHaveAttribute('data-summary-calls');
});

test('native inventory can inspect an app outside the dashboard payload', async ({ page }) => {
    await desktop(page);
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('quiet-worker');
    await page.locator('.inspect').filter({ hasText: 'Quiet service' }).click();
    await expect(inspector(page)).toContainText('Quiet service');
    expect(await calls(page)).toHaveLength(0);
    await send(page);
    await expect.poll(async () => (await calls(page)).length).toBe(1);
    expect((await calls(page))[0]).toMatchObject({ workloadId: 'quiet-group', pid: null });
});

test('unavailable Meta is explicit while telemetry and selection remain usable', async ({
    page,
}) => {
    await desktop(page, { unavailable: true });
    await selectRenderer(page);
    await send(page);
    await expect(inspector(page).getByRole('alert')).toContainText(
        'cloud budget cannot cover this request'
    );
    await expect(inspector(page)).toContainText('PID 9101');
    await expect(conversation(page).locator('.message:not(.user)')).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'System resources' })).toBeVisible();
});

test('inspector and composer are keyboard reachable in a small desktop window', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 650 });
    await page.goto('/');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).focus();
    await page.keyboard.press('Enter');
    await expect(inspector(page)).toBeFocused();
    await expect(inspector(page).getByRole('button', { name: 'Close inspector' })).toBeInViewport();
    await expect(inspector(page).getByRole('textbox')).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
    expect(await inspector(page).evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
    await page.keyboard.press('Escape');
    await expect(inspector(page)).toHaveCount(0);
    await expect(page.locator('.inspect').filter({ hasText: 'Example Browser' })).toBeFocused();
});
