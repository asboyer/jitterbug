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
    },
    webServer: {
        command: `npm run dev -- --port ${port}`,
        url: baseURL,
        reuseExistingServer: !process.env.CI,
    },
});
