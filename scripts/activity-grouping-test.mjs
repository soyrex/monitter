import assert from 'node:assert/strict';
import { groupConversationActivity, isShellActivity } from '../src/lib/activity-grouping.ts';

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
