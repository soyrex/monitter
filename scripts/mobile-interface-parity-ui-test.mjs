import { chromium, webkit, expect as baseExpect } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';

// Start the development server separately; this script never launches providers.
const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18449/';
const expect = baseExpect.configure({ timeout: 30000 });
const engine = process.env.MONITTER_TEST_ENGINE === 'chromium' ? chromium : webkit;
const browser = await engine.launch({ headless: true, ...(engine === chromium && process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}) });
mkdirSync('verification', { recursive: true });
const makeTask = (id, projectId = 'project-a') => ({ id, agentId: 'atlas', title: id === 'watched' ? 'Parity task' : 'Other task', status: 'running', nativeSessionId: `session-${id}`, archived: false, createdAt: 10, updatedAt: 20, parentTaskId: null, channelId: null, projectId, hostId: 'local', cwd: '/tmp/parity-fixture', provider: 'codex', model: 'fixture-model', modelSettings: null, sandbox: 'read-only' });
const makeRequest = (id, overrides = {}) => ({ id, projectId: 'project-a', taskId: 'watched', agentId: 'atlas', hostId: 'local', requestId: id, title: id, command: 'fixture only', cwd: '/tmp/parity-fixture', targetKey: 'fixture', status: 'ready', buildRunId: null, createdAt: 20, updatedAt: 20, ...overrides });
function fixture(base, theme) {
  return { ...base, goals: { watched: { objective: 'Bring the mobile interface up to date', status: 'active', tokensUsed: 100, tokenBudget: 1000, timeUsedSeconds: 65 } }, tasks: [makeTask('watched'), makeTask('other', 'project-b')],
    projects: ['a', 'b'].map(id => ({ id: `project-${id}`, name: `Project ${id.toUpperCase()}`, color: '#397e61', description: '', instructions: '', workspaces: [{ hostId: 'local', cwd: '/tmp/parity-fixture' }], buildManagementEnabled: true, ideas: [], ideasRevision: 0, createdAt: 1, updatedAt: 1 })),
    messages: [{ id: 'request', taskId: 'watched', role: 'user', text: 'Implement the current parity request', createdAt: 20 }],
    events: [{ id: 'progress', taskId: 'watched', kind: 'agent_progress', title: 'Checking the current request', detail: '', createdAt: 21 }],
    workPlans: [{ id: 'closed', taskId: 'watched', requestId: 'old', title: 'Previous plan', status: 'closed', items: [{ id: 'done', title: 'Earlier investigation', status: 'completed', note: null, updatedAt: 15 }], summary: 'Investigation complete', createdAt: 10, updatedAt: 15, closedAt: 15 }],
    buildRequests: [makeRequest('Foreign build', { taskId: 'other', projectId: 'project-b' })], buildRuns: [],
    settings: { ...base.settings, interfaceScale: 100, theme, busyMessageMode: 'queue', sendWithEnter: false },
  };
}
async function patch(page, update) {
  await page.evaluate(update => { const qa = window.__MONITTER_QA__; qa.setSnapshot({ ...qa.snapshot(), ...update }); }, update);
}
async function open(width, theme) {
  const page = await browser.newPage({ viewport: { width, height: width === 320 ? 640 : width === 390 ? 844 : 900 }, isMobile: width < 600, hasTouch: width < 600 });
  await page.clock.install();
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  const errors = [];
  let polyfillLoads = 0;
  page.on('pageerror', error => errors.push(error.message));
  if (process.env.MONITTER_TEST_MISSING_APIS === '1') {
    // toSorted is used during real AppSurface startup but is absent in some
    // browsers within plugin-legacy's modern target range (e.g. Chromium 105).
    await page.addInitScript(() => {
      delete Array.prototype.toSorted;
      window.__MONITTER_MISSING_API__ = typeof Array.prototype.toSorted;
    });
    await page.route(/\/polyfills\.[\w-]+\.js$/, async route => {
      polyfillLoads++;
      const response = await route.fetch();
      await page.waitForTimeout(100);
      expect(await page.evaluate(() => typeof Array.prototype.toSorted)).toBe('undefined');
      await expect(page.locator('.app-shell')).toHaveCount(0);
      await route.fulfill({ response });
    });
  }
  await page.addInitScript(readFileSync('scripts/ui-fixture.js', 'utf8'));
  await page.addInitScript(() => {
    // Route through the production getBridge test adapter, recording wire names.
    const direct = window.__MONITTER_BRIDGE__;
    const qa = window.__MONITTER_QA__;
    window.__MONITTER_BRIDGE__ = undefined;
    window.__MONITTER_TEST_BRIDGE__ = {
      invoke: async (method, args = {}) => {
        qa.calls.push({ method, args });
        if (method === 'get_task_goal' && qa.snapshot().goalReadError) throw new Error('Fixture goal temporarily unavailable');
        if (method === 'get_snapshot') return qa.snapshot();
        if (method === 'get_task_events') return { taskId: args.taskId, events: qa.snapshot().events.filter(event => event.taskId === args.taskId), hasMore: false, nextBefore: null };
        if (method === 'send_message') return direct.sendMessage(args.taskId, args.text, args.attachmentIds);
        if (method === 'save_project_ideas') {
          const state = qa.snapshot();
          const project = state.projects.find(project => project.id === args.projectId);
          if (project.ideasRevision !== args.expectedRevision) throw new Error('The Idea List changed elsewhere. Refresh before saving.');
          project.ideas = args.ideas;
          project.ideasRevision += 1;
          qa.setSnapshot(state);
          return state;
        }
        if (method === 'resolve_input') {
          const state = qa.snapshot();
          state.approvalRequests = state.approvalRequests.map(request => request.id === args.approvalId ? { ...request, status: 'approved', response: args.response } : request);
          qa.setSnapshot(state);
          return state;
        }
        const camel = method.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase());
        if (typeof direct[camel] === 'function') return direct[camel](...Object.values(args));
        if (method === 'get_task_git_status') return { repository: false, files: [], truncated: false };
        throw new Error(`Unimplemented parity fixture command: ${method}`);
      },
      listen: async (_event, handler) => direct.onChanged(handler),
    };
  });
  await page.route('**/api/access', route => route.fulfill({ json: { required: false } }));
  await page.route(url, async route => { const response = await route.fetch(); await route.fulfill({ response, body: (await response.text()).replace('<html', '<html data-monitter-lan="1"') }); });
  await page.goto(url, { timeout: 90000 });
  await expect(page.locator('.app-shell').first()).toBeVisible({ timeout: 60000 });
  if (process.env.MONITTER_TEST_MISSING_APIS === '1') {
    expect(polyfillLoads).toBe(1);
    expect(await page.evaluate(() => window.__MONITTER_MISSING_API__)).toBe('undefined');
    expect(await page.evaluate(() => typeof Array.prototype.toSorted)).toBe('function');
  }
  const base = await page.evaluate(() => window.__MONITTER_QA__.snapshot());
  await page.evaluate(value => window.__MONITTER_QA__.setSnapshot(value), fixture(base, theme));
  try {
    if (width < 600 && await page.locator('.sidebar').evaluate(element => element.inert)) await page.getByRole('button', { name: 'Open chats menu', exact: true }).first().click();
    await page.getByRole('tab', { name: 'Activity view', exact: true }).click();
    await page.getByRole('button', { name: 'Open chat Parity task', exact: true }).click();
  } catch (error) {
    await page.screenshot({ path: `verification/mobile-interface-open-failure-${theme}-${width}.png` });
    console.error('Root fixture page errors:', errors, '\nVisible content:', (await page.locator('body').innerText()).slice(-7000));
    throw error;
  }
  return { page, errors };
}
async function checks(width, theme) {
  console.log(`Checking ${engine.name()} ${theme} ${width}px`);
  const { page, errors } = await open(width, theme);
  try {
    const composer = page.getByLabel('Task message', { exact: true });
    await composer.fill('Retained unsent draft');
    await page.getByRole('button', { name: 'Watch task', exact: true }).click();
    const watch = page.getByTestId('task-watch');
    await expect(watch).toBeVisible();
    const goalCard = watch.getByTestId('watch-goal');
    await expect(goalCard).toContainText('Active goal');
    await expect(goalCard).toContainText('Bring the mobile interface up to date');
    await expect(goalCard).toContainText('100 / 1,000 tokens used');
    await expect(goalCard).toContainText('1:05 elapsed');
    await expect(watch).toContainText('Implement the current parity request');
    await expect(watch).toContainText('Checking the current request');
    await expect(watch).not.toContainText('Foreign build');
    await expect(watch.getByRole('region', { name: 'Agent work plan' })).toBeHidden();
    await watch.locator('summary').filter({ hasText: 'Last plan' }).click();
    await expect(watch).toContainText('Investigation complete');
    await page.getByRole('button', { name: 'Open chat', exact: true }).first().click();
    await expect(composer).toHaveValue('Retained unsent draft');
    await page.getByRole('button', { name: 'Watch task', exact: true }).click();
    // Preserve the last goal on a read error; count completed steps truthfully.
    const closedPlans = await page.evaluate(() => window.__MONITTER_QA__.snapshot().workPlans);
    await patch(page, { workPlans: [...closedPlans, { id: 'current-plan', taskId: 'watched', requestId: 'current', title: 'Verify mobile Watch', status: 'active', createdAt: 30, updatedAt: 30, items: [{ id: 'one', title: 'Inventory', status: 'completed' }, { id: 'skip', title: 'Deferred', status: 'skipped' }, { id: 'next', title: 'Verify the real mobile surface', status: 'in_progress' }] }], goalReadError: true });
    await expect(goalCard).toContainText('1 / 3 steps complete');
    await expect(goalCard).toContainText('Verify the real mobile surface');
    await page.getByRole('button', { name: 'Open chat', exact: true }).first().click();
    await page.getByRole('button', { name: 'Watch task', exact: true }).click();
    await expect(goalCard).toContainText('Showing last known goal · update unavailable');
    await expect(goalCard).toContainText('Bring the mobile interface up to date');
    // An idle Watch discovers a goal assigned after its first read, without
    // turning resource consumption into an objective-completion percentage.
    const tasks = await page.evaluate(() => window.__MONITTER_QA__.snapshot().tasks);
    await patch(page, { goalReadError: false, workPlans: closedPlans, goals: {}, tasks: tasks.map(task => task.id === 'watched' ? { ...task, status: 'idle' } : task) });
    await expect(goalCard).toHaveCount(0);
    await patch(page, { goals: { watched: { objective: 'A newly assigned idle goal', status: 'blocked', tokensUsed: 0, timeUsedSeconds: 0 } } });
    await page.clock.fastForward(61000);
    await expect(goalCard).toContainText('Goal blocked');
    await expect(goalCard).toContainText('A newly assigned idle goal');
    await expect(goalCard).toContainText('0 tokens used');
    await expect(goalCard).toContainText('0:00 elapsed');
    await expect(goalCard).not.toContainText('%');
    await page.clock.setSystemTime(new Date());
    await patch(page, { tasks });
    await patch(page, { buildRequests: [makeRequest('Shared current build')], buildRuns: [{ id: 'combined', projectId: 'project-a', ownerTaskId: 'peer', hostId: 'local', requestIds: ['Shared current build'], command: 'fixture only', cwd: '/tmp/parity-fixture', targetKey: 'fixture', status: 'running', summary: '', exitCode: null, createdAt: 30, updatedAt: 30, leaseExpiresAt: Date.now() + 60000 }] });
    await expect(watch).toContainText('Building · Shared current build');
    const runs = await page.evaluate(() => window.__MONITTER_QA__.snapshot().buildRuns);
    await patch(page, { buildRuns: runs.map(run => ({ ...run, leaseExpiresAt: 1 })) });
    await expect(watch).toContainText('Build needs attention');
    await patch(page, { buildRuns: [], buildRequests: [makeRequest('Ready current build')] });
    await expect(watch).toContainText('Waiting for build claim');
    await patch(page, { buildRequests: [makeRequest('Planned current build', { status: 'planned' })] });
    await expect(watch).toContainText('Preparing build');
    // A real structured question must use its request response path, never chat send.
    await patch(page, { approvalRequests: [{ id: 'question-request', taskId: 'watched', provider: 'codex', runId: 'fixture-run', tool: 'ask_user', summary: 'Choose the verification target', detail: '', risk: 'low', status: 'pending', createdAt: 35, resolvedAt: null, decision: null, input: { kind: 'questions', schema: null, url: null, questions: [{ id: 'target', header: 'Target', question: 'Which target should be verified?', isSecret: false, options: [] }] } }] });
    const answer = page.getByLabel('Which target should be verified?', { exact: true });
    await expect(answer).toBeVisible();
    await answer.fill('The mobile owner workspace');
    await page.getByRole('button', { name: 'Submit response', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'resolve_input').length)).toBe(1);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'resolve_input').args)).toMatchObject({ approvalId: 'question-request', response: { answers: { target: { answers: ['The mobile owner workspace'] } } } });
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'send_message').length)).toBe(0);
    await expect(answer).toHaveCount(0);
    if (process.env.MONITTER_TEST_PRODUCTION !== '1') await expect.poll(() => page.evaluate(async () => {
      const { readQuestionDraft, questionDraftKey } = await import('/src/lib/question-drafts.ts');
      return Object.keys(readQuestionDraft(questionDraftKey('watched', 'question-request')).values).length;
    })).toBe(0);

    // The user's normal busy-message preference is queue; Watch must explicitly steer.
    await page.getByRole('button', { name: 'Interject', exact: true }).click();
    await expect(composer).toHaveValue('Retained unsent draft');
    await expect(composer).toBeFocused();
    await composer.press('Escape');
    await expect(page.getByRole('button', { name: 'Interject', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Interject', exact: true }).press('Enter');
    await expect(composer).toBeFocused();
    await composer.fill('Steer the current request');
    await patch(page, { events: await page.evaluate(() => [...window.__MONITTER_QA__.snapshot().events, { id: 'unrelated-snapshot', taskId: 'other', kind: 'agent_progress', title: 'Unrelated task updated', detail: '', createdAt: 36 }]) });
    await expect(composer).toHaveValue('Steer the current request');
    await expect(composer).toBeFocused();
    await page.getByRole('button', { name: 'Send task message', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'send_message').length)).toBe(1);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'send_message').args)).toMatchObject({ taskId: 'watched', text: 'Steer the current request', deliveryMode: 'steer' });
    const geometry = await watch.evaluate(element => ({ overflow: element.scrollWidth > element.clientWidth + 1, rect: { left: element.getBoundingClientRect().left, right: element.getBoundingClientRect().right }, reduced: matchMedia('(prefers-reduced-motion: reduce)').matches, animation: getComputedStyle(element.querySelector('.live-line')).animationName }));
    expect(geometry.overflow).toBe(false);
    expect(geometry.rect.left).toBeGreaterThanOrEqual(-1);
    expect(geometry.rect.right).toBeLessThanOrEqual(width + 1);
    expect(geometry.reduced).toBe(true);
    expect(geometry.animation).toBe('none');
    await expect(composer).toHaveValue('');
    await page.getByRole('button', { name: 'Close Interject', exact: true }).click();
    await page.screenshot({ path: `verification/mobile-interface-parity-${engine.name()}-${theme}-${width}.png` });
    await page.getByRole('button', { name: 'Interject', exact: true }).click();
    await composer.fill('/terminal keep this literal');
    await composer.press('Control+Enter');
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'send_message').length)).toBe(2);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'send_message').at(-1).args)).toMatchObject({ text: '/terminal keep this literal', deliveryMode: 'steer' });
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'open_terminal').length)).toBe(0);
    // Open the actual workspace Idea List route and retain separate project drafts.
    async function openIdeas(name) {
      if (width < 600) await page.getByRole('button', { name: 'Open chats menu', exact: true }).first().click();
      await page.getByRole('tab', { name: 'Projects view', exact: true }).click();
      await page.getByRole('button', { name: `Open Idea List for Project ${name}`, exact: true }).click();
      await expect(page.getByRole('region', { name: `Project ${name} Idea List`, exact: true })).toBeVisible();
    }
    await openIdeas('A');
    await page.getByRole('button', { name: 'Capture idea', exact: true }).click();
    await page.getByLabel('Idea title', { exact: true }).fill('Retained project A draft');
    await page.getByLabel('Idea Markdown', { exact: true }).fill('Owner mobile Markdown details');
    await openIdeas('B');
    await page.getByRole('button', { name: 'Capture idea', exact: true }).click();
    await page.getByLabel('Idea title', { exact: true }).fill('Separate project B draft');
    await openIdeas('A');
    await expect(page.getByLabel('Idea title', { exact: true })).toHaveValue('Retained project A draft');
    await expect(page.getByLabel('Idea Markdown', { exact: true })).toHaveValue('Owner mobile Markdown details');
    await page.getByRole('button', { name: 'Save idea', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'save_project_ideas').length)).toBe(1);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'save_project_ideas').args)).toMatchObject({ projectId: 'project-a', expectedRevision: 0, ideas: [{ title: 'Retained project A draft', markdown: 'Owner mobile Markdown details' }] });
    await expect(page.getByLabel('Idea title', { exact: true })).toHaveCount(0);
    await page.screenshot({ path: `verification/mobile-ideas-route-${engine.name()}-${theme}-${width}.png` });
    expect(errors).toEqual([]);
  } catch (error) {
    await page.screenshot({ path: `verification/mobile-interface-check-failure-${theme}-${width}.png` });
    console.error('Fixture calls:', await page.evaluate(() => window.__MONITTER_QA__.calls));
    console.error('Root interaction page errors:', errors, '\nVisible content:', (await page.locator('body').innerText()).slice(-7000));
    throw error;
  } finally { await page.close(); }
}
try {
  for (const [width, theme] of [[320, 'light'], [390, 'dark'], [1280, 'light'], [1280, 'dark']]) await checks(width, theme);
  console.log('Root AppSurface mobile/interface parity checks passed');
} finally { await browser.close(); }
