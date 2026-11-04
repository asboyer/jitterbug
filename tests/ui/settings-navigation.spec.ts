import { expect, test, type Page } from '@playwright/test';
import { demoBootstrap, demoSnapshot } from '../../src/lib/demo';

async function openSettings(page: Page) {
    await mockSettings(page);
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
                        if (command === 'get_ai_activity') return [];
                        if (command === 'get_agent_runner')
                            return '/Applications/Jitterbug/jitterbug-run';
                        if (command === 'get_session_permissions')
                            return {
                                environment: { os: 'macos', version: '15.6', arch: 'arm64' },
                                groups: [],
                            };
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
        {
            bootstrap: structuredClone(demoBootstrap),
            snapshot: { ...demoSnapshot(), source: 'live' },
        }
    );
}

async function openAi(page: Page) {
    await page.getByRole('tab', { name: 'AI & models', exact: true }).click();
}

test('settings categories support keyboard navigation, including Meta', async ({ page }) => {
    await openSettings(page);
    await expect(page.getByRole('checkbox', { name: 'Watch for slowdowns' })).toBeVisible();
    await page.getByRole('tab', { name: 'Assistance' }).focus();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('tab', { name: 'Integrations' })).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('region', { name: 'Session optimization rights' })).toBeVisible();
    await page.keyboard.press('End');
    await expect(page.getByRole('tab', { name: 'AI & models' })).toBeFocused();
    await expect(page.getByRole('heading', { name: 'Where AI runs' })).toBeVisible();
    await page.keyboard.press('Home');
    await expect(page.getByRole('tab', { name: 'Assistance' })).toBeFocused();
});

test('Local, Hybrid, and Frontier save the chosen routing and preserve permissions', async ({
    page,
}) => {
    await openSettings(page);
    await page.getByRole('checkbox', { name: 'Show action popups' }).uncheck();
    await page.getByRole('button', { name: 'Save preferences' }).click();
    await expect(page.getByRole('status')).toHaveText('Preferences saved.');
    await openAi(page);
    for (const mode of ['Local', 'Hybrid', 'Frontier'])
        await expect(page.getByRole('radio', { name: new RegExp(`^${mode}`) })).toBeVisible();
    await page.getByRole('radio', { name: /^Local/ }).check();
    await page.getByRole('spinbutton', { name: 'Spending ceiling · USD' }).fill('1.25');
    await page.getByRole('button', { name: 'Save AI settings' }).click();
    await expect(page.getByRole('status')).toHaveText('AI settings saved.');
    const calls = await page
        .locator('html')
        .evaluate((el) => JSON.parse(el.dataset.settingsCalls ?? '[]'));
    const settings = calls.find((c: any) => c.command === 'save_settings').args.settings;
    expect(settings.mode).toBe('local_only');
    expect(settings.intelligent.provider).toBe('ollama');
    expect(settings.automation.action_popups).toBe(false);
    expect(settings.automation.agent_control).toBe(demoBootstrap.settings.automation.agent_control);
    expect(settings.budget_usd).toBe(1.25);
});

test('Meta setup and privacy controls remain reachable at the minimum window size', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 560 });
    await openSettings(page);
    await page.getByText('Thresholds & privacy', { exact: true }).click();
    await page.getByText('What is sent to a model', { exact: true }).click();
    await expect(page.getByRole('tabpanel')).toContainText('Paths, environment variables');
    await openAi(page);
    await page
        .getByRole('region', { name: 'Model providers' })
        .getByRole('button', { name: 'Add key' })
        .first()
        .click();
    await expect(page.getByLabel('Meta API key')).toHaveAttribute('type', 'password');
    await page.getByLabel('Meta API key').fill('not-a-real-credential');
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByLabel('Meta API key')).toHaveCount(0);
    expect(await page.locator('main').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
        true
    );
});
