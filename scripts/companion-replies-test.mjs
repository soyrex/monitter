import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/companion-replies.ts', import.meta.url), 'utf8')
  .replace(/^import type .*\n/, '');
const js = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exports = {};
new Function('exports', js)(exports);
const { createCompanionReplyInbox } = exports;

function task(id, status = 'completed', extra = {}) {
  return {
    id, agentId: 'agent', title: id, nativeSessionId: null, archived: false, status,
    createdAt: 1, updatedAt: 1, parentTaskId: null, channelId: null, projectId: null,
    hostId: 'local', cwd: '/tmp', provider: 'codex', model: 'test', sandbox: 'workspace-write',
    ...extra,
  };
}

function message(id, taskId, text, createdAt, extra = {}) {
  return { id, taskId, role: 'assistant', text, createdAt, phase: 'final_answer', streamStatus: 'complete', ...extra };
}

const alpha = task('alpha');
const beta = task('beta');

// Startup presents only the newest final from the initially selected chat and
// considers every historical final seen, including other eligible chats.
{
  const inbox = createCompanionReplyInbox();
  const history = [
    message('alpha-old', alpha.id, 'Older Alpha reply', 10),
    message('beta-old', beta.id, 'Historical Beta reply', 15),
    message('alpha-new', alpha.id, 'Newest Alpha reply', 20),
  ];
  let result = inbox.sync(history, [alpha, beta], alpha.id);
  assert.equal(result.message?.id, 'alpha-new');
  assert.equal(result.remaining, 0);
  result = inbox.dismiss();
  assert.equal(result.message, null);
  result = inbox.sync(history, [alpha, beta], alpha.id);
  assert.equal(result.message, null, 'repeated sync must not resurrect dismissed history');

  const betaReply = message('beta-next', beta.id, 'New Beta reply', 30);
  result = inbox.sync([...history, betaReply], [alpha, beta], alpha.id);
  assert.equal(result.message?.id, betaReply.id, 'new replies from any eligible chat enter the inbox');
}

// New finals queue globally by age, deduplicate through repeated snapshots, and
// accept a streaming message becoming final without requiring a new ID.
{
  const inbox = createCompanionReplyInbox();
  inbox.sync([], [alpha, beta], alpha.id);
  const streaming = message('same-id', beta.id, 'Beta finishing', 50, { streamStatus: 'streaming' });
  assert.equal(inbox.sync([streaming], [alpha, beta], alpha.id).message, null);
  const alphaReply = message('alpha-queued', alpha.id, 'Alpha finished first', 60);
  const finalizedStreaming = { ...streaming, streamStatus: 'complete' };
  let result = inbox.sync([alphaReply, finalizedStreaming], [alpha, beta], alpha.id);
  assert.equal(result.message?.id, 'same-id', 'older finalized stream result should be first');
  assert.equal(result.remaining, 1);
  result = inbox.sync([alphaReply, finalizedStreaming], [alpha, beta], alpha.id);
  assert.equal(result.message?.id, 'same-id');
  assert.equal(result.remaining, 1, 'reconnect/repeated snapshots must not duplicate queued replies');
  result = inbox.dismiss();
  assert.equal(result.message?.id, alphaReply.id);
  assert.equal(inbox.dismiss().message, null);
  assert.equal(inbox.sync([alphaReply, finalizedStreaming], [alpha, beta], alpha.id).message, null);
}

// Only final assistant replies qualify: commentary, interrupted/incomplete,
// user/system messages, and blank text stay out. Explicit final_answer accepts
// an omitted streamStatus; legacy unphased fallback requires a completed task
// and the last message in that task.
{
  const inbox = createCompanionReplyInbox();
  inbox.sync([], [alpha, beta], alpha.id);
  const blank = message('blank', alpha.id, '  ', 1);
  const commentary = message('commentary', alpha.id, 'progress', 2, { phase: 'commentary' });
  const streaming = message('streaming', alpha.id, 'still writing', 3, { streamStatus: 'streaming' });
  const interrupted = message('interrupted', alpha.id, 'stopped early', 4, { streamStatus: 'interrupted' });
  const user = { ...message('user', alpha.id, 'user text', 5), role: 'user' };
  const legacyGood = message('legacy-good', alpha.id, 'Legacy completed response', 6, { phase: undefined });
  const legacyNotLast = message('legacy-not-last', beta.id, 'Old legacy response', 7, { phase: undefined });
  const betaUser = { id: 'beta-user', taskId: beta.id, role: 'user', text: 'follow-up', createdAt: 8 };
  const explicitNoStatus = message('explicit-final', beta.id, 'Explicit final response', 9, { streamStatus: undefined });
  const tieTask = task('tie-task');
  const tieOld = message('tie-old', tieTask.id, 'Earlier assistant row', 10, { phase: undefined });
  const tieUser = { id: 'tie-user', taskId: tieTask.id, role: 'user', text: 'later row, same millisecond', createdAt: 10 };
  const tieNew = message('tie-new', tieTask.id, 'Last assistant row', 10, { phase: undefined });
  const userTieTask = task('user-tie-task');
  const beforeUser = message('before-user', userTieTask.id, 'Must stay hidden', 11, { phase: undefined });
  const sameTimeUser = { id: 'same-time-user', taskId: userTieTask.id, role: 'user', text: 'follow-up', createdAt: 11 };
  const messages = [blank, commentary, streaming, interrupted, user, legacyGood, legacyNotLast, betaUser, explicitNoStatus, tieOld, tieUser, tieNew, beforeUser, sameTimeUser];
  const result = inbox.sync(messages, [alpha, beta, tieTask, userTieTask], alpha.id);
  assert.equal(result.message?.id, 'legacy-good');
  assert.equal(result.remaining, 2);
  assert.equal(inbox.dismiss().message?.id, 'explicit-final');
  assert.equal(inbox.dismiss().message?.id, 'tie-new');
}

// The current queued item's content refreshes by ID, while deleted messages
// and tasks that cease to be eligible are pruned from the queue.
{
  const inbox = createCompanionReplyInbox();
  inbox.sync([], [alpha, beta], alpha.id);
  const first = message('first', alpha.id, 'Before update', 10);
  const second = message('second', beta.id, 'Will be pruned', 20);
  inbox.sync([first, second], [alpha, beta], alpha.id);
  const refreshed = { ...first, text: 'Updated reply body' };
  let result = inbox.sync([refreshed, second], [alpha, beta], alpha.id);
  assert.equal(result.message?.text, 'Updated reply body');
  assert.equal(result.remaining, 1);
  result = inbox.sync([refreshed], [alpha, beta], alpha.id);
  assert.equal(result.message?.id, first.id);
  assert.equal(result.remaining, 0, 'deleted queued message should be removed');
  result = inbox.sync([refreshed, second], [alpha], alpha.id);
  assert.equal(result.message?.id, first.id);
  assert.equal(result.remaining, 0, 'a reply from a no-longer-eligible task should be pruned');
}

// Dismiss advances exactly once and is stable across data refreshes.
{
  const inbox = createCompanionReplyInbox();
  inbox.sync([], [alpha, beta], alpha.id);
  const first = message('dismiss-first', alpha.id, 'First', 10);
  const second = message('dismiss-second', beta.id, 'Second', 20);
  inbox.sync([first, second], [alpha, beta], alpha.id);
  assert.equal(inbox.dismiss().message?.id, second.id);
  assert.equal(inbox.sync([first, second], [alpha, beta], alpha.id).message?.id, second.id);
  assert.equal(inbox.dismiss().message, null);
  assert.equal(inbox.sync([first, second], [alpha, beta], alpha.id).message, null);
}

console.log('companion reply inbox tests passed');
