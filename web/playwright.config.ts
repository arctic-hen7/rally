import { defineConfig } from '@playwright/test';
export default defineConfig({
    testDir: './tests',
    timeout: 45000,
    workers: 1,
    reporter: 'list',
    use: {
        baseURL: process.env.RALLY_URL || 'http://127.0.0.1:8787',
        launchOptions: { executablePath: process.env.CHROMIUM_PATH },
        viewport: { width: 1360, height: 850 },
        trace: 'retain-on-failure',
    },
});
