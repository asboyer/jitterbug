import { expect, test, type Page } from '@playwright/test';
import { demoBootstrap, demoSnapshot } from '../../src/lib/demo';

async function openSettings(page: Page) {
    await page.goto('/');
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
}

async function mockSettings(page: Page) {
    await page.addInitScript(
        ({ bootstrap, snapshot }) => {
            let callback = 0;
            const calls: { command: string; args: Record<string, any> }[] = [];
            Object.defineProperty(window, 'isTauri', { value: true });
            Object.defineProperty(window, '__TAURI_INTERNALS__', {
                value: {
                    transformCallback: () => ++callback,
                    unregisterCallback: () => {},
                    invoke: async (command: string, args: Record<string, any> = {}) => {
                        if (command.startsWith('plugin:event|')) return 1;
                        if (command === 'get_bootstrap') return structuredClone(bootstrap);
                        if (command === 'get_snapshot') return snapshot;
                        if (command === 'get_workload_icon') return null;
                        if (command === 'get_jitter_tally') return { entries: [], sampling: true };
                        if (command === 'get_system_default_history') {
                            return { entries: [], next_cursor: null, interrupted_operations: 0 };
                        }
                        calls.push({ command, args });
                        document.documentElement.dataset.settingsCalls = JSON.stringify(calls);
                        if (command === 'save_automation_policy') {
                            bootstrap.settings.automation = JSON.parse(JSON.stringify(args.policy));
                            return;
                        }
                        if (command === 'save_settings') {
                            const automation = bootstrap.settings.automation;
                            bootstrap.settings = {
                                ...JSON.parse(JSON.stringify(args.settings)),
                                automation,
                            };
                            return;
                        }
                        throw new Error(`Unexpected command: ${command}`);
                    },
                },
            });
        },
        { bootstrap: structuredClone(demoBootstrap), snapshot: demoSnapshot() }
    );
}

test('settings opens on a compact assistance page and supports keyboard categories', async ({
    page,
}) => {
    await openSettings(page);
    await expect(page.getByRole('tab', { name: 'Assistance' })).toHaveAttribute(
        'aria-selected',
        'true'
    );
    await expect(page.getByRole('checkbox', { name: 'Watch for slowdowns' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Cloud budget' })).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Save preferences' })).toBeInViewport();
    await page.getByRole('tab', { name: 'Assistance' }).focus();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('tab', { name: 'AI & budget' })).toBeFocused();
    await expect(page.getByRole('heading', { name: 'Where AI runs' })).toBeVisible();
    await page.keyboard.press('End');
    await expect(page.getByRole('tab', { name: 'System', exact: true })).toBeFocused();
    await expect(page.getByRole('region', { name: 'System settings baseline' })).toBeVisible();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('tab', { name: 'Assistance' })).toBeFocused();
    await page.keyboard.press('ArrowLeft');
    await expect(page.getByRole('tab', { name: 'System', exact: true })).toBeFocused();
    await page.keyboard.press('Home');
    await expect(page.getByRole('tab', { name: 'Assistance' })).toBeFocused();
});

test('draft edits survive category switches and save to the correct policy commands', async ({
    page,
}) => {
    await mockSettings(page);
    await openSettings(page);
    await page.getByRole('checkbox', { name: 'Show action popups' }).uncheck();
    await page.getByRole('tab', { name: 'AI & budget' }).click();
    await page.getByRole('radio', { name: /Local only/ }).check();
    await page.getByRole('spinbutton', { name: 'Spending ceiling · USD' }).fill('1.25');
    await page.getByRole('tab', { name: 'Connections' }).click();
    await expect(
        page.getByRole('checkbox', { name: 'Allow pauses without asking' })
    ).toBeDisabled();
    await expect(page.getByRole('region', { name: 'Agent access' })).toContainText(
        'may reach the agent’s model provider'
    );
    await page.getByRole('checkbox', { name: 'Allow agent access' }).check();
    await expect(page.getByRole('checkbox', { name: 'Allow pauses without asking' })).toBeEnabled();
    await expect(
        page.getByRole('checkbox', { name: 'Allow pauses without asking' })
    ).not.toBeChecked();
    await page.getByRole('tab', { name: 'Assistance' }).click();
    await expect(page.getByRole('checkbox', { name: 'Show action popups' })).not.toBeChecked();
    await page.getByRole('button', { name: 'Save preferences' }).click();
    await expect(page.getByRole('status')).toHaveText('Preferences saved.');
    await page.getByRole('tab', { name: 'AI & budget' }).click();
    await expect(page.getByRole('radio', { name: /Local only/ })).toBeChecked();
    await expect(page.getByRole('spinbutton', { name: 'Spending ceiling · USD' })).toHaveValue(
        '1.25'
    );
    await page.getByRole('button', { name: 'Save AI settings' }).click();
    const calls = await page
        .locator('html')
        .evaluate((element) => JSON.parse(element.dataset.settingsCalls ?? '[]'));
    expect(calls).toEqual([
        {
            command: 'save_automation_policy',
            args: {
                policy: {
                    ...demoBootstrap.settings.automation,
                    action_popups: false,
                    agent_control: true,
                },
            },
        },
        {
            command: 'save_settings',
            args: {
                settings: { ...demoBootstrap.settings, mode: 'local_only', budget_usd: 1.25 },
            },
        },
    ]);
});

test('advanced controls, costs and consent remain reachable at a narrow width', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await openSettings(page);
    await page.getByText('Jitter Snapshot', { exact: true }).click();
    await page.getByRole('radio', { name: /Only when I ask/ }).check();
    await expect(
        page.getByRole('checkbox', { name: 'Take snapshots automatically' })
    ).toBeDisabled();
    await page.getByText('Thresholds & privacy', { exact: true }).click();
    await expect(
        page.getByRole('spinbutton', { name: 'Whole-machine CPU threshold (%)' })
    ).toBeVisible();
    await page.getByText('What is sent to a model', { exact: true }).click();
    await expect(page.getByRole('tabpanel')).toContainText('Paths, environment variables');
    await page.getByRole('tab', { name: 'AI & budget' }).click();
    await page.getByText('Model routing', { exact: true }).click();
    await expect(page.getByRole('combobox', { name: 'intelligent provider' })).toBeVisible();
    await page.getByText('Usage estimate, not a provider bill', { exact: true }).click();
    await expect(page.getByRole('region', { name: 'Cloud budget' })).toContainText(
        '$0.10 per request'
    );
    await page.getByRole('tab', { name: 'Connections' }).click();
    await page.getByText('Connection setup & permissions', { exact: true }).click();
    await expect(page.getByRole('region', { name: 'Agent access' })).toContainText(
        'Any process running as your OS user can connect'
    );
    await page.getByText('Add or replace a key', { exact: true }).click();
    await expect(page.getByRole('textbox', { name: 'API key' })).toBeDisabled();
    expect(await page.locator('main').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
        true
    );
});
