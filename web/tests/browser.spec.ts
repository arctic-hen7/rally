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
    await expect(page.locator('.thread-bar .tab.active')).toHaveAccessibleName('Release notes');
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

test('installed service worker displays a background agent notification', async ({ playwright, baseURL }) => {
    // Use a persistent profile and Chromium's notification backend on headless CI
    const context = await playwright.chromium.launchPersistentContext('', {
        channel: 'chromium',
        executablePath: process.env.CHROMIUM_PATH,
        args: ['--disable-features=NativeNotifications'],
        permissions: ['notifications'],
    });
    try {
        const origin = new URL(baseURL!).origin;
        const page = context.pages()[0];
        await page.goto(origin);
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
        const registrationId = await registration;
        // Observe delivery even when a headless host immediately dismisses the notification
        const displayed: { tag: string; metadata: Record<string, string> }[] = [];
        cdp.on('BackgroundService.backgroundServiceEventReceived', ({ backgroundServiceEvent: event }) => {
            if (
                event.eventName === 'Notification displayed' &&
                event.serviceWorkerRegistrationId === registrationId
            ) {
                displayed.push({
                    tag: event.instanceId,
                    metadata: Object.fromEntries(event.eventMetadata.map(({ key, value }) => [key, value])),
                });
            }
        });
        await cdp.send('BackgroundService.startObserving', { service: 'notifications' });
        await cdp.send('BackgroundService.setRecording', { service: 'notifications', shouldRecord: true });
        await cdp.send('ServiceWorker.deliverPushMessage', {
            origin,
            registrationId,
            data: JSON.stringify({ title: 'Rally', body: 'Your agent has a question', thread: mission }),
        });
        await expect
            .poll(() => displayed)
            .toContainEqual({
                tag: mission,
                metadata: expect.objectContaining({ Title: 'Rally', Body: 'Your agent has a question' }),
            });
    } finally {
        await context.close();
    }
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

test('webhook senders and the multi-trigger routine editor', async ({ page }, info) => {
    await page.getByRole('navigation').getByRole('button', { name: 'Routines' }).click();
    await page.getByRole('button', { name: 'New webhook', exact: true }).click();
    await page.getByLabel('Webhook name').fill('Pull request updates');
    await page.getByRole('button', { name: 'Create webhook', exact: true }).click();
    await page.getByRole('button', { name: 'Pull request updates', exact: true }).click();
    await page.getByRole('button', { name: 'Generate token' }).click();
    await page.getByLabel('Sender name').fill('Test service');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.locator('.editor-page code.secret')).toHaveText(/[0-9a-f]{64}/);
    await page.getByRole('button', { name: 'Back to Routines' }).click();
    await page.getByRole('button', { name: 'New routine', exact: true }).click();
    await page.getByLabel('Routine name').fill('Triage updates');
    await page.getByLabel('Shell command').fill('true');
    await page.getByRole('button', { name: 'Add repository' }).click();
    await page.getByLabel('Git remote').fill('https://rally.invalid/team/scripts.git');
    await page.getByLabel('Checkout directory').fill('scripts');
    await page.getByRole('button', { name: 'Schedule', exact: true }).click();
    await page.getByRole('combobox', { name: 'Repeat', exact: true }).selectOption('Weekdays');
    await page.getByLabel('Time zone').fill('Australia/Sydney');
    await page.getByRole('button', { name: 'Webhook', exact: true }).click();
    await page.getByLabel('Endpoint').selectOption({ label: 'Pull request updates' });
    await expect(page.locator('.editor-page .trigger-card small')).toContainText('MON-FRI');
    await page.screenshot({ path: info.outputPath('routine-triggers.png') });
    await page.getByRole('button', { name: 'Create routine', exact: true }).click();
    await page.getByRole('button', { name: /Triage updates/ }).click();
    await expect(page.locator('.pane-content')).toContainText('2 triggers');
});

test('private skills, credential overrides and organisation model settings', async ({ page }) => {
    await page.getByRole('navigation').getByRole('button', { name: 'Skills' }).click();
    await page.getByRole('button', { name: 'New skill', exact: true }).click();
    await page.getByLabel('Skill name').fill('design-library');
    await page.getByLabel('Description', { exact: true }).fill('Shared design work');
    await page.getByRole('textbox', { name: 'Instructions' }).fill('Read the team guidance.');
    await page.getByRole('button', { name: 'Create skill', exact: true }).click();
    await page.getByRole('button', { name: /design-library/ }).click();
    await page.getByRole('button', { name: 'Manage secrets' }).click();
    await page.getByRole('button', { name: 'Add secret' }).click();
    await page.getByLabel('Environment variable').fill('DESIGN_LIBRARY');
    await page.getByLabel('Secret value').fill('synthetic-test');
    await page.getByRole('button', { name: 'Save secret' }).click();
    await page.getByRole('button', { name: 'Make private', exact: true }).click();
    await expect(page.getByLabel('Make this skill private')).toBeChecked();
    await page.getByRole('button', { name: 'Attach to agent or mission' }).click();
    await page.getByLabel('Agent or mission').selectOption(mission);
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.locator('.pane-content')).toContainText(mission);
    await page.goto('/#mission/' + mission);
    await page.getByRole('button', { name: 'Mission details', exact: true }).click();
    await page.getByRole('button', { name: 'Manage secrets' }).click();
    await expect(page.locator('.secret-row').filter({ hasText: 'DESIGN_LIBRARY' })).toContainText(
        'design-library',
    );
    await page
        .locator('.secret-row')
        .filter({ hasText: 'DESIGN_LIBRARY' })
        .getByRole('button', { name: 'Override' })
        .click();
    await page.getByLabel('Secret value').fill('mission-test');
    await page.getByRole('button', { name: 'Save secret' }).click();
    await expect(page.locator('.secret-row').filter({ hasText: 'DESIGN_LIBRARY' })).toContainText(
        'Overrides design-library',
    );
    await page.locator('button.profile').click();
    await page.getByLabel('WIP summary model').fill('test-summary');
    await page.getByRole('button', { name: 'Save model' }).click();
    await expect(page.getByRole('status')).toContainText('Saved');
});

test('delegate a personal Git connection to a repository and withdraw it', async ({ page }, info) => {
    const origin = process.env.RALLY_TEST_GIT_ORIGIN;
    test.skip(!origin, 'Requires the disposable HTTPS forge');
    const name = 'Browser Git account';
    await page.goto('/#Settings');
    await page.getByRole('button', { name: 'New connection' }).click();
    await page.getByLabel('Connection name').fill(name);
    await page.getByLabel('HTTPS forge host').fill(origin!);
    await page.getByLabel('Git username').fill('alice');
    await page.getByLabel('Access token').fill('alice-fixture');
    await page.getByRole('button', { name: 'Save connection' }).click();
    await expect(page.getByRole('button', { name: new RegExp(name) })).toBeVisible();
    await page.goto('/#mission/' + mission);
    await page.getByRole('button', { name: 'Mission details', exact: true }).click();
    await page.getByRole('button', { name: 'Add repository' }).click();
    await page.getByLabel('Checkout directory').fill('browser');
    await page.getByLabel('Git remote').fill(origin + '/project.git');
    await page.getByLabel('Git connection').selectOption({ label: name + ' · alice' });
    await page.getByRole('checkbox', { name: /Allow current and future/ }).check();
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByText(/Contributed by Morgan Lee/)).toBeVisible();
    await page.getByRole('button', { name: 'Withdraw my access' }).click();
    await expect(page.getByText('Hosted Git access paused for')).toBeVisible();
    await page.screenshot({ path: info.outputPath('git-transfer.png') });
    await page.getByRole('button', { name: 'Take over', exact: true }).first().click();
    await page.getByLabel('Git connection').selectOption({ label: name + ' · alice' });
    await page.getByRole('checkbox', { name: /Allow current and future/ }).check();
    await page.getByRole('button', { name: 'Use my Git access' }).click();
    await expect(page.getByRole('button', { name: 'Withdraw my access' })).toBeVisible();
});

test('private repository URL suggestions respect manual edits and requests can be attached read-only', async ({
    page,
}) => {
    test.skip(!process.env.RALLY_TEST_GIT_ORIGIN || !process.env.RALLY_TEST_HARNESS);
    const origin = process.env.RALLY_TEST_GIT_ORIGIN!;
    await page.goto('/#Settings');
    await page.getByRole('button', { name: 'Add URL', exact: true }).click();
    const source = page.getByLabel('Source Git URL');
    const destination = page.getByLabel('HTTPS destination');
    const scope = page.getByRole('combobox', { name: 'Match', exact: true });
    await source.fill('git@git.route.test:team/component.git');
    await source.press('Tab');
    await expect(destination).toHaveValue('https://git.route.test/team/component.git');
    await expect(scope).toHaveValue('repository');
    await destination.fill('');
    await source.fill('git@git.route.test:team/');
    await source.press('Tab');
    await expect(destination).toHaveValue('');
    await expect(scope).toHaveValue('prefix');
    await destination.fill(origin);
    await source.fill('git@git.route.test:team');
    await source.press('Tab');
    await expect(destination).toHaveValue(origin);
    await page.getByRole('button', { name: 'Save rule' }).click();
    await expect(page.getByText('ssh://git@git.route.test/team', { exact: true })).toBeVisible();

    const agent = await page.request.post('/api/agent.create', {
        data: { name: 'route-ui-' + Date.now(), model: 'fixture' },
    });
    const posted = await page.request.post('/api/post', {
        data: {
            thread: mission,
            body: 'ROUTING_FIXTURE ' + JSON.stringify({ mode: 'missing', origin }),
            mentions: [(await agent.json()).id],
            confirm: true,
        },
    });
    expect(posted.ok()).toBeTruthy();
    await page.goto('/#mission/' + mission);
    await expect(page.getByText('Git read access requested for', { exact: false })).toBeVisible();
    await page.getByLabel('About this repository request').click();
    await expect(
        page.getByText(
            'The URL of this repo matches an organisation routing rule, which may include public repos. Adding it will attach it read-only to this mission so Git can check it out using your credentials. Others in the mission will be able to implicitly read this repo, but not change it unless you add it fully to the mission as a workspace repo.',
        ),
    ).toBeVisible();
    await page.getByLabel('About this repository request').click();
    await page.getByRole('button', { name: 'Add to mission (read-only)' }).click();
    await page.getByLabel('Git connection').selectOption({ index: 1 });
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: 'Add to mission (read-only)' }).click();
    await expect(page.getByRole('button', { name: 'Add to mission (read-only)' })).toHaveCount(0);
    const result = await page.request.post('/api/mission.show', { data: { mission } });
    expect((await result.json()).read_only_repos[0].remote).toBe(origin + '/dependency.git');
});
