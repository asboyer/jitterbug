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

test('task cards show value immediately and the detail stays open through refreshes', async ({
    page,
}) => {
    await page.clock.install();
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    const active = page.getByRole('article', {
        name: 'Add search and keyboard navigation',
        exact: true,
    });
    const completed = page.getByRole('article', {
        name: 'Check keyboard focus behavior',
        exact: true,
    });
    await expect(active).toContainText('Test runners are using most');
    await expect(active).toContainText('Let heavy checks take turns');
    await expect(active.getByRole('button', { name: 'Preview managed checks' })).toBeVisible();
    const logos = page.locator('.task-cards .harness-badge img');
    await expect(logos).toHaveCount(4);
    for (const logo of await logos.all()) {
        await expect(logo).toBeVisible();
        await expect
            .poll(() => logo.evaluate((img: HTMLImageElement) => img.naturalWidth))
            .toBeGreaterThan(0);
    }
    await expect(completed.locator('.run-status.completed')).toHaveCSS(
        'color',
        'rgb(110, 213, 138)'
    );
    await expect(completed.getByRole('button', { name: /managed checks/ })).toHaveCount(0);
    await completed.getByRole('button', { name: 'View task' }).click();
    const detail = page.getByRole('region', { name: 'Task detail' });
    await detail.getByText('Sessions & evidence', { exact: false }).click();
    const session = detail.locator('details.agent').first();
    await session.locator(':scope > summary').click();
    await page.clock.fastForward(16_000);
    await expect(detail).toBeVisible();
    await expect(detail.locator('details.technical-detail')).toHaveAttribute('open');
    await expect(session).toHaveAttribute('open');
    await expect(detail.getByText('Meta · Muse Code', { exact: true }).first()).toBeAttached();
    await page.getByRole('button', { name: 'All agent tasks' }).click();
    await expect(completed.getByRole('button', { name: 'View task' })).toBeFocused();
    await page.clock.fastForward(11_000);
    await expect(detail).toHaveCount(0);
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
    await expect(assistant.locator('.observation')).toBeVisible();
    await expect(assistant.locator('details[open]')).toHaveCount(0);
    await assistant.getByRole('button', { name: 'Review change', exact: true }).click();
    const review = page.getByRole('dialog', { name: 'Review priority change' });
    await expect(review.getByRole('button', { name: 'Apply once', exact: true })).toBeDisabled();
    await review.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.clock.install();
    await page.clock.fastForward(10_000);
    expect(externalRequests).toEqual([]);
});

test('minimum window width keeps navigation and primary actions reachable', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    const workflow = page.getByRole('article', {
        name: 'Add search and keyboard navigation',
        exact: true,
    });
    const manage = workflow.getByRole('button', { name: 'Preview managed checks' });
    await manage.scrollIntoViewIfNeeded();
    await expect(manage).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});
