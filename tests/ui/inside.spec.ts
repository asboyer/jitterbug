import { expect, test } from '@playwright/test';

test('browser workloads list tabs on request with demo actions disabled', async ({ page }) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Activity', exact: true })
        .click();
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.getByRole('button', { name: /Example Browser/ }).click();
    const inside = page.getByRole('region', { name: 'Inside Example Browser' });
    await expect(inside).toContainText('See which tabs use the CPU and memory');
    await inside.getByRole('button', { name: 'Show tabs' }).click();
    await expect(inside.getByRole('listitem').first()).toContainText('Video call');
    await expect(inside).toContainText('shares a process with 1 other tab');
    await expect(inside.getByRole('button', { name: 'Close tab…' }).first()).toBeDisabled();
    await expect(inside).toContainText('Synthetic example.');
});

test('editor workloads list commands with pause, and other apps show no inside view', async ({
    page,
}) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Activity', exact: true })
        .click();
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Editor');
    await page.getByRole('button', { name: /Example Editor/ }).click();
    const inside = page.getByRole('region', { name: 'Inside Example Editor' });
    await expect(inside.getByRole('listitem').first()).toContainText('npm · sample-web-app');
    await expect(inside.getByRole('button', { name: 'Pause' }).first()).toBeDisabled();
    await expect(inside).toContainText('claude · feat-search');
    await page.getByRole('button', { name: '‹ Back to workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('model');
    await page.getByRole('button', { name: /Tiny local model/ }).click();
    await expect(
        page.getByRole('heading', { name: 'Tiny local model', exact: true })
    ).toBeVisible();
    await expect(page.getByRole('region', { name: /^Inside / })).toHaveCount(0);
});
