import { expect, test } from '@playwright/test';

test('example demonstrates detecting, investigating, and reviewing without leaving a task', async ({
    page,
}) => {
    const external: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith(test.info().project.use.baseURL!))
            external.push(request.url());
    });
    await page.clock.install();
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure' }).click();
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await expect(prompt).toContainText('Hey,');
    await expect(prompt).toContainText('I’m checking recent activity');
    await page.clock.fastForward(3000);
    await expect(prompt).toContainText('overlapping checks');
    await prompt.getByRole('button', { name: 'See task', exact: true }).click();
    await expect(prompt).toHaveCount(0);
    await expect(page.getByRole('region', { name: 'Agents', exact: true })).toBeVisible();
    await page.clock.fastForward(15_000);
    await expect(prompt).toHaveCount(0);
    expect(external).toEqual([]);
});

test('Later keeps an incident in Inbox and does not repeat on refresh', async ({ page }) => {
    await page.clock.install();
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure' }).click();
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await prompt.getByRole('button', { name: 'Later', exact: true }).click();
    await page.clock.fastForward(20_000);
    await expect(prompt).toHaveCount(0);
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await expect(
        page.getByRole('region', { name: 'Proposed optimizations' }).getByRole('button')
    ).toBeVisible();
});

test('demo entry shows demo agents and the assistant fits the minimum window', async ({ page }) => {
    await page.clock.install();
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('button', { name: 'Take a tour', exact: true }).click();
    await page.getByRole('button', { name: 'Skip tour', exact: true }).click();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Explore demo', exact: true }).click();
    await page.getByRole('navigation').getByRole('button', { name: 'Agents', exact: true }).click();
    await expect(
        page.getByRole('region', { name: 'Claude Code: Agents tab redesign' })
    ).toBeVisible();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure', exact: true }).click();
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await page.clock.fastForward(3000);
    const box = await prompt.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(760);
    expect(box!.y + box!.height).toBeLessThanOrEqual(560);
});
