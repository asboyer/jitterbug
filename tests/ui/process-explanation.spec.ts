import { expect, test, type Page } from '@playwright/test';
import { demoIncidents } from '../../src/lib/demo';
import { desktop } from './fixtures/monitor';

async function openBrowser(page: Page, launch = false) {
    await desktop(page, { launch });
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
}

test('recognized jobs and origin evidence remain available beside the automatic read', async ({
    page,
}) => {
    await openBrowser(page, true);
    const inspector = page.getByRole('dialog', { name: 'Process inspector' });
    await expect(inspector.getByRole('region', { name: 'App summary' })).toContainText(
        'Hermes is running its background gateway'
    );
    await inspector.getByRole('button', { name: 'Details', exact: true }).click();
    await expect(inspector).toContainText('Running command: hermes gateway run');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('fixture-sample')));
    await expect(inspector).toContainText('Running command: hermes gateway run');
    // The only model call is the automatic read of the selected app.
    const asks = JSON.parse((await page.locator('html').getAttribute('data-ask-calls')) ?? '[]');
    expect(asks.length).toBeLessThanOrEqual(1);
    for (const ask of asks) expect(ask.messages.at(-1).content).toMatch(/^Give me your read/);
});

test('a memory prompt closes after recovery while its findings stay in Inbox', async ({ page }) => {
    await openBrowser(page);
    const incident = demoIncidents()[0];
    incident.source = 'live';
    incident.stage = 'detected';
    incident.trigger.kind = 'memory_pressure';
    incident.trigger.key = 'memory-pressure';
    incident.proposal = null;
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    const prompt = page.getByRole('region', { name: 'Jitterbug noticed something' });
    await expect(prompt).toContainText(/your computer is short on memory/i);
    incident.stage = 'completed';
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    await expect(prompt).toContainText('No automatic change was applied');
    incident.stage = 'resolved';
    await page.evaluate((detail) => {
        window.dispatchEvent(new CustomEvent('fixture-incident', { detail }));
    }, incident);
    await expect(prompt).toHaveCount(0);
    await page
        .getByRole('navigation')
        .getByRole('button', { name: /^Inbox/ })
        .click();
    await expect(page.getByText('Memory pressure detected', { exact: true })).toBeVisible();
    await expect(page.getByText('Trigger cleared', { exact: true })).toBeVisible();
});

test('origin details fit a small window and keep names separate from PIDs', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 650 });
    await openBrowser(page);
    const inspector = page.getByRole('dialog', { name: 'Process inspector' });
    await inspector.getByRole('button', { name: 'Details', exact: true }).click();
    const heading = inspector.locator('.technical ol li').last();
    const name = await heading.locator('strong').boundingBox();
    const pid = await heading.locator('span').boundingBox();
    expect(pid!.x - name!.x - name!.width).toBeGreaterThanOrEqual(10);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});

test('chat preserves readable history in one keyboard-scrollable pane', async ({ page }) => {
    await openBrowser(page);
    const inspector = page.getByRole('dialog', { name: 'Process inspector' });
    const question = inspector.getByRole('textbox', { name: 'Message Jitterbug' });
    for (const text of [
        'Why is this app lagging?',
        'What should I watch for?',
        'Which helper uses memory?',
    ]) {
        await question.fill(text);
        await question.press('Enter');
        await expect(inspector.getByRole('button', { name: 'Send message' })).toBeDisabled();
        await expect(inspector.getByText('Thinking…', { exact: true })).toHaveCount(0);
    }
    const log = inspector.getByRole('log', { name: 'Process conversation' });
    // Three questions and answers, after Jitterbug's automatic read of the app.
    await expect(log.locator('.turn')).toHaveCount(7);
    await expect(log).toContainText('Why is this app lagging?');
    await log.focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('End');
    await expect.poll(() => log.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await expect(question).toBeInViewport();
    await expect(page.locator('.insight-card')).toHaveCount(0);
    await expect(page.getByRole('complementary', { name: 'Contextual assistant' })).toHaveCount(0);
});
