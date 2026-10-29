<script lang="ts">
  import { onMount } from 'svelte';
  import TaskTranscript from '$lib/components/TaskTranscript.svelte';
  let id = $state('chat');
  let items = $state<any[]>([]);
  let calls = 0;
  let requests: { taskId: string; before?: string }[] = [];
  let fail = false;
  let delay = false;
  let release: (() => void) | undefined;
  const rows = Array.from({length: 100}, (_, n) => ({id: `row-${n+1}`, taskId: 'chat', role: 'user', text: `relay-message-${n+1}`, createdAt: n+1, attachments: []}));
  const noop = () => {};
  const snapshot = { settings: {}, agents: [], mailBatches: [] };
  const task = $derived({id, title: id, provider: 'codex', status: 'completed', agentId: 'main', updatedAt: 1});
  const base = { active: true, agent: null, snapshot, optimisticMessages: [], steeringMessages: [], confirmedDeliveryIds: {}, pendingApprovals: [], selectedTaskStarting: false, goal: null, goalNote: '', clearingGoal: false, goalClearError: '', onClearGoal: noop, computerTools: [], scrollRevision: 0, busy: false, canShare: false, showHeader: false, maximised: false, senderName: () => null, operatorMessageText: (text: string) => text, approvalEventText: () => '', collaborations: [], subagents: [], menuOpen: false, onMenuChange: noop, formatTime: () => '', onOpenSubagent: noop, onOpenApproval: noop, onLoadFullEventDetail: async () => '', onStop: noop, onEditTask: noop, onShare: noop, onHandoff: noop, onCopyMessage: noop, onReplyMessage: noop, onForkMessage: noop, onMaximise: noop } as any;
  async function getTaskMessages(taskId: string, before?: string, limit = 64) {
    calls++;
    requests.push({taskId, before});
    if (delay) { delay = false; await new Promise<void>(resolve => release = resolve); }
    if (fail) { fail = false; throw new Error('fixture page failed'); }
    const end = taskId === 'chat' ? (before ? rows.findIndex(row => row.id === before) : rows.length) : 0;
    const start = Math.max(0, end - limit), messages = rows.slice(start, end);
    if (id === taskId) items = [...new Map([...items.map(item => [item.value.id, item.value]), ...messages.map(row => [row.id, row])]).values()].map(value => ({type: 'message', value}));
    return {messages, nextBeforeId: start ? messages[0].id : null, revision: 'rev'};
  }
  onMount(() => { (window as any).__ownerQA = { switch: (next: string) => { id = next; items = []; }, calls: () => calls, requests: () => requests, messages: () => items.map(item => item.value.id), fail: () => { fail = true; }, delay: () => { delay = true; }, release: () => release?.() }; });
</script>
{#snippet empty()}{/snippet}
<div class="fixture"><TaskTranscript {...base} {task} conversationItems={items} {getTaskMessages} avatarVisual={empty} messageAvatar={empty} deliveryStatus={empty} rightSidebar={empty} composer={empty} subagentDock={empty}/></div>
<style>:global(html,body,#app){height:100%;margin:0}.fixture{display:flex;height:100%;--line:#ccc;--panel:#fff;--paper:#fff;--ink:#222;--muted:#666;--accent:#087d76;--accent-ink:#05645f;--soft:#eee;--mono:monospace}</style>
