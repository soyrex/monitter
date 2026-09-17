import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const [surface, transcript, adapter, item, visor, sidebar, nativePermissions, nativeService] = await Promise.all([
  readFile(new URL('../src/lib/components/AppSurface.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/TaskTranscript.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/unified-subagents.ts', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/UnifiedSubagentItem.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/UnifiedSubagentVisor.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src/lib/components/UnifiedSubagentSidebar.svelte', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/permissions/default.toml', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8'),
]);

assert.match(surface, /detailTab:'run'\|'git'\|'timeline'\|'approvals'\|'subagents'/, 'pane state persists the Subagents tab');
assert.ok(surface.includes('Subagents <span>{taskSubagents.length}</span>'), 'tab shows the number of subagents');
assert.ok(surface.includes('event.kind === "collaboration"'), 'durable collaboration events are included in the chat stream');
assert.ok(surface.includes('unifiedSubagentsFromSnapshot(snapshot, selectedTask.id)'), 'one normalized projection drives all surfaces');
assert.ok(surface.includes('<UnifiedSubagentVisor') && surface.includes('<UnifiedSubagentSidebar') && !surface.includes('<UnifiedSubagentTabs'), 'the composer bar and visor are one unified surface');
assert.ok(surface.includes('taskSubagents.filter(activeUnifiedSubagent)') && surface.includes('items={activeTaskSubagents}'), 'only active subagents remain in the visor bar');
assert.match(surface, /\{#if activeTaskSubagents\.length\}[\s\S]*class="subagent-dock"/, 'the composer dock has no DOM shell when no subagent is active');
assert.ok(surface.includes('subagentLifecycleRevision') && surface.includes('{#key subagentLifecycleRevision}'), 'live session membership and terminal transitions invalidate the visor and sidebar');
assert.match(transcript, /\{@render composer\(\)\}[\s\S]*\{@render subagentDock\(\)\}/, 'mini-tabs are mounted directly below the composer');
assert.ok(transcript.includes('subagents, hasPendingApprovals'), 'inline subagent state participates in the frozen transcript projection');
assert.ok(transcript.includes('<UnifiedSubagentItem item={inlineSubagent}'), 'native and routed inline activity shares one renderer');
assert.ok(adapter.includes("source: 'collaboration'") && adapter.includes('sessionTree('), 'old routed state is normalized and nested delegation trees are traversed');
for (const component of [visor, sidebar]) assert.ok(component.includes('UnifiedSubagentItem'), 'every surface shares one subagent item renderer');
for (const label of ["queued: 'Starting'", "running: 'Working'", "completed: 'Done'", "error: 'Needs attention'"]) assert.ok(adapter.includes(label), `plain-language lifecycle label is present: ${label}`);
assert.ok(sidebar.includes('Active <span>{active.length}</span>') && sidebar.includes('Recent <span>{recent.length}</span>'), 'right sidebar has explicit Active and Recent sections');
assert.ok(sidebar.includes('No active subagents.') && sidebar.includes('No recent subagent work.'), 'sidebar empty states stay explicit');
assert.ok(visor.includes('selected.transcript') && visor.includes('aria-live="polite"') && visor.includes('Waiting for transcript activity'), 'visor exposes a live running transcript');
assert.ok(surface.includes('bridge.getSubagentTranscript') && adapter.includes('transcript:'), 'native and routed transcript sources are normalized');
assert.match(nativePermissions, /commands\.allow\s*=\s*\[[\s\S]*"get_subagent_transcript"[\s\S]*\]/, 'the packaged app grants its read-only transcript command');
assert.match(nativeService, /"get_subagent_transcript"\s*=>\s*\{[\s\S]*read_subagent_transcript/, 'the owner LAN bridge exposes the same read-only transcript command');
assert.ok(!item.includes('item.source') && !visor.includes('source') && !sidebar.includes('source'), 'transport source is never rendered');
console.log('Unified sliding subagent visor, live transcript and Active/Recent sidebar contract passed.');
