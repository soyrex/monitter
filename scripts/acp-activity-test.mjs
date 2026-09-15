import assert from 'node:assert/strict';
import { groupConversationActivity, toolPresentation, readableToolDetail } from '../src/lib/activity-grouping.ts';
const event = (id, at, update, taskId = 'task') => ({ id, taskId, kind: 'tool', title: 'ACP tool activity', createdAt: at, detail: JSON.stringify({sessionId:'session',update}) });
const start = { sessionUpdate:'tool_call', toolCallId:'one', title:'Read config', kind:'read', status:'pending', rawInput:{path:'config.json'} };
const events = [event('start',1,start), event('progress',2,{sessionUpdate:'tool_call_update',toolCallId:'one',status:'in_progress'}), event('end',4,{sessionUpdate:'tool_call_update',toolCallId:'one',status:'completed',content:[{type:'content',content:{type:'text',text:'File contents'}}]})];
const rows = groupConversationActivity([],events).flatMap(row => row.type==='tool-group' ? row.values : []);
assert.equal(rows.length,1);
assert.equal(toolPresentation(rows[0],true).label,'Read a file');
assert.match(readableToolDetail(rows[0]),/config.json/);
assert.match(readableToolDetail(rows[0]),/File contents/);
assert.doesNotMatch(readableToolDetail(rows[0]),/sessionId/);
assert.equal(toolPresentation(event('bad',5,{...start,status:'failed'}),false).label,'Tool failed');
assert.equal(toolPresentation(event('shell',5,{...start,kind:'execute',status:'completed'}),true).label,'Ran a command');
assert.equal(toolPresentation(event('plan',5,{sessionUpdate:'plan',entries:[]}),false).label,'Updated the plan');
const user={id:'user',taskId:'task',role:'user',text:'Next',createdAt:3};
assert.equal(groupConversationActivity([user],events).flatMap(row => row.type==='tool-group'?row.values:[]).length,2,'Do not merge reused IDs across turns');
assert.equal(groupConversationActivity([], [events[0],event('other',2,start,'another-task')]).flatMap(row=>row.type==='tool-group'?row.values:[]).length,2,'Do not merge separate tasks');
const failedCall=event('failed',6,{...start,toolCallId:'failed',status:'failed',content:[{type:'content',content:{type:'text',text:'Permission denied'}}]});
for (const compressed of [false,true]) {
 const timeline=groupConversationActivity([], [...events,failedCall],compressed);
 assert.ok(timeline.some(row=>row.type==='tool-group' && toolPresentation(row.values[0],false).label==='Tool failed'),'Failure remains visible alongside successful calls');
}
assert.match(readableToolDetail(failedCall),/Permission denied/);
assert.equal(toolPresentation({...events[0],detail:'{"update": [truncated]'},true).label,'Use a tool');
const planRows=groupConversationActivity([], [event('p1',1,{sessionUpdate:'plan',entries:[{content:'Read',status:'pending'}]}),event('p2',2,{sessionUpdate:'plan',entries:[{content:'Read',status:'completed'}]})]).flatMap(row=>row.type==='tool-group'?row.values:[]);
assert.equal(planRows.length,1);
assert.match(readableToolDetail(planRows[0]),/completed: Read/);
console.log('ACP activity checks passed: lifecycle, failures, plans, output, and turn/task isolation');
