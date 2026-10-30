import { expect, test } from '@playwright/test';

test('snapshot cases show a timeline, labeled findings, and an origin filter', async ({ page }) => {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    const filter = page.getByRole('group', { name: 'Filter by origin' });
    await filter.getByRole('button', { name: 'Manual', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Proposed optimizations' })).toHaveCount(0);
    const recent = page.getByRole('region', { name: 'Recent activity' });
    await expect(recent.getByRole('button')).toHaveCount(1);
    await expect(recent).toContainText('Jitter Snapshot: sample-web-app');
    await expect(recent).toContainText('Snapshot · Manual');
    await recent.getByRole('button').click();

    const assistant = page.getByRole('complementary', { name: 'Automatic resource assistant' });
    const report = assistant.getByRole('region', { name: 'Jitter Snapshot findings' });
    await expect(report.getByRole('img', { name: /Busiest CPU core/ })).toBeVisible();
    await expect(report.getByRole('heading', { level: 3 })).toHaveText([
        'What happened',
        'Likely cause',
        'What you can do',
        'If it happens again',
    ]);
    await expect(report).toContainText('You were using Example Editor.');
    await expect(report.getByText('Observed', { exact: true }).first()).toBeVisible();
    await expect(report.getByText('Inferred', { exact: true }).first()).toBeVisible();
    // Synthetic cases can be explored, never applied.
    await expect(report.getByRole('button', { name: 'Review change' })).toBeDisabled();
    await report.getByText(/^Findings/).click();
    await expect(report).toContainText('Strong evidence');
    await expect(report).toContainText('GPU, display, disk, and network');

    // Automatic and manual snapshots share one filter; plain alerts are left out.
    await filter.getByRole('button', { name: 'Snapshots', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Proposed optimizations' })).toHaveCount(0);
    await expect(recent.getByRole('button')).toHaveCount(1);
    await filter.getByRole('button', { name: 'All', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Proposed optimizations' })).toBeVisible();
});
