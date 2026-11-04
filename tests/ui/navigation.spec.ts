import { expect, test } from '@playwright/test';
import { desktop } from './fixtures/monitor';

test('Monitor combines resource health and the full activity table', async ({ page }) => {
    await desktop(page);
    await expect(page.getByRole('heading', { name: 'Monitor' })).toBeVisible();
    await page
        .getByRole('navigation', { name: 'Main navigation' })
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
    await page
        .getByRole('navigation', { name: 'Main navigation' })
        .getByRole('button', { name: 'Agents', exact: true })
        .click();
    await expect(page.getByRole('region', { name: 'Agents', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Shared validation queue' })).toHaveCount(0);
});

test('the narrow inspector has one focus outline and keeps its composer visible', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await desktop(page);
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
    const input = page.getByRole('textbox', { name: 'Message Jitterbug' });
    await input.fill('What can you tell me about this specific process, and what remains unknown?');
    await expect(input).toBeInViewport();
    expect(await input.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('none');
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
    await page.getByRole('button', { name: 'Close inspector' }).click();
    await expect(page.getByRole('textbox', { name: 'Filter workloads' })).toBeInViewport();
});
