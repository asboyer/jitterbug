import { expect, test } from '@playwright/test';

test('example demonstrates detecting, investigating, and reviewing without leaving a task', async ({
    page,
}) => {
    const external: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith('http://127.0.0.1:1420')) external.push(request.url());
    });
    await page.clock.install();
    await page.goto('/');
    await expect(page.getByRole('button', { name: 'See Jitterbug step in' })).toBeVisible();
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    await page
        .getByRole('article', { name: 'Check keyboard focus behavior', exact: true })
        .locator('.task-disclosure > summary')
        .click();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Simulate pressure' }).click();
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await expect(prompt).toContainText('Hey,');
    await expect(prompt).toContainText('I’m checking recent activity');
    await expect(
        page.getByRole('heading', { name: 'Check keyboard focus behavior', exact: true })
    ).toBeVisible();
    await page.clock.fastForward(3000);
    await expect(prompt).toContainText('overlapping checks');
    await prompt.getByRole('button', { name: 'See task', exact: true }).click();
    await expect(prompt).toHaveCount(0);
    await expect(
        page
            .getByRole('article', { name: 'Add search and keyboard navigation', exact: true })
            .locator('.task-disclosure > summary')
    ).toBeFocused();
    await page.getByRole('button', { name: 'Preview managed checks', exact: true }).click();
    const preview = page.getByRole('dialog', { name: 'Give your work room to finish' });
    await expect(preview).toContainText('NO PROCESSES CHANGED');
    await expect(preview).toContainText('Two checks run at the same time');
    await preview.getByRole('button', { name: 'Preview checks taking turns' }).click();
    await expect(preview).toContainText('One runs. The next waits for room.');
    await expect(preview).toContainText('Existing tests keep running');
    await page.keyboard.press('Escape');
    await expect(preview).toHaveCount(0);
    await expect(
        page.getByRole('button', { name: 'Preview managed checks', exact: true })
    ).toBeFocused();
    await page.clock.fastForward(15_000);
    await expect(prompt).toHaveCount(0);
    expect(external).toEqual([]);
});

test('Later keeps an incident in Inbox and does not repeat on refresh', async ({ page }) => {
    await page.clock.install();
    await page.goto('/');
    await page.getByRole('button', { name: 'See Jitterbug step in' }).click();
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

test('unsampled completed tasks show no empty graphs, metrics, or management action', async ({
    page,
}) => {
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    const task = page.getByRole('article', { name: 'Check a small utility function', exact: true });
    await expect(task).toContainText('Not measured');
    await expect(task.locator('.task-readings')).toHaveCount(0);
    await task.locator('.task-disclosure > summary').click();
    const detail = page.getByRole('region', { name: 'Task detail' });
    await expect(detail).toContainText('No resource samples are available');
    await expect(detail.getByRole('region', { name: 'Task resources' })).toHaveCount(0);
    await expect(detail.getByRole('button', { name: /managed checks/ })).toHaveCount(0);
    await expect(detail).not.toContainText('Measure a running task');
});

test('demo entry resets task selection and the assistant fits the minimum window', async ({
    page,
}) => {
    await page.clock.install();
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Agent tasks' }).click();
    await page
        .getByRole('article', { name: 'Check a small utility function', exact: true })
        .locator('.task-disclosure > summary')
        .click();
    await page.getByRole('button', { name: 'Take a tour', exact: true }).click();
    await page.getByRole('button', { name: 'Skip tour', exact: true }).click();
    await expect(
        page.getByRole('heading', { name: 'Check a small utility function', exact: true })
    ).toBeVisible();
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Explore demo', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Task detail' })).toHaveCount(0);
    await expect(
        page.getByRole('article', { name: 'Add search and keyboard navigation', exact: true })
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
