import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const source = await readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8');
for (const expected of [
  "detailTab:'run'|'git'|'timeline'|'approvals'",
  "['run', 'git', 'timeline', 'approvals']",
  "onclick={()=>detailTab='approvals'}>Approvals</button>",
  'class="detail-scroll approval-history-panel" class:hidden={detailTab!==\'approvals\'} aria-label="Approval history"',
  'No resolved approvals for this chat.',
  'resolvedApprovalRequests,',
  'type === "approval"',
  'openApprovalHistory(item.value)',
  '`approval-history-${paneId}-${request.id}`',
  '`approval-history-${paneId}-${request.id}`',
  'showDetail = true; detailTab = \'approvals\';',
  'request.input ? `Submitted input for ${subject}` : `Approved ${subject}`',
  'Denied ${subject}',
  'Approval expired for ${subject}',
  'Approval unavailable for ${subject}',
]) assert.ok(source.includes(expected), `Missing approval history contract: ${expected}`);

const transcriptStart = source.indexOf('<MessagePane resetKey={`task:${selectedTask.id}:${scrollRevision}`}>');
const transcript = source.slice(transcriptStart, source.indexOf('<QueuedMessages messages={currentQueuedMessages}', transcriptStart));
assert.ok(!transcript.includes('approval-history'), 'Full approval history cards must not remain in the transcript');
assert.ok(transcript.includes('approval-inline'), 'Transcript must retain concise resolved approval events');
assert.ok(source.includes('.approval-inline { display:flex; align-items:baseline; width:100%; min-height:30px;') && source.includes('padding:4px 2px; border:0;'), 'Approval events must remain compact borderless rows');
assert.ok(source.includes('.approval-inline > span { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }'), 'Approval rows must stay one line while retaining their full accessible title');
assert.ok(source.includes('.detail-empty {') && source.includes('font-size: calc(10.5px * var(--interface-font-ratio, 1));'), 'Run-detail empty states must use compact secondary text');
assert.ok(source.includes('.approval-history-panel > .detail-empty { padding-top: 0; }'), 'Approval history must share the compact empty-state treatment');
console.log('approval history detail contract: persistent Approvals tab, selected-task empty state, concise status-safe inline events, and no transcript history cards');
