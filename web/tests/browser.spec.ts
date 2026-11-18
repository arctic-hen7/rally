import { test, expect } from '@playwright/test';

const email = process.env.RALLY_TEST_EMAIL || 'morgan@rally.test';
const password = process.env.RALLY_TEST_PASSWORD || 'rally-local-test-2026';
const title = 'Plan the release checklist';
let mission = '';
test.beforeAll(async ({ request }) => {
    let login = await request.post('/auth/login', { data: { email, password } });
    if (!login.ok() && process.env.RALLY_BOOTSTRAP_SECRET) {
        login = await request.post('/auth/bootstrap', {
            data: { email, password, name: 'Morgan Lee', code: process.env.RALLY_BOOTSTRAP_SECRET },
        });
    }
    expect(login.ok()).toBeTruthy();
    const topic = 'qa' + Date.now();
    expect(
        (
            await request.post('/api/topic.create', { data: { name: 'Release engineering', slug: topic } })
        ).ok(),
    ).toBeTruthy();
    const m = await request.post('/api/mission.create', {
        data: {
            title,
            topic,
            description: 'Coordinate the rollout and record the reasoning behind release decisions.',
        },
    });
    mission = (await m.json()).id;
    for (const body of [
        'Release candidates should carry a link to the verification run.',
        'The rollout checklist belongs in the repository so that changes can be reviewed alongside the implementation.',
    ]) {
        expect((await request.post('/api/post', { data: { thread: mission, body } })).ok()).toBeTruthy();
    }
});
test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.getByLabel('Email', { exact: true }).fill(email);
    await page.getByLabel('Password', { exact: true }).fill(password);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Missions', exact: true })).toBeVisible();
});
test('desktop mission, messaging, search and thread controls', async ({ page }, info) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto('/#mission/' + mission);
    await expect(page.getByRole('heading', { name: title })).toBeVisible();
    await page
        .getByRole('textbox', { name: 'Message', exact: true })
        .fill('Preserve a searchable handoff for the release captain.');
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    await expect(
        page.getByText('Preserve a searchable handoff for the release captain.', { exact: true }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Mission details', exact: true }).click();
    await expect(page.getByText('Local workspace', { exact: true })).toBeVisible();
    await page.screenshot({ path: info.outputPath('mission-desktop.png') });
    await page.getByRole('button', { name: 'Search missions', exact: true }).click();
    await page.getByPlaceholder('Search decisions, messages, or missions…').fill('release captain');
    await expect(
        page
            .getByRole('dialog')
            .getByText('Preserve a searchable handoff for the release captain.', { exact: true })
            .first(),
    ).toBeVisible();
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Main', exact: true }).click();
    await page.getByRole('button', { name: 'New thread', exact: true }).click();
    await page.getByLabel('Thread title').fill('Release notes');
    await page.getByRole('button', { name: 'Create thread' }).click();
    await expect(page.getByRole('button', { name: 'Release notes', exact: true })).toBeVisible();
    expect(errors).toEqual([]);
});
test('mobile navigation, mission shell and PWA manifest', async ({ page }, info) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/#mission/' + mission);
    await expect(page.getByRole('heading', { name: title })).toBeVisible();
    await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: info.outputPath('mission-mobile.png') });
    await page.getByRole('button', { name: 'Open navigation' }).click();
    await page.getByRole('navigation').getByRole('button', { name: 'Agents', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
    expect((await (await page.request.get('/manifest.webmanifest')).json()).display).toBe('standalone');
});

test('agent pane streams the run, accepts a queued answer and steers the same invocation', async ({
    page,
}, info) => {
    test.skip(process.env.RALLY_TEST_HARNESS !== '1', 'Requires the deterministic hosted test adapter');
    await page.goto('/#mission/' + mission);
    const message = page.getByRole('textbox', { name: 'Message', exact: true });
    await message.fill('@engineer HOLD QUESTION');
    await page.getByRole('button', { name: 'Approve', exact: true }).click();
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const pane = page.locator('.side-pane.working');
    await expect(pane).toBeVisible();
    await expect(pane.getByText('Which scope?', { exact: false }).first()).toBeVisible();
    await pane.getByRole('button', { name: 'Small', exact: true }).click();
    await pane.getByRole('button', { name: 'Send answer', exact: true }).click();
    await expect(pane.getByText('Answered', { exact: true })).toBeVisible();
    await message.fill('An ordinary message in an active 1:1');
    await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeDisabled();
    await pane.getByPlaceholder('Steer @engineer…').fill('FINISH this task');
    await pane.getByRole('button', { name: 'Steer', exact: true }).click();
    await expect(
        page.locator('.timeline').getByText('Completed after steering.', { exact: true }),
    ).toBeVisible();
    await page.screenshot({ path: info.outputPath('agent-pane.png') });
});
