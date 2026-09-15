<script lang="ts">
  import { type Snippet } from 'svelte';
  import { Check, CircleStop, MoreHorizontal, Pencil, Share2, Terminal } from '@lucide/svelte';
  import type { Agent, ApprovalRequest, Collaboration, ComputerActivity, Goal, Message, RunEvent, Snapshot, Task } from '$lib/types';
  import type { OptimisticMessage } from '$lib/pane-outbox-types';
  import { autonaming } from '$lib/autoname-state';
  import { floating } from '$lib/floating';
  import { isBlankReasoning, isCancellationMessage, isContextClearedMessage, showThinkingFallback, type ConversationActivityItem } from '$lib/activity-grouping';
  import { splitOperatorMessage } from '$lib/operator-sharing';
  import AnimatedTitle from '$lib/components/AnimatedTitle.svelte';
  import TaskActivity from '$lib/components/TaskActivity.svelte';
  import MessagePane from '$lib/components/MessagePane.svelte';
  import TranscriptVirtualList from '$lib/components/TranscriptVirtualList.svelte';
  import RunActivity from '$lib/components/RunActivity.svelte';
  import SubagentActivity from '$lib/components/SubagentActivity.svelte';
  import ThinkingStatus from '$lib/components/ThinkingStatus.svelte';
  import MessageMeta from '$lib/components/MessageMeta.svelte';
  import Markdown from '$lib/components/Markdown.svelte';
  import AttachmentList from '$lib/components/AttachmentList.svelte';
  import ExpandableUserRequest from '$lib/components/ExpandableUserRequest.svelte';

  type Avatar = Snippet<[Agent | null | undefined, number?]>;
  type Delivery = Snippet<[OptimisticMessage]>;
  type EmptySnippet = Snippet;
  type CollaborationRecord = Collaboration;

  let {
    active,
    task,
    agent,
    snapshot,
    conversationItems,
    optimisticMessages,
    confirmedDeliveryIds,
    pendingApprovals,
    selectedTaskStarting,
    goal,
    goalNote,
    computerTools,
    scrollRevision,
    busy,
    canShare,
    showHeader,
    avatarVisual,
    messageAvatar,
    deliveryStatus,
    paneExpand,
    rightSidebar,
    composer,
    senderName,
    operatorMessageText,
    approvalEventText,
    collaborations,
    collaborationWasSteering,
    menuOpen,
    onMenuChange,
    formatTime,
    onOpenCollaboration,
    onOpenApproval,
    onLoadFullEventDetail,
    onStop,
    onEditTask,
    onShare,
  }: {
    active: boolean;
    task: Task;
    agent: Agent | null;
    snapshot: Snapshot;
    conversationItems: ConversationActivityItem[];
    optimisticMessages: OptimisticMessage[];
    confirmedDeliveryIds: Record<string, true>;
    pendingApprovals: ApprovalRequest[];
    selectedTaskStarting: boolean;
    goal: Goal | null;
    goalNote: string;
    computerTools: ComputerActivity[];
    scrollRevision: number;
    busy: boolean;
    canShare: boolean;
    showHeader: boolean;
    avatarVisual: Avatar;
    messageAvatar: Snippet<[Agent | null | undefined]>;
    deliveryStatus: Delivery;
    paneExpand: EmptySnippet;
    rightSidebar: EmptySnippet;
    composer: EmptySnippet;
    senderName: (message: Message) => string | null;
    operatorMessageText: (value: string) => string;
    approvalEventText: (request: ApprovalRequest) => string;
    collaborations: CollaborationRecord[];
    collaborationWasSteering: (value: CollaborationRecord) => boolean;
    menuOpen: boolean;
    onMenuChange: (open: boolean) => void;
    formatTime: (value: number) => string;
    onOpenCollaboration: (value: CollaborationRecord) => void;
    onOpenApproval: (request: ApprovalRequest) => void;
    onLoadFullEventDetail: (event: RunEvent, onChunk: (detail: string) => void) => Promise<string>;
    onStop: () => void;
    onEditTask: () => void;
    onShare: () => void;
  } = $props();

  let taskMenuAnchor = $state<HTMLButtonElement>();
  const latestUserRequest = $derived(conversationItems
    .flatMap(item => item.type === 'message' && item.value.role === 'user' ? [item.value] : [])
    .at(-1));
  const thinking = $derived.by(() => {
    if (task.status !== 'running' || pendingApprovals.length) return false;
    const latest = conversationItems.at(-1);
    return showThinkingFallback(conversationItems, true) ||
      (latest?.type === 'reasoning-group' && latest.values.every(isBlankReasoning));
  });
  const collaborationFor = (id: string) => collaborations.find(item => item.id === id);
</script>

{#if showHeader}<div class="conversation-head task-heading pane-task-header">
    <div class="task-heading-identity">
      <span class="avatar task-header-avatar" aria-label={agent?.name ?? 'Agent'}>{@render avatarVisual(agent, 18)}</span>
      <h1 class="task-title"><AnimatedTitle text={task.title} active={$autonaming[`task:${task.id}`]}/><button class="icon task-title-edit" aria-label="Task settings" title="Edit task" onclick={onEditTask}><Pencil size={14}/></button></h1>
    </div>
    <div class="task-actions">
      <div class="task-overflow">
        <button bind:this={taskMenuAnchor} class="icon" aria-label="Chat actions" aria-haspopup="menu" aria-expanded={menuOpen} onclick={()=>onMenuChange(!menuOpen)}><MoreHorizontal size={17}/></button>
        {#if menuOpen && taskMenuAnchor}<div use:floating={{anchor:taskMenuAnchor}} class="task-menu floating-panel" role="menu" aria-label="Chat actions">
          {#if canShare}<button role="menuitem" onclick={()=>{onMenuChange(false);onShare();}}><Share2 size={15}/>Share this chat</button>{/if}
        </div>{/if}
      </div>
      {@render paneExpand()}
      {@render rightSidebar()}
    </div>
  </div>
{/if}
<section class="conversation">
    <TaskActivity {goal} {goalNote} tools={computerTools} onstop={onStop} disabled={busy} />
    <MessagePane {active} {thinking} resetKey={`task:${task.id}:${scrollRevision}`} stickyRequest={!!latestUserRequest}>
      {#if conversationItems.length}<TranscriptVirtualList
        items={conversationItems}
        getKey={(item) => item.type === 'tool-group' || item.type === 'reasoning-group' ? `${item.type}:${item.values[0].id}` : item.value.id}
        stickyKey={latestUserRequest?.id ?? null}
        active={true}>
        {#snippet children(item, _index)}
          {#if item.type === 'activity'}
            {@const collaboration=item.value.kind==='collaboration' ? collaborationFor(item.value.detail) : null}
            {#if collaboration}<SubagentActivity {collaboration} agent={snapshot.agents.find(agent=>agent.id===collaboration.toAgentId)} eventTitle={item.value.title} steered={collaborationWasSteering(collaboration)} onclick={()=>onOpenCollaboration(collaboration)}/>{:else}<RunActivity event={item.value} onloaddetail={onLoadFullEventDetail}/>{/if}
          {:else if item.type === 'reasoning-group'}
            <RunActivity events={item.values} onloaddetail={onLoadFullEventDetail} running={task.status === 'running' && item === conversationItems.at(-1) && !pendingApprovals.length}>
              {#snippet avatar()}{@render messageAvatar(agent)}{/snippet}
            </RunActivity>
          {:else if item.type === 'tool-group'}
            <RunActivity events={item.values} onloaddetail={onLoadFullEventDetail} compressed={snapshot.settings.compressToolCalls === true} running={task.status === 'running'}/>
          {:else if item.type === 'approval'}
            {@const approvalText=approvalEventText(item.value)}
            <button class={`approval-inline ${item.value.status}`} onclick={()=>onOpenApproval(item.value)} title={approvalText} aria-label={`${approvalText}. Open approval history`}><span>{approvalText}</span><time>{formatTime(item.value.resolvedAt ?? item.value.createdAt)}</time></button>
          {:else}
            {@const message=item.value}
            {#if isContextClearedMessage(message)}<div class="context-cleared-event" role="separator" aria-label={`Context Cleared at ${formatTime(message.createdAt)}`}><span aria-hidden="true"></span><time datetime={new Date(message.createdAt).toISOString()}>{formatTime(message.createdAt)} · Context Cleared</time><span aria-hidden="true"></span></div>
            {:else if isCancellationMessage(message)}<div class="cancellation-event" role="status"><CircleStop size={15} aria-hidden="true"/><MessageMeta name={message.text} createdAt={message.createdAt}/></div>
            {:else if !message.collaborationId || !collaborationFor(message.collaborationId)}
              {@const optimistic=optimisticMessages.find(item=>item.id===message.id)}
              {@const confirmed=confirmedDeliveryIds[message.id]}
              {@const operator=message.role === 'user' ? splitOperatorMessage(message.text.replace(/^\[Two human operators are collaborating[^\n]*\]\n/, '')) : null}
              <article class:user={message.role==='user'} class:tinted={message.role==='user' && snapshot.settings.tintUserMessages} class:sticky-user-request={message.role==='user' && message.id===latestUserRequest?.id} class:system={message.role==='system'} class:final-answer={message.role==='assistant' && message.phase==='final_answer'} class:optimistic-message={!!optimistic} class="message" data-message-phase={message.phase} data-live-entry={message.streamStatus==='streaming'} data-delivery-status={optimistic?.status} aria-label={message.role==='user' && message.id===latestUserRequest?.id ? 'Latest user request' : undefined}>
                <MessageMeta name={senderName(message) ?? (message.role==='user' ? 'You' : message.role==='assistant' ? (agent?.name ?? 'Agent') : 'System')} createdAt={message.createdAt}>
                  {#snippet avatar()}{#if operator?.name}<span class="avatar message-avatar human-avatar" title={operator.name}>{operator.name.slice(0, 1).toUpperCase()}</span>{:else}{@render messageAvatar(message.senderAgentId ? snapshot.agents.find(agent=>agent.id===message.senderAgentId) : message.role==='assistant' ? agent : null)}{/if}{/snippet}
                  {#if optimistic}{@render deliveryStatus(optimistic)}{:else if confirmed}<span class="delivery-status" data-delivery-status="sent" role="status" aria-label="Sent" title="Sent"><Check size={13} aria-hidden="true"/></span>{/if}
                </MessageMeta>
                {#if message.role==='user' && message.id===latestUserRequest?.id}<ExpandableUserRequest text={operatorMessageText(message.text)}/>{:else}<Markdown text={message.role==='user' ? operatorMessageText(message.text) : message.text}/>{/if}
                <AttachmentList attachments={message.attachments ?? []}/>
                {#if message.streamStatus==='streaming'}<small class="delivery-status" role="status">Receiving…</small>{:else if message.streamStatus==='interrupted'}<small class="delivery-status">Partial reply · interrupted</small>{/if}
              </article>
            {/if}
          {/if}
        {/snippet}
      </TranscriptVirtualList>{:else if !pendingApprovals.length}<div class="blank-conversation"><Terminal size={24}/><h2>No messages yet</h2><p>Describe what you want this agent to do. Its actual output will appear here.</p></div>{/if}
      {#if showThinkingFallback(conversationItems, task.status==='running' || selectedTaskStarting, pendingApprovals.length > 0)}
        <ThinkingStatus {active} starting={task.status!=='running'} running={task.status==='running'} startedAt={latestUserRequest?.createdAt}>{#snippet avatar()}{@render messageAvatar(agent)}{/snippet}</ThinkingStatus>
      {/if}
    </MessagePane>
  {@render composer()}
</section>

<style>
  .avatar { display:grid; flex:none; place-items:center; width:23px; height:23px; border-radius:6px; color:var(--on-accent); background:var(--accent); font:calc(11px * var(--interface-font-ratio,1)) var(--mono); }
  .avatar :global(img) { width:100%; height:100%; object-fit:cover; border-radius:inherit; }
  .icon { display:grid; place-items:center; width:var(--density-control-size); height:var(--density-control-size); border-radius:6px; }
  .icon:hover { background:var(--soft); }
  .floating-panel { position:fixed; inset:auto; z-index:50; margin:0; box-sizing:border-box; overflow:auto; overscroll-behavior:contain; padding:5px; border:1px solid var(--line); border-radius:9px; color:var(--ink); background:var(--panel); box-shadow:0 12px 30px #0003; }
  .pane-task-header { grid-column: 1 / -1; grid-row: 1; }
  .task-heading-identity { display:flex; align-items:center; gap:10px; flex:1; min-width:0; }
  .conversation { --chat-content-max-width:900px; display:flex; min-width:0; min-height:0; flex:1; flex-direction:column; grid-column:1; grid-row:2; }
  .conversation-head { background:var(--paper); display:flex; flex-shrink:0; overflow:visible; align-items:flex-start; justify-content:space-between; gap:20px; padding:25px clamp(25px,4vw,50px) 17px; border-bottom:1px solid var(--line); }
  .task-heading { align-items:center; padding-top:13px; padding-bottom:13px; }
  .conversation-head h1 { flex:1; min-width:0; margin:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; font-size:calc(22px * var(--interface-font-ratio,1)); }
  .task-heading-identity > h1 { font-size:calc(13.2px * var(--interface-font-ratio,1)); }
  .conversation-head h1 :global(.animated-title) { display:block; min-width:0; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  .task-actions { display:flex; flex:none; align-items:center; gap:7px; }
  .pane-task-header > .task-actions { margin-left:auto; flex:none; flex-wrap:nowrap; }
  .task-heading-identity > .task-header-avatar { flex:none; width:var(--density-header-avatar-size); height:var(--density-header-avatar-size); }
  .task-title { display:inline-flex; min-width:0; align-items:center; gap:5px; }
  .task-title-edit { flex:none; opacity:0; color:var(--muted); transition:opacity .12s ease,color .12s ease; }
  .task-title:focus-within .task-title-edit { opacity:1; }
  @media (hover:hover) and (pointer:fine) { .task-heading:hover .task-title-edit { opacity:1; } }
  .task-title-edit:hover { color:var(--ink); }
  .task-overflow { position:relative; }
  .task-menu { display:grid; min-width:155px; }
  .task-menu button { display:flex; gap:7px; align-items:center; padding:7px; text-align:left; }
  .message { max-width:100%; margin:0 0 24px; }
  .approval-inline { display:flex; align-items:baseline; width:100%; min-height:30px; gap:8px; margin:0 0 4px; padding:4px 2px; border:0; color:var(--muted); background:transparent; text-align:left; font:calc(11px * var(--interface-font-ratio,1)) var(--interface-font,"IBM Plex Sans",sans-serif); }
  .approval-inline > span { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .approval-inline:hover,.approval-inline:focus-visible { color:var(--ink); text-decoration:underline; text-decoration-color:var(--accent); text-underline-offset:3px; }
  .approval-inline time { margin-left:auto; flex:none; color:var(--muted); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); }
  .approval-inline.denied { color:#a54c44; }
  .message.user.tinted { background:color-mix(in srgb,var(--accent) 16%,var(--panel)); }
  .message.user { margin-left:auto; padding:12px 14px; border-radius:10px 10px 3px 10px; background:var(--soft); }
  .message.final-answer { width:fit-content; padding:12px 14px; border:1px solid color-mix(in srgb,#4f9d69 18%,var(--line)); border-radius:10px 10px 10px 3px; background:color-mix(in srgb,#4f9d69 7%,var(--panel)); }
  .message.system { padding-left:12px; border-left:2px solid var(--line); color:var(--muted); }
  .cancellation-event { display:flex; align-items:center; gap:8px; min-height:30px; margin:2px 0 9px; padding:4px 2px; color:var(--muted); }
  .cancellation-event :global(.message-meta) { flex:1; min-width:0; margin:0; }
  .cancellation-event :global(.message-meta time) { margin-left:auto; }
  .cancellation-event :global(svg) { flex:none; color:#b56a54; }
  .context-cleared-event { display:flex; align-items:center; gap:10px; margin:5px 0 18px; color:var(--muted); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); text-transform:uppercase; letter-spacing:.045em; white-space:nowrap; }
  .context-cleared-event > span { height:1px; flex:1; background:var(--line); }
  .context-cleared-event time { flex:none; }
  .optimistic-message { border:1px solid color-mix(in srgb,var(--accent) 35%,var(--line)); }
  .delivery-status { display:inline-flex; align-items:center; margin-left:auto; color:var(--muted); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); text-transform:uppercase; letter-spacing:.04em; }
  :global(.delivery-status[data-delivery-status="sending"]) { color:var(--accent); }
  :global(.delivery-status[data-delivery-status="not-confirmed"]) { color:#bd655b; }
  .message :global(.markdown) { font-size:var(--chat-font-size,13px); line-height:var(--chat-line-height,1.65); }
  .message-avatar { width:20px; height:20px; flex-shrink:0; border-radius:5px; font-size:calc(10px * var(--interface-font-ratio,1)); }
  .human-avatar { background:var(--accent); color:var(--on-accent); }
  .blank-conversation { display:grid; min-height:260px; place-content:center; justify-items:center; max-width:360px; margin:auto; color:var(--muted); text-align:center; }
  .blank-conversation :global(svg) { color:var(--accent); }
  .blank-conversation h2 { margin:10px 0 5px; color:var(--ink); font-size:calc(15px * var(--interface-font-ratio,1)); }
  .blank-conversation p { margin:0; font-size:calc(12.5px * var(--interface-font-ratio,1)); line-height:1.55; }
  @media (hover:none),(pointer:coarse) { .task-title-edit { opacity:1; pointer-events:auto; } .approval-inline { min-height:44px; padding-block:8px; } }
  @container workspace-pane (width < 1000px) { .conversation { --chat-side-padding:20px; } .conversation-head { padding-left:20px; padding-right:20px; } }
  @media (max-width:640px) { .conversation-head { padding:12px; gap:8px; } .conversation-head h1 { font-size:calc(18px * var(--interface-font-ratio,1)); } .task-actions { flex-wrap:wrap; } }
  @media (max-height:500px) { .conversation-head { padding-top:10px; padding-bottom:10px; gap:6px; } .task-heading { padding-top:10px; } }
</style>
