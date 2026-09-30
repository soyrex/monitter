import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { chromium, webkit, expect as baseExpect } from '@playwright/test';
import WebSocket from 'ws';

const { createResumableDesktopSession, generatePairingKeyPair, bytesToBase64url, randomBytes } =
  await import(process.env.MONITTER_OWNER_TEST_BRIDGE);
const root = new URL('../', import.meta.url).pathname;
const screenshotDirectory = join(root, 'artifacts', 'companion-ui-' + new Date().toISOString().replaceAll(':', '-'));
await mkdir(screenshotDirectory, { recursive: true });

globalThis.WebSocket = WebSocket;
const expect = baseExpect.configure({ timeout: 30_000 });
const children = [];
let browser;
let desktop;
let phone;
const ownerCommands = [];
const sentMessages = [];
const approvalsResolved = [];
const inputsResolved = [];
const pageErrors = [];
let injectedReplyBatch = false;
const controllerTrusted = new Set();
const ownerTrusted = new Set();

function start(command, args, env, pattern) {
  const child = spawn(command, args, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child);
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error('Test server startup timed out: ' + output.slice(-500))), 60_000);
    child.once('error', reason => { clearTimeout(timer); reject(reason); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error('Test server exited ' + code + ': ' + output.slice(-500))); });
    const receive = data => {
      output += data.toString();
      const match = output.match(pattern);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', receive);
    child.stderr.on('data', receive);
  });
}

const agent = {
  id: 'companion-agent', name: 'Companion agent', provider: 'codex', model: 'test-model',
  hostId: 'local', cwd: '/tmp', sandbox: 'workspace-write', color: '#00A8F0', description: '',
  instructions: '', avatar: null, expertise: [], responsibilities: [], skills: [], collaborationEnabled: false,
};
const taskA = {
  id: 'companion-task-a', agentId: agent.id, title: 'Alpha chat', nativeSessionId: null,
  archived: false, status: 'idle', createdAt: Date.now() - 2000, updatedAt: Date.now(),
  parentTaskId: null, channelId: null, projectId: null, hostId: 'local', cwd: '/tmp',
  provider: 'codex', model: 'test-model', sandbox: 'workspace-write',
};
const taskB = { ...taskA, id: 'companion-task-b', title: 'Beta chat', createdAt: Date.now() - 1000, updatedAt: Date.now() - 1000 };
const approvalA = {
  id: 'approval-alpha', taskId: taskA.id, provider: 'codex', runId: 'run-alpha', tool: 'shell',
  summary: 'Run the reviewed Alpha command?', detail: JSON.stringify({ command: 'printf alpha', reason: 'Alpha chat requests permission.' }),
  risk: 'medium', status: 'pending', createdAt: Date.now(), resolvedAt: null, decision: null, rememberable: false,
};
const questionB = {
  id: 'question-beta', taskId: taskB.id, provider: 'codex', runId: 'run-beta', tool: 'request_user_input',
  summary: 'Where should I save the report?', detail: 'Choose a destination.', risk: 'low', status: 'pending',
  createdAt: Date.now(), resolvedAt: null, decision: null,
  input: { kind: 'questions', questions: [{ id: 'destination', header: 'Destination', question: 'Where should I save it?', isSecret: false,
    options: [{ label: 'Downloads', description: 'Save in Downloads.' }, { label: 'Documents', description: 'Save in Documents.' }] }], schema: null, url: null },
};
const snapshot = {
  hosts: [], agents: [agent], tasks: [taskA, taskB],
  messages: [
    { id: 'commentary-alpha', taskId: taskA.id, role: 'assistant', phase: 'commentary', streamStatus: 'complete', text: 'Internal progress commentary must stay out of final replies.', createdAt: Date.now() - 1000 },
    { id: 'final-alpha', taskId: taskA.id, role: 'assistant', phase: 'final_answer', streamStatus: 'complete', text: 'Alpha completed with a final answer.', createdAt: Date.now() },
  ],
  events: [], channels: [], projects: [], collaborations: [], queuedMessages: [], approvalRequests: [approvalA],
  settings: { theme: 'dark', accent: '#00A8F0', activityAnimation: 'off' },
};

const identity = { keyPair: await generatePairingKeyPair(), room: bytesToBase64url(randomBytes(18)) };
const desktopOptions = {
  relayUrl: '', identity,
  bridge: {
    getSnapshot: async () => snapshot, sendMessage: async () => snapshot,
    cancelTask: async () => snapshot, resumeTask: async () => snapshot,
    listTerminals: async () => [], readTerminal: async () => ({ chunks: [], nextSeq: 0, status: 'exited', exitCode: 0, truncated: false }),
  },
  isTrustedController: key => controllerTrusted.has(key),
  onControllerAccess: key => controllerTrusted.add(key),
  isTrustedOwnerWorkspace: key => ownerTrusted.has(key),
  onOwnerWorkspaceAccess: () => {},
  onOwnerInvoke: async (command, args) => {
    ownerCommands.push(command);
    if (command === 'resolve_approval') {
      approvalsResolved.push(args);
      const request = snapshot.approvalRequests.find(item => item.id === args.approvalId);
      if (request) Object.assign(request, { status: args.decision === 'deny' ? 'denied' : 'approved', decision: args.decision, resolvedAt: Date.now() });
      if (args.approvalId === approvalA.id && !injectedReplyBatch) {
        injectedReplyBatch = true;
        const longReply = ['## Beta update', '', 'Here is a detailed status report from the other chat.', '',
          ...Array.from({ length: 28 }, (_, index) => `- **Checkpoint ${index + 1}:** the report remains readable inside a bounded notification.`),
          '', '```text', 'The overlay must scroll without covering Mo or the composer.', '```'].join('\n');
        snapshot.messages.push(
          { id: 'final-beta-incoming', taskId: taskB.id, role: 'assistant', phase: 'final_answer', streamStatus: 'complete', text: longReply, createdAt: Date.now() + 1 },
          { id: 'final-alpha-next', taskId: taskA.id, role: 'assistant', phase: 'final_answer', streamStatus: 'complete', text: 'Alpha has a new reply queued behind Beta.', createdAt: Date.now() + 2 },
          { id: 'stream-alpha', taskId: taskA.id, role: 'assistant', phase: 'final_answer', streamStatus: 'streaming', text: 'This Alpha reply is still streaming.', createdAt: Date.now() + 3 },
          { id: 'commentary-beta', taskId: taskB.id, role: 'assistant', phase: 'commentary', streamStatus: 'complete', text: 'This is internal commentary, not a reply popup.', createdAt: Date.now() + 4 },
        );
      }
      return snapshot;
    }
    if (command === 'resolve_input') {
      inputsResolved.push(args);
      const request = snapshot.approvalRequests.find(item => item.id === args.approvalId);
      if (request) Object.assign(request, { status: 'approved', response: args.response, resolvedAt: Date.now() });
      const streamed = snapshot.messages.find(item => item.id === 'stream-alpha');
      if (streamed) Object.assign(streamed, { streamStatus: 'complete', text: 'Alpha’s reply is now final.' });
      return snapshot;
    }
    if (command === 'send_message') { sentMessages.push(args); return snapshot; }
    if (command === 'get_owner_appearance') return {
      version: 1, appTheme: { light: 'monitter', dark: 'monitter', accent: null, contrast: 0 },
      surfaceTint: 3, terminalTheme: 'monitter',
    };
    if (command === 'get_command_capabilities') return {
      protocolVersion: 1,
      commands: ['get_snapshot', 'get_usage_overview', 'list_codex_accounts', 'send_message', 'resolve_approval', 'resolve_input',
        'get_task_git_status', 'get_task_goal', 'list_terminals', 'get_task_model_settings', 'get_task_slash_commands'],
    };
    if (['get_snapshot', 'send_message', 'resolve_approval', 'resolve_input'].includes(command)) return snapshot;
    if (['list_codex_accounts', 'list_terminals', 'get_task_slash_commands'].includes(command)) return [];
    if (command === 'get_usage_overview') return { generatedAt: Date.now(), capturedSince: null, subscriptions: [], providerTotals: [], recentRuns: [] };
    return null;
  },
};

try {
  const relayUrl = await start(process.execPath, ['scripts/relay-server.mjs'],
    { MONITTER_RELAY_PORT: '0', MONITTER_RELAY_HOST: '127.0.0.1' }, /listening on (ws:\/\/[^\s]+)/);
  desktopOptions.relayUrl = relayUrl;
  const baseUrl = await start(process.execPath,
    ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '31977', '--strictPort'],
    { NO_COLOR: '1' }, /Local:\s+(http:\/\/[^\s]+)/);
  desktop = await createResumableDesktopSession(desktopOptions);

  browser = await webkit.launch();
  phone = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  phone.on('pageerror', reason => { pageErrors.push(String(reason)); console.error('Companion WebKit page error:', reason); });
  phone.on('console', message => { if (message.type() === 'error') console.error('Companion WebKit console:', message.text()); });
  await phone.addInitScript(() => {
    window.__nativeSpeechCalls = [];
    window.__nativeCompanionCalls = [];
    window.__emitNativeSpeech = (sessionId, type, text = '', error = '', level) => window.dispatchEvent(new CustomEvent('monitter:native-speech', {
      detail: { sessionId, type, ...(text ? { text } : {}), ...(error ? { error } : {}), ...(typeof level === 'number' ? { level } : {}) },
    }));
    const speechHandler = {
      postMessage(payload) {
        const message = typeof payload === 'string' ? JSON.parse(payload) : payload;
        window.__nativeSpeechCalls.push(message);
        const emit = (type, extra = {}) => window.__emitNativeSpeech(message.sessionId, type, extra.text ?? '', extra.error ?? '');
        if (message.action === 'status') setTimeout(() => emit('availability', { available: true }), 0);
        // A start acknowledgement is the only automatic event; speech is injected by the test.
        if (message.action === 'start') setTimeout(() => emit('started'), 0);
      },
    };
    const companionHandler = { postMessage(payload) { window.__nativeCompanionCalls.push(payload); } };
    window.webkit ??= {};
    window.webkit.messageHandlers ??= {};
    window.webkit.messageHandlers.monitterSpeech = speechHandler;
    window.webkit.messageHandlers.monitterCompanion = companionHandler;
  });
  await phone.goto(new URL('/mobile', baseUrl).href, { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(phone.getByRole('button', { name: 'Scan desktop code' })).toBeVisible();
  await phone.evaluate(invitation => window.dispatchEvent(new CustomEvent('monitter:qr', { detail: invitation })), desktop.invitation);
  await expect.poll(() => desktop.getStatus()).toBe('pending');
  await desktop.approve();
  await expect.poll(() => desktop.getOwnerWorkspaceState()).toBe('requested');
  const phoneKey = desktop.getPeerControllerPublicKey();
  assert.ok(phoneKey, 'paired phone identity was missing');
  ownerTrusted.add(phoneKey);
  await desktop.approveOwnerWorkspace();
  await phone.waitForURL(url => url.pathname === '/', { timeout: 30_000 });
  await expect(phone.locator('.app-shell')).toHaveCount(1);
  const mobileBrand = phone.locator('.sidebar .brand');
  const mobileBrandOrder = await mobileBrand.evaluate(node => [...node.children].filter(child => child instanceof HTMLButtonElement).map(child => child.getAttribute('aria-label')));
  assert.deepEqual(mobileBrandOrder.slice(0, 3), ['Open Settings', 'Open Companion Mode', 'Open global overview'],
    'mobile companion shortcut is not beside Settings and left of the Monitter logo');
  await expect(phone.getByRole('button', { name: 'Open Companion Mode' }).first()).toBeVisible();
  await phone.getByRole('button', { name: 'Open Companion Mode' }).first().click();

  const companion = phone.locator('main.companion');
  await expect(companion).toBeVisible();
  const pinnedChat = phone.getByRole('combobox', { name: 'Pinned chat' });
  await pinnedChat.selectOption(taskA.id);
  const replyPopup = phone.locator('[aria-label="Agent reply"]');
  await expect(replyPopup.getByText('Alpha completed with a final answer.', { exact: true })).toBeVisible();
  await expect(replyPopup.getByText('Internal progress commentary must stay out of final replies.', { exact: true })).toHaveCount(0);
  await expect(companion.locator('.reply-card')).toHaveCount(0);
  const face = companion.locator('.avatar');
  const faceCore = face.locator('.avatar-core');
  const manualTalk = phone.getByRole('button', { name: 'Tap to talk', exact: true });
  const wakeToggle = phone.getByRole('switch', { name: 'Hey Mo', exact: true });
  await expect(face).toBeVisible();
  await expect(manualTalk).toBeVisible();
  await expect(wakeToggle).toBeVisible();
  await expect(wakeToggle).not.toBeChecked();
  await expect(phone.getByRole('textbox', { name: 'Edit message' })).toHaveCount(0);
  await expect(phone.getByRole('button', { name: 'Send message', exact: true })).toHaveCount(0);
  const faceBounds = await faceCore.boundingBox();
  const talkBounds = await manualTalk.boundingBox();
  assert.ok(talkBounds && talkBounds.height >= 422 && talkBounds.y >= 0 && talkBounds.y + talkBounds.height <= 844,
    'Mo tap-to-talk control is too small or outside the first portrait viewport');
  assert.ok(faceBounds && Math.abs(faceBounds.width / talkBounds.width - .6) < .02 &&
    Math.abs(faceBounds.height / talkBounds.height - .6) < .02 &&
    Math.abs(faceBounds.x + faceBounds.width / 2 - talkBounds.x - talkBounds.width / 2) < 2 &&
    Math.abs(faceBounds.y + faceBounds.height / 2 - talkBounds.y - talkBounds.height / 2) < 2,
    'Mo face should be 60% of and centered within the unchanged talk area');
  await expect(phone.getByText('Internal progress commentary must stay out of final replies.', { exact: true })).toHaveCount(0);
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-portrait.png') });
  await phone.getByRole('button', { name: 'Dismiss agent reply', exact: true }).click();
  await expect(replyPopup).toHaveCount(0);
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-portrait-idle.png') });
  const darkBackground = await companion.evaluate(node => getComputedStyle(node).backgroundColor);
  snapshot.settings.theme = 'light';
  await expect.poll(() => companion.evaluate(node => getComputedStyle(node).backgroundColor)).not.toBe(darkBackground);
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-light.png') });
  snapshot.settings.theme = 'dark';
  await expect.poll(() => companion.evaluate(node => getComputedStyle(node).backgroundColor)).toBe(darkBackground);
  const composer = phone.getByRole('textbox', { name: 'Edit message' });
  const sendButton = phone.getByRole('button', { name: 'Send message', exact: true });
  await phone.setViewportSize({ width: 844, height: 390 });
  await expect(pinnedChat).toBeVisible();
  assert.ok(await companion.evaluate(node => node.scrollWidth <= window.innerWidth), 'landscape companion overflowed horizontally');
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-landscape.png') });
  await phone.setViewportSize({ width: 390, height: 844 });

  // A request arriving for Beta leaves Alpha selected and its own approval actionable.
  snapshot.approvalRequests.push(questionB);
  await expect(phone.getByRole('button', { name: /1 in other chats/ })).toBeVisible();
  await expect(pinnedChat).toHaveValue(taskA.id);
  await expect(phone.getByText('Run the reviewed Alpha command?', { exact: true })).toBeVisible();
  assert.equal(await pinnedChat.inputValue(), taskA.id, 'initial reply popup changed the selected task');
  await phone.getByRole('button', { name: 'Approve once', exact: true }).click();
  await expect.poll(() => approvalsResolved.length).toBe(1);
  assert.deepEqual(approvalsResolved[0], { approvalId: approvalA.id, decision: 'approve_once' });
  assert.equal(snapshot.approvalRequests.find(item => item.id === approvalA.id)?.status, 'approved');

  await expect(replyPopup.getByText('Beta chat · Companion agent', { exact: true })).toBeVisible();
  await expect(replyPopup.getByRole('button', { name: 'Reply in this chat', exact: true })).toBeVisible();
  assert.equal(await pinnedChat.inputValue(), taskA.id, 'incoming Beta reply automatically retargeted the pinned chat');
  await expect(replyPopup.getByText('This Alpha reply is still streaming.', { exact: true })).toHaveCount(0);
  await expect(replyPopup.getByText('This is internal commentary, not a reply popup.', { exact: true })).toHaveCount(0);
  const popupContent = replyPopup.locator('.reply-popup-content');
  const popupBounds = await replyPopup.boundingBox();
  const faceBoundsForPopup = await companion.locator('.avatar').boundingBox();
  const popupScrollState = await popupContent.evaluate(node => ({ scrollHeight: node.scrollHeight, clientHeight: node.clientHeight }));
  assert.ok(popupBounds && popupBounds.y >= 0 && popupBounds.y + popupBounds.height <= 844,
    'reply popup exceeds the portrait viewport');
  assert.ok(popupScrollState.scrollHeight > popupScrollState.clientHeight, 'long Markdown reply is not scrollable within the popup');
  assert.ok(faceBoundsForPopup && popupBounds, 'face control or reply popup is missing');
  const faceClickBounds = await manualTalk.boundingBox();
  assert.ok(faceClickBounds, 'Mo face button is missing while a reply popup is visible');
  await face.click({ position: { x: faceClickBounds.width / 2, y: faceClickBounds.height * .78 } });
  await expect(phone.getByRole('button', { name: 'Stop listening', exact: true })).toBeVisible();
  const popupFaceSession = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
  assert.ok(popupFaceSession, 'reply popup blocked a tap on the exposed lower face');
  await emit(popupFaceSession, 'partial', 'Popup did not block the Mo tap target');
  const recordingFaceBounds = await face.boundingBox();
  assert.ok(recordingFaceBounds, 'recording face disappeared behind the live draft');
  await face.click({ position: { x: recordingFaceBounds.width / 2, y: recordingFaceBounds.height * .78 } });
  await expect.poll(() => phone.evaluate(id => window.__nativeSpeechCalls.some(call => call.action === 'stop' && call.sessionId === id), popupFaceSession)).toBe(true);
  await emit(popupFaceSession, 'final', 'Popup did not block the Mo tap target');
  await expect(companion.locator('.outgoing-text')).toHaveText('Popup did not block the Mo tap target');
  await phone.getByRole('button', { name: 'Edit', exact: true }).click();
  await composer.fill('');
  await expect(sendButton).toHaveCount(0);
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-reply-popup-portrait.png') });
  await phone.setViewportSize({ width: 844, height: 390 });
  const landscapePopupBounds = await replyPopup.boundingBox();
  assert.ok(landscapePopupBounds && landscapePopupBounds.y >= 0 && landscapePopupBounds.y + landscapePopupBounds.height <= 390,
    'landscape reply popup exceeds the viewport');
  await expect(phone.getByRole('button', { name: 'Tap to talk', exact: true })).toBeVisible();
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-reply-popup-landscape.png') });
  await phone.setViewportSize({ width: 390, height: 844 });
  await phone.getByRole('button', { name: 'Dismiss agent reply', exact: true }).click();
  await expect(replyPopup.getByText('Alpha has a new reply queued behind Beta.', { exact: true })).toBeVisible();
  await expect(replyPopup.getByRole('button', { name: 'Reply in this chat', exact: true })).toHaveCount(0);
  assert.equal(await pinnedChat.inputValue(), taskA.id, 'queued Alpha reply changed the selected task');
  await phone.getByRole('button', { name: 'Dismiss agent reply', exact: true }).click();
  await expect(replyPopup).toHaveCount(0);

  // The face remains the primary talk action; text entry is a deliberate edit of a voice draft.
  await pinnedChat.selectOption(taskB.id);
  await manualTalk.click();
  await expect(phone.getByRole('button', { name: 'Stop listening', exact: true })).toBeVisible();
  const typedDraftSession = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
  assert.ok(typedDraftSession, 'tapping Mo did not start native capture');
  await emit(typedDraftSession, 'partial', 'Typed message pinned to Beta');
  await emit(typedDraftSession, 'final', 'Typed message pinned to Beta');
  await expect(companion.locator('.outgoing-text')).toHaveText('Typed message pinned to Beta');
  await phone.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(composer).toBeVisible();
  await composer.fill('Typed message pinned to Beta');
  await sendButton.click();
  await expect.poll(() => sentMessages.length).toBe(1);
  assert.equal(sentMessages[0]?.taskId, taskB.id, 'edited message was not sent to the pinned task');
  assert.equal(sentMessages[0]?.text, 'Typed message pinned to Beta');

  await pinnedChat.selectOption(taskB.id);
  await expect(phone.getByText('Where should I save the report?', { exact: true })).toBeVisible();
  await phone.getByRole('textbox', { name: 'Where should I save it?' }).fill('Downloads');
  await phone.getByRole('button', { name: 'Submit response', exact: true }).click();
  await expect.poll(() => inputsResolved.length).toBe(1);
  assert.deepEqual(inputsResolved[0], { approvalId: questionB.id, response: { answers: { destination: { answers: ['Downloads'] } } } });
  await expect(replyPopup.getByText('Alpha’s reply is now final.', { exact: true })).toBeVisible();
  await expect(replyPopup.getByRole('button', { name: 'Reply in this chat', exact: true })).toBeVisible();
  assert.equal(await pinnedChat.inputValue(), taskB.id, 'final Alpha reply automatically retargeted the selected task');
  await phone.getByRole('button', { name: 'Dismiss agent reply', exact: true }).click();
  await expect(replyPopup).toHaveCount(0);
  await expect(phone.getByRole('heading', { name: 'Needs your attention', exact: true })).toHaveCount(0);

  async function startWake() {
    if (!(await wakeToggle.isChecked())) await wakeToggle.check();
    await expect(phone.getByText('Listening for Hey Mo', { exact: true })).toBeVisible();
    const sessionId = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
    assert.ok(sessionId, 'native wake listener did not start');
    return sessionId;
  }
  async function emit(sessionId, type, text, level) {
    await phone.evaluate(value => window.__emitNativeSpeech(value.sessionId, value.type, value.text, '', value.level), { sessionId, type, text, level });
  }

  const autoSend = phone.getByRole('checkbox', { name: /^Auto-send after a 10-second pause/ });
  await expect(autoSend).not.toBeChecked();
  // A recognition correction from the wake phrase to "a monitor" is still only the trigger.
  let wakeSession = await startWake();
  await emit(wakeSession, 'partial', 'Hey Mo');
  await emit(wakeSession, 'final', 'a monitor');
  await phone.waitForTimeout(500);
  assert.equal(sentMessages.length, 1, 'recognizer correction "a monitor" was sent as a command');
  const resetWakeSession = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
  assert.ok(resetWakeSession && resetWakeSession !== wakeSession, 'unrecognized wake correction did not reset the listener');

  // The recognized "A Mo" correction preserves the wake and opens a bare-command session.
  wakeSession = resetWakeSession;
  await emit(wakeSession, 'partial', 'Hey Mo');
  await emit(wakeSession, 'final', 'A Mo');
  const revisedWakeSession = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
  assert.ok(revisedWakeSession && revisedWakeSession !== wakeSession, 'wake-only correction did not rotate into a bare-command session');

  // A subsequent command is reviewed in the editable draft and requires Send.
  wakeSession = revisedWakeSession;
  await emit(wakeSession, 'partial', 'create a reminder for tomorrow');
  await emit(wakeSession, 'final', 'create a reminder for tomorrow');
  await expect(companion.locator('.outgoing-text')).toHaveText('create a reminder for tomorrow');
  await expect(wakeToggle).not.toBeChecked();
  await expect(companion.locator('.delivery-notice')).toHaveCount(0);
  const largeSendBounds = await phone.getByRole('button', { name: 'Send message', exact: true }).boundingBox();
  assert.ok(largeSendBounds && largeSendBounds.height >= 64, 'voice review Send action is not large enough to tap confidently');
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-outgoing-review.png') });
  await phone.waitForTimeout(3_300);
  assert.equal(sentMessages.length, 1, 'review-first voice command auto-sent without explicit confirmation');
  await phone.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => sentMessages.length).toBe(2);
  assert.equal(sentMessages[1]?.taskId, taskB.id, 'voice command retargeted away from the pinned chat');
  assert.equal(sentMessages[1]?.text, 'create a reminder for tomorrow');

  // Optional pause-based sending waits beyond the first three seconds and fires once.
  await autoSend.check();
  wakeSession = await startWake();
  await emit(wakeSession, 'partial', 'Hey Mo, send after a pause');
  await emit(wakeSession, 'final', 'Hey Mo, send after a pause');
  await expect(companion.locator('.outgoing-text')).toHaveText('send after a pause');
  await phone.waitForTimeout(3_300);
  assert.equal(sentMessages.length, 2, 'optional auto-send fired before the pause threshold');
  await expect.poll(() => sentMessages.length, { timeout: 12_000 }).toBe(3);
  assert.equal(sentMessages[2]?.taskId, taskB.id);
  assert.equal(sentMessages[2]?.text, 'send after a pause');
  assert.equal(sentMessages.filter(item => item.text === 'send after a pause').length, 1, 'pause auto-send fired more than once');

  // Editing a review-first draft cancels it; switching the selected task cannot retarget late audio.
  await autoSend.uncheck();
  wakeSession = await startWake();
  await emit(wakeSession, 'partial', 'Hey Mo, keep this on Beta');
  await emit(wakeSession, 'final', 'Hey Mo, keep this on Beta');
  await expect(companion.locator('.outgoing-text')).toHaveText('keep this on Beta');
  await phone.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(composer).toBeVisible();
  await composer.fill('');
  await pinnedChat.selectOption(taskA.id);
  await emit(wakeSession, 'final', 'late stale words');
  await phone.waitForTimeout(500);
  assert.equal(sentMessages.length, 3, 'late speech after editing or task switching was sent');
  assert.equal(await pinnedChat.inputValue(), taskA.id);

  await pinnedChat.selectOption(taskB.id);
  wakeSession = await startWake();
  await emit(wakeSession, 'partial', 'Hey Mo, do not auto retry this');
  await emit(wakeSession, 'final', 'Hey Mo, do not auto retry this');
  await expect(companion.locator('.outgoing-text')).toHaveText('do not auto retry this');
  desktop.close();
  await expect(phone.getByRole('status', { name: 'Desktop offline', exact: true })).toBeVisible();
  await expect(companion.locator('.outgoing-text')).toHaveText('do not auto retry this');
  desktop = await createResumableDesktopSession(desktopOptions);
  await expect(phone.getByRole('status', { name: 'Desktop connected', exact: true })).toBeVisible({ timeout: 45_000 });
  await expect(companion.locator('.outgoing-text')).toHaveText('do not auto retry this');
  await expect(replyPopup).toHaveCount(0);
  await phone.waitForTimeout(500);
  assert.equal(sentMessages.length, 3, 'reconnection retried a reviewed wake command');

  await phone.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(composer).toBeVisible();
  await composer.fill('Preserve this typed draft');
  wakeSession = await startWake();
  await emit(wakeSession, 'partial', 'Hey Mo, add this by voice');
  await emit(wakeSession, 'final', 'Hey Mo, add this by voice');
  await expect(composer).toHaveValue('Preserve this typed draft\nadd this by voice');
  await phone.waitForTimeout(3_300);
  assert.equal(sentMessages.length, 3, 'voice over a typed draft auto-sent without review');
  await phone.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => sentMessages.length).toBe(4);
  assert.equal(sentMessages[3]?.taskId, taskB.id, 'composed voice plus typed draft missed the pinned task');
  assert.equal(sentMessages[3]?.text, 'Preserve this typed draft\nadd this by voice');

  const sentCountBeforeManual = sentMessages.length;
  await phone.getByRole('button', { name: 'Tap to talk', exact: true }).click();
  await expect(phone.getByRole('button', { name: 'Stop listening', exact: true })).toBeVisible();
  const manualSession = await phone.evaluate(() => [...window.__nativeSpeechCalls].reverse().find(call => call.action === 'start')?.sessionId ?? null);
  assert.ok(manualSession, 'manual native speech did not start');
  const mouth = companion.locator('.mouth');
  const idleMouthHeight = await mouth.evaluate(node => getComputedStyle(node).height);
  await emit(manualSession, 'level', undefined, 0.8);
  await expect(face).toHaveClass(/speaking/);
  await expect.poll(() => mouth.evaluate(node => getComputedStyle(node).height)).not.toBe(idleMouthHeight);
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-speaking.png') });
  await emit(manualSession, 'level', undefined, 0);
  await expect(face).not.toHaveClass(/speaking/);
  await expect.poll(() => mouth.evaluate(node => getComputedStyle(node).height)).toBe(idleMouthHeight);
  await emit(manualSession, 'partial', 'A manual voice draft');
  await emit(manualSession, 'final', 'A manual voice draft');
  await expect(companion.locator('.outgoing-text')).toHaveText('A manual voice draft');
  await phone.getByRole('button', { name: 'Edit', exact: true }).click();
  await composer.fill('A manual voice draft, edited');
  await phone.setViewportSize({ width: 390, height: 390 });
  await composer.focus();
  await expect(companion).toHaveAttribute('data-keyboard-composer', 'true');
  await expect(companion.locator('.hero')).toBeHidden();
  const keyboardSendBounds = await sendButton.boundingBox();
  assert.ok(keyboardSendBounds && keyboardSendBounds.y >= 0 && keyboardSendBounds.y + keyboardSendBounds.height <= 390,
    'review Send was covered in the reduced keyboard viewport');
  await phone.screenshot({ path: join(screenshotDirectory, 'companion-keyboard-review.png') });
  await composer.blur();
  await phone.setViewportSize({ width: 390, height: 844 });
  await phone.waitForTimeout(3_300);
  assert.equal(sentMessages.length, sentCountBeforeManual, 'manual voice transcript auto-sent without review');

  // Drafts survive closing and reopening the companion in this paired page.
  await phone.getByRole('button', { name: 'Close Companion Mode', exact: true }).click();
  await expect(phone.locator('main.companion')).toHaveCount(0);
  await phone.getByRole('button', { name: 'Open Companion Mode' }).first().click();
  await expect(phone.locator('main.companion')).toBeVisible();
  const reopenedPinnedChat = phone.getByRole('combobox', { name: 'Pinned chat' });
  await reopenedPinnedChat.selectOption(taskB.id);
  await expect(companion.locator('.outgoing-text')).toHaveText('A manual voice draft, edited');

  wakeSession = await startWake();
  await phone.getByRole('button', { name: 'Close Companion Mode', exact: true }).click();
  await expect(phone.locator('main.companion')).toHaveCount(0);
  const nativeCalls = await phone.evaluate(() => ({ speech: window.__nativeSpeechCalls, companion: window.__nativeCompanionCalls }));
  assert.ok(nativeCalls.speech.some(call => call.action === 'cancel' && call.sessionId === wakeSession), 'closing did not cancel native speech');
  assert.ok(nativeCalls.companion.some(call => call.active === false), 'closing did not deactivate native companion state');
  assert.equal(sentMessages.length, 4, 'closing the companion sent a message');
  assert.ok(ownerCommands.includes('resolve_approval') && ownerCommands.includes('resolve_input') && ownerCommands.includes('send_message'),
    'expected actions did not cross the encrypted owner relay');
  assert.deepEqual(pageErrors, [], 'companion raised WebKit page errors: ' + pageErrors.join(' | '));
  console.log('Encrypted owner-relay companion UI passed: approvals/questions, final-only replies, task routing, review-first and optional pause auto-send, stale event protection, disconnect recovery, close cleanup, and portrait/landscape screenshots at ' + screenshotDirectory);
} finally {
  desktop?.close();
  await browser?.close();
  for (const child of children) child.kill('SIGTERM');
}
