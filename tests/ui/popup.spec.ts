import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 360, height: 300 } });

test('action popup fits its window, plays the example, and hides the result', async ({ page }) => {
    await page.clock.install();
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup).toContainText('Hey, sample-web-app has stayed busy.');
    await expect(popup).toContainText('EXAMPLE');
    await expect(popup).toContainText('+1 more');
    // Current use, then each option with what it would leave you with.
    await expect(popup).toContainText(
        /Now: sample-web-app uses [\d.]+% CPU · [\d.]+ \w+ · \d+ processes/
    );
    await expect(popup).toContainText('After: same usage; your other apps go first');
    await expect(popup).toContainText(/Quit sample-web-app\s*After: frees about [\d.]+% CPU/);
    await expect(popup).toContainText('Every sample-web-app window closes');
    await expect(popup).toContainText('Example only. Nothing will be applied.');
    await expect(popup).not.toContainText('/example');
    await expect(popup.getByRole('button', { name: 'Details' })).toBeDisabled();
    for (const name of ['Apply', 'Quit…', 'Always lower for this app', 'Later']) {
        const box = await popup.getByRole('button', { name, exact: true }).boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x + box!.width).toBeLessThanOrEqual(360);
        expect(box!.y + box!.height).toBeLessThanOrEqual(300);
    }
    expect(await popup.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(
        true
    );
    await popup.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(popup).toContainText('Applying the priority change…');
    await page.clock.fastForward(1000);
    await expect(popup).toContainText('Checking the result…');
    await page.clock.fastForward(1500);
    // The result says what changed, what it means, and what was measured.
    await expect(popup.getByRole('heading')).toContainText('Lowered');
    await expect(popup).toContainText('keeps running');
    await expect(popup).toContainText(/→ [\d.]+% · whole Mac CPU [\d.]+% → [\d.]+%/);
    await expect(popup).toContainText('doesn’t cap CPU');
    expect(await popup.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(
        true
    );
    // Pointing at a finished result keeps it open; it hides 12 s after the pointer leaves.
    await page.clock.fastForward(13000);
    await expect(popup).toBeVisible();
    await popup.dispatchEvent('pointerleave');
    await page.clock.fastForward(12000);
    await expect(popup).toHaveCount(0);
});

test('Later and Escape close the popup without applying anything', async ({ page }) => {
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await popup.getByRole('button', { name: 'Later', exact: true }).click();
    await expect(popup).toHaveCount(0);
    await page.reload();
    await expect(popup).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(popup).toHaveCount(0);
});

test('action popup respects reduced motion', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup).toHaveCSS('animation-name', 'none');
});

test('quitting needs a second click and reports before and after', async ({ page }) => {
    await page.clock.install();
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await popup.getByRole('button', { name: 'Quit…', exact: true }).click();
    // The first click only arms the button; nothing has started.
    await expect(popup.getByRole('button', { name: 'Confirm', exact: true })).toBeVisible();
    await expect(popup).not.toContainText('Quitting');
    await page.clock.fastForward(5500);
    await expect(popup.getByRole('button', { name: 'Quit…', exact: true })).toBeVisible();
    await popup.getByRole('button', { name: 'Quit…', exact: true }).click();
    await popup.getByRole('button', { name: 'Confirm', exact: true }).click();
    await expect(popup).toContainText('Quitting sample-web-app…');
    await page.clock.fastForward(1500);
    await expect(popup.getByRole('heading')).toContainText('Closed sample-web-app');
    await expect(popup).toContainText(
        /whole Mac CPU [\d.]+% → [\d.]+% · free memory [\d.]+ GiB → [\d.]+ GiB/
    );
    expect(await popup.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(
        true
    );
});

test('a tab offer closes only that tab and a session offer pauses without closing', async ({
    page,
}) => {
    await page.clock.install();
    await page.goto('/?view=popup&example=tab');
    let popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup).toContainText('Close the “Video call” tab');
    await expect(popup).toContainText('The rest of Example Browser stays open.');
    await popup.getByRole('button', { name: 'Close…', exact: true }).click();
    await popup.getByRole('button', { name: 'Confirm', exact: true }).click();
    await page.clock.runFor(4000);
    await expect(popup).toContainText('Closed the “Video call” tab');
    await page.goto('/?view=popup&example=pause');
    popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup).toContainText('Pause claude · feat-search in Example Editor');
    await expect(popup).toContainText('no CPU until resumed; memory stays allocated');
    await expect(popup).toContainText('Nothing is closed.');
    await expect(popup.getByRole('button', { name: 'Pause…', exact: true })).toBeVisible();
});
