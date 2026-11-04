import { expect, test, type Page } from '@playwright/test';
import { demoIncidents } from '../../src/lib/demo';
import { desktop } from './fixtures/monitor';

async function openBrowser(page: Page, launch = false) {
    await desktop(page, { launch });
    await page.locator('.inspect').filter({ hasText: 'Example Browser' }).click();
}

test('runtime details lead with the running job and do not request an interpreter biography', async ({
    page,
}) => {
    await openBrowser(page, true);
    await expect(page.getByRole('heading', { name: 'Recognized job' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Hermes · Background gateway' })).toBeVisible();
    await expect(page.getByLabel('Origin overview')).toContainText('Hermes · Gateway log wrapper');
    await expect(page.locator('html')).not.toHaveAttribute('data-explanation-requests');
    await page.getByRole('button', { name: /Show origin evidence/ }).click();
    await expect(page.getByText('Running command: hermes gateway run')).toBeVisible();
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('fixture-sample')));
    await expect(page.getByText('Running command: hermes gateway run')).toBeVisible();
    await expect(page.locator('html')).not.toHaveAttribute('data-explanation-requests');
});

test('a memory prompt closes after recovery while its findings stay in Inbox', async ({ page }) => {
    await openBrowser(page);
    await expect(page.getByRole('region', { name: 'Process activity insight' })).toContainText(
        'Activity of PID'
    );
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
    await expect(prompt).toContainText('your computer is short on memory');
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

test('origin headings have deliberate spacing and the compact page fits a small window', async ({
    page,
}) => {
    await page.setViewportSize({ width: 760, height: 650 });
    await openBrowser(page);
    await page.getByRole('button', { name: 'Show origin evidence' }).click();
    const origin = page.getByRole('region', { name: 'Process origins' });
    await expect(origin).toBeFocused();
    const heading = origin.locator('.ancestor-heading').last();
    const name = await heading.locator('strong').boundingBox();
    const pid = await heading.locator('.pid').boundingBox();
    expect(pid!.x - name!.x - name!.width).toBeGreaterThanOrEqual(10);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        760
    );
});

test('the investigation keeps its latest answer readable and preserves keyboard-scrollable history', async ({
    page,
}) => {
    await openBrowser(page);
    await page.getByRole('button', { name: 'Ask a follow-up' }).click();
    const assistant = page.getByRole('complementary', { name: 'Contextual assistant' });
    const question = assistant.getByRole('textbox', { name: 'Ask Jitterbug' });
    await question.fill('Why is this app lagging?');
    await assistant.getByRole('button', { name: 'Ask ↑', exact: true }).click();
    await assistant.getByRole('button', { name: 'The whole window is stuck', exact: true }).click();
    await expect(assistant.locator('.insight-card')).toHaveCount(1);
    const history = assistant.locator('.conversation-history');
    await expect(history).not.toHaveAttribute('open');
    await history.locator('summary').click();
    await expect(history).toContainText('Why is this app lagging?');
    const scroll = assistant.getByRole('log', { name: 'Investigation conversation' });
    await expect(scroll).toHaveCSS('scrollbar-width', 'none');
    await scroll.focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('End');
    await expect.poll(() => scroll.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await expect(assistant.getByRole('textbox')).toBeVisible();
});
