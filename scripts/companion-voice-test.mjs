import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/companion-voice.ts', import.meta.url), 'utf8')
  .replace(/^import .*mobile-speech';\n/, `const { sendNativeSpeechCommand, subscribeNativeSpeech } = globalThis.__bridge;\n`);
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;

let now = 1;
let timerId = 0;
const timers = new Map();
function fakeSetTimeout(fn, delay) { const id = ++timerId; timers.set(id, { at: now + delay, fn }); return id; }
function fakeClearTimeout(id) { timers.delete(id); }
function advance(ms) {
  const end = now + ms;
  while (true) {
    const next = [...timers.entries()].sort((a, b) => a[1].at - b[1].at)[0];
    if (!next || next[1].at > end) break;
    now = next[1].at; timers.delete(next[0]); next[1].fn();
  }
  now = end;
}

const listeners = new Set();
const commands = [];
const window = { webkit: { messageHandlers: {
  monitterSpeech: { postMessage: payload => commands.push(payload) },
  monitterCompanion: { postMessage: payload => commands.push({ companion: payload }) },
} } };
const documentListeners = new Set();
const document = {
  visibilityState: 'visible',
  addEventListener: (_name, fn) => documentListeners.add(fn),
  removeEventListener: (_name, fn) => documentListeners.delete(fn),
};
globalThis.__bridge = {
  sendNativeSpeechCommand(action, sessionId, locale) { commands.push({ action, sessionId, locale }); return true; },
  subscribeNativeSpeech(fn) { listeners.add(fn); return () => listeners.delete(fn); },
};
const exports = {};
new Function('exports', 'require', 'setTimeout', 'clearTimeout', 'window', 'document', 'crypto', js)(exports, () => {}, fakeSetTimeout, fakeClearTimeout, window, document, { randomUUID: () => `session-${++timerId}` });
const { createCompanionVoice, isCompanionAvailable, setCompanionActive } = exports;

function harness() {
  const states = [];
  const transcripts = [];
  const levels = [];
  const voice = createCompanionVoice({ onstate: state => states.push(state), ontranscript: (...args) => transcripts.push(args), onlevel: level => levels.push(level) });
  return { voice, states, transcripts, levels };
}
function activeSession() { return [...commands].reverse().find(value => value.action === 'start')?.sessionId; }
function event(sessionId, type, text, error) { for (const listener of [...listeners]) listener({ sessionId, type, text, error }); }
function levelEvent(sessionId, level) { for (const listener of [...listeners]) listener({ sessionId, type: 'level', level }); }

assert.equal(isCompanionAvailable(), true);
assert.equal(setCompanionActive(true), true);
assert.deepEqual(commands.at(-1), { companion: { active: true } });

// Microphone levels belong to the active native speech session. A retired
// session cannot animate Mo after rotation or stop, while the current one can.
{
  const { voice, levels } = harness();
  voice.listen('live-levels');
  const first = activeSession();
  assert.deepEqual(levels, [0, 0], 'starting should clear any prior speaking animation');
  event(first, 'started');
  levelEvent(first, 0.37);
  assert.deepEqual(levels, [0, 0, 0.37]);
  advance(45_000);
  const rotated = activeSession();
  assert.notEqual(rotated, first);
  assert.equal(levels.at(-1), 0, 'rotating should clear the retired session level');
  levelEvent(first, 0.91);
  levelEvent(rotated, 0.62);
  assert.deepEqual(levels, [0, 0, 0.37, 0, 0.62]);
  voice.stop();
  assert.equal(levels.at(-1), 0, 'stop should clear the active level');
  levelEvent(rotated, 0.8);
  assert.deepEqual(levels, [0, 0, 0.37, 0, 0.62, 0]);
  voice.destroy();
}

// Stop while waiting for a wake phrase must cancel, never rotate/restart listening.
{
  const { voice, transcripts, states } = harness();
  voice.listen('stop-waiting');
  const id = activeSession();
  event(id, 'started');
  voice.finish();
  assert.equal(states.at(-1).phase, 'off');
  assert.equal(commands.at(-1).action, 'cancel');
  event(id, 'final', 'Hey Mo, this result is stale');
  assert.deepEqual(transcripts, []);
  voice.destroy();
}

// Ambient speech cannot escape before a standalone phrase; a completed ambient
// recognition renews, and events from its retired session are ignored.
{
  const { voice, transcripts } = harness();
  voice.listen('task-a');
  const old = activeSession();
  event(old, 'started');
  event(old, 'partial', 'hey mocha open the door');
  event(old, 'final', 'the weather is nice');
  const current = activeSession();
  assert.notEqual(current, old);
  event(old, 'partial', 'hey moeuse leak this');
  assert.deepEqual(transcripts, []);
  event(current, 'partial', 'Hey, Mo!');
  event(current, 'partial', 'Hey, Mo! open the project');
  event(current, 'final', 'Hey Mo, open the project');
  assert.deepEqual(transcripts, [['open the project', 'task-a', 'wake']]);
  voice.destroy();
}

// A final containing the complete phrase and command delivers immediately. A
// phrase-only final moves to a fresh recognition session for the next utterance.
{
  const { voice, transcripts } = harness();
  voice.listen('first-final');
  let id = activeSession();
  event(id, 'started');
  event(id, 'final', 'Hey Mo, do the thing');
  assert.deepEqual(transcripts, [['do the thing', 'first-final', 'wake']]);
  voice.listen('phrase-only');
  id = activeSession();
  event(id, 'final', 'Hey Mo');
  const commandSession = activeSession();
  assert.notEqual(commandSession, id);
  event(commandSession, 'partial', 'turn on the light');
  event(commandSession, 'final', 'turn on the light');
  assert.deepEqual(transcripts.at(-1), ['turn on the light', 'phrase-only', 'wake']);
  voice.destroy();
}

// Phrase and command in one result preserve the words after the phrase. Multiple
// final events still produce one callback, pinned to the task active at listen.
{
  const { voice, transcripts } = harness();
  voice.listen('task-pinned');
  const id = activeSession();
  event(id, 'started');
  event(id, 'partial', 'Hey Moe, summarize this');
  event(id, 'final', 'Hey Moe, summarize this');
  event(id, 'final', 'Hey Moe, summarize this again');
  assert.deepEqual(transcripts, [['summarize this', 'task-pinned', 'wake']]);
  voice.destroy();
}

// Silence finishes captured speech; cancellation removes timers/listeners and
// prevents any delayed completion. Cancel spoken after the phrase is discarded.
{
  const { voice, transcripts, states } = harness();
  voice.listen('task-silence');
  let id = activeSession();
  event(id, 'partial', 'Hey Mo, hello there');
  advance(1_799);
  assert.equal(transcripts.length, 0);
  advance(1);
  event(id, 'final', 'Hey Mo, hello there');
  assert.deepEqual(transcripts, [['hello there', 'task-silence', 'wake']]);
  voice.listen('task-cancel');
  id = activeSession();
  event(id, 'partial', 'Hey Mo cancel');
  assert.equal(transcripts.length, 1);
  assert.equal(states.at(-1).phase, 'off');
  advance(20_000);
  assert.equal(transcripts.length, 1);
}

// Once a wake phrase followed ambient words, a later recognizer revision that
// drops the phrase cannot reinterpret the ambient prefix as the command.
{
  const { voice, transcripts } = harness();
  voice.listen('no-leak');
  const id = activeSession();
  event(id, 'partial', 'the weather is nice');
  event(id, 'partial', 'the weather is nice, hey mo');
  event(id, 'partial', 'the weather is nice, turn off the lights');
  assert.deepEqual(transcripts, []);
  event(id, 'final', 'the weather is nice, hey mo, turn off the lights');
  assert.deepEqual(transcripts, [['turn off the lights', 'no-leak', 'wake']]);
  voice.destroy();
}

// Manual recording shares the native session path but tags the result correctly.
{
  const { voice, transcripts } = harness();
  voice.record('manual-task');
  const id = activeSession();
  event(id, 'started');
  event(id, 'final', 'add a note');
  assert.deepEqual(transcripts, [['add a note', 'manual-task', 'manual']]);
  voice.destroy();
}

// A partial wake phrase followed by the final phrase still starts command capture.
{
  const { voice, transcripts } = harness();
  voice.listen('paused-after-wake');
  const wake = activeSession();
  event(wake, 'partial', 'Hey Mo');
  event(wake, 'final', 'Hey Mo');
  const capture = activeSession();
  assert.notEqual(capture, wake);
  event(capture, 'started');
  event(capture, 'final', 'show the result');
  assert.deepEqual(transcripts, [['show the result', 'paused-after-wake', 'wake']]);
  voice.destroy();
  const previous = commands.length;
  voice.listen('should-not-start');
  assert.equal(commands.length, previous);
}

// iPhone recognition can revise a confirmed partial wake phrase to the short
// final forms "a mo" or "a moe". Neither final is a command: it must open a
// new command session, where ordinary speech is accepted without a wake prefix.
for (const revisedWake of ['A Mo', 'A Moe']) {
  const { voice, transcripts } = harness();
  voice.listen(`revised-${revisedWake.toLowerCase().replace(' ', '-')}`);
  const wake = activeSession();
  event(wake, 'started');
  event(wake, 'partial', 'Hey Mo');
  event(wake, 'final', revisedWake);
  assert.deepEqual(transcripts, [], `${revisedWake} must not be delivered as a wake-only command`);
  const capture = activeSession();
  assert.notEqual(capture, wake, 'a revised wake-only final must start command capture');
  event(capture, 'started');
  event(capture, 'final', 'open the current project');
  assert.deepEqual(transcripts, [['open the current project', `revised-${revisedWake.toLowerCase().replace(' ', '-')}`, 'wake']]);
  voice.destroy();
}

// A later final can remove a command that appeared in a partial. It must clear
// that stale candidate, keep waiting, and capture only the later command.
{
  const { voice, transcripts } = harness();
  voice.listen('stale-command');
  const wake = activeSession();
  event(wake, 'started');
  event(wake, 'partial', 'Hey Mo, old command');
  event(wake, 'final', 'Hey Mo');
  assert.deepEqual(transcripts, []);
  const capture = activeSession();
  assert.notEqual(capture, wake);
  event(capture, 'started');
  event(capture, 'final', 'the new command');
  assert.deepEqual(transcripts, [['the new command', 'stale-command', 'wake']]);
  voice.destroy();
}

// A final revision that drops a previously confirmed wake phrase cannot send
// either the stale partial command or unrelated revised ambient speech.
// Likewise, "a mo" alone while waiting is not a wake phrase.
{
  const { voice, transcripts } = harness();
  voice.listen('unrelated-revision');
  let wake = activeSession();
  event(wake, 'started');
  event(wake, 'partial', 'Hey Mo, old command');
  event(wake, 'final', 'the weather is nice');
  assert.deepEqual(transcripts, []);
  let waiting = activeSession();
  assert.notEqual(waiting, wake);
  event(waiting, 'started');
  event(waiting, 'partial', 'a mo');
  event(waiting, 'final', 'a mo');
  assert.deepEqual(transcripts, []);
  const afterFalseWake = activeSession();
  assert.notEqual(afterFalseWake, waiting);
  event(afterFalseWake, 'partial', 'Hey Mo, fresh command');
  event(afterFalseWake, 'final', 'Hey Mo, fresh command');
  assert.deepEqual(transcripts, [['fresh command', 'unrelated-revision', 'wake']]);
  voice.destroy();
}

// After a partial confirms the trigger, a cumulative final may revise the
// phrase itself to "A Mo" or "A Moe" while retaining the actual command. Only
// that confirmed wake prefix is stripped; the command remains intact.
for (const revisedPrefix of ['A Mo', 'A Moe']) {
  const { voice, transcripts } = harness();
  const target = `same-session-${revisedPrefix.toLowerCase().replace(' ', '-')}`;
  voice.listen(target);
  const id = activeSession();
  event(id, 'started');
  event(id, 'partial', 'Hey Mo');
  event(id, 'final', `${revisedPrefix}, show the latest status`);
  assert.deepEqual(transcripts, [['show the latest status', target, 'wake']]);
  voice.destroy();
}

// A partial wake candidate is not sufficient to deliver if the native session
// stops or errors before a final confirms the phrase and command.
for (const ending of ['stopped', 'error']) {
  const { voice, transcripts } = harness();
  voice.listen(`unconfirmed-${ending}`);
  const id = activeSession();
  event(id, 'started');
  event(id, 'partial', 'Hey Mo, maybe this is ambient');
  event(id, ending, undefined, ending === 'error' ? 'Recognition interrupted.' : undefined);
  assert.deepEqual(transcripts, [], `an unconfirmed wake partial must not deliver on ${ending}`);
  voice.destroy();
}

// Default wake behavior remains review-first: a completed command is returned
// immediately with the original three callback arguments.
{
  const { voice, transcripts } = harness();
  voice.listen('review-first');
  const id = activeSession();
  event(id, 'started');
  event(id, 'final', 'Hey Mo, prepare a summary');
  assert.deepEqual(transcripts, [['prepare a summary', 'review-first', 'wake']]);
  voice.destroy();
}

// Auto-send stays quiet for a full eleven seconds after the latest changed
// speech. Native final boundaries retain settled words, identical partial/final
// pairs do not duplicate text or extend the timer, and a new segment resets it.
{
  const { voice, transcripts } = harness();
  voice.listen('auto-multi', true);
  const wake = activeSession();
  event(wake, 'started');
  event(wake, 'partial', 'Hey Mo');
  event(wake, 'final', 'A Mo');
  assert.deepEqual(transcripts, []);

  const first = activeSession();
  event(first, 'started');
  event(first, 'partial', 'show the current');
  event(first, 'partial', 'show the current project');
  event(first, 'final', 'show the current project');
  const second = activeSession();
  assert.notEqual(second, first);
  advance(5_000);
  event(second, 'started');
  event(second, 'partial', 'and list recent files');
  event(second, 'final', 'and list recent files');
  assert.deepEqual(transcripts, []);
  advance(10_999);
  assert.deepEqual(transcripts, []);
  advance(1);
  assert.deepEqual(transcripts, [['show the current project and list recent files', 'auto-multi', 'wake', true]]);
  voice.destroy();
}

// An early native final after the wake phrase alone must create a command
// session; ordinary command text is then captured and only auto-sent at 11s.
{
  const { voice, transcripts } = harness();
  voice.listen('auto-wake-only', true);
  const wake = activeSession();
  event(wake, 'started');
  event(wake, 'final', 'Hey Mo');
  const capture = activeSession();
  assert.notEqual(capture, wake);
  event(capture, 'started');
  event(capture, 'final', 'open the dashboard');
  assert.deepEqual(transcripts, []);
  advance(10_999);
  assert.deepEqual(transcripts, []);
  advance(1);
  assert.deepEqual(transcripts, [['open the dashboard', 'auto-wake-only', 'wake', true]]);
  voice.destroy();
}

// A fresh no-speech native segment after a settled command can rotate without
// sending early or losing the original eleven-second deadline. Permission and
// availability failures still stop visibly and never auto-send.
{
  for (const noSpeechError of ['No speech detected', 'Speech recognition did not return a final transcript.']) {
    const { voice, transcripts } = harness();
    const taskId = `auto-empty-${noSpeechError.startsWith('No speech') ? 'detected' : 'final'}`;
    voice.listen(taskId, true);
    const wake = activeSession();
    event(wake, 'started');
    event(wake, 'partial', 'Hey Mo, check the build');
    event(wake, 'final', 'Hey Mo, check the build');
    const empty = activeSession();
    event(empty, 'started');
    advance(4_000);
    event(empty, 'error', undefined, noSpeechError);
    const renewed = activeSession();
    assert.notEqual(renewed, empty);
    assert.deepEqual(transcripts, []);
    advance(6_999);
    assert.deepEqual(transcripts, []);
    advance(1);
    assert.deepEqual(transcripts, [['check the build', taskId, 'wake', true]]);
    voice.destroy();
  }

  const denied = harness();
  denied.voice.listen('denied-auto', true);
  event(activeSession(), 'error', undefined, 'Microphone access was denied.');
  assert.deepEqual(denied.transcripts, []);
  assert.equal(denied.states.at(-1).phase, 'error');
  denied.voice.destroy();
}

// Manual auto mode accumulates utterances across early finals. Explicit finish
// overrides auto preference and leaves the result for review (three arguments).
{
  const { voice, transcripts } = harness();
  voice.record('manual-auto', true);
  const first = activeSession();
  event(first, 'started');
  event(first, 'partial', 'summarize');
  event(first, 'final', 'summarize');
  const second = activeSession();
  event(second, 'started');
  event(second, 'partial', 'the notes');
  event(second, 'final', 'the notes');
  advance(10_999);
  assert.deepEqual(transcripts, []);
  advance(1);
  assert.deepEqual(transcripts, [['summarize the notes', 'manual-auto', 'manual', true]]);

  voice.record('manual-review', true);
  const review = activeSession();
  event(review, 'started');
  event(review, 'partial', 'dictate this');
  voice.finish();
  event(review, 'final', 'dictate this');
  assert.deepEqual(transcripts.at(-1), ['dictate this', 'manual-review', 'manual']);
  voice.destroy();
}

// Stopping or hiding during the auto-silence window cancels the pending send;
// late events from the retired native session remain ignored.
{
  const { voice, transcripts } = harness();
  voice.listen('cancel-auto', true);
  const wake = activeSession();
  event(wake, 'started');
  event(wake, 'partial', 'Hey Mo, keep this private');
  voice.stop();
  event(wake, 'final', 'Hey Mo, keep this private');
  advance(12_000);
  assert.deepEqual(transcripts, []);

  voice.listen('hidden-auto', true);
  const hidden = activeSession();
  event(hidden, 'started');
  event(hidden, 'partial', 'Hey Mo, do not send');
  document.visibilityState = 'hidden';
  for (const listener of [...documentListeners]) listener();
  event(hidden, 'final', 'Hey Mo, do not send');
  advance(12_000);
  assert.deepEqual(transcripts, []);
  document.visibilityState = 'visible';
  voice.destroy();
}

console.log('Companion voice trigger boundaries, task pinning, stale sessions, cancellation, silence, and manual capture passed.');
