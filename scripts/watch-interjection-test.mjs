import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
const source = readFileSync(new URL('../src/lib/watch-interjection.ts', import.meta.url), 'utf8');
const exports = {};
runInNewContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, { exports });
const { captureWatchInterjection, projectWatchInterjection } = exports;
const assistant = (id, text, createdAt, streamStatus = 'complete', taskId = 'owner') => ({ id, taskId, role: 'assistant', text, createdAt, streamStatus });
const user = (id = 'ack', extra = {}) => ({ id, taskId: 'owner', role: 'user', text: 'Please adjust', createdAt: 101, ...extra });
const snapshot = { messages: [assistant('old', 'Existing reply', 50), assistant('stream', 'Working', 90, 'streaming'), user('old-user', { createdAt: 60 })], queuedMessages: [] };
let state = captureWatchInterjection(snapshot, 'owner', 'Please adjust', 100);
assert.equal(state.assistantBaseline.length, 2);
assert.equal(projectWatchInterjection(snapshot, state).response, null);
assert.equal(projectWatchInterjection(snapshot, state).state, state);
snapshot.messages.push(assistant('other', 'Another task reply', 102, 'complete', 'other'));
assert.equal(projectWatchInterjection(snapshot, state).response, null);
snapshot.messages.push(user('peer', { senderAgentId: 'peer' }));
assert.equal(projectWatchInterjection(snapshot, state).state.acknowledged, false);
snapshot.messages.push(user());
let projection = projectWatchInterjection(snapshot, state);
assert.equal(projection.state.acknowledged, true);
assert.equal(projection.response, null);
assert.equal(projection.pending, true);
state = projection.state;
// A late-loaded old row is history even when its ID was absent from capture.
snapshot.messages.push(assistant('old-unloaded', 'Older history', 10));
assert.equal(projectWatchInterjection(snapshot, state).response, null);
snapshot.messages = snapshot.messages.map(message => message.id === 'stream' ? { ...message, text: 'Working on the adjustment' } : message);
projection = projectWatchInterjection(snapshot, state);
assert.equal(projection.response.id, 'stream');
assert.equal(projection.status, 'Receiving reply…');
assert.equal(projection.finished, false);
state = projection.state;
// Acknowledgement may disappear from a compact snapshot while the same reply streams.
snapshot.messages = [assistant('stream', 'Adjusted result', 90, 'complete')];
projection = projectWatchInterjection(snapshot, state);
assert.equal(projection.response.text, 'Adjusted result');
assert.equal(projection.finished, true);
assert.equal(projection.pending, false);
state = projection.state;
snapshot.messages = [];
assert.equal(projectWatchInterjection(snapshot, state).response.text, 'Adjusted result');
snapshot.messages = [assistant('new', 'Latest result', 120)];
projection = projectWatchInterjection(snapshot, state);
assert.equal(projection.response.id, 'new');
state = projection.state;
snapshot.messages = [assistant('stream', 'Older streaming row', 90)];
assert.equal(projectWatchInterjection(snapshot, state).response.id, 'new');
// Captures and projections are independent and preserve their inputs.
const fresh = captureWatchInterjection({ messages: [], queuedMessages: [] }, 'owner', 'Please adjust', 100);
const queue = { id: 'queue', taskId: 'owner', text: 'Please adjust', createdAt: 101, status: 'sending', origin: 'steering', error: null };
projection = projectWatchInterjection({ messages: [], queuedMessages: [queue] }, fresh);
assert.equal(projection.status, 'Steering into the current turn…');
assert.equal(fresh.acknowledged, false);
assert.equal(fresh.response, null);
assert.equal(projectWatchInterjection({ messages: [], queuedMessages: [{ ...queue, status: 'queued' }] }, fresh).status, 'Steer unavailable · queued for the next turn');
projection = projectWatchInterjection({ messages: [], queuedMessages: [{ ...queue, status: 'error', error: 'Fixture failure' }] }, fresh);
assert.equal(projection.status, 'Fixture failure');
assert.equal(projection.finished, true);
assert.equal(projection.pending, false);
// A phone clock ahead of the desktop must not discard a backend-timed reply.
const skewed = captureWatchInterjection({ messages: [], queuedMessages: [] }, 'owner', 'Please adjust', 999999);
const skewedSnapshot = { messages: [user(), assistant('clock-safe', 'Backend reply', 102)], queuedMessages: [] };
const skewedReply = projectWatchInterjection(skewedSnapshot, skewed);
assert.equal(skewedReply.response.id, 'clock-safe');
assert.equal(skewedReply.state.acknowledgedAt, 101);
// A subsequent owner turn closes correlation; its response never replaces ours.
const laterTurn = user('next-owner-turn', { text: 'A different request', createdAt: 110 });
const fenced = projectWatchInterjection({ messages: [...skewedSnapshot.messages, laterTurn, assistant('unrelated', 'Later turn result', 120)], queuedMessages: [] }, skewedReply.state);
assert.equal(fenced.response.id, 'clock-safe');
assert.equal(fenced.state.closed, true);
assert.equal(fenced.finished, true);
assert.equal(projectWatchInterjection({ messages: [assistant('unrelated', 'Later turn result', 120)], queuedMessages: [] }, fenced.state).response.id, 'clock-safe');
// Even a repeated text is a new owner turn once its ID differs from the ack.
const repeated = projectWatchInterjection({ messages: [...skewedSnapshot.messages, user('repeated', { createdAt: 110 }), assistant('later', 'Wrong reply', 120)], queuedMessages: [] }, skewedReply.state);
assert.equal(repeated.state.closed, true);
assert.equal(repeated.response.id, 'clock-safe');
// Peer traffic does not close the owner's exchange.
const peerTurn = projectWatchInterjection({ messages: [...skewedSnapshot.messages, user('peer-turn', { senderAgentId: 'peer', createdAt: 110 })], queuedMessages: [] }, skewedReply.state);
assert.equal(peerTurn.state.closed, false);
// If one compact snapshot first reveals both turns, take only a reply between them.
const firstFence = projectWatchInterjection({ messages: [...skewedSnapshot.messages, laterTurn, assistant('unrelated', 'Later turn result', 120)], queuedMessages: [] }, skewed);
assert.equal(firstFence.response.id, 'clock-safe');
assert.equal(firstFence.state.closed, true);
const noReply = projectWatchInterjection({ messages: [user(), laterTurn, assistant('unrelated', 'Later turn result', 120)], queuedMessages: [] }, skewed);
assert.equal(noReply.response, null);
assert.equal(noReply.pending, false);
assert.equal(noReply.finished, true);
// The same-ID streaming row must also stop changing after a later owner turn.
const streamingBase = captureWatchInterjection({ messages: [assistant('streaming', 'Before', 90, 'streaming')], queuedMessages: [] }, 'owner', 'Please adjust', 999999);
const streamingReply = projectWatchInterjection({ messages: [user(), assistant('streaming', 'Our partial reply', 90, 'streaming')], queuedMessages: [] }, streamingBase);
const streamingFence = projectWatchInterjection({ messages: [user(), laterTurn, assistant('streaming', 'Changed in a later turn', 90, 'complete')], queuedMessages: [] }, streamingReply.state);
assert.equal(streamingFence.response.text, 'Our partial reply');
assert.equal(streamingFence.pending, false);
console.log('Watch interjection projection checks passed');
