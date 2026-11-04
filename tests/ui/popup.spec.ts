import { expect, test } from '@playwright/test';
import { demoActionPopup } from '../../src/lib/demo';

test.use({ viewport: { width: 360, height: 300 } });

for (const view of ['popup', 'popup&example=tab', 'lab']) {
    test(`old ${view} URL cannot generate a fake result`, async ({ page }) => {
        await page.goto(`/?view=${view}`);
        await expect(page.getByText('EXAMPLE', { exact: true })).toHaveCount(0);
        await expect(page.getByRole('button', { name: 'Apply', exact: true })).toHaveCount(0);
        await expect(page.getByText('sample-web-app has stayed busy')).toHaveCount(0);
    });
}

test('native popup actions call the native command and never manufacture completion', async ({
    page,
}) => {
    const payload = demoActionPopup();
    if (payload.kind !== 'case') throw new Error('Expected case fixture');
    payload.source = 'live';
    await page.addInitScript((payload) => {
        Object.defineProperty(window, 'isTauri', { value: true });
        Object.defineProperty(window, '__TAURI_INTERNALS__', {
            value: {
                transformCallback: () => 1,
                unregisterCallback: () => {},
                invoke: async (command: string) => {
                    if (command.startsWith('plugin:event|')) return 1;
                    if (command === 'get_action_popup') return payload;
                    document.documentElement.dataset.action = command;
                },
            },
        });
    }, payload);
    await page.goto('/?view=popup');
    await expect(page.getByText('LIVE', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-action', 'approve_optimization');
    await expect(page.getByText('Checking the result…')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Later', exact: true })).toBeInViewport();
});
