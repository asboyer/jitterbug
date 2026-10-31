import { expect, test } from '@playwright/test';

test('browser workloads list tabs automatically with demo actions disabled', async ({ page }) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Activity', exact: true })
        .click();
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.getByRole('button', { name: /Example Browser/ }).click();
    const inside = page.getByRole('region', { name: 'Inside Example Browser' });
    // The demo stands in for a browser connection the person already allowed.
    await expect(inside.getByRole('listitem').first()).toContainText('Video call');
    await expect(inside.getByRole('button', { name: 'Refresh' })).toBeVisible();
    await expect(inside).toContainText('2 sleeping tabs');
    await expect(inside).not.toContainText('Recipes');
    await inside.getByRole('button', { name: 'List' }).click();
    await expect(inside).toContainText('Recipes');
    await expect(inside).toContainText('shares a process with 1 other tab');
    await expect(inside.getByRole('button', { name: 'Close tab…' }).first()).toBeDisabled();
    await expect(inside.getByText('Synthetic example · About these readings')).toBeVisible();
    await expect(inside).toContainText('Closing a tab discards unsaved form input');
    await expect(inside.locator('.reading-notes p')).not.toBeVisible();
    await inside.getByText('Synthetic example · About these readings').click();
    await expect(inside.locator('.reading-notes p')).toBeVisible();
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

test('processes are named by the tab or session they run, not by helper name', async ({ page }) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Activity', exact: true })
        .click();
    await page.getByRole('button', { name: 'All workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.getByRole('button', { name: /Example Browser/ }).click();
    await page
        .locator('summary')
        .filter({ hasText: /^Processes/ })
        .click();
    const list = page.getByRole('group', { name: 'Workload processes' });
    await expect(list.getByRole('button', { name: 'Inspect Video call' })).toBeVisible();
    await expect(list).toContainText('meet.example.com');
    await expect(list).toContainText('2 tabs: Docs, Mail');
    await expect(list).toContainText('Graphics (shared by all tabs)');
    await expect(list).not.toContainText('Helper (Renderer)');
    await expect(list).not.toContainText('PID');
    // Identical helpers collapse into one row; the technical names stay one click away.
    const network = list.getByRole('button', { name: 'Network & storage, 2 processes' });
    await network.click();
    await expect(network).toHaveAttribute('aria-expanded', 'true');
    await expect(list).toContainText('Example Browser Helper');
    await list.getByRole('button', { name: /PID 9105/ }).click();
    await expect(page.getByRole('region', { name: 'Process origins' })).toContainText('PID 9105');
    await page.getByRole('textbox', { name: 'Filter processes' }).fill('video');
    await expect(list.getByRole('button')).toHaveCount(1);

    await page.getByRole('button', { name: '‹ Back to workloads' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Editor');
    await page.getByRole('button', { name: /Example Editor/ }).click();
    await page
        .locator('summary')
        .filter({ hasText: /^Processes/ })
        .click();
    await expect(list).toContainText('Claude Code session · feat-search');
    await expect(list.getByRole('button', { name: 'Idle shell, 2 processes' })).toBeVisible();
});
