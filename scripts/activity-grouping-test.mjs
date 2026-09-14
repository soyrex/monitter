import assert from 'node:assert/strict';
import { CANCELLATION_EVENT_TITLE, contextCompactionPhase, groupConversationActivity, isCancellationEvent, isCancellationMessage, isNativeMessageTransportArtifact, isShellActivity, readableToolDetail, reasoningSummary, showThinkingFallback, toolCategory, toolFileChanges, toolPresentation } from '../src/lib/activity-grouping.ts';

const event = (id, createdAt, title, detail) => ({ id, taskId: 'task', kind: 'tool', title, detail, createdAt });
const message = (id, createdAt) => ({ id, taskId: 'task', role: 'assistant', text: 'reply', createdAt, attachments: [] });
const approval = (id, createdAt, resolvedAt = null) => ({ id, taskId: 'task', provider: 'codex', runId: `run:${id}`, tool: 'computer', summary: 'Allow computer use', detail: '', risk: 'medium', status: 'approved', createdAt, resolvedAt, decision: 'approve_once' });
const groups = (messages, events, compress = false) => groupConversationActivity(messages, events, compress).filter(item => item.type === 'tool-group');

const nativeTransport = (id, createdAt, type) => event(id, createdAt, type, JSON.stringify({ type, id: `native-${id}` }));
const transportTimeline = groupConversationActivity(
  [{ ...message('user-bubble', 1), role: 'user' }, message('assistant-bubble', 4)],
  [nativeTransport('user-echo', 1.1, 'userMessage'), event('real-tool', 2, 'Run command', '{"type":"command_execution"}'), nativeTransport('agent-echo', 3, 'agentMessage')],
);
assert.deepEqual(transportTimeline.map(item => item.type), ['message', 'tool-group', 'message'], 'Native conversation lifecycle echoes are represented by their ordinary bubbles, not tool rows');
assert.equal(isNativeMessageTransportArtifact(nativeTransport('user-echo', 1, 'userMessage')), true);
assert.equal(isNativeMessageTransportArtifact(nativeTransport('agent-echo', 1, 'agentMessage')), true);
assert.equal(isNativeMessageTransportArtifact(event('named-agent-message', 1, 'agentMessage', '{"type":"command_execution","command":"agentMessage"}')), false, 'A real tool is never hidden by its display title');
assert.equal(isNativeMessageTransportArtifact(event('unstructured', 1, 'userMessage', 'plain text')), false, 'Legacy/unstructured events remain visible rather than being guessed away');
const cancellation = { id: 'cancelled', taskId: 'task', kind: 'status', title: CANCELLATION_EVENT_TITLE, detail: '', createdAt: 5 };
assert.equal(isCancellationEvent(cancellation), true);
assert.equal(isCancellationEvent({ ...cancellation, title: 'Cancellation requested' }), false);
assert.equal(isCancellationMessage({ id: 'cancelled-message', taskId: 'task', role: 'system', text: CANCELLATION_EVENT_TITLE, createdAt: 5, attachments: [] }), true);
assert.equal(isCancellationMessage({ id: 'agent-message', taskId: 'task', role: 'assistant', text: CANCELLATION_EVENT_TITLE, createdAt: 5, attachments: [] }), false);
assert.deepEqual(groupConversationActivity([{ id: 'cancelled-message', taskId: 'task', role: 'system', text: CANCELLATION_EVENT_TITLE, createdAt: 5, attachments: [] }], []).map(item => item.type), ['message'], 'The durable cancellation transcript record remains an inline conversation item');
console.log('native message transport artifacts are absorbed by ordinary chat bubbles');

// Screenshot-style empty and real web searches remain two raw entries together.
let result = groups([], [event('search-empty', 1, 'Web search', '{"tool":"web_search","query":""}'), event('search-query', 2, 'Web search', '{"tool":"web_search","query":"cats"}')]);
assert.equal(result.length, 1); assert.deepEqual(result[0].values.map(item => item.id), ['search-empty', 'search-query']);

// Different query strings do not alter the stable tool identity.
result = groups([], [event('one', 1, 'Web search', '{"tool":"web_search","query":"one"}'), event('two', 2, 'Web search', '{"tool":"web_search","query":"two"}')]);
assert.equal(result.length, 1);

// Messages are hard turn boundaries. Reasoning only separates tools when
// compression is off; compressed summaries span thinking time.
assert.equal(groups([message('reply', 2)], [event('before', 1, 'Web search', '{}'), event('after', 3, 'Web search', '{}')]).length, 2);
assert.equal(groups([], [event('before', 1, 'Web search', '{}'), { ...event('reasoning', 2, 'Reasoning', 'thinking'), kind: 'reasoning' }, event('after', 3, 'Web search', '{}')]).length, 2);
result = groups([], [event('before', 1, 'Web search', '{}'), { ...event('reasoning', 2, 'Reasoning', 'thinking'), kind: 'reasoning' }, event('after', 3, 'Web search', '{}')], true);
assert.equal(result.length, 1);
assert.equal(result[0].values.length, 2);

// Generic tool_result envelopes cannot collapse because their type/name is not a tool identity.
assert.equal(groups([], [event('a', 1, 'Tool result', '{"type":"tool_result","name":"first"}'), event('b', 2, 'Tool result', '{"type":"tool_result","name":"second"}')]).length, 2);

// Native command executions group despite distinct command arguments.
result = groups([], [event('ls', 1, 'Run command', '{"type":"command_execution","command":"ls"}'), event('pwd', 2, 'Run command', '{"type":"command_execution","command":"pwd"}')]);
assert.equal(result.length, 1);

// Incidental result names never replace a specific tool title.
assert.equal(groups([], [event('web', 1, 'Web search', '{"name":"first"}'), event('shell', 2, 'Run command', '{"name":"first"}')]).length, 2);
// Lifecycle updates can alternate native JSON, generic JSON, and plain text.
assert.equal(groups([], [event('a', 1, 'web_search', ''), event('b', 2, 'Web search', '{"type":"web_search","query":"one"}'), event('c', 3, 'web_search', '{}'), event('d', 4, 'Web search', 'second query')]).length, 1);
assert.equal(groups([], [event('a', 1, 'Tool result', ''), event('b', 2, 'Tool result', 'output')]).length, 2);
// Related connector methods form a single series; other services remain separate.
assert.equal(groups([], [event('a',1,'gmail.search_emails','query'),event('b',2,'gmail.read_email','email'),event('c',3,'gmail.read_email','email2')]).length,1);
assert.equal(groups([], [event('a',1,'gmail.search_emails','query'),event('b',2,'slack.search','query')]).length,2);
assert.equal(groups([], [event('a',1,'mcp__gmail__search_emails','query'),event('b',2,'mcp__gmail__read_email','email')]).length,1);
console.log('activity grouping assertions passed');

const approvalTimeline = groupConversationActivity([message('before', 1), message('after', 5)], [event('tool', 3, 'Run command', '')], false, [approval('resolved', 2, 4)]);
assert.deepEqual(approvalTimeline.map(item => item.type), ['message', 'tool-group', 'approval', 'message']);
assert.equal(approvalTimeline[2].value.id, 'resolved');
assert.equal(groupConversationActivity([], [], false, [{ ...approval('pending', 1), status: 'pending', resolvedAt: null }]).length, 0, 'Pending approvals belong in the dock, never the historical timeline');
console.log('approval resolution appears inline at its resolution timestamp');

const mixed = [event('cmd1',1,'/bin/zsh -lc ls',''),event('web',2,'web_search',''),event('cmd2',3,'/bin/zsh -lc pwd','')];
assert.equal(groupConversationActivity([],mixed,true).length,2);
assert.deepEqual(groupConversationActivity([],mixed,true).map(item=>item.type === 'tool-group' ? item.values.map(value=>value.id) : []),[['web'],['cmd1','cmd2']]);
assert.equal(groupConversationActivity([message('boundary',2.5)],mixed,true).length,4);
assert.equal(groupConversationActivity([],mixed,false).length,3);

assert.equal(isShellActivity(event('shell',1,'/bin/zsh -lc git status','')),true);
assert.equal(isShellActivity(event('shell',1,'Run command','')),true);
assert.equal(isShellActivity(event('shell',1,'git status','{"type":"command_execution"}')),true);
assert.equal(isShellActivity(event('search',1,'web_search','')),false);

const compaction = (id, at, phase) => event(`compaction-${id}-${phase}`, at, 'ContextCompaction', JSON.stringify({ type: 'ContextCompaction', id, monitterPhase: phase }));
let compactions = groupConversationActivity([], [compaction('one', 1, 'started'), compaction('one', 15_300, 'completed')], true);
assert.equal(compactions.length, 1, 'A matching compaction lifecycle stays together when compression is enabled');
assert.equal(compactions[0].values.length, 2);
assert.equal(contextCompactionPhase(compactions[0].values[0]), 'started');
assert.equal(contextCompactionPhase(compactions[0].values[1]), 'completed');
compactions = groupConversationActivity([], [compaction('one', 1, 'started'), event('tool', 2, 'Run command', '{"type":"command_execution"}'), compaction('one', 15_300, 'completed')], true);
assert.equal(compactions.length, 3, 'Compression must not merge a compaction lifecycle with unrelated tools');
compactions = groupConversationActivity([], [compaction('one', 1, 'started'), compaction('one', 2, 'completed'), compaction('two', 3, 'started'), compaction('two', 4, 'completed')], true);
assert.equal(compactions.length, 2, 'Separate native compactions stay separate even when adjacent');
console.log('context compaction lifecycle grouping assertions passed');

const reasoning = (id, at, detail = '') => ({ ...event(id, at, 'Reasoning', detail), kind: 'reasoning' });
const emptyPayload = '{"content":[],"id":"rs_123","summary":[],"type":"reasoning"}';
for (const empty of ['', '  ', 'null', '{}', '[]', emptyPayload, '{"summary":[{"type":"summary_text","text":"  "}],"encrypted_content":"opaque"}']) {
  assert.equal(reasoningSummary(empty), '');
}
assert.equal(reasoningSummary(' A real plain-text summary. '), 'A real plain-text summary.');
assert.equal(reasoningSummary('{"summary":[{"type":"summary_text","text":"First point"},{"type":"summary_text","text":"Second point"}],"id":"secret-id"}'), 'First point\n\nSecond point');
assert.equal(reasoningSummary('{"summary":["Summary"],"content":["Duplicate content"]}'), 'Summary');
assert.equal(reasoningSummary('{"text":"Readable text"}'), 'Readable text');
assert.equal(reasoningSummary('{"content":[{"type":"text","text":"Readable content"}]}'), 'Readable content');
assert.equal(reasoningSummary('An unfinished { paragraph'), 'An unfinished { paragraph');

for (const compress of [false, true]) {
  const blanks = [reasoning('r1', 1, emptyPayload), reasoning('r2', 2), reasoning('r3', 3, 'null')];
  const combined = groupConversationActivity([], blanks, compress);
  assert.equal(combined.length, 1);
  assert.equal(combined[0].type, 'reasoning-group');
  assert.deepEqual(combined[0].values, blanks);
  assert.equal(blanks[0].detail, emptyPayload, 'Stored payloads must remain untouched');
  assert.equal(groupConversationActivity([], [blanks[0], event('tool', 2, 'Run command', '{}'), blanks[2]], compress).length, 2, 'Later tool activity replaces an earlier pending-thinking row');
  const withSummary = groupConversationActivity([], [blanks[0], reasoning('summary', 2, 'Actual summary'), blanks[2]], compress);
  assert.equal(withSummary.length, 1, 'Neighboring pending and completed reasoning updates share one bubble');
  assert.equal(withSummary[0].values.length, 3);
  assert.deepEqual(groupConversationActivity([message('reply', 2)], [blanks[0], blanks[2]], compress).map(item => item.type), ['message', 'reasoning-group']);
  assert.deepEqual(groupConversationActivity([message('reply', 4)], blanks, compress).map(item => item.type), ['message']);
  assert.equal(groupConversationActivity([{ ...message('reply', 4), role: 'user' }], blanks, compress).length, 1, 'A new user message also replaces old placeholders');
  assert.equal(groupConversationActivity([{ ...message('reply', 4), text: '', streamStatus: 'streaming' }], blanks, compress).length, 2, 'An empty streaming envelope is not a visible replacement');
  assert.equal(groupConversationActivity([{ ...message('reply', 4), text: '', attachments: [{ id: 'image' }] }], blanks, compress).length, 1, 'Attachment-only replies replace the status');
  assert.equal(groupConversationActivity([message('reply', 3)], blanks, compress).length, 1, 'Equal-timestamp replies replace placeholders too');
  assert.equal(groupConversationActivity([{ ...message('reply', 4), taskId: 'other-task' }], blanks, compress).length, 2, 'Another task cannot clear this status');
  assert.deepEqual(groupConversationActivity([message('reply', 4)], [reasoning('summary', 2, 'Actual summary')], compress).map(item => item.type), ['reasoning-group', 'message']);
  assert.equal(groupConversationActivity([], [blanks[0], { ...blanks[1], taskId: 'other-task' }], compress).length, 2);
}
const summaries = groupConversationActivity([], [reasoning('s1', 1, 'First check.'), reasoning('s2', 2, 'Second check.'), reasoning('s3', 3, 'Second check.')]);
assert.equal(summaries.length, 1, 'Consecutive visible reasoning summaries render as one bubble');
assert.deepEqual(summaries[0].values.map(value=>value.id), ['s1','s2','s3']);
assert.deepEqual(groupConversationActivity([message('boundary', 2)], [reasoning('before', 1, 'Before.'), reasoning('after', 3, 'After.')]).map(item=>item.type), ['reasoning-group','message','reasoning-group']);
const acrossThinking = groupConversationActivity([], [
  event('web-1', 1, 'Web search', '{}'), event('web-2', 2, 'Web search', '{}'),
  reasoning('pending-1', 3),
  event('web-3', 4, 'Web search', '{}'), event('web-4', 5, 'Web search', '{}'),
  compaction('middle', 6, 'started'), compaction('middle', 7, 'completed'),
  reasoning('pending-2', 8),
  event('web-5', 9, 'Web search', '{}'), event('web-6', 10, 'Web search', '{}'),
], true);
assert.deepEqual(acrossThinking.map(item=>item.type), ['tool-group','tool-group']);
assert.equal(acrossThinking[0].values.every(item=>item.title === 'ContextCompaction'), true);
assert.deepEqual(acrossThinking[1].values.map(item=>item.id), ['web-1','web-2','web-3','web-4','web-5','web-6']);
console.log('reasoning summary normalization and consecutive grouping assertions passed');

assert.equal(showThinkingFallback([], true), true);
assert.equal(showThinkingFallback([], false), false);
assert.equal(showThinkingFallback([], true, true), false);
assert.equal(showThinkingFallback(groupConversationActivity([], [reasoning('active', 1)]), true), false, 'Reasoning already provides the waiting row');
assert.equal(showThinkingFallback([{ type: 'message', value: message('reply', 2) }], true), false, 'A reply replaces the waiting row even before run completion');
assert.equal(showThinkingFallback([{ type: 'message', value: { ...message('sent', 3), role: 'user' } }], true), true);
assert.equal(showThinkingFallback([{ type: 'reasoning-group', values: [reasoning('old', 1, 'Old summary')] }, { type: 'message', value: { ...message('sent', 3), role: 'user' } }], true), true, 'Historical reasoning does not suppress a later turn waiting state');
console.log('single thinking status and reply/approval suppression assertions passed');

const category = (title, detail = '') => toolCategory(event(`tool-${title}`, 0, title, detail));
assert.equal(category('fileChange', '{"type":"fileChange"}'), 'edit');
assert.equal(category('file_change', '{"type":"file_change"}'), 'edit');
assert.equal(category('apply_patch'), 'edit');
assert.equal(category('commandExecution'), 'shell');
assert.equal(category('imageView'), 'image');
assert.equal(category('ToolSearch'), 'tool_search');
assert.equal(category('ScheduleWakeup'), 'schedule');
assert.equal(category('gmail.read_email'), 'mcp');
assert.deepEqual(toolPresentation(event('file', 0, 'fileChange', '{"type":"fileChange"}'), true), { icon: 'file-pen', label: 'Edit a file' });
assert.deepEqual(toolPresentation(event('file', 0, 'fileChange', '{"type":"fileChange"}'), false), { icon: 'file-pen', label: 'Edited a file' });
assert.equal(readableToolDetail(event('file', 0, 'fileChange', JSON.stringify({ type: 'fileChange', id: 'opaque', changes: [{ path: '/tmp/example.txt', kind: { type: 'update' }, diff: '@@ -1 +1 @@\n-old\n+new\n' }] }))), 'Updated /tmp/example.txt (+1 −1)');
const encodedChange = JSON.stringify(JSON.stringify({ type: 'fileChange', changes: [{ path: '/tmp/double-encoded.txt', kind: { type: 'update' }, diff: '@@ -1 +1 @@\n-before\n+after\n' }] }));
assert.equal(readableToolDetail(event('encoded-file', 0, 'fileChange', encodedChange)), 'Updated /tmp/double-encoded.txt (+1 −1)');
assert.equal(toolFileChanges(event('encoded-file', 0, 'fileChange', encodedChange))[0].path, '/tmp/double-encoded.txt');
const wrappedChange = JSON.stringify({ content: [{ type: 'text', text: JSON.stringify({ changes: [{ path: '/tmp/wrapped.txt', kind: { type: 'update' }, diff: '@@ -1 +1 @@\n-old\n+new\n' }] }) }] });
assert.equal(readableToolDetail(event('wrapped-file', 0, 'fileChange', wrappedChange)), 'Updated /tmp/wrapped.txt (+1 −1)');
assert.equal(toolFileChanges(event('wrapped-file', 0, 'fileChange', wrappedChange))[0].path, '/tmp/wrapped.txt');
const truncatedChange = '{"changes":[{"diff":"@@ -1 +1 @@\\n-old\\n+new\\n","kind":{"type":"update"},"path":"/tmp/large-file.ts… [truncated]';
assert.equal(readableToolDetail(event('truncated-file', 0, 'fileChange', truncatedChange)), 'Updated /tmp/large-file.ts (+1 −1)');
assert.equal(toolFileChanges(event('truncated-file', 0, 'fileChange', truncatedChange))[0].diff, '@@ -1 +1 @@\n-old\n+new\n');
assert.equal(readableToolDetail(event('tools', 0, 'ToolSearch', '{"query":"select:mcp__monitter__list_agents,mcp__gmail__read_email","max_results":2}')), 'Tools requested:\n• monitter.list agents\n• gmail.read email');
assert.equal(readableToolDetail(event('image', 0, 'imageView', '{"type":"imageView","id":"opaque","path":"/tmp/screenshot.png"}')), 'Viewed /tmp/screenshot.png');
assert.equal(readableToolDetail(event('gmail', 0, 'gmail.search_emails', JSON.stringify({ content: [{ type: 'text', text: 'Action completed.' }], structured_content: { emails: [{ subject: 'Shipment update', from_: 'Carrier' }, { subject: 'Invoice', from_: 'Supplier' }] } }))), '2 emails\n\n• Shipment update — Carrier\n\n• Invoice — Supplier');
console.log('tool activity names are normalized into human-friendly labels and icons');
