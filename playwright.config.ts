import { defineConfig } from '@playwright/test';

export default defineConfig({
    testDir: './tests/ui',
    fullyParallel: false,
    workers: 1,
    timeout: 30_000,
    use: {
        baseURL: 'http://127.0.0.1:1420',
        viewport: { width: 1120, height: 760 },
        colorScheme: 'dark',
        trace: 'retain-on-failure',
    },
    webServer: {
        command: 'npm run dev',
        url: 'http://127.0.0.1:1420',
        reuseExistingServer: !process.env.CI,
    },
});
