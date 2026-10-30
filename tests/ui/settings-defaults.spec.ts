import { expect, test, type Page } from '@playwright/test';
import { demoBootstrap, demoSnapshot } from '../../src/lib/demo';

const baselineValues = { output_volume: 0.42, output_muted: false };

async function mockDesktop(page: Page) {
    const snapshot = demoSnapshot();
    snapshot.source = 'live';
    const bootstrap = structuredClone(demoBootstrap);
    await page.addInitScript(
        ({ snapshot, bootstrap, baselineValues }) => {
            let callbackId = 0;
            let historyCalls = 0;
            let savedSequence = 3;
            const calls: { command: string; args: Record<string, unknown> }[] = [];
            const view = (
                id: string,
                current: boolean,
                createdAt: number,
                values: typeof baselineValues | null = baselineValues
            ) => ({
                id,
                current,
                created_at_ms: createdAt,
                summary: values ? 'volume 42% · unmuted' : null,
                values,
                compatible: true,
                error: values ? null : 'This system baseline is corrupt.',
            });
            Object.defineProperty(window, 'isTauri', { value: true });
            Object.defineProperty(window, '__TAURI_INTERNALS__', {
                value: {
                    transformCallback: () => ++callbackId,
                    unregisterCallback: () => {},
                    invoke: async (command: string, args: Record<string, unknown> = {}) => {
                        if (command.startsWith('plugin:event|')) return 1;
                        if (command === 'get_snapshot') return snapshot;
                        if (command === 'get_bootstrap') return bootstrap;
                        if (command === 'get_workload_icon') return null;
                        calls.push({ command, args });
                        document.documentElement.dataset.settingsCalls = JSON.stringify(calls);
                        if (command === 'get_system_default_history') {
                            historyCalls += 1;
                            if (historyCalls === 1)
                                return {
                                    entries: [
                                        view('0000000000002-0000000000000002', true, 2_000),
                                        view(
                                            '0000000000001-0000000000000001',
                                            false,
                                            Number.MAX_VALUE,
                                            null
                                        ),
                                    ],
                                    next_cursor: '0000000000001-0000000000000001',
                                    interrupted_operations: 0,
                                };
                            return {
                                entries: [view('0000000000000-0000000000000000', false, 1_000)],
                                next_cursor: null,
                                interrupted_operations: 0,
                            };
                        }
                        if (command === 'set_system_default') {
                            const suffix = String(savedSequence++).padStart(16, '0');
                            return view(`0000000000003-${suffix}`, true, 3_000);
                        }
                        if (command === 'preview_system_restore')
                            return {
                                token: 'one-use-preview',
                                expires_at_ms: Date.now() + 300_000,
                                target: view(String(args.id), false, 1_000),
                                changes: [
                                    { field: 'Output volume', before: '80.0%', after: '42.0%' },
                                    { field: 'Output mute', before: 'Muted', after: 'Unmuted' },
                                ],
                            };
                        if (command === 'cancel_system_restore') return;
                        if (command === 'confirm_system_restore') {
                            if (document.documentElement.dataset.partialRollback === 'true')
                                return {
                                    status: 'partial_failure',
                                    outcomes: [
                                        {
                                            setting: 'output_volume',
                                            field: 'Output volume',
                                            before: '80.0%',
                                            target: '42.0%',
                                            status: 'readback_mismatch',
                                            error: 'readback mismatch',
                                        },
                                    ],
                                    current: { output_volume: 0.8, output_muted: true },
                                    journal_id: 'journal',
                                    message: 'Rollback stopped after a failed write or readback.',
                                };
                            return {
                                status: 'verified',
                                outcomes: [
                                    {
                                        setting: 'output_volume',
                                        field: 'Output volume',
                                        before: '80.0%',
                                        target: '42.0%',
                                        status: 'verified',
                                        error: null,
                                    },
                                ],
                                current: baselineValues,
                                journal_id: 'journal',
                                message:
                                    'Requested settings were written and verified by fresh native readback.',
                            };
                        }
                        throw new Error(`Unexpected command: ${command}`);
                    },
                },
            });
        },
        { snapshot, bootstrap, baselineValues }
    );
}

test('system baseline has explicit native scope and exact confirmed rollback', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await expect(defaults).toContainText('default output device');
    await expect(defaults).toContainText('Provider configuration');
    await expect(defaults).toContainText('not a performance improvement');
    await expect(defaults).not.toContainText('provider and model choices for all three tiers');

    await defaults.getByRole('button', { name: 'Set Default' }).click();
    await expect(defaults).toContainText('Actual system audio settings captured');
    await defaults.getByRole('button', { name: 'View past defaults' }).click();
    const history = page.getByRole('dialog', { name: 'System baseline history' });
    await history.getByRole('button', { name: 'Load older defaults' }).click();
    await history.getByRole('button', { name: /Preview past system baseline/ }).click();
    const confirmation = page.getByRole('dialog', { name: 'Confirm system settings rollback' });
    await expect(confirmation).toContainText('80.0% → 42.0%');
    await expect(confirmation).toContainText('Muted → Unmuted');
    await confirmation.getByRole('button', { name: 'Cancel' }).click();
    await expect(history).toBeVisible();

    await history.getByRole('button', { name: /Preview past system baseline/ }).click();
    await page
        .getByRole('dialog', { name: 'Confirm system settings rollback' })
        .getByRole('button', { name: 'Roll back system settings' })
        .click();
    await expect(history).toContainText('verified by fresh native readback');
    const calls = await page
        .locator('html')
        .evaluate((element) => JSON.parse(element.dataset.settingsCalls ?? '[]'));
    expect(calls).toContainEqual({
        command: 'cancel_system_restore',
        args: { token: 'one-use-preview' },
    });
    expect(calls).toContainEqual({
        command: 'confirm_system_restore',
        args: { token: 'one-use-preview', confirmed: true },
    });
});

test('history pagination and repeated captures keep modal DOM bounded', async ({ page }) => {
    await mockDesktop(page);
    await page.setViewportSize({ width: 760, height: 560 });
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    for (let index = 0; index < 25; index += 1) {
        await defaults.getByRole('button', { name: 'Set Default' }).click();
    }
    await defaults.getByRole('button', { name: 'View past defaults' }).click();
    const dialog = page.getByRole('dialog');
    expect(await dialog.locator('article.default-entry').count()).toBeLessThanOrEqual(20);
    const bounds = await dialog.boundingBox();
    expect(bounds!.height).toBeLessThanOrEqual(528);
    expect(bounds!.width).toBeLessThanOrEqual(728);
    const scroller = dialog.getByRole('region', { name: 'System baseline snapshots' });
    expect(await scroller.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
        true
    );
    await dialog.getByRole('button', { name: 'Load older defaults' }).click();
    await expect(dialog.locator('article.default-entry')).toHaveCount(2);
    const historyCalls = await page
        .locator('html')
        .evaluate((element) =>
            JSON.parse(element.dataset.settingsCalls ?? '[]').filter(
                (call: { command: string }) => call.command === 'get_system_default_history'
            )
        );
    expect(historyCalls.at(-1).args.cursor).toBe('0000000000003-0000000000000008');
});

test('partial native readback failure is explicit and never presented as success', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('button', { name: 'View past defaults' }).click();
    const history = page.getByRole('dialog');
    await history.getByRole('button', { name: 'Load older defaults' }).click();
    await page.locator('html').evaluate((element) => {
        element.dataset.partialRollback = 'true';
    });
    await history.getByRole('button', { name: /Preview past system baseline/ }).click();
    await history.getByRole('button', { name: 'Roll back system settings' }).click();

    await expect(history.getByRole('alert')).toContainText('failed write or readback');
    await expect(history).toContainText('readback mismatch');
});

test('browser preview is nonpersistent and modal closes accessibly', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await expect(defaults).toContainText('Preview only');
    await expect(defaults).toContainText('nothing is stored or restored');
    await expect(defaults.getByRole('button', { name: 'Set Default' })).toBeDisabled();
});

test('history opens without inline growth and Escape restores trigger focus', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    const before = await defaults.boundingBox();
    const opener = defaults.getByRole('button', { name: 'View past defaults' });
    await opener.click();
    await expect(page.getByRole('dialog', { name: 'System baseline history' })).toBeVisible();
    expect(await defaults.boundingBox()).toEqual(before);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(opener).toBeFocused();
});
