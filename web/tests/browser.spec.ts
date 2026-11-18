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
    await expect(page.getByRole('heading', { name: 'Missions', exact: true })).toBeVisible({
        timeout: 20000,
    });
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

test('installed service worker displays a background agent notification', async ({ page, context }) => {
    await context.grantPermissions(['notifications']);
    const origin = new URL(page.url()).origin;
    const cdp = await context.newCDPSession(page);
    await page.waitForFunction(() => navigator.serviceWorker.controller?.state === 'activated');
    const registration = new Promise<string>((resolve) => {
        cdp.on('ServiceWorker.workerVersionUpdated', ({ versions }) => {
            const current = versions.find(
                (v) => v.scriptURL === origin + '/sw.js' && v.status === 'activated',
            );
            if (current) resolve(current.registrationId);
        });
    });
    await cdp.send('ServiceWorker.enable');
    await cdp.send('ServiceWorker.deliverPushMessage', {
        origin,
        registrationId: await registration,
        data: JSON.stringify({ title: 'Rally', body: 'Your agent has a question', thread: mission }),
    });
    await expect
        .poll(() =>
            page.evaluate(async () => {
                const reg = await navigator.serviceWorker.ready;
                const items = await reg.getNotifications();
                return items.map((n) => n.body);
            }),
        )
        .toContain('Your agent has a question');
    await cdp.detach();
});

test('skill authoring page supports browser back, Markdown preview and editing', async ({ page }, info) => {
    await page.getByRole('navigation').getByRole('button', { name: 'Skills' }).click();
    await page.getByRole('button', { name: 'New skill', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'New skill' })).toBeVisible();
    await page.goBack();
    await expect(page.getByRole('heading', { name: 'Skills', exact: true })).toBeVisible();
    await page.goForward();
    await page.getByLabel('Skill name').fill('decision-journal');
    await page.getByLabel('Description', { exact: true }).fill('Capture the reasons behind a decision.');
    await page
        .getByRole('textbox', { name: 'Instructions' })
        .fill('## Decisions\nRecord the alternatives considered.');
    await page.getByRole('button', { name: 'Preview', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Decisions' })).toBeVisible();
    await page.screenshot({ path: info.outputPath('skill-editor.png') });
    await page.getByRole('button', { name: 'Create skill', exact: true }).click();
    await page.getByRole('button', { name: /decision-journal/ }).click();
    await page.getByRole('button', { name: 'Edit skill', exact: true }).click();
    await expect(page.getByLabel('Description', { exact: true })).toHaveValue(
        'Capture the reasons behind a decision.',
    );
    await expect(page.getByRole('textbox', { name: 'Instructions' })).toContainText('alternatives');
});

test('sidebar thread navigation, status changes and full-height footer hover', async ({ page }) => {
    expect(
        (
            await page.request.post('/api/thread.create', { data: { mission, title: 'Sidebar test thread' } })
        ).ok(),
    ).toBeTruthy();
    expect(
        (
            await page.request.post('/api/capture', {
                data: {
                    mission,
                    title: 'Local review',
                    source: 'codex',
                    session: 'sidebar-session',
                    role: 'user',
                    key: 'initial',
                    body: 'Context from the local checkout.',
                },
            })
        ).ok(),
    ).toBeTruthy();
    const rename = await page.request.post('/api/thread.rename', {
        data: { thread: mission, title: 'Release planning' },
    });
    expect(rename.ok()).toBeTruthy();
    await page.goto('/#mission/' + mission);
    await expect(page.getByRole('heading', { name: title })).toBeVisible();
    await expect(page.locator('.thread-bar .tab.active')).toHaveAccessibleName('Release planning');
    await expect(page.locator('.thread-bar .tab.active .default-star')).toBeVisible();
    await expect(page.locator('.thread-bar .tab.active')).toHaveAttribute(
        'aria-description',
        'Default thread',
    );
    const tree = page.locator('.mission-tree').filter({ has: page.locator('button[aria-current="page"]') });
    await tree.getByRole('button', { name: 'Show conversations in ' + title }).click();
    await tree.getByRole('button', { name: 'Sidebar test thread', exact: true }).click();
    await expect(page.locator('.thread-bar .tab.active')).toHaveText('Sidebar test thread');
    await tree.locator('summary').filter({ hasText: 'External sessions' }).click();
    await tree.getByRole('button', { name: 'Local review', exact: true }).click();
    await expect(page.locator('.timeline')).toContainText('Context from the local checkout.');
    await page.getByLabel('Mission status').selectOption('in-progress');
    await expect(page.getByLabel('Mission status')).toHaveValue('in-progress');
    const profile = page.locator('button.profile');
    await profile.hover();
    const bottom = await profile.evaluate((e) => ({
        row: e.getBoundingClientRect().bottom,
        sidebar: e.closest('aside')!.getBoundingClientRect().bottom,
    }));
    expect(Math.abs(bottom.row - bottom.sidebar)).toBeLessThan(1);
});

test('workspace settings save a status color and the new agent page supplies its model', async ({ page }) => {
    await page.getByRole('navigation').getByRole('button', { name: 'Agents' }).click();
    await page.getByRole('button', { name: 'New agent', exact: true }).click();
    await expect(page.getByLabel('Model', { exact: true })).toHaveValue('gpt-6.2-sol');
    await page.getByRole('button', { name: 'Cancel' }).click();
    await page.locator('button.profile').click();
    await page.getByRole('button', { name: 'New status' }).click();
    await page.getByLabel('Status name').fill('Ready to publish');
    await page.getByRole('radio', { name: 'Rose' }).check();
    await page.getByRole('button', { name: 'Create status' }).click();
    await expect(
        page.locator('.setting-line').filter({ hasText: 'Ready to publish' }).locator('.status-dot.rose'),
    ).toBeVisible();
});
