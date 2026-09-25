import { chromium, expect } from '@playwright/test';

const url = process.env.MONITTER_TEST_URL || 'http://127.0.0.1:18479/';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });

try {
  await page.addInitScript({ path: 'scripts/ui-fixture.js' });
  await page.addInitScript(() => {
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = { transformCallback: () => 1, invoke: async () => undefined };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: async () => ({ getTracks: () => [{ stop() {} }] }) },
    });
    window.MediaRecorder = class {
      state = 'inactive';
      mimeType = 'audio/webm';
      start() { this.state = 'recording'; }
      stop() {
        this.state = 'inactive';
        this.ondataavailable?.({ data: new Blob([new Uint8Array([1])], { type: this.mimeType }) });
        this.onstop?.();
      }
    };
    window.AudioContext = class {
      async decodeAudioData() {
        return { numberOfChannels: 1, sampleRate: 16_000, length: 16, getChannelData: () => new Float32Array(16) };
      }
      async close() {}
    };
  });

  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 120_000 });
  await expect(page.getByRole('button', { name: 'New chat with Atlas' })).toBeVisible({ timeout: 120_000 });
  await page.evaluate(() => {
    const qa = window.__MONITTER_QA__;
    const snapshot = qa.snapshot();
    const now = Date.now();
    snapshot.tasks.push({
      id: 'voice-chat', agentId: 'atlas', title: 'Voice chat', nativeSessionId: null,
      status: 'idle', archived: false, createdAt: now, updatedAt: now,
      parentTaskId: null, channelId: null, projectId: null, hostId: 'local',
      cwd: '/tmp/monitter-ui-test', provider: 'codex', model: '', sandbox: 'read-only',
    });
    qa.setSnapshot(snapshot);
    qa.voiceTranscriptions = [];
    window.__MONITTER_BRIDGE__.transcribeVoiceMessage = async audioBase64 => {
      qa.voiceTranscriptions.push(audioBase64);
      return { text: 'Transcript only message' };
    };
  });

  await page.locator('.task-row').filter({ hasText: 'Voice chat' }).first().click();
  const recorder = page.locator('.composer .voice-recorder');
  await expect(recorder.getByRole('button', { name: 'Record voice message' })).toBeVisible();
  await recorder.getByRole('button', { name: 'Record voice message' }).click();
  await expect(recorder.getByRole('status')).toContainText('0:00 / 1:30');
  await expect(recorder.getByRole('status')).not.toContainText('Recording');
  await recorder.getByRole('button', { name: 'Stop recording' }).click();

  const composer = page.getByRole('textbox', { name: 'Task message', exact: true });
  await expect(composer).toHaveValue('Transcript only message');
  expect(await page.evaluate(() => window.__MONITTER_QA__.voiceTranscriptions.length)).toBe(1);
  expect(await page.evaluate(() => window.__MONITTER_QA__.calls.filter(call => call.method === 'storeAttachment'))).toEqual([]);
  await page.getByRole('button', { name: 'Send task message' }).click();
  await expect.poll(() => page.evaluate(() => window.__MONITTER_QA__.calls.find(call => call.method === 'sendMessage'))).toMatchObject({
    args: { taskId: 'voice-chat', text: 'Transcript only message', attachmentIds: [] },
  });

  console.log('Voice recording shows only the timer and sends transcript text without a WAV attachment.');
} finally {
  await browser.close();
}
