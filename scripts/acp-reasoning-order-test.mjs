import assert from 'node:assert/strict';
import { groupConversationActivity, withAcpFinalAnswers } from '../src/lib/activity-grouping.ts';

const message = (id, createdAt, role = 'assistant') => ({
  id, taskId: 'gemini', role, text: id, createdAt, attachments: [], streamStatus: 'complete',
});
const thought = (id, createdAt) => ({
  id, taskId: 'gemini', kind: 'reasoning', title: 'Reasoning', detail: id, createdAt,
});

const history = [message('prompt', 1, 'user'), message('narration', 2), message('answer', 3)];
const rendered = withAcpFinalAnswers(history, false);
assert.equal(rendered[1].phase, undefined);
assert.equal(rendered[2].phase, 'final_answer');
assert.equal(history[2].phase, undefined, 'The durable snapshot is not modified');
assert.equal(withAcpFinalAnswers(history, true)[2].phase, undefined);

const timeline = groupConversationActivity(rendered, [thought('late', 4)]);
assert.deepEqual(timeline.map(item => item.type), ['message', 'message', 'reasoning-group', 'message']);
assert.equal(timeline.at(-1).value.id, 'answer');
assert.deepEqual(groupConversationActivity(history, [thought('late', 4)]).map(item => item.type),
  ['message', 'message', 'message', 'reasoning-group'], 'Unmarked replies retain their event order');
assert.deepEqual(groupConversationActivity(rendered, [thought('early', 2.5)]).map(item => item.type),
  ['message', 'message', 'reasoning-group', 'message']);
const nextPrompt = message('next-prompt', 3.5, 'user');
assert.deepEqual(groupConversationActivity([...rendered, nextPrompt], [thought('next-turn', 4)]).map(item => item.type),
  ['message', 'message', 'message', 'message', 'reasoning-group'], 'Reasoning from the next turn must not cross its user message');
assert.deepEqual(groupConversationActivity(rendered, [{ ...thought('other-task', 4), taskId: 'other' }]).map(item => item.type),
  ['message', 'message', 'message', 'reasoning-group'], 'Reasoning from another task must not move');

console.log('ACP final answer and reasoning order assertions passed');
