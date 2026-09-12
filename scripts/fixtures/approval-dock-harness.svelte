<script lang="ts">
  import { onMount } from 'svelte';
  import ApprovalDock from '../../src/lib/components/ApprovalDock.svelte';
  import type { ApprovalRequest } from '../../src/lib/types';

  const base = (id: string, overrides: Partial<ApprovalRequest> = {}): ApprovalRequest => ({
    id, taskId: 'task-1', provider: 'codex', runId: `run-${id}`, tool: 'shell', summary: `Review ${id} before continuing`, detail: JSON.stringify({ command: 'npm run migration', reason: 'This changes the local workspace.' }), risk: 'medium', status: 'pending', createdAt: 1, resolvedAt: null, decision: null, ...overrides,
  });
  let requests = $state<ApprovalRequest[]>([
    base('approve'),
    base('expired', { status: 'expired', resolvedAt: 2, decision: 'deny' }),
    base('deny', { risk: 'high' }),
    base('input', { tool: 'mcp_elicit', summary: 'Choose a deployment environment', input: { kind: 'questions', questions: [{ id: 'environment', header: 'Environment', question: 'Which environment?', isSecret: false, options: [{ label: 'Staging', description: 'Safe test target' }] }], schema: null, url: null } }),
  ]);
  const calls: { id: string; value: unknown }[] = [];
  function resolve(request: ApprovalRequest, decision: 'approve_once' | 'deny') { calls.push({ id: request.id, value: decision }); requests = requests.map(item => item.id === request.id ? { ...item, status: decision === 'deny' ? 'denied' : 'approved', decision, resolvedAt: 3 } : item); }
  function input(request: ApprovalRequest, response: unknown) { calls.push({ id: request.id, value: response }); requests = requests.map(item => item.id === request.id ? { ...item, status: 'approved', response, resolvedAt: 4 } : item); }
  onMount(() => {
    (window as Window & { __APPROVAL_DOCK_QA__?: { calls: () => unknown } }).__APPROVAL_DOCK_QA__ = { calls: () => calls };
    return () => { delete (window as Window & { __APPROVAL_DOCK_QA__?: unknown }).__APPROVAL_DOCK_QA__; };
  });
</script>

<main>
  <div class="toolbar">Chat toolbar</div>
  <div class="transcript" aria-label="Conversation transcript">{#each Array(32) as _, index}<p>Transcript message {index + 1}: this deliberately tall conversation remains scrollable behind the approval dock.</p>{/each}</div>
  <ApprovalDock {requests} onresolve={resolve} oninput={input}/>
  <form class="composer" aria-label="Message composer"><textarea aria-label="Message" placeholder="Message the agent"></textarea><button>Send</button></form>
</main>

<style>
  :global(html, body, #app) { height: 100%; margin: 0; overflow: hidden; }
  main { height: 100%; min-height: 0; display: flex; flex-direction: column; --panel: #fff; --bg: #f6f8f8; --line: #c8d0d0; --ink: #172020; --muted: #5d6a6a; --accent: #087d76; --accent-ink: #05645f; --on-accent: #fff; --mono: ui-monospace, monospace; --chat-content-max-width: 760px; --chat-side-padding: 18px; --approval-dock-reserve: 148px; }.toolbar { flex: none; height: 36px; padding: 0 18px; display: flex; align-items: center; border-bottom: 1px solid #c8d0d0; }
  .transcript { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; background: #f6f8f8; }.transcript p { max-width: 720px; margin: 0 auto 12px; padding: 11px; border-radius: 8px; background: #fff; }
  .composer { flex: none; display: flex; gap: 8px; width: min(760px, calc(100% - 36px)); margin: 0 auto 12px; }.composer textarea { min-height: 50px; flex: 1; box-sizing: border-box; }.composer button { min-width: 64px; }
</style>
