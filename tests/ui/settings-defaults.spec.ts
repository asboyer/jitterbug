import { expect, test, type Page } from '@playwright/test';
import { demoBootstrap, demoSnapshot } from '../../src/lib/demo';
import type { SupportedSettings } from '../../src/lib/types';

const savedSettings: SupportedSettings = {
    mode: 'cloud_only',
    decision: { provider: 'jev', model: 'jev-1.14.0' },
    cheap: { provider: 'anthropic', model: 'claude-haiku-5-5' },
    intelligent: { provider: 'meta', model: 'muse-spark-1.2' },
    decision_enabled: true,
    budget_usd: 6,
};

async function mockDesktop(page: Page) {
    const snapshot = demoSnapshot();
    snapshot.source = 'live';
    const bootstrap = structuredClone(demoBootstrap);
    await page.addInitScript(
        ({ snapshot, bootstrap, savedSettings }) => {
            let callbackId = 0;
            let historyCalls = 0;
            let savedSequence = 3;
            const calls: { command: string; args: Record<string, unknown> }[] = [];
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
                        if (command === 'get_settings_default_history') {
                            historyCalls += 1;
                            if (historyCalls === 1)
                                return {
                                    entries: [
                                        {
                                            id: '0000000000002-0000000000000002',
                                            current: true,
                                            created_at_ms: 2_000,
                                            summary: 'Hybrid privacy · $2.00 ceiling',
                                            settings: bootstrap.settings,
                                            error: null,
                                        },
                                        {
                                            id: '0000000000001-0000000000000001',
                                            current: false,
                                            created_at_ms: Number.MAX_VALUE,
                                            summary: null,
                                            settings: null,
                                            error: 'This saved default is corrupt.',
                                        },
                                    ],
                                    next_cursor: '0000000000001-0000000000000001',
                                };
                            return {
                                entries: [
                                    {
                                        id: '0000000000000-0000000000000000',
                                        current: false,
                                        created_at_ms: 1_000,
                                        summary: 'Cloud only privacy · $6.00 ceiling',
                                        settings: savedSettings,
                                        error: null,
                                    },
                                ],
                                next_cursor: null,
                            };
                        }
                        if (command === 'set_settings_default') {
                            const suffix = String(savedSequence++).padStart(16, '0');
                            return {
                                id: `0000000000003-${suffix}`,
                                current: true,
                                created_at_ms: 3_000,
                                summary: 'Hybrid privacy · $2.00 ceiling',
                                settings: bootstrap.settings,
                                error: null,
                            };
                        }
                        if (command === 'preview_settings_restore')
                            return {
                                token: 'one-shot-preview',
                                expires_at_ms: Date.now() + 300_000,
                                target: {
                                    id: args.id,
                                    current: false,
                                    created_at_ms: 1_000,
                                    summary: 'Cloud only privacy · $6.00 ceiling',
                                    settings: savedSettings,
                                    error: null,
                                },
                                changes: [
                                    {
                                        field: 'Privacy mode',
                                        before: 'Hybrid',
                                        after: 'Cloud only',
                                    },
                                    { field: 'Budget ceiling', before: '$2.00', after: '$6.00' },
                                ],
                            };
                        if (command === 'cancel_settings_restore') return;
                        if (command === 'confirm_settings_restore') {
                            bootstrap.settings = {
                                ...savedSettings,
                                automation: bootstrap.settings.automation,
                            };
                            if (document.documentElement.dataset.failRestore === 'true')
                                throw 'Cannot durably persist local application state.';
                            return bootstrap.settings;
                        }
                        throw new Error(`Unexpected command: ${command}`);
                    },
                },
            });
        },
        { snapshot, bootstrap, savedSettings }
    );
}

test('settings defaults use paginated backend history and confirmed restore', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'Saved settings defaults' });
    await expect(defaults).toContainText('privacy mode');
    await expect(defaults).toContainText('provider and model choices for all three tiers');
    await expect(defaults).toContainText('Automation policy and permissions');
    await expect(defaults).toContainText('not an operating-system backup');
    await expect(defaults).toContainText('This saved default is corrupt.');
    await expect(defaults).toContainText('Timestamp unavailable');
    await expect(
        defaults.getByRole('button', { name: /Rollback current saved default/ })
    ).toBeVisible();

    await page.getByLabel('Cloud budget ceiling (USD)').fill('9');
    await defaults.getByRole('button', { name: 'Set current saved settings as default' }).click();
    await expect(defaults).toContainText('Saved settings captured');
    const setCall = await page
        .locator('html')
        .evaluate((element) =>
            JSON.parse(element.dataset.settingsCalls ?? '[]').find(
                (call: { command: string }) => call.command === 'set_settings_default'
            )
        );
    expect(setCall.args).toEqual({});

    await defaults.getByRole('button', { name: 'Load older defaults' }).click();
    await expect(defaults).toContainText('Current saved default');
    await expect(defaults).toContainText('Hybrid privacy · $2.00 ceiling');
    await expect(
        defaults.getByRole('button', { name: /Rollback current saved default/ })
    ).toBeVisible();
    await expect(defaults.getByRole('button', { name: 'Return to latest defaults' })).toBeVisible();
    await defaults
        .getByRole('button', { name: /Preview historic restore from/ })
        .first()
        .click();
    let dialog = page.getByRole('dialog', { name: 'Confirm settings restore' });
    await expect(dialog).toContainText('Hybrid → Cloud only');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);

    await defaults
        .getByRole('button', { name: /Preview historic restore from/ })
        .first()
        .click();
    dialog = page.getByRole('dialog', { name: 'Confirm settings restore' });
    await dialog.getByRole('button', { name: 'Restore saved settings' }).click();
    await expect(page.getByLabel('Cloud budget ceiling (USD)')).toHaveValue('6');
    await expect(page.getByText('Settings restored and read back from disk.')).toBeVisible();
    const calls = await page
        .locator('html')
        .evaluate((element) => JSON.parse(element.dataset.settingsCalls ?? '[]'));
    expect(calls).toContainEqual({
        command: 'cancel_settings_restore',
        args: { token: 'one-shot-preview' },
    });
    expect(calls).toContainEqual({
        command: 'confirm_settings_restore',
        args: { token: 'one-shot-preview', confirmed: true },
    });
});

test('settings default pages and repeated captures keep bounded DOM state', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'Saved settings defaults' });

    for (let index = 0; index < 25; index += 1) {
        await defaults
            .getByRole('button', { name: 'Set current saved settings as default' })
            .click();
    }
    expect(await defaults.locator('article.default-entry').count()).toBeLessThanOrEqual(20);

    await defaults.getByRole('button', { name: 'Load older defaults' }).click();
    await expect(defaults.locator('article.default-entry')).toHaveCount(2);
    const historyCalls = await page
        .locator('html')
        .evaluate((element) =>
            JSON.parse(element.dataset.settingsCalls ?? '[]').filter(
                (call: { command: string }) => call.command === 'get_settings_default_history'
            )
        );
    // After 25 captures, the last visible entry is capture 8, not the original page cursor.
    expect(historyCalls.at(-1).args.cursor).toBe('0000000000003-0000000000000008');
    await expect(defaults.getByText('Current saved default')).toBeVisible();
    await defaults.getByRole('button', { name: 'Return to latest defaults' }).click();
    await expect(defaults.getByText('Current saved default')).toBeVisible();
});

test('uncertain restore failure refreshes effective settings while preserving the error', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'Saved settings defaults' });
    await defaults.getByRole('button', { name: 'Load older defaults' }).click();
    await page.locator('html').evaluate((element) => {
        element.dataset.failRestore = 'true';
    });
    await defaults.getByRole('button', { name: /Preview historic restore from/ }).click();
    await page
        .getByRole('dialog', { name: 'Confirm settings restore' })
        .getByRole('button', { name: 'Restore saved settings' })
        .click();

    await expect(page.getByLabel('Cloud budget ceiling (USD)')).toHaveValue('6');
    await expect(defaults.getByRole('alert')).toContainText(
        'Cannot durably persist local application state.'
    );
});

test('browser preview states that saved defaults are nonpersistent', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    const defaults = page.getByRole('region', { name: 'Saved settings defaults' });
    await expect(defaults).toContainText('Preview only');
    await expect(defaults).toContainText('nothing is stored or restored');
    await expect(
        defaults.getByRole('button', { name: 'Set current saved settings as default' })
    ).toBeDisabled();
});
