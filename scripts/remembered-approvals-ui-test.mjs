// UI fixtures only: no real permission decisions or agents are invoked.
import { chromium, webkit, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18461/';
for (const [engine, viewport] of [[chromium, { width: 1440, height: 1000 }], [webkit, { width: 390, height: 844 }]]) {
  const browser = await engine.launch({ headless: true });
  try {
    const mobile = engine === webkit;
    const page = await browser.newPage({ viewport, hasTouch: mobile, isMobile: mobile });
    const activate = locator => mobile ? locator.tap() : locator.click();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript({ content: readFileSync('scripts/ui-fixture.js', 'utf8') });
    await page.addInitScript(() => {
      const qa = window.__MONITTER_QA__, s = qa.snapshot();
      s.agents[0].provider = 'claude';
      s.tasks = [{ id: 'approval-chat', agentId: 'atlas', title: 'Approval test chat', status: 'running', archived: false, parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp/monitter-ui-test', provider: 'claude', model: '', sandbox: 'harness-configured', nativeSessionId: null, createdAt: 1, updatedAt: 1 }];
      s.approvalRequests = [{ id: 'remember-request', taskId: 'approval-chat', provider: 'claude', runId: 'request-1', tool: 'Read', summary: 'Read README.md', detail: '{"file_path":"/tmp/monitter-ui-test/README.md"}', risk: 'low', status: 'pending', createdAt: 2, resolvedAt: null, decision: null, rememberable: true, ruleId: null }];
      const makeRule = (id, agentId = 'atlas') => ({ id, agentId, hostId: 'local', provider: 'claude', cwd: '/tmp/monitter-ui-test', tool: 'Read', summary: 'Read README.md', detail: '{"file_path":"/tmp/monitter-ui-test/README.md"}', scopeDescription: 'This exact action for this agent, host and working folder.', createdAt: 3, lastUsedAt: null, useCount: 0 });
      s.approvalRules = [makeRule('other-rule', 'another-agent')];
      qa.setSnapshot(s);
      window.__REMEMBER_QA__ = { failRevoke: false, makeRule };
      window.__MONITTER_BRIDGE__.resolveApproval = async (approvalId, decision) => {
        qa.calls.push({ method: 'resolveApproval', approvalId, decision });
        await new Promise(resolve => setTimeout(resolve, 300));
        const next = qa.snapshot(), request = next.approvalRequests.find(r => r.id === approvalId);
        request.status = decision === 'deny' ? 'denied' : 'approved';
        request.decision = decision;
        request.resolvedAt = Date.now();
        if (decision === 'approve_always') {
          request.ruleId = 'saved-rule';
          next.approvalRules.push(makeRule('saved-rule'));
        }
        qa.setSnapshot(next);
        return next;
      };
      window.__MONITTER_BRIDGE__.revokeApprovalRule = async ruleId => {
        qa.calls.push({ method: 'revokeApprovalRule', ruleId });
        await new Promise(resolve => setTimeout(resolve, 200));
        if (window.__REMEMBER_QA__.failRevoke) throw Error('QA revoke failure');
        const next = qa.snapshot();
        next.approvalRules = next.approvalRules.filter(r => r.id !== ruleId);
        qa.setSnapshot(next);
        return next;
      };
    });
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120000 });
    const openChat = async () => {
      const task = page.locator('[data-task-id="approval-chat"] .task-select');
      const back = page.getByRole('button', { name: 'Back to chats', exact: true });
      if (!(await task.isVisible())) await activate(back);
      await activate(task);
    };
    await expect(page.locator('.sidebar')).toBeVisible({ timeout: 90000 }).catch(async error => {
      console.error({ pageErrors: errors, body: (await page.locator('body').innerText()).slice(0, 2000) });
      throw error;
    });
    await openChat();
    const dock = page.getByRole('region', { name: 'Pending approvals', exact: true });
    const always = dock.getByRole('button', { name: 'Always allow exact action', exact: true });
    await expect(always).toBeVisible();
    await activate(always);
    await expect(always).toBeDisabled();
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.snapshot().approvalRules.length)).toBe(2);
    expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(c => c.method === 'resolveApproval'))).toEqual([{ method: 'resolveApproval', approvalId: 'remember-request', decision: 'approve_always' }]);

    const show = page.getByRole('button', { name: 'Show right sidebar', exact: true });
    if (await show.isVisible()) await activate(show);
    await activate(page.getByRole('tab', { name: 'Approvals', exact: true }));
    const sidebar = page.getByLabel('Right sidebar', { exact: true });
    const rule = sidebar.locator('[data-approval-rule-id="saved-rule"]');
    await expect(rule).toBeVisible();
    await expect(sidebar.locator('[data-approval-rule-id="other-rule"]')).toHaveCount(0);
    await page.evaluate(() => { window.__REMEMBER_QA__.failRevoke = true; });
    await activate(rule.getByRole('button', { name: /Revoke/ }));
    await expect(page.getByText('QA revoke failure', { exact: true })).toBeVisible();
    await expect(rule).toBeVisible();
    await page.evaluate(() => { window.__REMEMBER_QA__.failRevoke = false; });
    await activate(rule.getByRole('button', { name: /Revoke/ }));
    await expect(rule).toHaveCount(0);

    await page.evaluate(() => {
      const qa = window.__MONITTER_QA__, next = qa.snapshot();
      next.approvalRules.push(window.__REMEMBER_QA__.makeRule('settings-rule'));
      qa.setSnapshot(next);
    });
    const dismiss = page.getByRole('button', { name: 'Close run detail', exact: true });
    if (await dismiss.isVisible()) await activate(dismiss);
    await page.keyboard.press('Meta+,');
    const settings = page.locator('.settings-pane');
    await activate(settings.getByRole('button', { name: 'Approvals', exact: true }));
    await expect(settings.locator('[data-approval-rule-id]')).toHaveCount(2);
    await expect(settings.locator('[data-approval-rule-id="settings-rule"]')).toContainText('Atlas');
    await expect(settings.locator('[data-approval-rule-id="settings-rule"]')).toContainText('This Mac');
    await mkdir('verification/remembered-approvals', { recursive: true });
    await settings.screenshot({ path: `verification/remembered-approvals/settings-${engine.name()}.png` });
    await activate(settings.locator('[data-approval-rule-id="settings-rule"]').getByRole('button', { name: /Revoke/ }));
    await expect(settings.locator('[data-approval-rule-id="settings-rule"]')).toHaveCount(0);
    await expect(settings.locator('[data-approval-rule-id="other-rule"]')).toBeVisible();

    await openChat();
    await page.evaluate(() => {
      const qa = window.__MONITTER_QA__, next = qa.snapshot();
      next.approvalRequests[0] = { ...next.approvalRequests[0], tool: 'Edit', status: 'pending', decision: null, ruleId: null, resolvedAt: null, input: null, sessionScope: 'file_changes' };
      qa.setSnapshot(next);
    });
    const session = dock.getByRole('button', { name: 'Allow all edits this session', exact: true });
    await expect(session).toBeVisible();
    await activate(session);
    await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.filter(c => c.method === 'resolveApproval').at(-1)?.decision)).toBe('approve_session');

    for (const request of [
      { rememberable: false, sessionScope: undefined, input: null },
      { rememberable: true, sessionScope: undefined, input: { kind: 'questions', schema: null, url: null, questions: [{ id: 'answer', header: 'Answer', question: 'What should I do?', isSecret: false, options: [] }] } },
    ]) {
      await page.evaluate(patch => {
        const qa = window.__MONITTER_QA__, next = qa.snapshot();
        next.approvalRequests[0] = { ...next.approvalRequests[0], ...patch, status: 'pending', decision: null, ruleId: null, resolvedAt: null };
        qa.setSnapshot(next);
      }, request);
      await expect(dock).toBeVisible();
      await expect(dock.getByRole('button', { name: 'Always allow exact action', exact: true })).toHaveCount(0);
      await expect(dock.getByRole('button', { name: 'Allow all edits this session', exact: true })).toHaveCount(0);
    }
    expect(errors).toEqual([]);
    console.log(`${engine.name()}: exact and session approval, single dispatch, scoped sidebar rules, revoke error/retry, Settings revoke and unsupported/input gating passed`);
  } finally { await browser.close(); }
}
