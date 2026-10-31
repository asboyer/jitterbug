import { expect, test, type Page } from '@playwright/test';
import { demoBootstrap, demoSnapshot } from '../../src/lib/demo';

const baselineValues = {
    output_volume: 0.42,
    output_muted: false,
    preferences: { dock_autohide: { kind: 'absent' }, key_repeat: { kind: 'integer', value: 6 } },
};

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
                coverage: [
                    {
                        id: 'dock_autohide',
                        label: 'Dock auto-hide',
                        group: 'Desktop & Dock',
                        status: 'supported',
                        included: true,
                        detail: 'Current user override only.',
                        verification: 'Saved preference only — live effect not verified.',
                        pane: 'dock',
                    },
                ],
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
                        if (command === 'get_system_capabilities')
                            return [
                                {
                                    id: 'dock_autohide',
                                    label: 'Dock auto-hide',
                                    group: 'Desktop & Dock',
                                    status: 'supported',
                                    detail: 'Current user override only.',
                                    verification:
                                        'Saved preference only — live effect not verified.',
                                    pane: 'dock',
                                },
                                {
                                    id: 'hardware_100',
                                    label: 'AC Power / sleep',
                                    group: 'Power profiles',
                                    status: 'needs_permission',
                                    included: true,
                                    detail: 'Automatic rollback executor implemented. Requires externally provided authorization.',
                                    verification:
                                        'Saved pmset profile readback; effective sleep not verified.',
                                    pane: null,
                                },
                                {
                                    id: 'system_sleep',
                                    label: 'System idle sleep',
                                    group: 'Power & display',
                                    status: 'unavailable',
                                    detail: 'Native privileged authorization is not implemented. No password prompt or helper.',
                                    verification: 'Not captured; never restored',
                                    pane: 'battery',
                                },
                            ];
                        if (command === 'open_system_settings_pane') return;
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
                                    {
                                        field: 'Dock auto-hide',
                                        group: 'Desktop & Dock',
                                        before: 'Boolean: true',
                                        after: 'Absent (no override in this scope)',
                                        verification:
                                            'Saved preference only; live effect not verified',
                                    },
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

test('persistent-only scope distinguishes hardware observations from restorable values', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await defaults.getByText('What to save', { exact: true }).click();
    await expect(
        defaults.getByRole('checkbox', { name: 'Finder storage', exact: true })
    ).toBeChecked();
    await expect(
        defaults.getByRole('checkbox', { name: /sample rate|display mode|sleep|microphone/i })
    ).toHaveCount(0);
    await expect(defaults).not.toContainText('Included');
    await expect(defaults).not.toContainText('Never included');
});

test('automatic hardware executors are selectable and disclose interruption and authorization', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await defaults.getByText('What to save', { exact: true }).click();
    for (const group of ['Audio hardware', 'Displays', 'Power profiles']) {
        await expect(defaults.getByRole('checkbox', { name: group, exact: true })).toBeChecked();
    }
    await defaults.getByRole('button', { name: 'View past defaults' }).click();
    const dialog = page.getByRole('dialog');
    await dialog.getByRole('button', { name: /Rollback current system baseline/ }).click();
    await expect(dialog).toContainText('interrupt playback or recording');
    await expect(dialog).toContainText('reboot persistence is not verified');
    await dialog.getByRole('button', { name: 'Roll back system settings' }).click();
    const calls = await page
        .locator('html')
        .evaluate((el) => JSON.parse(el.dataset.settingsCalls ?? '[]'));
    expect(
        calls.filter((call: { command: string }) => call.command === 'confirm_system_restore')
    ).toEqual([
        { command: 'confirm_system_restore', args: { token: 'one-use-preview', confirmed: true } },
    ]);
});

test('baseline actions stay on set default and rollback', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await expect(defaults.getByRole('button', { name: /grant|authorize/i })).toHaveCount(0);
    await expect(defaults.getByRole('button', { name: 'Set Default' })).toBeVisible();
    await expect(defaults.getByRole('button', { name: 'View past defaults' })).toBeVisible();
});

test('capture selection is explicit and saved preference previews disclose verification scope', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await defaults.getByText('What to save', { exact: true }).click();
    for (const group of [
        'Audio',
        'Audio hardware',
        'Displays',
        'Power profiles',
        'Desktop & Dock',
        'Keyboard',
        'Pointer',
        'Finder storage',
    ]) {
        await defaults.getByRole('checkbox', { name: group, exact: true }).uncheck();
    }
    await expect(defaults.getByRole('button', { name: 'Set Default' })).toBeDisabled();
    await defaults.getByRole('checkbox', { name: 'Keyboard', exact: true }).check();
    await defaults.getByRole('button', { name: 'Set Default' }).click();
    const calls = await page
        .locator('html')
        .evaluate((el) => JSON.parse(el.dataset.settingsCalls ?? '[]'));
    expect(calls).toContainEqual({ command: 'set_system_default', args: { groups: ['Keyboard'] } });
    await expect(defaults).toContainText('Saved. This is now your current baseline.');
    await defaults.getByRole('button', { name: 'View past defaults' }).click();
    const history = page.getByRole('dialog');
    await history.getByRole('button', { name: /Rollback current system baseline/ }).click();
    await expect(
        history.getByRole('heading', { name: 'Desktop & Dock', exact: true })
    ).toBeVisible();
    await expect(history).toContainText('Absent (no override in this scope)');
    await expect(history).toContainText('No Dock or system process is restarted');
});

test('system baseline has explicit native scope and exact confirmed rollback', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await expect(defaults).toContainText('Save how this PC is set up');
    await expect(defaults).not.toContainText('Included');
    await expect(defaults).not.toContainText('Never included');
    await expect(defaults).not.toContainText('provider and model choices for all three tiers');

    await defaults.getByRole('button', { name: 'Set Default' }).click();
    await expect(defaults).toContainText('Saved. This is now your current baseline.');
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
    await page.getByRole('tab', { name: 'System', exact: true }).click();
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
    await page.getByRole('tab', { name: 'System', exact: true }).click();
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
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await expect(defaults).toContainText('Preview only');
    await expect(defaults).toContainText('nothing is stored or restored');
    await expect(defaults.getByRole('button', { name: 'Set Default' })).toBeDisabled();
});

test('history opens without inline growth and Escape restores trigger focus', async ({ page }) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    const defaults = page.getByRole('region', { name: 'System settings baseline' });
    await defaults.scrollIntoViewIfNeeded();
    const before = await defaults.boundingBox();
    const opener = defaults.getByRole('button', { name: 'View past defaults' });
    await opener.click();
    await expect(page.getByRole('dialog', { name: 'System baseline history' })).toBeVisible();
    const after = await defaults.boundingBox();
    expect(after?.width).toEqual(before?.width);
    expect(after?.height).toEqual(before?.height);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(opener).toBeFocused();
});

test('current baseline rollback opens the confirmation before accessing its dialog', async ({
    page,
}) => {
    await mockDesktop(page);
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'System', exact: true }).click();
    await page
        .getByRole('region', { name: 'System settings baseline' })
        .getByRole('button', { name: /Rollback current system baseline/ })
        .click();
    await expect(
        page.getByRole('dialog', { name: 'Confirm system settings rollback' })
    ).toBeVisible();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'System baseline history' })).toBeVisible();
});
