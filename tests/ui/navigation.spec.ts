import { expect, test } from '@playwright/test';

test('Monitor combines resource health and the full activity table', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Monitor' })).toBeVisible();
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await expect(page.getByRole('heading', { name: 'Monitor', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'System resources' })).toContainText('Normal');
    await expect(page.getByRole('region', { name: 'Activity monitor' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Shared validation queue' })).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'Agent workflows' })).toHaveCount(0);
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    await expect(page.getByRole('heading', { name: 'Example Browser', exact: true })).toBeVisible();
    await page.getByRole('navigation').getByRole('button', { name: 'Agents', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Agents', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Shared validation queue' })).toHaveCount(0);
});

test('simulated pressure and inbox keep explanations concise and demo actions disabled', async ({
    page,
}) => {
    // Synthetic exploration must not contact providers.
    const externalRequests: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith(test.info().project.use.baseURL!))
            externalRequests.push(request.url());
    });
    await page.goto('/');
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure', exact: true }).click();
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await expect(page.getByRole('region', { name: 'System resources' })).toContainText('High');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible();
    await page.getByRole('region', { name: 'Proposed optimizations' }).getByRole('button').click();
    const assistant = page.getByRole('complementary', { name: 'Automatic resource assistant' });
    await expect(assistant.locator('.observation')).toBeVisible();
    await expect(assistant.locator('details[open]')).toHaveCount(0);
    await assistant.getByRole('button', { name: 'Review change', exact: true }).click();
    const review = page.getByRole('dialog', { name: 'Review priority change' });
    await expect(
        review.getByRole('button', { name: 'Move to Background', exact: true })
    ).toBeDisabled();
    await review.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.clock.install();
    await page.clock.fastForward(10_000);
    expect(externalRequests).toEqual([]);
});

test('Agents shows what each conversation is doing and lets tests take turns', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agents', exact: true }).click();
    const agents = page.getByRole('region', { name: 'Agents', exact: true });
    await expect(agents.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
    await expect(agents).not.toContainText(/\d+ agents/);
    const claude = page.getByRole('region', { name: 'Claude Code: Agents tab redesign' });
    await expect(claude).toContainText('Delegating: Validate the Rust change');
    await expect(claude).toContainText('Make the agents tab show what each agent is doing');
    await expect(claude).toContainText('Run the Python test suite');
    await expect(page.getByRole('region', { name: 'Waiting for you' })).toContainText('API docs');
    await agents.getByRole('button', { name: 'Let agents take turns' }).click();
    const setup = page.getByRole('region', { name: 'Connect your agents' });
    await expect(setup.getByRole('checkbox', { name: 'Connect Codex' })).toBeChecked();
    await setup.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(setup).toContainText('heard from it');
    await setup.getByRole('button', { name: 'Done', exact: true }).click();
    const line = page.getByRole('region', { name: 'Tests and builds taking turns' });
    await expect(line).toContainText('pytest');
    await expect(claude).toContainText('Waiting to run cargo test');
    await expect(line.locator('.turn.running')).toHaveCount(1);
});

test('minimum window width keeps navigation and primary actions reachable', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agents', exact: true }).click();
    const start = page.getByRole('button', { name: 'Let agents take turns' });
    await start.scrollIntoViewIfNeeded();
    await expect(start).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});
