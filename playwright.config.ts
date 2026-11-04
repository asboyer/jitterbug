import { defineConfig } from '@playwright/test';

const port = process.env.JITTERBUG_UI_PORT ?? '1420';
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
    testDir: './tests/ui',
    fullyParallel: false,
    workers: 1,
    timeout: 30_000,
    use: {
        baseURL,
        viewport: { width: 1120, height: 760 },
        colorScheme: 'dark',
        trace: 'retain-on-failure',
        // Specs that mock the desktop app start as returning users; onboarding has its own spec.
        storageState: {
            cookies: [],
            origins: [
                {
                    origin: baseURL,
                    localStorage: [{ name: 'jitterbug.onboarding.v1.done', value: '1' }],
                },
            ],
        },
    },
    webServer: {
        command: `npm run dev -- --port ${port}`,
        url: baseURL,
        reuseExistingServer: !process.env.CI,
    },
});
