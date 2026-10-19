<script lang="ts">
  import { onMount } from 'svelte';
  import TaskTranscript from '../../src/lib/components/TaskTranscript.svelte';
  import type { ConversationActivityItem } from '../../src/lib/activity-grouping';
  import type { RunEvent } from '../../src/lib/types';
  const event = (id: string, createdAt: number, kind = 'tool', title = 'commandExecution', detail = '{}') => ({ id, taskId: 'here', createdAt, kind, title, detail }) as RunEvent;
  const user = (id: string, createdAt: number, role = 'user') => ({ type: 'message', value: { id, taskId: 'here', createdAt, role, text: id } }) as ConversationActivityItem;
  let items = $state<ConversationActivityItem[]>([]);
  let boundaries = $state<RunEvent[]>([]);
  let steering = $state<any[]>([]);
  let collaborations = $state<any[]>([]);
  let opened = $state('');
  const task = { id: 'here', title: 'Notice fixture', provider: 'codex', status: 'running', agentId: 'main', updatedAt: 1 };
  const snapshot = { settings: {}, agents: [{ id: 'peer', name: 'Recorded peer' }], mailBatches: [] };
  const peer = { id: 'child', taskId: 'there', collaborationId: 'outgoing', agentName: 'Recorded peer', title: 'Scoped child', status: 'completed' };
  const noop = () => {};
  const base = { active: true, task, agent: null, snapshot, optimisticMessages: [], confirmedDeliveryIds: {}, pendingApprovals: [], selectedTaskStarting: false, goal: null, goalNote: '', clearingGoal: false, goalClearError: '', onClearGoal: noop, computerTools: [], scrollRevision: 0, busy: false, canShare: false, showHeader: false, maximised: false, senderName: () => null, operatorMessageText: (text: string) => text, approvalEventText: () => '', menuOpen: false, onMenuChange: noop, formatTime: () => '12:00', onOpenSubagent: (item: any) => opened = item.id, onOpenApproval: noop, onLoadFullEventDetail: async (event: RunEvent) => event.detail, onStop: noop, onEditTask: noop, onShare: noop, onHandoff: noop, onCopyMessage: noop, onReplyMessage: noop, onForkMessage: noop, onMaximise: noop } as any;
  function activity() {
    collaborations = [];
    const old = event('old-tool', 2), live = event('live-tool', 5);
    items = [user('old request', 1), { type: 'tool-group', values: [old] }, user('current request', 4), { type: 'tool-group', values: [live] }, user('assistant commentary', 6, 'assistant'), user('queued steering', 7)];
    boundaries = [event('old-end', 3, 'status', 'turn.completed'), event('start', 4.5, 'status', 'turn.started')];
    steering = [{ id: 'queued steering', taskId: 'here', createdAt: 7, text: 'queued steering', attachmentIds: [], status: 'queued' }];
  }
  function exchange() {
    steering = [];
    collaborations = ['outgoing', 'incoming'].map(id => ({ id, fromTaskId: id === 'outgoing' ? 'here' : 'there', toTaskId: id === 'outgoing' ? 'there' : 'here', fromAgentId: 'peer', toAgentId: 'peer', text: 'Recorded request', result: 'Recorded result', error: null, status: 'completed' }));
    items = collaborations.flatMap((record, index) => [
      { type: 'activity', value: event(`${record.id}-queued`, index * 3 + 1, 'collaboration', 'Collaboration queued', record.id) },
      { type: 'message', value: { id: `${record.id}-body`, taskId: 'here', createdAt: index * 3 + 2, role: 'assistant', text: record.text, collaborationId: record.id, attachments: [{ id: `${record.id}-attachment`, name: `${record.id}.txt`, mimeType: 'text/plain', size: 3, path: '/fixture.txt' }] } },
    ]) as ConversationActivityItem[];
  }
  onMount(() => {
    activity();
    (window as any).__TRANSCRIPT_NOTICES__ = { activity, exchange, finish: () => boundaries = [...boundaries, event('end', 8, 'status', 'turn.completed')], opened: () => opened };
  });
</script>
{#snippet empty()}{/snippet}
<div class="fixture"><TaskTranscript {...base} conversationItems={items} turnBoundaryEvents={boundaries} steeringMessages={steering} {collaborations} subagents={[peer] as any} avatarVisual={empty} messageAvatar={empty} deliveryStatus={empty} rightSidebar={empty} composer={empty} subagentDock={empty}/></div>
<style>:global(html,body,#app){height:100%;margin:0}.fixture{display:flex;height:100%;--line:#ccc;--panel:#fff;--paper:#fff;--ink:#222;--muted:#666;--accent:#087d76;--accent-ink:#05645f;--soft:#eee;--mono:monospace}</style>
