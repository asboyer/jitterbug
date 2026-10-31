import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 360, height: 300 } });

test('action popup fits its window, plays the example, and hides the result', async ({ page }) => {
    await page.clock.install();
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup.getByRole('heading')).toHaveText('sample-web-app has stayed busy');
    await expect(popup).toContainText('EXAMPLE');
    await expect(popup).toContainText('+1 more');
    // Current use, then each option as its consequence.
    await expect(popup).toContainText(/Now: [\d.]+% CPU · [\d.]+ \w+(?! ·)/);
    await expect(popup).not.toContainText('processes');
    await expect(popup).toContainText('Move sample-web-app to Background');
    await expect(popup).toContainText('After: keeps running; other apps get CPU and disk first');
    await expect(popup).toContainText(/Quit sample-web-app\s*After: frees about [\d.]+% CPU/);
    await expect(popup).toContainText('Every window closes; unsaved work is lost.');
    // A permanent rule is never one click away in the popup.
    await expect(popup).not.toContainText('Always lower');
    await expect(popup).toContainText('Example only. Nothing will be applied.');
    await expect(popup).not.toContainText('/example');
    await expect(popup.getByRole('button', { name: 'Details' })).toBeDisabled();
    for (const name of ['Apply', 'Quit…', 'Later']) {
        const box = await popup.getByRole('button', { name, exact: true }).boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x + box!.width).toBeLessThanOrEqual(360);
        expect(box!.y + box!.height).toBeLessThanOrEqual(300);
    }
    expect(await popup.evaluate((element) => element.scrollHeight <= element.clientHeight)).toBe(
        true
    );
    await popup.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(popup).toContainText('Moving sample-web-app to Background…');
    await page.clock.fastForward(1000);
    await expect(popup).toContainText('Checking the result…');
    await page.clock.fastForward(1500);
    // The result says what changed, what it means, and what was measured.
    await expect(popup.getByRole('heading')).toContainText(
        'Moved sample-web-app to Background · Undo'
    );
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

test('moving an app to Background can be undone from the result', async ({ page }) => {
    await page.clock.install();
    await page.goto('/?view=popup');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await popup.getByRole('button', { name: 'Apply', exact: true }).click();
    await page.clock.fastForward(2500);
    await popup.getByRole('button', { name: 'Undo', exact: true }).click();
    await expect(popup.getByRole('heading')).toHaveText('Moved sample-web-app back to Normal');
    await expect(popup).toContainText('competes for CPU and disk as before');
    await expect(popup.getByRole('button', { name: 'Undo', exact: true })).toHaveCount(0);
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

test('an agent request names the agent, quotes its reason, and still needs a click', async ({
    page,
}) => {
    await page.goto('/?view=popup&example=agent');
    const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
    await expect(popup).toContainText(
        'An agent in claude · fix-login asks to close the tab “Video call”'
    );
    await expect(popup).toContainText('Reason given: “The video call tab uses 640 MiB');
    await expect(popup).not.toContainText('Apply');
    await expect(popup.getByRole('button', { name: 'Close…', exact: true })).toBeVisible();
});

for (const example of ['', 'tab', 'pause', 'agent']) {
    test(`every button of the ${example || 'default'} example stays inside the window`, async ({
        page,
    }) => {
        await page.goto(example ? `/?view=popup&example=${example}` : '/?view=popup');
        const popup = page.getByRole('region', { name: 'Jitterbug needs you' });
        await expect(popup).toBeVisible();
        for (const button of await popup.getByRole('button').all()) {
            const box = await button.boundingBox();
            expect(box).not.toBeNull();
            expect(box!.y + box!.height).toBeLessThanOrEqual(300);
        }
        // Nothing is clipped: neither the popup nor its options need to scroll.
        for (const element of [popup, popup.getByRole('list')]) {
            expect(await element.evaluate((node) => node.scrollHeight <= node.clientHeight)).toBe(
                true
            );
        }
    });
}
