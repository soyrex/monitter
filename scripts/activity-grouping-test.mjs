import assert from 'node:assert/strict';
import { groupConversationActivity, isShellActivity, reasoningSummary } from '../src/lib/activity-grouping.ts';

const event = (id, createdAt, title, detail) => ({ id, taskId: 'task', kind: 'tool', title, detail, createdAt });
const message = (id, createdAt) => ({ id, taskId: 'task', role: 'assistant', text: 'reply', createdAt, attachments: [] });
const groups = (messages, events) => groupConversationActivity(messages, events).filter(item => item.type === 'tool-group');

// Screenshot-style empty and real web searches remain two raw entries together.
let result = groups([], [event('search-empty', 1, 'Web search', '{"tool":"web_search","query":""}'), event('search-query', 2, 'Web search', '{"tool":"web_search","query":"cats"}')]);
assert.equal(result.length, 1); assert.deepEqual(result[0].values.map(item => item.id), ['search-empty', 'search-query']);

// Different query strings do not alter the stable tool identity.
result = groups([], [event('one', 1, 'Web search', '{"tool":"web_search","query":"one"}'), event('two', 2, 'Web search', '{"tool":"web_search","query":"two"}')]);
assert.equal(result.length, 1);

// A message or reasoning activity is a hard boundary.
assert.equal(groups([message('reply', 2)], [event('before', 1, 'Web search', '{}'), event('after', 3, 'Web search', '{}')]).length, 2);
assert.equal(groups([], [event('before', 1, 'Web search', '{}'), { ...event('reasoning', 2, 'Reasoning', 'thinking'), kind: 'reasoning' }, event('after', 3, 'Web search', '{}')]).length, 2);

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

const mixed = [event('cmd1',1,'/bin/zsh -lc ls',''),event('web',2,'web_search',''),event('cmd2',3,'/bin/zsh -lc pwd','')];
assert.equal(groupConversationActivity([],mixed,true).length,1);
assert.equal(groupConversationActivity([],mixed,true)[0].values.length,3);
assert.equal(groupConversationActivity([message('boundary',2.5)],mixed,true).length,3);
assert.equal(groupConversationActivity([],mixed,false).length,3);

assert.equal(isShellActivity(event('shell',1,'/bin/zsh -lc git status','')),true);
assert.equal(isShellActivity(event('shell',1,'Run command','')),true);
assert.equal(isShellActivity(event('shell',1,'git status','{"type":"command_execution"}')),true);
assert.equal(isShellActivity(event('search',1,'web_search','')),false);

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
  for (const boundary of [event('tool', 2, 'Run command', '{}'), reasoning('summary', 2, 'Actual summary')]) {
    assert.equal(groupConversationActivity([], [blanks[0], boundary, blanks[2]], compress).length, 3);
  }
  assert.equal(groupConversationActivity([message('reply', 2)], [blanks[0], blanks[2]], compress).length, 3);
  assert.equal(groupConversationActivity([], [blanks[0], { ...blanks[1], taskId: 'other-task' }], compress).length, 2);
}
console.log('reasoning summary normalization and consecutive grouping assertions passed');
