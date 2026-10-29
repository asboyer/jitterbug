import { expect, test } from '@playwright/test';

test('activity answers the main question without queue or agent history', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Activity', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Computer health' })).toContainText(
        'Your computer has room to work'
    );
    await expect(page.getByRole('region', { name: 'Top resource users' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Shared validation queue' })).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'Agent workflows' })).toHaveCount(0);
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.getByRole('button', { name: /Example Browser/ }).click();
    await expect(page.getByRole('heading', { name: 'Example Browser', exact: true })).toBeVisible();
    await page.getByRole('navigation').getByRole('button', { name: 'Validation queue' }).click();
    await expect(page.getByRole('region', { name: 'Shared validation queue' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Agent workflows' })).toHaveCount(0);
});

test('task and nested disclosures retain user choice across telemetry refreshes', async ({
    page,
}) => {
    await page.clock.install();
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    const active = page.locator('details.workflow').filter({
        has: page.getByRole('heading', {
            name: 'Add search and keyboard navigation',
            exact: true,
        }),
    });
    const completed = page.locator('details.workflow').filter({
        has: page.getByRole('heading', { name: 'Check keyboard focus behavior', exact: true }),
    });
    await expect(active).not.toHaveAttribute('open');
    const logos = page.locator('.workflow-heading .harness-badge img');
    await expect(logos).toHaveCount(5);
    for (const logo of await logos.all()) {
        await expect(logo).toBeVisible();
        await expect
            .poll(() => logo.evaluate((img: HTMLImageElement) => img.naturalWidth))
            .toBeGreaterThan(0);
    }
    await completed.locator(':scope > summary').click();
    await completed.getByText('Sessions & evidence', { exact: false }).click();
    const session = completed.locator('details.agent').first();
    await session.locator(':scope > summary').click();
    await page.clock.fastForward(16_000);
    await expect(completed).toHaveAttribute('open');
    await expect(completed.locator('details.technical-detail')).toHaveAttribute('open');
    await expect(session).toHaveAttribute('open');
    await expect(completed.getByRole('button', { name: 'Manage this workload' })).toBeVisible();
    await expect(completed.locator('.run-status.completed')).toHaveCSS(
        'color',
        'rgb(110, 213, 138)'
    );
    await expect(completed.getByText('Meta · Muse Code', { exact: true }).first()).toBeAttached();
    await completed.locator(':scope > summary').click();
    await page.clock.fastForward(11_000);
    await expect(completed).not.toHaveAttribute('open');
    await expect(active).not.toHaveAttribute('open');
});

test('simulated pressure and inbox keep explanations concise and demo actions disabled', async ({
    page,
}) => {
    // Synthetic exploration must not contact providers.
    const externalRequests: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith('http://127.0.0.1:1420'))
            externalRequests.push(request.url());
    });
    await page.goto('/');
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure', exact: true }).click();
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Activity', exact: true })
        .click();
    await expect(page.getByRole('region', { name: 'Computer health' })).toContainText(
        'Your computer is working hard'
    );
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible();
    await page.getByRole('region', { name: 'Proposed optimizations' }).getByRole('button').click();
    const assistant = page.getByRole('complementary', { name: 'Automatic resource assistant' });
    await expect(assistant.getByText('What happened', { exact: true })).toBeVisible();
    await expect(assistant.locator('details[open]')).toHaveCount(0);
    await expect(
        assistant.getByRole('button', { name: 'Apply once & measure', exact: true })
    ).toBeDisabled();
    await page.clock.install();
    await page.clock.fastForward(10_000);
    expect(externalRequests).toEqual([]);
});

test('minimum window width keeps navigation and primary actions reachable', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    const workflow = page.locator('details.workflow').first();
    await workflow.locator(':scope > summary').click();
    const manage = workflow.getByRole('button', { name: 'Manage this workload' });
    await manage.scrollIntoViewIfNeeded();
    await expect(manage).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});
