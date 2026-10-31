import { expect, test } from '@playwright/test';

async function openSuggestion(page: import('@playwright/test').Page) {
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await page.getByRole('region', { name: 'Proposed optimizations' }).getByRole('button').click();
    return page.getByRole('complementary', { name: 'Automatic resource assistant' });
}

test('incident explanation is optional and permissions appear only in review', async ({ page }) => {
    const assistant = await openSuggestion(page);
    await expect(assistant.getByText('Priority stays lower', { exact: false })).toHaveCount(0);
    await expect(assistant.getByRole('button', { name: /Always allow/ })).toHaveCount(0);
    const info = assistant.getByRole('button', { name: 'About this optimization', exact: true });
    await info.click();
    const explanation = assistant.getByRole('region', { name: 'About this optimization' });
    await expect(explanation).toContainText('It keeps running');
    // WebKit pointer clicks can leave focus outside the trigger.
    await info.evaluate((element) => (element as HTMLButtonElement).blur());
    await page.keyboard.press('Escape');
    await expect(explanation).toBeHidden();
    await expect(info).toBeFocused();
    await info.click();
    await assistant.getByRole('button', { name: 'Review change' }).press('Enter');
    const review = page.getByRole('dialog', { name: 'Review priority change' });
    await expect(review).toContainText('does not free memory');
    await expect(review).toContainText('until you move it back to Normal');
    await expect(review.getByRole('checkbox')).toHaveCount(0);
    await expect(review.getByRole('button', { name: 'Move to Background' })).toBeDisabled();
    await page.keyboard.press('Escape');
    await expect(review).toHaveCount(0);
    await expect(explanation).toBeHidden();
    await expect(assistant.getByRole('button', { name: 'Review change' })).toBeFocused();
    await assistant.getByText('View details', { exact: true }).click();
    await assistant.getByText('Full analysis', { exact: true }).click();
    await expect(assistant.locator('.full-analysis')).toBeVisible();
});

test('tour completes all steps without external calls and restores the previous view', async ({
    page,
}) => {
    const external: string[] = [];
    page.on('request', (request) => {
        if (!request.url().startsWith('http://127.0.0.1:1420')) external.push(request.url());
    });
    await page.goto('/');
    await page
        .getByRole('navigation')
        .getByRole('button', { name: 'Workloads', exact: true })
        .click();
    await page.getByRole('button', { name: /Example Browser/ }).click();
    await page.getByRole('button', { name: 'Take a tour', exact: true }).click();
    const tour = page.getByRole('dialog', { name: 'Jitterbug tour' });
    await expect(tour).toBeVisible();
    await expect(tour.getByRole('button', { name: 'Back', exact: true })).toBeDisabled();
    for (let step = 1; step <= 6; step++) {
        await expect(tour.getByLabel(`Step ${step} of 6`)).toBeVisible();
        await expect(tour.locator('.spotlight')).toBeVisible();
        await expect(tour.locator('.tour-card')).toBeInViewport();
        await tour
            .getByRole('button', { name: step === 6 ? 'Finish' : 'Next', exact: true })
            .click();
    }
    await expect(tour).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Example Browser', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Take a tour', exact: true })).toBeFocused();
    await page.reload();
    await expect(page.getByRole('button', { name: 'Take a quick tour' })).toHaveCount(0);
    expect(external).toEqual([]);
});

test('tour supports back, keyboard exit, replay, and the smallest window', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('button', { name: 'Take a quick tour' }).click();
    const tour = page.getByRole('dialog', { name: 'Jitterbug tour' });
    await tour.getByRole('button', { name: 'Next', exact: true }).click();
    await tour.getByRole('button', { name: 'Back', exact: true }).click();
    await expect(tour.getByLabel('Step 1 of 6')).toBeVisible();
    for (let step = 1; step < 6; step++) {
        await tour.getByRole('button', { name: 'Next', exact: true }).click();
        const bounds = await tour.locator('.tour-card').boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.y).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(760);
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(560);
    }
    await page.keyboard.press('Escape');
    await expect(tour).toHaveCount(0);
    await page.getByRole('button', { name: 'Take a tour', exact: true }).click();
    await expect(tour.getByLabel('Step 1 of 6')).toBeVisible();
    await tour.getByRole('button', { name: 'Skip tour', exact: true }).click();
    await expect(tour).toHaveCount(0);
});
