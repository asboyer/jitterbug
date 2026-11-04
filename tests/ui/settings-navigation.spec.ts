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
                        if (command === 'get_agent_runner')
                            return '/Applications/Jitterbug/jitterbug-run';
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

async function openAi(page: Page) {
    await page.getByRole('navigation').getByRole('button', { name: 'AI', exact: true }).click();
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
    await expect(page.getByRole('button', { name: 'Save preferences' })).toBeInViewport();
    await page.getByRole('tab', { name: 'Assistance' }).focus();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('tab', { name: 'Integrations' })).toBeFocused();
    await expect(page.getByRole('checkbox', { name: 'Allow agent access' })).toBeVisible();
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

test('assistance and AI pages save to the correct commands', async ({ page }) => {
    await mockSettings(page);
    await openSettings(page);
    await page.getByRole('checkbox', { name: 'Show action popups' }).uncheck();
    await page.getByRole('tab', { name: 'Integrations' }).click();
    await expect(
        page.getByRole('checkbox', { name: 'Allow pauses without asking' })
    ).toBeDisabled();
    await expect(page.getByRole('region', { name: 'Agent access' })).toContainText(
        'may reach the agent’s model provider'
    );
    await page.getByRole('checkbox', { name: 'Allow agent access' }).check();
    await expect(page.getByRole('checkbox', { name: 'Allow pauses without asking' })).toBeEnabled();
    await page.getByRole('tab', { name: 'Assistance' }).click();
    await expect(page.getByRole('checkbox', { name: 'Show action popups' })).not.toBeChecked();
    await page.getByRole('button', { name: 'Save preferences' }).click();
    await expect(page.getByRole('status')).toHaveText('Preferences saved.');
    await openAi(page);
    await page.getByRole('radio', { name: /^Local/ }).check();
    await expect(page.getByRole('combobox', { name: 'Deep investigation model' })).toHaveValue(
        'ollama/qwen3.5:0.8b'
    );
    await page.getByRole('spinbutton', { name: 'Spending ceiling · USD' }).fill('1.25');
    await page.getByRole('button', { name: 'Save AI settings' }).click();
    await expect(page.getByRole('status')).toHaveText('AI settings saved.');
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
                settings: {
                    ...demoBootstrap.settings,
                    // The AI page opened after the policy save, so it carries the saved policy.
                    automation: {
                        ...demoBootstrap.settings.automation,
                        action_popups: false,
                        agent_control: true,
                    },
                    mode: 'local_only',
                    intelligent: { provider: 'ollama', model: 'qwen3.5:0.8b' },
                    budget_usd: 1.25,
                },
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
    await page.getByRole('tab', { name: 'Integrations' }).click();
    await page.getByText('Connection setup & permissions', { exact: true }).click();
    await expect(page.getByRole('region', { name: 'Agent access' })).toContainText(
        'Any process running as your OS user can connect'
    );
    expect(await page.locator('main').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
        true
    );
    await openAi(page);
    await page.getByRole('checkbox', { name: 'Smart routing' }).check();
    await expect(page.getByRole('combobox', { name: 'Routing model' })).toBeVisible();
    await page.getByText('Usage estimate, not a provider bill', { exact: true }).click();
    await expect(page.getByRole('region', { name: 'Spending' })).toContainText('$0.10 per request');
    const providers = page.getByRole('region', { name: 'Model providers' });
    await expect(providers.getByRole('listitem').first()).toContainText('Key needed');
    await providers.getByRole('button', { name: 'Add key' }).first().click();
    await expect(page.getByRole('textbox', { name: 'Meta API key' })).toBeDisabled();
    expect(await page.locator('main').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
        true
    );
});

test('the sidebar switch moves between local and frontier and flags a missing key', async ({
    page,
}) => {
    await openSettings(page);
    const sidebar = page.getByRole('navigation');
    await sidebar.getByLabel(/^AI mode: Hybrid/).click();
    await sidebar.getByRole('radio', { name: /^Frontier/ }).click();
    await expect(sidebar.getByRole('radio', { name: /^Frontier/ })).toHaveAttribute(
        'aria-checked',
        'true'
    );
    await expect(sidebar).toContainText('Meta needs an API key.');
    await sidebar.getByRole('button', { name: 'Add key' }).click();
    await expect(page.getByRole('heading', { name: 'AI', exact: true })).toBeVisible();
    await expect(page.getByRole('radio', { name: /^Frontier/ })).toBeChecked();
    await expect(page.getByRole('main')).toContainText('Meta needs an API key.');
    await sidebar.getByLabel(/^AI mode: Frontier/).click();
    await sidebar.getByRole('radio', { name: /^Local/ }).click();
    await expect(sidebar.getByRole('radiogroup', { name: 'Where AI runs' })).toBeHidden();
    await expect(sidebar.getByLabel(/^AI mode: Local/)).toBeVisible();
    await expect(page.getByRole('radio', { name: /^Local/ })).toBeChecked();
});

test('onboarding is opt-in in the preview and ends on the front page with a tour offer', async ({
    page,
}) => {
    await page.goto('/');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Replay onboarding' }).click();
    const dialog = page.getByRole('dialog');
    await expect(
        dialog.getByRole('heading', { name: 'Where should Jitterbug think?' })
    ).toBeVisible();
    await expect(dialog.getByRole('radio', { name: /Meta Muse Spark 1.3/ })).toBeChecked();
    await dialog.getByRole('button', { name: 'Continue' }).click();
    await expect(page.getByRole('heading', { name: 'Add your Meta API key' })).toBeVisible();
    await dialog.locator('input[type="password"]').fill('demo-key-123456');
    await page.getByRole('button', { name: 'Save and continue' }).click();
    await expect(page.getByRole('heading', { name: 'Jitterbug is ready' })).toBeVisible();
    await expect(page.getByRole('dialog')).toContainText('Hybrid · Meta Muse Spark 1.3');
    await page.getByRole('button', { name: 'Go to Jitterbug' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Take a quick tour' })).toBeVisible();
});

test('a first desktop launch without keys opens onboarding, and skipping it sticks', async ({
    page,
}) => {
    await mockSettings(page);
    await page.addInitScript(() => {
        if (!sessionStorage.getItem('reloaded')) localStorage.clear();
    });
    await page.goto('/');
    await expect(
        page.getByRole('heading', { name: 'Where should Jitterbug think?' })
    ).toBeVisible();
    await page.getByRole('button', { name: 'Skip setup' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.evaluate(() => sessionStorage.setItem('reloaded', '1'));
    await page.reload();
    await expect(page.getByRole('navigation')).toBeVisible();
    await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('an agent asking while access is off gets a one-tap prompt', async ({ page }) => {
    await page.goto('/');
    await page.getByText('Demo & testing', { exact: true }).click();
    await page.getByRole('button', { name: 'Preview agent access request' }).click();
    const ask = page.getByRole('alert').filter({ hasText: 'Claude Code wants to see' });
    await expect(ask).toContainText('folder names and tab titles');
    await ask.getByRole('button', { name: 'Allow' }).click();
    await expect(ask).toHaveCount(0);
    await page.getByRole('navigation').getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('tab', { name: 'Integrations' }).click();
    await expect(page.getByRole('checkbox', { name: 'Allow agent access' })).toBeChecked();
});
