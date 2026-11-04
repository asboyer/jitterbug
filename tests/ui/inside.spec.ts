import { expect, test } from '@playwright/test';

test('browser workloads list tabs automatically with demo actions disabled', async ({ page }) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Browser');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    await page
        .getByRole('dialog')
        .getByRole('button', { name: /^Processes / })
        .click();
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
    // A background tab can be paused; a paused one shows when it resumes and offers Resume.
    await expect(inside.getByRole('button', { name: 'Pause', exact: true }).first()).toBeDisabled();
    const paused = inside.getByRole('listitem').filter({ hasText: 'News live blog' });
    await expect(paused).toContainText(/Paused · resumes \d/);
    await expect(paused.getByRole('button', { name: 'Resume' })).toBeVisible();
    await expect(paused.getByRole('button', { name: 'Pause', exact: true })).toHaveCount(0);
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
        .getByRole('button', { name: 'Monitor', exact: true })
        .click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('Editor');
    await page.locator('.inspect').filter({ hasText: 'Example Editor' }).click();
    await page
        .getByRole('dialog')
        .getByRole('button', { name: /^Processes / })
        .click();
    const inside = page.getByRole('region', { name: 'Inside Example Editor' });
    await expect(inside.getByRole('listitem').first()).toContainText('npm · sample-web-app');
    await expect(inside.getByRole('button', { name: 'Pause' }).first()).toBeDisabled();
    await expect(inside).toContainText('claude · feat-search');
    await page.getByRole('button', { name: 'Close inspector' }).click();
    await page.getByRole('textbox', { name: 'Filter workloads' }).fill('model');
    await page.locator('.inspect').filter({ hasText: 'Tiny local model' }).click();
    await expect(
        page.getByRole('heading', { name: 'Tiny local model', exact: true })
    ).toBeVisible();
    await expect(page.getByRole('region', { name: /^Inside / })).toHaveCount(0);
});

test('processes show observed tab and session names while preserving exact PID selection', async ({
    page,
}) => {
    await page.goto('/');
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    const inspector = page.getByRole('dialog', { name: 'Process inspector' });
    await inspector.getByRole('button', { name: /^Processes / }).click();
    const list = inspector.getByRole('group', { name: 'App processes' });
    await expect(list.locator('.member').filter({ hasText: 'Video call' })).toBeVisible();
    await expect(list).toContainText('meet.example.com');
    expect(await list.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await expect(list).toContainText('2 tabs: Docs, Mail');
    await expect(list).toContainText('Graphics (shared by all tabs)');
    await list.getByRole('textbox', { name: 'Filter processes' }).fill('video');
    await expect(list.locator('.member')).toHaveCount(1);
    await list.locator('.member').click();
    await expect(inspector.getByRole('heading', { name: 'Video call' })).toBeVisible();
    await expect(inspector).toContainText('PID 9101');
    await expect(inspector.getByRole('log').locator('.message')).toHaveCount(0);
    await inspector.getByRole('button', { name: 'Close inspector' }).click();
    await page.locator('.inspect').filter({ hasText: 'Example Editor' }).click();
    await inspector.getByRole('button', { name: /^Processes / }).click();
    await expect(inspector.getByRole('group', { name: 'App processes' })).toContainText(
        'Claude Code session · feat-search'
    );
});
