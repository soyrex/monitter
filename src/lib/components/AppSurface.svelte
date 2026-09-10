<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { isTauri } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    Bot,
    Archive,
    Activity,
    ArrowUp,
    Search,
    Check,
    ChevronDown,
    ChevronRight,
    RotateCw,
    Cloud,
    Command,
    Folder,
    HardDrive,
    LoaderCircle,
    LayoutGrid,
    MessageSquare,
    MoreHorizontal,
    Network,
    PanelRight,
    PanelLeft,
    Paperclip,
    Play,
    Plus,
    Radio,
    Save,
    Settings2,
    Square,
    Terminal,
    Trash2,
    Wifi,
    X,
  } from "@lucide/svelte";
  import type {
    Agent,
    Collaboration,
    Goal,
    Channel,
    Host,
    ProbeResult,
    Project,
    SidebarView,
    Snapshot,
    Task,
    ModelSettings,
    Attachment,
    AttachmentTarget,
    AttachmentFileData,
  } from "$lib/types";
  import { getBridge } from "$lib/bridge";
  import Modal from "$lib/components/Modal.svelte";
  import Markdown from "$lib/components/Markdown.svelte";
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import TaskActivity from "$lib/components/TaskActivity.svelte";
  import { activeComputerTools } from "$lib/activity";
  import RunActivity from "$lib/components/RunActivity.svelte";
  import MessagePane from "$lib/components/MessagePane.svelte";
  import GitPane from "$lib/components/GitPane.svelte";
  import PaneGrid from '$lib/components/PaneGrid.svelte';
  import AppSurface from './AppSurface.svelte';
  import type { PaneLayout, PaneTabTransfer } from '$lib/panes';
  import { paneIds } from '$lib/panes';
  import ArchivedChats from "$lib/components/ArchivedChats.svelte";
  import ModelPicker from '$lib/components/ModelPicker.svelte';
  import AttachmentList from '$lib/components/AttachmentList.svelte';
  import {readBrowserFile,thumbnail,nativeBlob} from '$lib/attachment-files';
  import { floating } from "$lib/floating";

  let { embedded = false, paneId = 'main', active = true, parentSnapshot = null, onSnapshot, onTabDrop, onLayout, onSelection }:
    { embedded?: boolean; paneId?: string; active?: boolean; parentSnapshot?: Snapshot | null;
      onSnapshot?: (value: Snapshot) => void; onTabDrop?: (id: string, edge: DropEdge, data: PaneTabTransfer) => void;
      onLayout?: (mode: 'single' | 'columns' | 'grid') => void; onSelection?: (taskId: string | null) => void } = $props();
  type DropEdge = 'center' | 'left' | 'right' | 'top' | 'bottom';
  type TabPayload = { tab: PaneTabTransfer; draft?: TaskDraft; text?: string; attachments?:Attachment[]; attachmentContext?:string;recipients?:string[] };
  type PaneState = { openTaskIds:string[];openDraftIds:string[];openChannelIds:string[];taskDrafts:Record<string,TaskDraft>;drafts:Record<string,string>;selectedTaskId:string|null;currentDraftId:string|null;selectedChannelId:string|null;pane:typeof pane;focusedAgentId:string|null;focusedProjectId:string|null;showDetail:boolean;detailTab:'run'|'git';queuedAttachments:Record<string,Attachment[]>;attachmentContexts:Record<string,string>;channelRecipients:Record<string,string[]> };
  let layout = $state<PaneLayout>({id:'main'}), activePaneId = $state('main');
  let paneRefs = $state<Record<string, { openTask: (task: Task) => void; openChannel: (channel: Channel) => void; openTaskComposer: (parentId?: string | null, agentId?: string | null, projectId?: string | null) => void; takeTab: (tab: PaneTabTransfer) => TabPayload | null; receiveTab: (payload: TabPayload) => void; allTabs: () => PaneTabTransfer[]; captureState:()=>PaneState; restoreState:(value:PaneState)=>void; hasPending:()=>boolean;attachNativeFiles:(paths:string[])=>Promise<void> }>>({});
  let paneSelections = $state<Record<string,string|null>>({});
  let layoutMenu = $state(false), layoutAnchor = $state<HTMLButtonElement>();
  let compactDetail = $state(false);
  let queuedAttachments=$state<Record<string,Attachment[]>>({}), attachmentContexts=$state<Record<string,string>>({}), pendingUploads=$state<Record<string,boolean>>({});
  let filePicker=$state<HTMLInputElement>();
  const currentAttachments=$derived(queuedAttachments[currentDraftKey() ?? ''] ?? []);
  const filesBusy=$derived(pendingUploads[currentDraftKey() ?? ''] ?? false);
  let openChannelIds = $state<string[]>([]);
  let channelRecipients = $state<Record<string,string[]>>({});
  $effect(()=>{ if(embedded && parentSnapshot) snapshot=parentSnapshot; });
  $effect(()=>{ onSelection?.(selectedTaskId); });

  const accents = ["#3f9d6a", "#3978d4", "#8755c7", "#c44c79", "#c27524"];
  const bridge = getBridge();
  const macPlatform = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
  const modifierLabel = macPlatform ? '⌘' : 'Ctrl+';
  const nativeMac = $derived(
    !embedded && typeof navigator !== "undefined" && isTauri() && /Mac/.test(navigator.userAgent));
  let snapshot = $state<Snapshot | null>(null),
    selectedTaskId = $state<string | null>(null),
    selectedChannelId = $state<string | null>(null),
    pane = $state<"overview" | "task" | "channel" | "agent" | "project">("overview");
  let showDetail = $state(true),
    busy = $state(false),
    error = $state(""),
    notice = $state(""),
    composer = $state(""),
    composerPending = $state<Record<string, boolean>>({}),
    taskTitle = $state("");
  const canSend=$derived(Boolean(composer.trim() || currentAttachments.length) && !filesBusy);
  let scaleQueued = $state<number | null>(null), scaleInFlight = $state<number | null>(null), scaleSaving = false;
  let slashOpen = $state(false), slashIndex = $state(0);
  let taskMenu = $state(false), monitterMenu = $state(false);
  let sidebarCollapsed = $state(false), sidebarViewMenu = $state(false);
  let railAgentId = $state<string | null>(null);
  let railAnchor = $state<HTMLButtonElement>();
  let viewAnchor = $state<HTMLButtonElement>();
  let taskMenuAnchor = $state<HTMLButtonElement>();
  let monitterMenuAnchor = $state<HTMLButtonElement>();
  let detailTab = $state<'run' | 'git'>('run');
  let gitState = $state<{ repository: boolean | null; error: string; loading: boolean }>({ repository: null, error: '', loading: false });
  let gitPane = $state<GitPane>();
  const railAgent = $derived(snapshot?.agents.find(agent => agent.id === railAgentId) ?? null);
  const sidebarViews = [{ id: 'standard', label: 'Standard', icon: Bot }, { id: 'activity', label: 'Activity', icon: Activity }, { id: 'projects', label: 'Projects', icon: Folder }] as const;
  let returnToMonitterMenu = $state(false);
  $effect(() => {
    if (modal === null && returnToMonitterMenu) {
      returnToMonitterMenu = false;
      void tick().then(() => document.querySelector<HTMLButtonElement>('[aria-label="Monitter menu"]')?.focus());
    }
  });
  let modal = $state<
      | "agent"
      | "hosts"
      | "host"
      | "taskSettings"
      | "channel"
      | "appearance"
      | "archived"
      | "project"
      | "deleteProject"
      | "directory"
      | null
    >(null),
    agentDraft = $state<Agent | null>(null),
    hostDraft = $state<Host | null>(null),
    channelDraft = $state<Channel | null>(null),
    projectDraft = $state<Project | null>(null),
    probe = $state<ProbeResult | null>(null),
    recipients = $state<string[]>([]),
    drafts = $state<Record<string, string>>({});
  type TaskDraft = { modelSettings?:ModelSettings;modelAgentId?:string; id: string; text: string; title: string; agentId: string; projectId: string; parentId: string | null; nativeSessionId: string; createdTaskId?: string };
  type CollaborationRecord = Collaboration;
  type AgentProfile = Agent & { expertise?: string[]; responsibilities?: string[]; skills?: string[]; collaborationEnabled?: boolean };
  let taskDrafts = $state<Record<string, TaskDraft>>({});
  let openDraftIds = $state<string[]>([]);
  let currentDraftId = $state<string | null>(null);
  let taskAgentId = $state(""),
    taskProjectId = $state(""),
    taskParentId = $state<string | null>(null),
    taskNativeSessionId = $state(""),
    renameTitle = $state("");
  let palette = $state<"switch" | "controls" | null>(null);
  let directoryQuery = $state("");
  let focusedAgentId = $state<string | null>(null);
  let focusedProjectId = $state<string | null>(null);
  let collapsedProjects = $state<Record<string, boolean>>({});
  const projects = $derived(snapshot?.projects ?? []);
  const focusedProject = $derived(projects.find(project => project.id === focusedProjectId) ?? null);
  const sidebarView = $derived(snapshot?.settings.sidebarView ?? 'standard');
  const currentSidebarView = $derived(sidebarViews.find(view => view.id === sidebarView)!);
  const activeTasks = $derived(snapshot?.tasks.filter(task => !task.archived) ?? []);
  const activityTasks = $derived([...activeTasks].sort((a,b) =>
    Number(b.status === 'running') - Number(a.status === 'running') || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id)));
  const taskFormAgent = $derived(snapshot?.agents.find(agent => agent.id === taskAgentId));
  const taskFormProject = $derived(projects.find(project => project.id === taskProjectId));
  const taskFormCwd = $derived(taskFormProject?.workspaces.find(workspace => workspace.hostId === taskFormAgent?.hostId)?.cwd || taskFormAgent?.cwd || snapshot?.hosts.find(host => host.id === taskFormAgent?.hostId)?.defaultCwd || '');
  const focusedAgent = $derived(snapshot?.agents.find(agent=>agent.id===focusedAgentId) ?? null);
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let snapshotIssued = 0,
    snapshotApplied = 0;
  let appliedScale = 0;
  let openTaskIds = $state<string[]>([]);
  let scrollRevision = $state(0);
  const openDrafts = $derived(openDraftIds.flatMap(id => taskDrafts[id] ? [taskDrafts[id]] : []));
  const currentTaskDraft = $derived(currentDraftId ? taskDrafts[currentDraftId] ?? null : null);
  const openTasks = $derived(openTaskIds.flatMap(id => {
    const task = snapshot?.tasks.find(task => task.id === id);
    return task && !task.archived ? [task] : [];
  }));
  const selectedTask = $derived(
    snapshot?.tasks.find((t) => t.id === selectedTaskId) ?? null,
  );
  const selectedAgent = $derived(
    snapshot?.agents.find((a) => a.id === selectedTask?.agentId) ?? null,
  );
  const selectedHost = $derived(
    snapshot?.hosts.find((h) => h.id === selectedTask?.hostId) ??
      snapshot?.hosts.find((h) => h.kind === "local") ??
      null,
  );
  const messages = $derived(
    selectedTask
      ? (snapshot?.messages.filter((m) => m.taskId === selectedTask.id) ?? [])
      : [],
  );
  const events = $derived(
    selectedTask
      ? (snapshot?.events
          .filter((e) => e.taskId === selectedTask.id)
          .sort((a, b) => a.createdAt - b.createdAt) ?? [])
      : [],
  );
  let goal = $state<Goal | null>(null);
  let goalError = $state("");
  const goalKey = $derived(selectedTask?.nativeSessionId && selectedTask.provider === "codex"
    ? `${selectedTask.id}|${selectedTask.nativeSessionId}|${selectedTask.status}` : "");
  $effect(() => {
    const [taskId, , status] = goalKey.split("|");
    goal = null;
    goalError = "";
    if (!taskId) return;
    let cancelled = false, pending = false;
    async function refreshGoal() {
      if (pending) return;
      pending = true;
      try {
        const result = await bridge.getTaskGoal(taskId);
        if (!cancelled) { goal = result; goalError = ""; }
      } catch (reason) {
        if (!cancelled) goalError = text(reason);
      } finally { pending = false; }
    }
    void refreshGoal();
    const interval = status === "running" ? setInterval(refreshGoal, 15000) : undefined;
    return () => { cancelled = true; if (interval) clearInterval(interval); };
  });
  const goalNote = $derived(selectedTask?.provider === "hermes" ? events.filter(event=>event.kind === "goal").at(-1)?.detail ?? "" : "");
  const computerTools = $derived(activeComputerTools(selectedTask, messages, events));
  const visibleEvents = $derived(events.filter(event => event.kind !== "computer" &&
    (event.kind !== "tool" || snapshot?.settings.showToolActivity !== false) &&
    (event.kind !== "reasoning" || snapshot?.settings.showReasoningSummaries !== false),
  ));
  const conversationItems = $derived([
    ...messages.map(message => ({ type: "message" as const, value: message })),
    ...visibleEvents.filter(event => event.kind === "tool" ||
      (event.kind === "reasoning" && event.detail.trim())).map(event => ({ type: "activity" as const, value: event })),
  ].sort((a, b) => a.value.createdAt - b.value.createdAt));
  const collaborations = $derived(((snapshot as (Snapshot & { collaborations?: CollaborationRecord[] }) | null)?.collaborations ?? []));
  const taskCollaborations = $derived(selectedTask ? collaborations.filter(item => item.fromTaskId === selectedTask.id || item.toTaskId === selectedTask.id) : []);
  function taskIsStepping(task: Task) {
    if (task.status !== 'running') return false;
    const boundary = (snapshot?.messages.filter(message => message.taskId === task.id && message.role === 'user').at(-1)?.createdAt ?? task.updatedAt);
    return (snapshot?.messages.some(message => message.taskId === task.id && message.role === 'assistant' && message.createdAt >= boundary) ?? false)
      || (snapshot?.events.some(event => event.taskId === task.id && event.createdAt >= boundary && (
        ['tool', 'reasoning', 'computer', 'output'].includes(event.kind)
        || (event.kind === 'status' && /^turn[.\s_-]started$/i.test(event.title))
      )) ?? false);
  }
  const selectedTaskStarting = $derived(!!(selectedTask && (composerPending[`task:${selectedTask.id}`] || (selectedTask.status === 'running' && !taskIsStepping(selectedTask)))));
  const selectedTaskStepping = $derived(!!(selectedTask && taskIsStepping(selectedTask)));
  function setComposerPending(key: string, pending: boolean) { if (pending) composerPending[key] = true; else delete composerPending[key]; }
  function watchPane(node: HTMLElement) {
    const resize = new ResizeObserver(() => {
      const narrow = node.clientWidth < 700;
      if (narrow && !compactDetail) showDetail = false;
      compactDetail = narrow;
    });
    resize.observe(node);
    return { destroy: () => resize.disconnect() };
  }
  function routeTask(task: Task) {
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openTask(task); else openTask(task);
  }
  function routeChannel(channel: Channel) {
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openChannel(channel); else openChannel(channel);
  }
  function routeDraft(agentId: string) {
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openTaskComposer(null, agentId); else openTaskComposer(null, agentId);
  }
  function resizeSplit(id: string, ratio: number) {
    function resize(node: PaneLayout): PaneLayout {
      if (!('axis' in node)) return node;
      return node.id === id ? {...node,ratio:Math.max(.15,Math.min(.85,ratio))}
        : {...node,first:resize(node.first),second:resize(node.second)};
    }
    layout = resize(layout);
  }
  export function allTabs(): PaneTabTransfer[] {
    return [...openTaskIds.map(id=>({sourcePaneId:paneId,kind:'task' as const,id})),
      ...openDraftIds.map(id=>({sourcePaneId:paneId,kind:'draft' as const,id})),
      ...openChannelIds.map(id=>({sourcePaneId:paneId,kind:'channel' as const,id}))];
  }
  export function hasPending() { return Object.values(composerPending).some(Boolean) || Object.values(pendingUploads).some(Boolean); }
  export function captureState():PaneState {
    saveCurrentDraft();
    return JSON.parse(JSON.stringify({openTaskIds,openDraftIds,openChannelIds,taskDrafts,drafts,selectedTaskId,currentDraftId,selectedChannelId,pane,focusedAgentId,focusedProjectId,showDetail,detailTab,queuedAttachments,attachmentContexts,channelRecipients}));
  }
  export function restoreState(value:PaneState) {
    ({openTaskIds,openDraftIds,openChannelIds,taskDrafts,drafts,selectedTaskId,currentDraftId,selectedChannelId,pane,focusedAgentId,focusedProjectId,showDetail,detailTab,queuedAttachments,attachmentContexts,channelRecipients}=value);
    composer=drafts[currentDraftKey() ?? ''] ?? '';
    recipients=selectedChannelId?channelRecipients[selectedChannelId]??[]:[];
    const draft=currentDraftId?taskDrafts[currentDraftId]:null;
    if(draft) {composer=draft.text;taskTitle=draft.title;taskAgentId=draft.agentId;taskProjectId=draft.projectId;taskParentId=draft.parentId;taskNativeSessionId=draft.nativeSessionId;}
  }
  function captureChildren() {
    return Object.fromEntries(paneIds(layout).filter(id=>id!=='main').flatMap(id=>paneRefs[id]?[[id,paneRefs[id].captureState()]]:[]));
  }
  function layoutPending() { return hasPending() || paneIds(layout).some(id=>paneRefs[id]?.hasPending()); }
  export function takeTab(tab: PaneTabTransfer): TabPayload | null {
    saveCurrentDraft();
    if (composerPending[`${tab.kind}:${tab.id}`] || pendingUploads[`${tab.kind}:${tab.id}`]) return null;
    const attachments=queuedAttachments[`${tab.kind}:${tab.id}`], attachmentContext=attachmentContexts[`${tab.kind}:${tab.id}`];
    if (tab.kind === 'draft') {
      const draft = taskDrafts[tab.id];
      if (!draft) return null;
      const payload = {tab,draft:{...draft},text:draft.text,attachments,attachmentContext};
      closeTaskDraft(tab.id); delete taskDrafts[tab.id]; delete drafts[`draft:${tab.id}`];
      return payload;
    }
    const key = `${tab.kind}:${tab.id}`, text = drafts[key] ?? '';
    if (tab.kind === 'task') closeTaskTab(tab.id);
    else {
      openChannelIds = openChannelIds.filter(id=>id!==tab.id);
      if (selectedChannelId === tab.id) openOverview();
    }
    delete drafts[key];
    return {tab,text,attachments,attachmentContext,recipients:channelRecipients[tab.id]};
  }
  export function receiveTab(payload: TabPayload) {
    const {tab} = payload;
    if(tab.kind==='channel')channelRecipients[tab.id]=payload.recipients ?? [];
    queuedAttachments[`${tab.kind}:${tab.id}`]=payload.attachments ?? [];
    if(payload.attachmentContext) attachmentContexts[`${tab.kind}:${tab.id}`]=payload.attachmentContext;
    if (tab.kind === 'draft' && payload.draft) {
      taskDrafts[tab.id] = payload.draft;
      openTaskDraft(payload.draft);
    } else if (tab.kind === 'task') {
      const task = snapshot?.tasks.find(task=>task.id===tab.id);
      if (task) { drafts[`task:${tab.id}`] = payload.text ?? ''; openTask(task); }
    } else if (tab.kind === 'channel') {
      const channel = snapshot?.channels.find(channel=>channel.id===tab.id);
      if (channel) { drafts[`channel:${tab.id}`] = payload.text ?? ''; openChannel(channel); }
    }
  }
  async function setLayout(mode: 'single' | 'columns' | 'grid') {
    layoutMenu = false;
    if (embedded) { onLayout?.(mode); return; }
    if(layoutPending()) {notice='Wait for the message to be accepted before changing layout.';return;}
    const saved=captureChildren();
    const existing = paneIds(layout), count = mode==='single'?1:mode==='columns'?2:4;
    const ids = ['main',...existing.filter(id=>id!=='main')].slice(0,count);
    while(ids.length<count) ids.push(crypto.randomUUID());
    for (const id of existing.filter(id=>!ids.includes(id))) {
      const source = paneRefs[id];
      for (const tab of source?.allTabs() ?? []) { const payload=source.takeTab(tab); if(payload) receiveTab(payload); }
    }
    const pair = (first: PaneLayout, second: PaneLayout, axis: 'horizontal'|'vertical' = 'horizontal'): PaneLayout => ({id:crypto.randomUUID(),axis,ratio:.5,first,second});
    layout = count===1 ? {id:'main'} : count===2 ? pair({id:ids[0]},{id:ids[1]})
      : pair(pair({id:ids[0]},{id:ids[1]}),pair({id:ids[2]},{id:ids[3]}),'vertical');
    if(!ids.includes(activePaneId)) activePaneId='main';
    await tick();
    for(const id of ids) if(saved[id]) paneRefs[id]?.restoreState(saved[id]);
  }
  async function dropTab(targetId: string, edge: DropEdge, tab: PaneTabTransfer) {
    if(embedded) { onTabDrop?.(targetId,edge,tab); return; }
    const ids=paneIds(layout);
    if (!ids.includes(tab.sourcePaneId) || !ids.includes(targetId)) return;
    if (edge==='center' && targetId===tab.sourcePaneId) return;
    if (edge!=='center' && ids.length>=4) { notice='Up to four panes are available. Drop in the centre to move a tab.'; return; }
    if(edge!=='center' && layoutPending()) {notice='Wait for the message to be accepted before splitting a pane.';return;}
    const source=tab.sourcePaneId==='main'?{takeTab}:paneRefs[tab.sourcePaneId];
    const payload=source?.takeTab(tab); if(!payload) return;
    const saved=edge!=='center'?captureChildren():{};
    let destination=targetId;
    if(edge!=='center') {
      destination=crypto.randomUUID();
      function insert(node:PaneLayout):PaneLayout {
        if('axis' in node) return {...node,first:insert(node.first),second:insert(node.second)};
        if(node.id!==targetId) return node;
        const fresh={id:destination}, before=edge==='left'||edge==='top';
        return {id:crypto.randomUUID(),axis:edge==='left'||edge==='right'?'horizontal':'vertical',ratio:.5,first:before?fresh:node,second:before?node:fresh};
      }
      layout=insert(layout);
    }
    await tick();
    for(const id of paneIds(layout)) if(saved[id]) paneRefs[id]?.restoreState(saved[id]);
    if(destination==='main') receiveTab(payload); else paneRefs[destination]?.receiveTab(payload);
    activePaneId=destination;
  }
  function dragTab(event:DragEvent,kind:PaneTabTransfer['kind'],id:string) {
    saveCurrentDraft();
    if(!event.dataTransfer || composerPending[`${kind}:${id}`] || pendingUploads[`${kind}:${id}`]) {event.preventDefault();return;}
    event.dataTransfer.effectAllowed='move';
    event.dataTransfer.setData('application/x-monitter-tab',JSON.stringify({sourcePaneId:paneId,kind,id}));
  }
  function tabBarOver(event:DragEvent) {
    if(event.dataTransfer?.types.includes('application/x-monitter-tab')) {event.preventDefault();event.stopPropagation();}
  }
  function tabBarDrop(event:DragEvent) {
    const raw=event.dataTransfer?.getData('application/x-monitter-tab'); if(!raw)return;
    event.preventDefault();event.stopPropagation();
    try { void dropTab(paneId,'center',JSON.parse(raw)); } catch { /* Ignore non-Monitter data. */ }
  }
  function attachmentTargets():{target:AttachmentTarget;scope:string}[] {
    if(pane==='task' && !currentDraftId && selectedTask) return [{target:{taskId:selectedTask.id},scope:`${selectedTask.hostId}:${selectedTask.cwd}`}];
    const agentIds=pane==='channel'?recipients:currentDraftId?[taskAgentId]:[];
    return agentIds.flatMap(agentId=>{
      const agent=snapshot?.agents.find(item=>item.id===agentId);if(!agent)return [];
      const projectId=pane==='channel'?null:taskProjectId||null;
      const cwd=snapshot?.projects.find(project=>project.id===projectId)?.workspaces.find(workspace=>workspace.hostId===agent.hostId)?.cwd || agent.cwd || snapshot?.hosts.find(host=>host.id===agent.hostId)?.defaultCwd || '';
      return [{target:{agentId,projectId},scope:`${agent.hostId}:${cwd}`}];
    }).filter((target,index,all)=>all.findIndex(other=>other.scope===target.scope)===index);
  }
  const attachmentScope=$derived(attachmentTargets().map(item=>item.scope).sort().join('|'));
  $effect(()=>{
    const key=currentDraftKey();
    if(key && !pendingUploads[key] && queuedAttachments[key]?.length && attachmentContexts[key]!==attachmentScope) {
      queuedAttachments[key]=[];
      notice='The working folder or recipients changed. Attach the files again for this destination.';
    }
  });
  function removeAttachment(id:string) {
    const key=currentDraftKey();if(!key)return;
    const item=queuedAttachments[key]?.find(item=>item.id===id);
    queuedAttachments[key]=(queuedAttachments[key]??[]).filter(other=>item?.sourceId?other.sourceId!==item.sourceId:other.id!==id);
  }
  function clearAttachments(key:string,ids:string[]) {queuedAttachments[key]=(queuedAttachments[key]??[]).filter(item=>!ids.includes(item.id));}
  async function attachFiles(items:(File|string)[]) {
    const key=currentDraftKey(), targets=attachmentTargets(), scope=attachmentScope;
    if(!key || filesBusy || busy || selectedTask?.status==='running')return;
    if(!targets.length) {error='Choose an agent to receive these files.';return;}
    pendingUploads[key]=true;error='';
    try {
      for(const item of items) {
        const file:AttachmentFileData=typeof item==='string'?await bridge.readAttachmentFile(item):await readBrowserFile(item);
        const preview=await thumbnail(typeof item==='string'?nativeBlob(file):item), sourceId=crypto.randomUUID();
        const uploaded:Attachment[]=[];
        for(const {target} of targets) uploaded.push(await bridge.storeAttachment(target,file,preview,sourceId));
        attachmentContexts[key]=scope;
        queuedAttachments[key]=[...(queuedAttachments[key]??[]),...uploaded];
      }
    } catch(reason) {error=text(reason);}
    finally {pendingUploads[key]=false;}
  }
  export function attachNativeFiles(paths:string[]) {return attachFiles(paths);}
  function fileDrop(node:HTMLElement) {
    const over=(event:DragEvent)=>{if(event.dataTransfer?.types.includes('Files')){event.preventDefault();event.stopPropagation();node.classList.add('drop-files');}};
    const leave=()=>node.classList.remove('drop-files');
    const drop=(event:DragEvent)=>{leave();const files=Array.from(event.dataTransfer?.files??[]);if(files.length){event.preventDefault();event.stopPropagation();void attachFiles(files);}};
    const paste=(event:ClipboardEvent)=>{const files=Array.from(event.clipboardData?.files??[]);if(files.length){event.preventDefault();void attachFiles(files);}};
    node.addEventListener('dragover',over);node.addEventListener('dragleave',leave);node.addEventListener('drop',drop);node.addEventListener('paste',paste);
    return {destroy(){node.removeEventListener('dragover',over);node.removeEventListener('dragleave',leave);node.removeEventListener('drop',drop);node.removeEventListener('paste',paste);}};
  }
  const draftModelSettings=$derived(currentTaskDraft?.modelAgentId===taskAgentId ? currentTaskDraft?.modelSettings ?? null : null);
  async function changeModel(settings:ModelSettings) {
    const draft=currentTaskDraft, agentId=taskAgentId, taskId=selectedTask?.id ?? draft?.createdTaskId;
    if(taskId) {
      const ticket=++snapshotIssued;
      const result=await bridge.setTaskModelSettings(taskId,settings);
      applySnapshot(result,ticket);
    }
    if(draft && taskDrafts[draft.id]) taskDrafts[draft.id]={...taskDrafts[draft.id],modelSettings:settings,modelAgentId:agentId};
  }
  const activeTurnDelivery = "Delivered to the recipient’s active turn via its Monitter inbox.";
  const collaborationStatus = (item: CollaborationRecord) => item.result === activeTurnDelivery ? "Delivered" : item.status;
  const profileList = (value: string[] | undefined) => (value ?? []).join("\n");
  const parseProfileList = (value: string) => value.split(/\n/);
  const senderName = (message: (typeof messages)[number]) => {
    const sender = (message as typeof message & { senderAgentId?: string | null }).senderAgentId;
    return sender ? snapshot?.agents.find(agent => agent.id === sender)?.name ?? "Agent" : null;
  };
  const delegated = $derived(
    selectedTask
      ? (snapshot?.tasks.filter((t) => t.parentTaskId === selectedTask.id) ??
          [])
      : [],
  );
  const activeChannel = $derived(
    snapshot?.channels.find((c) => c.id === selectedChannelId) ?? null,
  );
  const activeChannelTasks = $derived(
    activeChannel
      ? snapshot?.tasks.filter(task => task.channelId === activeChannel.id && task.status === 'running') ?? []
      : [],
  );
  const activeChannelStarting = $derived(
    !!(activeChannel && (composerPending[`channel:${activeChannel.id}`]
      || (activeChannelTasks.length && !activeChannelTasks.some(taskIsStepping)))),
  );
  const localHost = $derived(
    snapshot?.hosts.find((h) => h.kind === "local") ?? null,
  );
  const defaultAgent = $derived(
    snapshot?.agents.find((a) => a.provider === "codex") ??
      snapshot?.agents[0] ??
      null,
  );
  const text = (reason: unknown) =>
    reason instanceof Error ? reason.message : String(reason);
  const avatarSrc = (agent: Agent | null | undefined) => {
    const value = agent?.avatar;
    return value && /^data:image\/(png|jpeg|webp);base64,/i.test(value) ? value : null;
  };
  async function chooseAvatar(file?: File) {
    if (!agentDraft || !file) return;
    if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type) || file.size > 2 * 1024 * 1024) { error = 'Choose a PNG, JPEG, or WebP image up to 2 MiB.'; return; }
    const target = agentDraft;
    const reader = new FileReader();
    reader.onerror = () => { if (agentDraft === target) error = 'Could not read this image.'; };
    reader.onload = () => {
      if (agentDraft !== target || modal !== 'agent') return;
      if (typeof reader.result === 'string' && reader.result.length <= 3 * 1024 * 1024) target.avatar = reader.result;
      else error = 'Avatar image is too large.';
    };
    reader.readAsDataURL(file);
  }
  const date = (time: number) =>
    new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    }).format(time);
  const longDetail = (detail: string) => detail.length > 1200;
  const detailPreview = (detail: string) =>
    `${detail.slice(0, 360).replace(/\s+/g, " ").trim()}…`;
  const detailLength = (detail: string) =>
    new Intl.NumberFormat(undefined).format(detail.length);
  const relative = (time: number) => {
    const mins = Math.round((Date.now() - time) / 60000);
    return mins < 1
      ? "now"
      : mins < 60
        ? `${mins}m ago`
        : `${Math.round(mins / 60)}h ago`;
  };
  const blankHost = (): Host => ({
    id: "",
    name: "",
    kind: "local",
    address: "",
    user: "",
    port: 0,
    identityFile: "",
    defaultCwd: "",
    codexPath: "",
    claudePath: "",
    opencodePath: "",
    hermesPath: "",
  });
  const blankAgent = (): Agent => ({
    id: "",
    name: "",
    description: "",
    instructions: "",
    provider: "codex",
    model: "",
    hostId: localHost?.id ?? "",
    cwd: localHost?.defaultCwd ?? "",
    color: "#3f9d6a",
    avatar: null,
    expertise: [], responsibilities: [], skills: [], collaborationEnabled: true,
    sandbox: "read-only",
  });
  const blankChannel = (): Channel => ({
    id: "",
    name: "",
    description: "",
    agentIds: [],
    messages: [],
  });
  const isSnapshot = (value: unknown): value is Snapshot =>
    typeof value === "object" &&
    value !== null &&
    "agents" in value &&
    "tasks" in value;
  function rgb(hex: string) {
    return [1, 3, 5].map((index) =>
      Number.parseInt(hex.slice(index, index + 2), 16),
    );
  }
  function luminance(channels: number[]) {
    return channels
      .map((channel) => {
        const value = channel / 255;
        return value <= 0.04045
          ? value / 12.92
          : ((value + 0.055) / 1.055) ** 2.4;
      })
      .reduce(
        (sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index],
        0,
      );
  }
  function contrast(a: number[], b: number[]) {
    const x = luminance(a),
      y = luminance(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  }
  function readable(colour: number[], background: number[], target: number) {
    for (let step = 0; step <= 100; step++) {
      const adjusted = colour.map((channel) =>
        Math.round(channel + ((target - channel) * step) / 100),
      );
      if (contrast(adjusted, background) >= 4.5) return adjusted;
    }
    return [target, target, target];
  }
  function applyAppearance(settings: Snapshot["settings"]) {
    const root = document.documentElement,
      colour = rgb(settings.accent),
      white = [255, 255, 255],
      black = [0, 0, 0];
    root.dataset.theme = settings.theme;
    root.style.setProperty("--accent", settings.accent);
    root.style.setProperty("--accent-rgb", colour.join(", "));
    root.style.setProperty(
      "--accent-light-ink",
      `rgb(${readable(colour, rgb("#fbf8f2"), 0).join(", ")})`,
    );
    root.style.setProperty(
      "--accent-dark-ink",
      `rgb(${readable(colour, rgb("#191918"), 255).join(", ")})`,
    );
    root.style.setProperty(
      "--on-accent",
      contrast(colour, white) >= contrast(colour, black) ? "#fff" : "#000",
    );
    const scale = Math.min(200, Math.max(80, settings.interfaceScale ?? 125));
    root.style.setProperty("--interface-scale", String(scale / 100));
    if (isTauri() && appliedScale !== scale) {
      appliedScale = scale;
      void getCurrentWebview().setZoom(scale / 100).catch(reason => {
        appliedScale = 0;
        error = `Could not apply interface scale: ${text(reason)}`;
      });
    }
  }
  function applySnapshot(next: Snapshot, ticket: number) {
    if (ticket < snapshotApplied) return;
    snapshotApplied = ticket;
    snapshot = next;
    if(embedded) onSnapshot?.(next); else applyAppearance(next.settings);
  }
  async function reload() {
    const ticket = ++snapshotIssued;
    try {
      applySnapshot(await bridge.getSnapshot(), ticket);
    } catch (reason) {
      error = text(reason);
    }
  }
  async function run<T>(
    action: () => Promise<T>,
    success = "",
  ): Promise<T | null> {
    const ticket = ++snapshotIssued;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await action();
      if (isSnapshot(result)) applySnapshot(result, ticket);
      notice = success;
      return result;
    } catch (reason) {
      error = text(reason);
      return null;
    } finally {
      busy = false;
    }
  }
  function debouncedReload() {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(reload, 125);
  }
  onMount(() => {
    if (embedded) return;
    let unlisten: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;
    let mounted = true;
    void (async () => {
      try {
        const stopListening = await bridge.onChanged(debouncedReload);
        if (!mounted) {
          stopListening();
          return;
        }
        unlisten = stopListening;
        await reload();
        if(isTauri()) {
          const stopDrop=await getCurrentWebview().onDragDropEvent(event=>{
            if(event.payload.type!=='drop')return;
            const payload=event.payload;
            void (async()=>{
              const physical=await getCurrentWindow().innerSize();
              const x=payload.position.x/(physical.width/window.innerWidth), y=payload.position.y/(physical.height/window.innerHeight);
              const composerNode=document.elementFromPoint(x,y)?.closest('.composer');
              if(!composerNode)return;
              const id=composerNode.closest<HTMLElement>('[data-pane-id]')?.dataset.paneId ?? 'main';
              if(id==='main') await attachNativeFiles(payload.paths); else await paneRefs[id]?.attachNativeFiles(payload.paths);
            })().catch(reason=>{error=text(reason)});
          });
          if(mounted)unlistenDrop=stopDrop;else stopDrop();
        }
      } catch (reason) {
        if (mounted) error = text(reason);
      }
    })();
    return () => {
      mounted = false;
      clearTimeout(refreshTimer);
      unlisten?.();
      unlistenDrop?.();
    };
  });
  function currentDraftKey() {
    if (pane === "task" && currentDraftId) return `draft:${currentDraftId}`;
    if (pane === "task" && selectedTaskId) return `task:${selectedTaskId}`;
    if (pane === "channel" && selectedChannelId)
      return `channel:${selectedChannelId}`;
    return null;
  }
  function saveCurrentDraft() {
    taskMenu = false;
    monitterMenu = false;
    railAgentId = null;
    sidebarViewMenu = false;
    const key = currentDraftKey();
    if(pane==='channel' && selectedChannelId)channelRecipients[selectedChannelId]=[...recipients];
    if (key) drafts[key] = composer;
    if (pane === "task" && currentDraftId && taskDrafts[currentDraftId]) {
      taskDrafts[currentDraftId] = { ...taskDrafts[currentDraftId], text: composer, title: taskTitle, agentId: taskAgentId, projectId: taskProjectId, parentId: taskParentId, nativeSessionId: taskNativeSessionId };
    }
  }
  function openOverview() {
    saveCurrentDraft();
    selectedTaskId = null;
    currentDraftId = null;
    selectedChannelId = null;
    composer = "";
    focusedAgentId = null;
    focusedProjectId = null;
    pane = "overview";
  }
  export function openTask(task: Task) {
    saveCurrentDraft();
    if (!openTaskIds.includes(task.id)) openTaskIds = [...openTaskIds, task.id];
    selectedTaskId = task.id;
    currentDraftId = null;
    selectedChannelId = null;
    composer = drafts[`task:${task.id}`] ?? "";
    focusedAgentId = null;
    focusedProjectId = null;
    pane = "task";
    scrollRevision += 1;
  }
  function closeTaskTab(id: string) {
    openTaskIds = openTaskIds.filter(openId => openId !== id);
    if (selectedTaskId === id) {
      const next = openTasks.at(-1);
      if (next) openTask(next); else openOverview();
    }
  }
  export function openChannel(channel: Channel) {
    if(!openChannelIds.includes(channel.id)) openChannelIds=[...openChannelIds,channel.id];
    saveCurrentDraft();
    selectedChannelId = channel.id;
    selectedTaskId = null;
    currentDraftId = null;
    recipients = channelRecipients[channel.id] ?? [];
    composer = drafts[`channel:${channel.id}`] ?? "";
    focusedAgentId = null;
    focusedProjectId = null;
    pane = "channel";
    scrollRevision += 1;
  }
  export function openTaskComposer(parentId: string | null = null, agentId: string | null = null, projectId?: string | null) {
    saveCurrentDraft();
    const id = crypto.randomUUID();
    const project = projectId === undefined ? (parentId ? snapshot?.tasks.find(task=>task.id===parentId)?.projectId ?? '' : focusedProjectId ?? selectedTask?.projectId ?? '') : projectId ?? '';
    const agent = agentId ?? (parentId ? selectedTask?.agentId ?? defaultAgent?.id ?? '' : focusedAgent?.id ?? defaultAgent?.id ?? '');
    taskDrafts[id] = { id, text: '', title: '', agentId: agent, projectId: project, parentId, nativeSessionId: '' };
    openDraftIds = [...openDraftIds, id]; currentDraftId = id; selectedTaskId = null; selectedChannelId = null; pane = 'task';
    composer = ''; taskTitle = ''; taskAgentId = agent; taskProjectId = project; taskParentId = parentId; taskNativeSessionId = ''; scrollRevision += 1;
  }
  function openTaskDraft(draft: TaskDraft) {
    saveCurrentDraft();
    if (!openDraftIds.includes(draft.id)) openDraftIds = [...openDraftIds, draft.id];
    currentDraftId = draft.id; selectedTaskId = null; selectedChannelId = null; pane = 'task';
    composer = draft.text; taskTitle = draft.title; taskAgentId = draft.agentId; taskProjectId = draft.projectId; taskParentId = draft.parentId; taskNativeSessionId = draft.nativeSessionId; scrollRevision += 1;
  }
  function closeTaskDraft(id: string) { saveCurrentDraft(); openDraftIds = openDraftIds.filter(item => item !== id); if (currentDraftId === id) { currentDraftId = null; const next = openDrafts.at(-1); if (next) openTaskDraft(next); else openOverview(); } }

  function isSendKey(event: KeyboardEvent) {
    return event.key === "Enter" && !event.isComposing && !event.shiftKey && !event.altKey &&
      (snapshot?.settings.sendWithEnter || event.metaKey || event.ctrlKey);
  }
  async function send() {
    if (busy || handleSlashSubmit()) return;
    if (currentDraftId) { await createTask(); return; }
    if (!selectedTask || selectedTask.status === "running" || !canSend || busy) return;
    const taskId = selectedTask.id;
    const sentDraft = composer;
    const key = `task:${taskId}`, attachmentIds=currentAttachments.map(item=>item.id);
    setComposerPending(key, true);
    try { const result = await run(() => bridge.sendMessage(taskId, promptText(sentDraft),attachmentIds)); if (result) {clearSentDraft(key, sentDraft);clearAttachments(key,attachmentIds);}  }
    finally { setComposerPending(key, false); }
  }
  function clearSentDraft(key: string, sentDraft: string) {
    if (currentDraftKey() === key) scrollRevision += 1;
    if (currentDraftKey() === key && composer === sentDraft) composer = "";
    if (drafts[key] === sentDraft) delete drafts[key];
  }
  async function createTask() {
    if (busy) return;
    const draftId = currentDraftId;
    const draft = draftId ? taskDrafts[draftId] : null;
    if (!draftId || !draft || !canSend || !taskAgentId) { error = "Choose an agent and write a message."; return; }
    const textToSend = promptText(composer), attachmentIds=currentAttachments.map(item=>item.id);
    const captured = { text: composer, title: taskTitle, agentId: taskAgentId, projectId: taskProjectId, parentId: taskParentId, nativeSessionId: taskNativeSessionId };
    const values = { modelSettings:draftModelSettings, agentId: captured.agentId, title: captured.title.trim() || textToSend.slice(0, 72) || 'New chat', nativeSessionId: captured.nativeSessionId.trim() || null, parentTaskId: captured.parentId, channelId: null, projectId: captured.projectId || null };
    taskDrafts[draftId] = { ...draft, ...captured };
    let taskId = draft.createdTaskId;
    busy = true; error = ''; notice = '';
    setComposerPending(`draft:${draftId}`, true);
    try {
      if (!taskId) {
        const task = await bridge.createTask(values);
        taskId = task.id;
        if (taskDrafts[draftId]) taskDrafts[draftId] = { ...taskDrafts[draftId], createdTaskId: taskId };
      }
      const result = await bridge.sendMessage(taskId, textToSend,attachmentIds);
      clearAttachments(`draft:${draftId}`,attachmentIds);
      if (isSnapshot(result)) applySnapshot(result, ++snapshotIssued);
      const latestText = currentDraftId === draftId ? composer : taskDrafts[draftId]?.text ?? captured.text;
      if (latestText !== captured.text) drafts[`task:${taskId}`] = latestText;
      const wasOpen = openDraftIds.includes(draftId);
      delete taskDrafts[draftId];
      delete drafts[`draft:${draftId}`];
      openDraftIds = openDraftIds.filter(id => id !== draftId);
      if (wasOpen && !openTaskIds.includes(taskId)) openTaskIds = [...openTaskIds, taskId];
      if (currentDraftId === draftId) {
        currentDraftId = null;
        const created = snapshot?.tasks.find(task => task.id === taskId);
        if (created) openTask(created);
      }
    } catch (reason) { error = text(reason); }
    finally { busy = false; setComposerPending(`draft:${draftId}`, false); }
  }

  async function saveAgent() {
    if (
      agentDraft &&
      (await run(() => bridge.saveAgent({ ...agentDraft!, expertise: (agentDraft!.expertise ?? []).map(value=>value.trim()).filter(Boolean), responsibilities: (agentDraft!.responsibilities ?? []).map(value=>value.trim()).filter(Boolean), skills: (agentDraft!.skills ?? []).map(value=>value.trim()).filter(Boolean) }), "Agent saved."))
    )
      modal = null;
  }
  async function saveHost() {
    if (
      hostDraft &&
      (await run(() => bridge.saveHost(hostDraft!), "Host saved."))
    )
      modal = null;
  }
  async function saveChannel() {
    if (
      channelDraft &&
      (await run(() => bridge.saveChannel(channelDraft!), "Channel saved."))
    )
      modal = null;
  }
  function editProject(project?: Project) {
    projectDraft = {
      id: project?.id ?? '', name: project?.name ?? '', description: project?.description ?? '',
      workspaces: (snapshot?.hosts ?? []).map(host => ({hostId: host.id, cwd: project?.workspaces.find(workspace=>workspace.hostId===host.id)?.cwd ?? ''})),
    };
    modal = 'project';
  }
  async function saveProject() {
    if (!projectDraft) return;
    const previousIds = new Set(projects.map(project=>project.id));
    const draft = {...projectDraft, name:projectDraft.name.trim(), workspaces:projectDraft.workspaces.filter(workspace=>workspace.cwd.trim()).map(workspace=>({...workspace,cwd:workspace.cwd.trim()}))};
    const result = await run(()=>bridge.saveProject(draft), 'Project saved.');
    if (result) {
      modal = null;
      const saved = result.projects.find(project=>draft.id ? project.id===draft.id : !previousIds.has(project.id));
      if (saved) openProject(saved);
    }
  }
  function openProject(project: Project) {
    saveCurrentDraft();
    currentDraftId = null;
    selectedTaskId = null;
    selectedChannelId = null;
    focusedAgentId = null;
    focusedProjectId = project.id;
    collapsedProjects[project.id] = false;
    composer = '';
    pane = 'project';
  }
  async function deleteProject() {
    if (!projectDraft?.id) return;
    const projectId = projectDraft.id;
    if (await run(()=>bridge.deleteProject(projectId), 'Project removed. Its chats are now under No project.')) {
      modal = null;
      if (focusedProjectId === projectId) openOverview();
    }
  }
  async function setSidebarView(view: SidebarView) {
    if (!snapshot || busy) return;
    await run(()=>bridge.saveSettings({...snapshot!.settings,sidebarView:view}));
  }
  async function moveTaskProject(taskId: string, projectId: string) {
    await run(()=>bridge.setTaskProject(taskId, projectId || null), 'Chat project updated.');
  }
  async function checkHost() {
    if (hostDraft) {
      const result = await run(() => bridge.probeHost(hostDraft!));
      if (result) probe = result;
    }
  }
  async function resumeTask() {
    const task = selectedTask;
    if (!task?.nativeSessionId || task.status === 'running' || task.archived || busy) return;
    const key = `task:${task.id}`;
    setComposerPending(key, true);
    try {
      if (await run(() => bridge.resumeTask(task.id))) {
        if (selectedTaskId === task.id) scrollRevision += 1;
      }
    } finally { setComposerPending(key, false); }
  }
  async function renameTask() {
    if (!selectedTask || !renameTitle.trim()) return;
    if (
      await run(
        () => bridge.renameTask(selectedTask.id, renameTitle.trim()),
        "Task renamed.",
      )
    )
      modal = null;
  }
  async function deleteArchivedTask(task: Task, removeNativeFiles: boolean) {
    const result = await bridge.deleteArchivedTask(task.id, removeNativeFiles);
    applySnapshot(result, ++snapshotIssued);
    closeTaskTab(task.id);
    delete drafts[`task:${task.id}`];
  }
  async function archiveTask(task: Task, archived = true) {
    if (await run(() => bridge.setTaskArchived(task.id, archived), archived ? "Chat archived. Restore it from Archived chats." : "Chat restored.")) {
      if (archived) closeTaskTab(task.id); else openTask({...task, archived:false});
    }
  }
  function openAgent(agent: Agent) {
    saveCurrentDraft();
    currentDraftId = null;
    selectedTaskId = null;
    selectedChannelId = null;
    composer = "";
    focusedAgentId = agent.id;
    focusedProjectId = null;
    pane = "agent";
  }
  const slashItems = $derived([
    { id: "new", label: "/new", detail: "Open a local New chat draft" },
    { id: "settings", label: "/settings", detail: "Open Monitter preferences" },
    ...(pane === "task" ? [{ id: "project", label: "/project", detail: "Choose the project for this chat" }] : []),
    ...(selectedTask?.status === "running" ? [{ id: "stop", label: "/stop", detail: "Stop this running task" }] : []),
    ...(selectedTask?.nativeSessionId && selectedTask.status !== "running" && !selectedTask.archived ? [{ id: "resume", label: "/resume", detail: "Continue this native session in Monitter" }] : []),
    ...(selectedTask?.provider === "codex" && selectedTask.nativeSessionId ? [{ id: "goal", label: "/goal", detail: "Read this Codex goal" }] : []),
  ]);
  const slashVisibleItems = $derived(slashItems.filter(item => item.label.startsWith(composer.trim().toLowerCase()) || composer.trim() === "/"));
  function isSlashCommand(value: string) {
    return /^\/[a-zA-Z0-9_-]*(?:\s|$)/.test(value.trim());
  }
  function promptText(value: string) {
    const trimmed = value.trim();
    return trimmed.startsWith("//") ? trimmed.slice(1) : trimmed;
  }
  function updateSlash(value: string) {
    slashOpen = isSlashCommand(value);
    slashIndex = 0;
  }
  function handleSlashSubmit() {
    if (!isSlashCommand(composer)) return false;
    const command = composer.trim().toLowerCase();
    void selectSlash(slashItems.find(item => item.label === command));
    return true;
  }
  async function selectSlash(item?: { id: string }) {
    if (busy) return;
    if (!item) {
      error = "This command is not available through Monitter. Use the native terminal for harness commands, or start with // to send a literal slash message.";
      return;
    }
    const task = selectedTask;
    const agentId = currentDraftId ? taskAgentId : selectedAgent?.id ?? null;
    const projectId = currentDraftId ? taskProjectId : task?.projectId ?? focusedProjectId;
    composer = "";
    slashOpen = false;
    error = "";
    notice = "";
    saveCurrentDraft();
    if (item.id === "new") openTaskComposer(null, agentId, projectId);
    else if (item.id === "settings") modal = "appearance";
    else if (item.id === "project") {
      if (currentTaskDraft) document.getElementById("task-project")?.focus();
      else if (task) { renameTitle = task.title; modal = "taskSettings"; }
    } else if (item.id === "stop" && task) await run(()=>bridge.cancelTask(task.id), "Stopping task…");
    else if (item.id === "resume") await resumeTask();
    else if (item.id === "goal" && task) {
      try {
        const result = await bridge.getTaskGoal(task.id);
        if (selectedTaskId === task.id) { goal = result; notice = result ? "Goal updated." : "This task has no active Codex goal."; }
      } catch(reason) { error = text(reason); }
    }
  }
  function handleComposerKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (slashOpen && isSlashCommand(composer)) {
      if (event.key === "ArrowDown") { event.preventDefault(); slashIndex = Math.min(slashIndex + 1, Math.max(0, slashVisibleItems.length - 1)); return; }
      if (event.key === "ArrowUp") { event.preventDefault(); slashIndex = Math.max(0, slashIndex - 1); return; }
      if (event.key === "Escape") { event.preventDefault(); slashOpen = false; return; }
      if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void selectSlash(slashVisibleItems[slashIndex]); return; }
    }
    if (isSendKey(event)) { event.preventDefault(); if (pane === "channel") void sendChannel(); else void send(); }
  }
  function queueScale(delta: number) {
    const base = scaleQueued ?? scaleInFlight ?? snapshot?.settings.interfaceScale ?? 125;
    scaleQueued = Math.max(80, Math.min(200, base + delta));
    void flushScale();
  }
  async function flushScale() {
    if (scaleSaving) return;
    scaleSaving = true;
    try {
      while (scaleQueued !== null) {
        const target = scaleQueued; scaleQueued = null; scaleInFlight = target;
        const settings = snapshot?.settings; if (!settings) break;
        const result = await bridge.saveSettings({ ...settings, interfaceScale: target });
        if (isSnapshot(result)) applySnapshot(result, ++snapshotIssued);
      }
    } catch (reason) { error = text(reason); }
    finally { scaleInFlight = null; scaleSaving = false; }
  }
  function dismissMonitterMenu(event: PointerEvent) {
    if (!(event.target instanceof Element) || !event.target.closest('.layout-control')) layoutMenu=false;
    if (!(event.target instanceof Element) || !event.target.closest('.monitter-menu')) monitterMenu = false;
    if (!(event.target instanceof Element) || !event.target.closest('.task-overflow')) taskMenu = false;
    if (!(event.target instanceof Element) || !event.target.closest('.sidebar-views')) sidebarViewMenu = false;
    if (!(event.target instanceof Element) || !event.target.closest('.agent-rail, .rail-chats')) railAgentId = null;
  }
  function handleShortcuts(event: KeyboardEvent) {
    if(embedded ? !active : activePaneId !== 'main' && !modal && !palette) return;
    if(event.key==='Escape' && compactDetail && showDetail && !modal && !palette && !taskMenu && !monitterMenu && !railAgentId && !sidebarViewMenu && !layoutMenu) {event.preventDefault();showDetail=false;return;}
    if(event.key==='Escape' && layoutMenu) {event.preventDefault();layoutMenu=false;return;}
    if (event.key === 'Escape' && (monitterMenu || taskMenu || sidebarViewMenu || railAgentId)) { event.preventDefault(); monitterMenu = false; taskMenu = false; sidebarViewMenu = false; railAgentId = null; return; }

    if ((event.metaKey || event.ctrlKey) && event.key === ',' && !event.altKey && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      monitterMenu = false; sidebarViewMenu = false; taskMenu = false; railAgentId = null;
      palette = null; modal = 'appearance';
      return;
    }

    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.isComposing && (event.key === "+" || event.key === "=" || event.code === "NumpadAdd" || event.key === "-" || event.code === "NumpadSubtract")) {
      event.preventDefault();
      queueScale(event.key === "-" || event.code === "NumpadSubtract" ? -5 : 5);
      return;
    }
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey || event.isComposing) return;
    const key = event.key.toLowerCase();
    if (key !== "k" && key !== "p") return;
    event.preventDefault();
    if (modal) return;
    palette = key === "k" ? "switch" : "controls";
  }

  const switchItems = $derived([
    ...Object.values(taskDrafts).map(draft => ({id:`draft:${draft.id}`,label:draft.title || 'New chat',group:'Draft chats',detail:snapshot?.agents.find(agent=>agent.id===draft.agentId)?.name ?? 'Agent'})),
    ...(snapshot?.tasks ?? []).toSorted((a,b)=>b.updatedAt-a.updatedAt).map(task=>({
      id:`task:${task.id}`,label:task.title,group:task.archived ? "Archived chats" : "Chats",
      detail:`${task.archived ? "Select to restore · " : ""}${snapshot?.agents.find(agent=>agent.id===task.agentId)?.name ?? "Agent"}`,
      keywords:`${task.provider} ${task.cwd}`,
    })),
    ...(snapshot?.channels ?? []).map(channel=>({id:`channel:${channel.id}`,label:channel.name,group:"Channels",detail:channel.description})),
    ...(snapshot?.agents ?? []).map(agent=>({id:`agent:${agent.id}`,label:agent.name,group:"Agents",detail:`${agent.provider} · ${agent.description}`})),
    ...projects.map(project=>({id:`project:${project.id}`,label:project.name,group:'Projects',detail:project.description || `${activeTasks.filter(task=>task.projectId===project.id).length} chats`})),
  ]);
  const controlItems = $derived([
    {id:"new-task",label:"New chat",group:"Create"},
    {id:"new-agent",label:"New agent",group:"Create"},
    {id:"agent-directory",label:"Agent directory",detail:"Find agents by expertise, responsibility, or skill",group:"Collaborate"},
    {id:"new-channel",label:"New channel",group:"Create"},
    {id:"new-project",label:"New project",group:"Create"},
    ...(['standard','activity','projects'] as SidebarView[]).map(view=>({id:`sidebar:${view}`,label:`${view[0].toUpperCase()+view.slice(1)} sidebar view`,group:'Sidebar',checked:sidebarView===view})),
    {id:"appearance",label:"Appearance and preferences",detail:"Accent colour, scale, theme and conversation settings",group:"Settings"},
    {id:"hosts",label:"Manage hosts",detail:"Local and SSH connections",group:"Settings"},
    {id:"archived",label:"Archived chats",detail:"Restore or permanently delete archived chats",group:"Workspace"},
    {id:"tools",label:"Show tool activity",checked:snapshot?.settings.showToolActivity !== false,group:"Toggles"},
    {id:"reasoning",label:"Show reasoning summaries",checked:snapshot?.settings.showReasoningSummaries !== false,group:"Toggles"},
    {id:"dim-panes",label:"Dim inactive panes",checked:snapshot?.settings.dimInactivePanes ?? true,group:"Toggles"},
    {id:"enter",label:"Enter to send",checked:snapshot?.settings.sendWithEnter ?? false,group:"Toggles"},
    {id:"detail",label:"Show run detail",checked:showDetail,group:"Toggles"},
    {id:"scale-up",label:"Increase interface scale",detail:`${snapshot?.settings.interfaceScale ?? 125}% → up to 200%`,group:"Appearance",disabled:(snapshot?.settings.interfaceScale ?? 125)>=200},
    {id:"scale-down",label:"Decrease interface scale",group:"Appearance",disabled:(snapshot?.settings.interfaceScale ?? 125)<=80},
    {id:"scale-reset",label:"Reset interface scale to 125%",group:"Appearance"},
    ...["light","dark","system"].map(theme=>({id:`theme:${theme}`,label:`${theme[0].toUpperCase()+theme.slice(1)} theme`,checked:snapshot?.settings.theme===theme,group:"Appearance"})),
    ...(selectedTask ? [{id:"archive",label:"Archive current chat",group:"Current chat",disabled:selectedTask.status==="running"},
      ...(selectedTask.status==="running" ? [{id:"stop",label:"Stop current chat",group:"Current chat"}] : [])] : []),
  ].map(item=>({...item,disabled:busy || ("disabled" in item && item.disabled)})));
  async function selectPalette(id: string) {
    if (palette === "switch") {
      palette = null;
      const [kind,itemId] = id.split(":");
      if (kind === "draft") {
        const draft = taskDrafts[itemId]; if (draft) openTaskDraft(draft);
      } else if (kind === "task") {
        const task = snapshot?.tasks.find(task=>task.id===itemId);
        if (task?.archived) await archiveTask(task,false); else if (task) openTask(task);
      } else if (kind === "channel") {
        const channel = snapshot?.channels.find(channel=>channel.id===itemId);
        if (channel) openChannel(channel);
      } else if (kind === 'project') {
        const project = projects.find(project=>project.id===itemId);
        if (project) openProject(project);
      } else {
        const agent = snapshot?.agents.find(agent=>agent.id===itemId);
        if (agent) openAgent(agent);
      }
      return;
    }
    const settings = snapshot?.settings;
    if (!settings || busy) return;
    if (id === "tools") await run(()=>bridge.saveSettings({...settings,showToolActivity:!settings.showToolActivity}));
    else if (id === "reasoning") await run(()=>bridge.saveSettings({...settings,showReasoningSummaries:!settings.showReasoningSummaries}));
    else if (id === "dim-panes") await run(()=>bridge.saveSettings({...settings,dimInactivePanes:!(settings.dimInactivePanes ?? true)}));
    else if (id === "enter") await run(()=>bridge.saveSettings({...settings,sendWithEnter:!settings.sendWithEnter}));
    else if (id === "detail") showDetail = !showDetail;
    else if (id.startsWith("scale-")) { if (id === "scale-reset") { scaleQueued = 125; void flushScale(); } else queueScale(id === "scale-up" ? 5 : -5); }
    else if (id.startsWith("theme:")) await run(()=>bridge.saveSettings({...settings,theme:id.slice(6) as "light"|"dark"|"system"}));
    else if (id.startsWith('sidebar:')) await setSidebarView(id.slice(8) as SidebarView);
    else {
      palette = null;
      if (id === "new-task") openTaskComposer();
      if (id === "new-agent") { agentDraft=blankAgent(); modal="agent"; }
      if (id === "agent-directory") { directoryQuery=""; modal="directory"; }
      if (id === "new-channel") { channelDraft=blankChannel(); modal="channel"; }
      if (id === 'new-project') editProject();
      if (id === "hosts" || id === "appearance") modal=id;
      if (id === "archive" && selectedTask) await archiveTask(selectedTask);
      if (id === "archived") modal = 'archived';
      if (id === "stop" && selectedTask) await run(()=>bridge.cancelTask(selectedTask.id));
    }
  }
  function toggleRecipient(id: string) {
    recipients = recipients.includes(id)
      ? recipients.filter((entry) => entry !== id)
      : [...recipients, id];
  }
  async function stopChannel() {
    for (const task of activeChannelTasks) await run(() => bridge.cancelTask(task.id), "Stopping channel task…");
  }
  async function sendChannel() {
    if (busy || activeChannelTasks.length || handleSlashSubmit()) return;
    if (!activeChannel || !canSend || !recipients.length) { error = "Choose at least one agent to receive this channel message."; return; }
    const channelId = activeChannel.id, sentDraft = composer, agentIds = [...recipients], key = `channel:${channelId}`, attachmentIds=currentAttachments.map(item=>item.id);
    setComposerPending(key, true);
    try { if (await run(() => bridge.sendChannelMessage(channelId, promptText(sentDraft), agentIds,attachmentIds))) {clearSentDraft(key, sentDraft);clearAttachments(key,attachmentIds);}  }
    finally { setComposerPending(key, false); }
  }
</script>

<svelte:head><meta name="theme-color" content="#e9e3d8" /></svelte:head>
<svelte:window onkeydown={handleShortcuts} onpointerdown={dismissMonitterMenu} />

{#snippet slashMenu()}
  {#if slashOpen && isSlashCommand(composer)}
    <div class="slash-menu" role="menu" aria-label="Monitter commands">
      <p class="slash-caption">Monitter commands</p>
      {#each slashVisibleItems as item, index}
        <button type="button" role="menuitem" class:active={index===slashIndex} disabled={busy} onclick={()=>selectSlash(item)}><b>{item.label}</b><span>{item.detail}</span></button>
      {:else}
        <p>Use the native terminal for harness commands. Start with // to send a literal slash message.</p>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet sidebarChat(task: Task, detail = false)}
  {@const agent = snapshot?.agents.find(item=>item.id===task.agentId)}
  <div class="task-row" class:current={task.id === (activePaneId==='main'?selectedTaskId:paneSelections[activePaneId])} data-task-id={task.id}>
    <button class="task-select" onclick={() => routeTask(task)} title={task.title}>
      {#if detail}<span class="avatar small" style={`--agent-color:${agent?.color ?? '#3f9d6a'}`} title={agent?.name ?? 'Agent'} aria-label={agent?.name ?? 'Agent'}>{#if avatarSrc(agent)}<img src={avatarSrc(agent)!} alt="" />{:else}{(agent?.name ?? 'A').slice(0,1).toUpperCase()}{/if}</span>{/if}
      <span class={`dot ${task.status}`}></span><span class="chat-copy"><span>{task.title}</span>
        {#if detail}<span class="chat-meta">{snapshot?.agents.find(agent=>agent.id===task.agentId)?.name ?? 'Agent'} · {relative(task.updatedAt)}</span>{/if}
      </span>
    </button>
    <small>{task.status === 'running' ? 'live' : relative(task.updatedAt)}</small>
    <div class="chat-actions">
      <button aria-label={`Archive chat ${task.title}`} title="Archive chat" disabled={busy || task.status === 'running'} onclick={()=>archiveTask(task)}><Archive size={12}/></button>
    </div>
  </div>
{/snippet}

{#snippet attachmentTools()}
  <div class="attachment-tools"><button class="icon" aria-label="Attach files" title="Attach files, or drop or paste them here" disabled={busy || filesBusy || selectedTask?.status==='running'} onclick={()=>filePicker?.click()}>{#if filesBusy}<LoaderCircle class="spin" size={16}/>{:else}<Paperclip size={16}/>{/if}</button>{#if filesBusy}<small role="status">Saving attachments…</small>{/if}</div>
  <input class="attachment-input" bind:this={filePicker} type="file" multiple aria-label="Choose attachments" onchange={event=>{const files=Array.from(event.currentTarget.files??[]);event.currentTarget.value='';void attachFiles(files)}}/>
{/snippet}

{#snippet messageAvatar(agent: Agent | null | undefined)}
  {#if agent}<span class="avatar message-avatar" style={`--agent-color:${agent.color}`} title={agent.name}>{#if avatarSrc(agent)}<img src={avatarSrc(agent)!} alt=""/>{:else}{agent.name.slice(0,1).toUpperCase()}{/if}</span>{/if}
{/snippet}

{#snippet workspaceView()}
  <section class="workspace" use:watchPane>
    <header class="topbar" data-tauri-drag-region>
      <nav class="tabs" aria-label="Open tasks" data-tauri-drag-region ondragover={tabBarOver} ondrop={tabBarDrop}>
        <button class="tab" class:active={pane === "overview"}
          aria-pressed={pane === "overview"} onclick={openOverview}>Overview</button>
        {#each openDrafts as draft (draft.id)}
          <div class="tab-entry" class:active={currentDraftId === draft.id}><button class="tab" draggable={!composerPending[`draft:${draft.id}`]} ondragstart={event=>dragTab(event,'draft',draft.id)} onclick={() => openTaskDraft(draft)}><span class="dot idle"></span><span>{draft.title || 'New chat'}</span></button><button class="close-tab" aria-label="Close draft" onclick={() => closeTaskDraft(draft.id)}><X size={12}/></button></div>
        {/each}
        {#each openTasks as task (task.id)}
          <div class="tab-entry" class:active={selectedTaskId === task.id}>
            <button class="tab" draggable={!composerPending[`task:${task.id}`]} ondragstart={event=>dragTab(event,'task',task.id)} aria-pressed={selectedTaskId === task.id}
              onclick={() => openTask(task)} title={task.title}>
              <span class={`dot ${task.status}`}></span><span>{task.title}</span>
            </button>
            <button class="close-tab" aria-label={`Close tab ${task.title}`}
              onclick={() => closeTaskTab(task.id)}><X size={12} /></button>
          </div>
        {/each}
        {#if pane === "agent" && focusedAgent}<button class="tab active" aria-pressed="true"><Bot size={13}/>{focusedAgent.name}</button>{/if}
        {#if pane === 'project' && focusedProject}<button class="tab active" aria-pressed="true"><Folder size={13}/>{focusedProject.name}</button>{/if}
        {#each openChannelIds as channelId}{@const channel=snapshot?.channels.find(item=>item.id===channelId)}{#if channel}
          <div class="tab-entry" class:active={selectedChannelId===channelId}><button class="tab" aria-pressed={selectedChannelId===channelId} draggable={!composerPending[`channel:${channelId}`]} ondragstart={event=>dragTab(event,'channel',channelId)} onclick={()=>openChannel(channel)}><Radio size={13}/><span>{channel.name}</span></button><button class="close-tab" aria-label={`Close channel tab ${channel.name}`} onclick={()=>{saveCurrentDraft();openChannelIds=openChannelIds.filter(id=>id!==channelId);if(selectedChannelId===channelId)openOverview()}}><X size={12}/></button></div>
        {/if}{/each}
      </nav>
      <div class="top-actions" data-tauri-drag-region>
        <div class="layout-control"><button class="icon" bind:this={layoutAnchor} aria-label="Pane layout" title="Pane layout" aria-expanded={layoutMenu} onclick={()=>layoutMenu=!layoutMenu}><LayoutGrid size={16}/></button>
          {#if layoutMenu && layoutAnchor}<div class="view-menu floating-panel" role="menu" aria-label="Pane layout" use:floating={{anchor:layoutAnchor}}>
            <button role="menuitem" onclick={()=>setLayout('single')}>One pane</button><button role="menuitem" onclick={()=>setLayout('columns')}>Two columns</button><button role="menuitem" onclick={()=>setLayout('grid')}>2 × 2 grid</button>
          </div>{/if}
        </div>
        <span class="environment"
          ><i></i>{selectedHost?.kind === "ssh"
            ? "Remote host"
            : "Local host"}</span
        ><button
          class="icon"
          aria-label={showDetail ? "Hide right sidebar" : "Show right sidebar"}
          title={showDetail ? "Hide right sidebar" : "Show right sidebar"}
          aria-pressed={showDetail}
          onclick={() => (showDetail = !showDetail)}
          ><PanelRight size={16} /></button
        >
      </div>
    </header>
    {#if error}<div class="alert error" role="alert">
        <X size={16} /><span>{error}</span><button
          aria-label="Dismiss error"
          onclick={() => (error = "")}><X size={15} /></button
        >
      </div>{/if}{#if notice}<div class="alert notice" role="status">
        <Check size={16} /><span>{notice}</span><button
          aria-label="Dismiss notice"
          onclick={() => (notice = "")}><X size={15} /></button
        >
      </div>{/if}{#if !bridge.available}<div class="preview-banner">
        <Command size={14} /> Browser design preview — connect the native app to
        use hosts, agents, and tasks.
      </div>{/if}
    {#if !snapshot}<div class="loading">
        <LoaderCircle size={22} /><span>Loading your workspace…</span
        >{#if error}<button onclick={reload}>Try again</button>{/if}
      </div>
    {:else if pane === 'project' && focusedProject}<section class="overview project-overview">
      <div class="overview-head"><div><p class="eyebrow">PROJECT</p><h1>{focusedProject.name}</h1><p>{focusedProject.description || 'A shared project for your agents.'}</p></div>
        <div class="project-overview-actions"><button class="secondary" aria-label={`Edit project ${focusedProject.name}`} onclick={()=>editProject(focusedProject)}><Settings2 size={15}/>Edit project</button><button class="primary" onclick={()=>openTaskComposer(null,null,focusedProject.id)}><Plus size={16}/>New chat</button></div>
      </div>
      {#if focusedProject.workspaces.length}<dl class="project-folders">{#each focusedProject.workspaces as workspace}<div><dt><HardDrive size={13}/>{snapshot.hosts.find(host=>host.id===workspace.hostId)?.name ?? 'Host'}</dt><dd>{workspace.cwd}</dd></div>{/each}</dl>{/if}
      <div class="agent-chats">{#each activityTasks.filter(task=>task.projectId===focusedProject.id) as task (task.id)}<button class="overview-task" onclick={()=>openTask(task)}><span class={`dot ${task.status}`}></span><div><b>{task.title}</b><small>{snapshot.agents.find(agent=>agent.id===task.agentId)?.name ?? 'Agent'} · {task.provider} · {relative(task.updatedAt)}</small></div></button>{:else}<p class="hint">No chats yet. Choose any agent to start working on this project.</p>{/each}</div>
    </section>
    {:else if pane === "agent" && focusedAgent}<section class="overview">
        <div class="overview-head"><div><p class="eyebrow">AGENT · {focusedAgent.provider}</p><h1>{focusedAgent.name}</h1><p>{focusedAgent.description}</p></div>
          <button class="primary" onclick={()=>openTaskComposer(null,focusedAgent.id)}><Plus size={16}/>New chat</button></div>
        <div class="agent-chats">{#each snapshot.tasks.filter(task=>task.agentId===focusedAgent.id && !task.archived) as task}<button class="overview-task" onclick={()=>openTask(task)}><span class={`dot ${task.status}`}></span><div><b>{task.title}</b><small>{relative(task.updatedAt)}</small></div></button>{:else}<p class="hint">No chats yet. Start one with {focusedAgent.name}.</p>{/each}</div>
      </section>
    {:else if pane === "overview" || (pane === "task" && !selectedTask && !currentTaskDraft) || (pane === "channel" && !activeChannel) || (pane === 'project' && !focusedProject)}<section
        class="overview"
      >
        <div class="overview-head">
          <div>
            <p class="eyebrow">YOUR WORKSPACE</p>
            <h1>
              {snapshot.agents.length
                ? "Everything in motion."
                : "Start with one agent."}
            </h1>
            <p>
              {snapshot.agents.length
                ? "Tasks stay with the agent, host and folder that started them."
                : "Create a Codex agent, choose where it works, then give it a task."}
            </p>
          </div>
          <button
            class="primary"
            onclick={() =>
              snapshot!.agents.length
                ? openTaskComposer()
                : ((agentDraft = blankAgent()), (modal = "agent"))}
            ><Plus size={16} />{snapshot.agents.length
              ? "Start a task"
              : "Create agent"}</button
          >
        </div>
        {#if !snapshot.hosts.length || !snapshot.agents.length}<section
            class="onboarding"
          >
            <div class="onboard-number">01</div>
            <div>
              <h2>
                {snapshot.hosts.length
                  ? "Create your first agent"
                  : "Connect this Mac"}
              </h2>
              <p>
                {snapshot.hosts.length
                  ? "Codex is ready for local work. Give your agent a clear purpose and folder."
                  : "Monitter uses your existing Codex sign-in and CLI. It does not copy credentials or alter your setup."}
              </p>
              <button
                class="secondary"
                onclick={() =>
                  !snapshot!.hosts.length
                    ? ((hostDraft = blankHost()), (modal = "host"))
                    : ((agentDraft = blankAgent()), (modal = "agent"))}
                >{snapshot.hosts.length
                  ? "Create Codex agent"
                  : "Add local host"}</button
              >
            </div>
          </section>{/if}
        <div class="overview-grid">
          {#each [["running", "Running"], ["idle", "Ready"], ["completed", "Finished"], ["error", "Needs attention"]] as [status, label]}{@const tasks =
              snapshot.tasks.filter((task) => task.status === status && !task.archived)}
            <section class="status-group">
              <header>
                <h2>{label}</h2>
                <span>{tasks.length}</span>
              </header>
              {#if tasks.length}{#each tasks as task}<button
                    class="overview-task"
                    onclick={() => openTask(task)}
                    ><span class={`dot ${task.status}`}></span>
                    <div>
                      <b>{task.title}</b><small
                        >{snapshot.agents.find(
                          (agent) => agent.id === task.agentId,
                        )?.name ?? "Unknown agent"} · {task.provider}</small
                      >
                    </div>
                    <span>{relative(task.updatedAt)}</span></button
                  >{/each}{:else}<p>No tasks here.</p>{/if}
            </section>{/each}
        </div>
      </section>
    {:else if pane === "channel" && activeChannel}<section class="conversation">
        <div class="conversation-head">
          <div>
            <p class="eyebrow">CHANNEL</p>
            <h1>{activeChannel.name}</h1>
            <p>
              {activeChannel.description ||
                "Send only to the agents you choose."}
            </p>
          </div>
          <button
            class="secondary"
            onclick={() => {
              channelDraft = {
                ...activeChannel,
                agentIds: [...activeChannel.agentIds],
                messages: activeChannel.messages,
              };
              modal = "channel";
            }}><Settings2 size={15} /> Edit channel</button
          >
        </div>
        <MessagePane resetKey={`channel:${activeChannel.id}:${scrollRevision}`}>
          {#if activeChannel.messages.length}{#each activeChannel.messages as message}<article
                class:user={message.role === "user"}
                class="message"
              >
                <div class="message-meta">
                  {@render messageAvatar(snapshot.agents.find(agent=>agent.id===message.agentId))}
                  <span
                    >{message.role === "user"
                      ? "You"
                      : (snapshot.agents.find((a) => a.id === message.agentId)
                          ?.name ?? "Agent")}</span
                  ><time>{date(message.createdAt)}</time>
                </div>
                <Markdown text={message.text} /><AttachmentList attachments={message.attachments ?? []}/>
              </article>{/each}{:else}<div class="blank-conversation">
              <MessageSquare size={24} />
              <h2>Start this channel</h2>
              <p>
                Pick the agents who should receive your message. Each gets an
                explicit linked task.
              </p>
            </div>{/if}
        </MessagePane>
        <div class="composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
          {@render slashMenu()}
          <textarea
            bind:value={composer}
            aria-label="Channel message"
            placeholder="Message this channel…"
            oninput={(event) => updateSlash(event.currentTarget.value)}
            onkeydown={handleComposerKeydown}
          ></textarea>
          <div class="composer-footer">
            {@render attachmentTools()}
            <div class="recipient-picker">
              <span>Send to</span
              >{#each snapshot.agents.filter( (a) => activeChannel.agentIds.includes(a.id), ) as agent}<button
                  class:selected={recipients.includes(agent.id)}
                  aria-pressed={recipients.includes(agent.id)}
                  onclick={() => toggleRecipient(agent.id)}>{agent.name}</button
                >{/each}
            </div>
            {#if activeChannelTasks.length}<button class="danger composer-control" aria-label="Stop channel tasks" title={activeChannelStarting ? "Starting channel — stop" : "Stop channel tasks"} onclick={stopChannel}>{#if activeChannelStarting}<LoaderCircle class="spin" size={15}/>{:else}<Square size={15}/>{/if}</button>{:else}<button class="primary composer-control" aria-label={activeChannelStarting ? "Starting channel message" : "Send channel message"} title={activeChannelStarting ? "Starting…" : "Send"} disabled={busy || !canSend} onclick={sendChannel}>{#if activeChannelStarting}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button>{/if}
          </div>
        </div>
      </section>
    {:else if currentTaskDraft}
      <section class="draft-layout" aria-label="New chat draft">
        <div class="draft-content">
          <div class="draft-intro">
            <p class="eyebrow">NEW CHAT</p>
            <h1>What would you like to work on?</h1>
            <p>Ask a question, explore a project, or describe a change. Your agent starts when you send.</p>
          </div>
          <div class="draft-options form-grid">
            <label>Agent<select aria-label="Agent" bind:value={taskAgentId} disabled={busy || !!currentTaskDraft.createdTaskId}>{#each snapshot.agents as agent}<option value={agent.id}>{agent.name} · {agent.provider}</option>{/each}</select></label>
            <label>Project<select id="task-project" aria-label="Project" bind:value={taskProjectId} disabled={busy || !!currentTaskDraft.createdTaskId}><option value="">No project</option>{#each projects as project}<option value={project.id}>{project.name}</option>{/each}</select></label>
          </div>
          {#if taskFormAgent}<p class="task-workspace-preview"><Folder size={13}/><span><b>{snapshot.hosts.find(host=>host.id===taskFormAgent.hostId)?.name ?? 'Host'}</b><code>{taskFormCwd}</code></span></p>{/if}
          <div class="composer draft-composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
            {@render slashMenu()}
            <textarea bind:value={composer} aria-label="Task message" placeholder="Describe what you want this agent to do…" oninput={(event)=>updateSlash(event.currentTarget.value)} onkeydown={handleComposerKeydown}></textarea>
            <div class="composer-footer">{@render attachmentTools()}<div class="composer-right"><ModelPicker target={currentTaskDraft.createdTaskId?{taskId:currentTaskDraft.createdTaskId}:{agentId:taskAgentId,projectId:taskProjectId||null}} settings={draftModelSettings} fallbackModel={taskFormAgent?.model??''} disabled={busy||filesBusy} onchange={changeModel}/><button class="primary composer-control" aria-label={composerPending[`draft:${currentDraftId}`] ? "Starting task" : "Send task message"} title={composerPending[`draft:${currentDraftId}`] ? "Starting…" : "Send"} disabled={busy || !canSend || !taskAgentId} onclick={send}>{#if composerPending[`draft:${currentDraftId}`]}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button></div></div>
          </div>
          <div class="suggestions" aria-label="Suggestions">
            <button onclick={()=>{composer='Review this project and suggest the next concrete step.'; updateSlash(composer);}}>Review this project</button>
            <button onclick={()=>{composer='Investigate the current issue and report what you find.'; updateSlash(composer);}}>Investigate an issue</button>
            <button onclick={()=>{composer='Plan the implementation before making changes.'; updateSlash(composer);}}>Plan work</button>
          </div>
          <details class="draft-advanced">
            <summary>Chat options</summary>
            <div class="form-grid">
              <label>Task title <span class="optional">Optional</span><input aria-label="Task title" bind:value={taskTitle} disabled={busy || !!currentTaskDraft.createdTaskId} placeholder="Named from your first message"/></label>
              <label>Existing native session ID <span class="optional">Optional</span><input aria-label="Existing native session ID" bind:value={taskNativeSessionId} disabled={busy || !!currentTaskDraft.createdTaskId} placeholder="Resume an idle native session"/></label>
            </div>
          </details>
          {#if taskParentId}<p class="hint">Delegated from {snapshot.tasks.find(task=>task.id===taskParentId)?.title ?? 'the selected task'}.</p>{/if}
        </div>
      </section>
    {:else if selectedTask}<section class="task-layout" class:detail-hidden={!showDetail || compactDetail} class:compact-detail={compactDetail}>
        <section class="conversation">
          <div class="conversation-head task-heading">
            <h1>{selectedTask.title}</h1>
            <div class="task-actions">
              {#if selectedTask.status === "running"}<button class="danger icon" aria-label="Stop" title="Stop" disabled={busy} onclick={() => run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")}><Square size={14}/></button>{/if}
              {#if selectedTask.nativeSessionId && ["interrupted","error"].includes(selectedTask.status)}<button class="icon" aria-label="Resume session" disabled={busy || selectedTask.archived} title="Reconnect and resume this session" onclick={resumeTask}><RotateCw size={15}/></button>{/if}
              <div class="task-overflow"><button bind:this={taskMenuAnchor} class="icon" aria-label="Task actions" aria-expanded={taskMenu} onclick={()=>taskMenu=!taskMenu}><MoreHorizontal size={17}/></button>{#if taskMenu && taskMenuAnchor}<div use:floating={{anchor:taskMenuAnchor}} class="task-menu floating-panel"><button onclick={()=>{taskMenu=false;openTaskComposer(selectedTask.id)}}><Bot size={14}/>Delegate</button><button aria-label="Task settings" onclick={()=>{taskMenu=false;renameTitle=selectedTask.title;taskProjectId=selectedTask.projectId??'';modal='taskSettings'}}><Settings2 size={14}/>Task settings</button><button aria-label={showDetail ? "Hide run detail" : "Show run detail"} onclick={()=>{taskMenu=false;showDetail=!showDetail}}><PanelRight size={14}/>{showDetail ? "Hide" : "Show"} run detail</button></div>{/if}</div>
            </div>
          </div>
          <TaskActivity {goal} {goalNote} tools={computerTools} onstop={() => selectedTask && run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")} disabled={busy} />
          <MessagePane resetKey={`task:${selectedTask.id}:${scrollRevision}`}>
            {#if conversationItems.length}{#each conversationItems as item (item.value.id)}
              {#if item.type === "activity"}<RunActivity event={item.value} />
              {:else}{@const message = item.value}<article
                  class:user={message.role === "user"}
                  class:system={message.role === "system"}
                  class="message"
                >
                  <div class="message-meta">
                    {@render messageAvatar(message.senderAgentId ? snapshot.agents.find(agent=>agent.id===message.senderAgentId) : message.role==='assistant' ? selectedAgent : null)}
                    <span
                      >{senderName(message) ?? (message.role === "user" ? "You" : message.role === "assistant" ? (selectedAgent?.name ?? "Agent") : "System")}</span
                    ><time>{date(message.createdAt)}</time>
                  </div>
                  <Markdown text={message.text} /><AttachmentList attachments={message.attachments ?? []}/>
                </article>{/if}{/each}{:else}<div class="blank-conversation">
                <Terminal size={24} />
                <h2>No messages yet</h2>
                <p>
                  Describe what you want this agent to do. Its actual output
                  will appear here.
                </p>
              </div>{/if}
          </MessagePane>
          <div class="composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
            {@render slashMenu()}
            <textarea
              bind:value={composer}
              aria-label="Task message"
              placeholder={`Message ${selectedAgent?.name ?? "agent"}…`}
              disabled={selectedTask.status === "running"}
              oninput={(event) => updateSlash(event.currentTarget.value)}
              onkeydown={handleComposerKeydown}
            ></textarea>
            <div class="composer-footer">
              <div class="composer-left">{@render attachmentTools()}<span>{snapshot.settings.sendWithEnter ? "↵ send · ⇧↵ new line" : `${modifierLabel}↵ send · ↵ new line`}</span></div>
              <div class="composer-right">
                <ModelPicker target={{taskId:selectedTask.id}} settings={selectedTask.modelSettings??null} fallbackModel={selectedTask.model} disabled={busy||selectedTask.status==='running'} onchange={changeModel}/>
{#if selectedTask.status === "running"}<button class="danger composer-control" aria-label="Stop current task" title={selectedTaskStepping ? "Stop current task" : "Starting task — stop"} onclick={() => run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")}>{#if selectedTaskStepping}<Square size={15}/>{:else}<LoaderCircle class="spin" size={15}/>{/if}</button>{:else}<button class="primary composer-control" aria-label={selectedTaskStarting ? "Starting task" : "Send task message"} title={selectedTaskStarting ? "Starting…" : "Send"} disabled={busy || !canSend} onclick={send}>{#if selectedTaskStarting}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button>{/if}
              </div>
            </div>
          </div>
        </section>
        {#if compactDetail && showDetail}<button class="detail-backdrop" aria-label="Dismiss right sidebar" onclick={()=>showDetail=false}></button>{/if}
        <aside class="run-detail" class:closed={!showDetail} aria-label="Right sidebar">
            <div class="detail-tabs">
              <button class:active={detailTab==='run' || gitState.repository!==true} aria-pressed={detailTab==='run' || gitState.repository!==true} onclick={()=>detailTab='run'}>Run detail</button>
              {#if gitState.repository}<button class:active={detailTab==='git'} aria-pressed={detailTab==='git'} onclick={()=>detailTab='git'}>Git changes</button>{/if}
              <button aria-label="Close run detail" onclick={() => (showDetail = false)}><X size={14} /></button>
            </div>
            <div class="git-slot" class:hidden={detailTab!=='git' || gitState.repository!==true}>
              <GitPane bind:this={gitPane} taskId={selectedTask.id} active={showDetail} onStatus={value=>{gitState=value}}/>
            </div>
            <div class="detail-scroll" class:hidden={detailTab==='git' && gitState.repository===true}>
              {#if gitState.error}<p class="git-probe-error">Git status unavailable. <button title={gitState.error} onclick={()=>gitPane?.refreshStatus()}>Retry</button><small>{gitState.error}</small></p>{/if}
              <details class="agent-identity" open aria-label="Agent identity"><summary><span class="avatar identity-avatar" style={`--agent-color:${selectedAgent?.color ?? '#3f9d6a'}`}>{#if avatarSrc(selectedAgent)}<img src={avatarSrc(selectedAgent)!} alt="" />{:else}{(selectedAgent?.name ?? 'A').slice(0,1).toUpperCase()}{/if}</span><span><b>{selectedAgent?.name ?? 'Agent'}</b><small>{selectedTask.provider}{selectedTask.model ? ` · ${selectedTask.model}` : ''}</small></span></summary><div class="identity-actions"><button onclick={()=>{if(selectedAgent){agentDraft={...selectedAgent};modal='agent'}}}>Change avatar</button><p>{selectedAgent?.description || 'No agent description.'}</p></div></details>
              <dl>
                <div>
                  <dt>harness</dt>
                  <dd>{selectedTask.provider}</dd>
                </div>
                <div>
                  <dt>model</dt>
                  <dd>{selectedTask.model || "Harness default"}</dd>
                </div>
                <div>
                  <dt>host</dt>
                  <dd>{selectedHost?.name ?? "Unknown host"}</dd>
                </div>
                <div class="task-folder"><dt>folder</dt><dd><code>{selectedTask.cwd || "No folder set"}</code></dd></div>
                <div><dt>project</dt><dd>{projects.find(project=>project.id===selectedTask.projectId)?.name ?? 'No project'}</dd></div>
                <div>
                  <dt>permissions</dt>
                  <dd>{selectedTask.sandbox === "harness-configured" ? "Harness permissions" : selectedTask.sandbox}</dd>
                </div>
                {#if selectedTask.nativeSessionId}<div>
                    <dt>native session</dt>
                    <dd class="session-id" title={selectedTask.nativeSessionId}>
                      {selectedTask.nativeSessionId}
                    </dd>
                  </div>{/if}
              </dl>
              <section class="detail-section">
                {#if goalError}<details class="goal-lookup-error"><summary>Goal status unavailable</summary><p>{goalError}</p></details>{/if}
                <h3>TIMELINE</h3>
                {#if visibleEvents.length}{#each visibleEvents as event}<article
                      class={`event ${event.kind}`}
                    >
                      <time>{date(event.createdAt)}</time>
                      <div>
                        <b>{event.title}</b
                        >{#if event.detail}{#if longDetail(event.detail)}<p
                              class="event-preview"
                            >
                              {detailPreview(event.detail)}
                            </p>
                            <details class="event-overflow">
                              <summary
                                aria-label={`Show full detail for ${event.title || "timeline event"}`}
                                >Show full detail · {detailLength(event.detail)}
                                characters</summary
                              >
                              <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable diagnostic text must be keyboard reachable) -->
                              <pre
                                tabindex="0"
                                aria-label={`Full detail for ${event.title || "timeline event"}`}>{event.detail}</pre>
                            </details>{:else}<p>{event.detail}</p>{/if}{/if}
                      </div>
                    </article>{/each}{:else}<p class="detail-empty">
                    Run events will appear here as the CLI reports them.
                  </p>{/if}
              </section>
              <section class="detail-section collaboration-list"><h3>COLLABORATION <span>{taskCollaborations.length}</span></h3>{#each taskCollaborations as collaboration}<button class="collaboration-row" onclick={()=>{const id=collaboration.fromTaskId===selectedTask?.id?collaboration.toTaskId:collaboration.fromTaskId; const task=snapshot?.tasks.find(item=>item.id===id); if(task) openTask(task)}}><span class={`dot ${collaboration.status === 'running' ? 'running' : collaboration.status === 'error' ? 'error' : 'completed'}`}></span><span><b>{collaboration.kind === 'delegation' ? 'Delegation' : 'Agent message'} · {snapshot?.agents.find(agent=>agent.id===(collaboration.fromTaskId===selectedTask?.id?collaboration.toAgentId:collaboration.fromAgentId))?.name ?? 'Agent'}</b><small>{collaborationStatus(collaboration)} · {relative(collaboration.updatedAt)}</small>{#if collaboration.result}<em>{collaboration.result}</em>{/if}{#if collaboration.error}<em class="collaboration-error">{collaboration.error}</em>{:else if !collaboration.result}<em>{collaboration.text}</em>{/if}</span></button>{:else}<p class="detail-empty">No routed agent messages or delegations yet.</p>{/each}</section>
              <section class="detail-section">
                <h3>DELEGATED TASKS <span>{delegated.length}</span></h3>
                {#if delegated.length}{#each delegated as task}<button
                      class="delegated"
                      onclick={() => openTask(task)}
                      ><span class={`dot ${task.status}`}></span>
                      <div>
                        <b>{task.title}</b><small
                          >{snapshot.agents.find((a) => a.id === task.agentId)
                            ?.name ?? "Agent"}</small
                        >
                      </div></button
                    >{/each}{:else}<p class="detail-empty">
                    Delegated tasks will be linked here.
                  </p>{/if}<button
                  class="delegate-button"
                  onclick={() => openTaskComposer(selectedTask.id)}
                  ><Plus size={15} /> Delegate a task</button
                >
              </section>
            </div>
          </aside>
      </section>{/if}
  </section>
{/snippet}

<main class:preview={!bridge.available} class:native-mac={nativeMac} class:sidebar-collapsed={sidebarCollapsed} class:embedded class="app-shell">
  {#if !embedded}<div class="window-toolbar" data-tauri-drag-region>
    <button class="icon" aria-label={sidebarCollapsed ? 'Expand main sidebar' : 'Collapse main sidebar'} title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'} aria-pressed={sidebarCollapsed} onclick={()=>{sidebarCollapsed=!sidebarCollapsed;railAgentId=null;sidebarViewMenu=false;monitterMenu=false}}><PanelLeft size={17}/></button>
  </div>
  <aside class="sidebar" aria-label="Agents and tasks">
    <div class="sidebar-window-space" data-tauri-drag-region></div>
    <div class="brand monitter-menu" data-tauri-drag-region>
      {#if !sidebarCollapsed}<strong>monitter</strong>{/if}<button bind:this={monitterMenuAnchor} class="icon" aria-label="Monitter menu" title="Monitter menu" aria-expanded={monitterMenu} onclick={(event)=>{event.stopPropagation();monitterMenu=!monitterMenu}}><ChevronDown size={16}/></button>
      {#if monitterMenu && monitterMenuAnchor}<div use:floating={{anchor:monitterMenuAnchor}} class="monitter-dropdown floating-panel" role="menu"><button role="menuitem" onclick={()=>{monitterMenu=false;returnToMonitterMenu=true;modal='appearance'}}><Settings2 size={15}/>Preferences</button><button role="menuitem" onclick={()=>{monitterMenu=false;returnToMonitterMenu=true;modal='hosts'}}><Network size={15}/>Hosts</button><button role="menuitem" onclick={()=>{monitterMenu=false;returnToMonitterMenu=true;directoryQuery='';modal='directory'}}><Bot size={15}/>Agent directory</button><button role="menuitem" onclick={()=>{monitterMenu=false;returnToMonitterMenu=true;modal='archived'}}><Archive size={15}/>Archived chats</button></div>{/if}
    </div>
    {#if !sidebarCollapsed}<div class="sidebar-views">
      <button bind:this={viewAnchor} class="view-selector" aria-label={`Sidebar view: ${currentSidebarView.label}`} aria-haspopup="menu" aria-expanded={sidebarViewMenu} onclick={()=>sidebarViewMenu=!sidebarViewMenu}>
        <currentSidebarView.icon size={14}/><span>{currentSidebarView.label}</span><ChevronDown size={12}/>
      </button>
      {#if sidebarViewMenu && viewAnchor}<div use:floating={{anchor:viewAnchor}} class="view-menu floating-panel" role="menu" aria-label="Sidebar view">
        {#each sidebarViews as view}<button role="menuitemradio" aria-checked={sidebarView === view.id} disabled={busy || !bridge.available || !snapshot} onclick={()=>{sidebarViewMenu=false;void setSidebarView(view.id)}}><view.icon size={14}/><span>{view.label}</span>{#if sidebarView === view.id}<Check size={12}/>{/if}</button>{/each}
      </div>{/if}
    </div>
    <nav class="side-scroll">
      {#if sidebarView === 'standard'}
      <div class="section-label">
        <span>AGENTS</span><button
          aria-label="New agent"
          onclick={() => {
            agentDraft = blankAgent();
            modal = "agent";
          }}><Plus size={15} /></button
        >
      </div>
      {#if snapshot?.agents.length}{#each snapshot.agents as agent}{@const agentTasks =
            snapshot.tasks.filter(
              (task) => task.agentId === agent.id && !task.parentTaskId && !task.archived,
            )}
          <section class="agent-group">
            <div class="agent-row">
              <span class="avatar" style={`--agent-color:${agent.color}`}
                >{#if avatarSrc(agent)}<img src={avatarSrc(agent)!} alt="" />{:else}{agent.name.slice(0, 1).toUpperCase()}{/if}</span
              ><button
                class="agent-name"
                onclick={() => {
                  agentDraft = { ...agent };
                  modal = "agent";
                }}
                ><b>{agent.name}</b><small
                  >{agent.provider}{agent.model
                    ? ` · ${agent.model}`
                    : ""}</small
                ></button
              >{#if !openTasks.some(task=>task.agentId===agent.id)}<button class="quiet" aria-label={`New chat with ${agent.name}`} title="New chat" onclick={() => routeDraft(agent.id)}><Plus size={15}/></button>{/if}<button
                class="quiet"
                aria-label={`Edit ${agent.name}`}
                onclick={() => {
                  agentDraft = { ...agent };
                  modal = "agent";
                }}><MoreHorizontal size={15} /></button
              >
            </div>
            <div class="task-tree">
              {#each agentTasks as task}{@render sidebarChat(task)}{/each}{#if !agentTasks.length}<p class="empty-tree">No chats yet</p>{/if}
            </div>
          </section>{/each}{:else}<div class="side-empty">
          <Bot size={18} />
          <p>Agents hold their own tasks and settings.</p>
          <button
            class="text-button"
            onclick={() => {
              agentDraft = blankAgent();
              modal = "agent";
            }}>Create first agent</button
          >
        </div>{/if}
      {:else if sidebarView === 'activity'}
        <div class="section-label"><span>ACTIVITY</span><button aria-label="New chat" title="New chat" onclick={()=>openTaskComposer()}><Plus size={15}/></button></div>
        <p class="view-hint">Running first, then most recent.</p>
        <div class="activity-list">{#each activityTasks as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">No chats yet</p>{/each}</div>
      {:else}
        <div class="section-label"><span>PROJECTS</span><button aria-label="New project" title="New project" onclick={()=>editProject()}><Plus size={15}/></button></div>
        {#each projects as project (project.id)}
          {@const projectTasks = activityTasks.filter(task=>task.projectId===project.id)}
          <section class="project-group" aria-label={`Project ${project.name}`}>
            <div class="project-row" class:current={focusedProjectId === project.id || selectedTask?.projectId === project.id}>
              <button class="folder-toggle" aria-label={`${collapsedProjects[project.id] ? 'Expand' : 'Collapse'} project ${project.name}`} aria-expanded={!collapsedProjects[project.id]} onclick={()=>collapsedProjects[project.id]=!collapsedProjects[project.id]}>
                {#if collapsedProjects[project.id]}<ChevronRight size={13}/>{:else}<ChevronDown size={13}/>{/if}
              </button>
              <button class="project-name" aria-label={`Open project ${project.name}`} onclick={()=>openProject(project)}><Folder size={14}/><span>{project.name}</span><small>{projectTasks.length}</small></button>
              <button class="quiet" aria-label={`New chat in ${project.name}`} title="New chat" onclick={()=>openTaskComposer(null,null,project.id)}><Plus size={14}/></button>
              <button class="quiet" aria-label={`Edit project ${project.name}`} title="Edit project" onclick={()=>editProject(project)}><MoreHorizontal size={14}/></button>
            </div>
            {#if !collapsedProjects[project.id]}<div class="task-tree">
              {#each projectTasks as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">No chats yet</p>{/each}
            </div>{/if}
          </section>
        {:else}<p class="view-hint">Group chats from any agent in a project.</p>{/each}
        <section class="project-group" aria-label="No project">
          <button class="unassigned-folder" aria-expanded={!collapsedProjects.unassigned} onclick={()=>collapsedProjects.unassigned=!collapsedProjects.unassigned}>
            {#if collapsedProjects.unassigned}<ChevronRight size={13}/>{:else}<ChevronDown size={13}/>{/if}<Folder size={14}/><span>No project</span><small>{activeTasks.filter(task=>!task.projectId).length}</small>
          </button>
          {#if !collapsedProjects.unassigned}<div class="task-tree">
            {#each activityTasks.filter(task=>!task.projectId) as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">All chats are organised.</p>{/each}
          </div>{/if}
        </section>
      {/if}
      <div class="section-label channels-label">
        <span>CHANNELS</span><button
          aria-label="New channel"
          onclick={() => {
            channelDraft = blankChannel();
            modal = "channel";
          }}><Plus size={15} /></button
        >
      </div>
      {#each snapshot?.channels ?? [] as channel}<button
          class:current={channel.id === selectedChannelId}
          class="channel-row"
          onclick={() => routeChannel(channel)}
          ><Radio size={14} /><span>{channel.name}</span><small
            >{channel.agentIds.length}</small
          ></button
        >{/each}
    </nav>
    {:else}<nav class="agent-rail" aria-label="Agents">
      {#each snapshot?.agents ?? [] as agent}<button class="rail-avatar" class:current={railAgentId === agent.id || selectedAgent?.id === agent.id} aria-label={`Chats with ${agent.name}`} title={agent.name} aria-expanded={railAgentId === agent.id} onclick={(event)=>{railAnchor=event.currentTarget;railAgentId=railAgentId===agent.id?null:agent.id}}>
        <span class="avatar" style={`--agent-color:${agent.color}`}>{#if avatarSrc(agent)}<img src={avatarSrc(agent)!} alt="" />{:else}{agent.name.slice(0,1).toUpperCase()}{/if}</span>
        {#if activeTasks.some(task=>task.agentId===agent.id && task.status==='running')}<span class="rail-running" aria-label="Running"></span>{/if}
      </button>{/each}
      <button class="icon" aria-label="New agent" title="New agent" onclick={()=>{agentDraft=blankAgent();modal='agent'}}><Plus size={17}/></button>
      <button class="icon" aria-label="Switch channel, chat or agent" title={`Switch channel, chat or agent (${modifierLabel}K)`} onclick={()=>palette='switch'}><Search size={16}/></button>
    </nav>{/if}
    {#if railAgent && railAnchor}<div class="rail-chats floating-panel" role="dialog" aria-label={`${railAgent.name} chats`} use:floating={{anchor:railAnchor,side:'right'}}>
      <header><strong>{railAgent.name}</strong><button class="icon" aria-label="Close agent chats" onclick={()=>railAgentId=null}><X size={14}/></button></header>
      <div class="rail-chat-list">{#each activityTasks.filter(task=>task.agentId===railAgent.id) as task (task.id)}{@render sidebarChat(task)}{:else}<p class="detail-empty">No chats yet.</p>{/each}</div>
      <button class="rail-new-chat" aria-label={`New chat with ${railAgent.name}`} onclick={()=>routeDraft(railAgent!.id)}><Plus size={14}/>New chat</button>
    </div>{/if}
  </aside>{/if}
  {#if embedded}{@render workspaceView()}{:else}<div class="pane-grid">
    <PaneGrid {layout} {activePaneId} dimInactivePanes={snapshot?.settings.dimInactivePanes ?? true} inactivePaneOpacity={snapshot?.settings.inactivePaneOpacity ?? .6} onactivate={id=>activePaneId=id} onresize={resizeSplit} ondropTab={dropTab}>
      {#snippet children(id)}{#if id==='main'}{@render workspaceView()}{:else}
        <AppSurface embedded={true} paneId={id} active={activePaneId===id && !modal && !palette} parentSnapshot={snapshot}
          onSnapshot={value=>applySnapshot(value,++snapshotIssued)} onTabDrop={dropTab} onLayout={setLayout}
          onSelection={taskId=>paneSelections[id]=taskId} bind:this={paneRefs[id]}/>
      {/if}{/snippet}
    </PaneGrid>
  </div>{/if}
</main>

<Modal title="Agent directory" open={modal === 'directory'} onclose={()=>modal=null}><div class="form agent-directory"><label>Find agents<input aria-label="Find agents" bind:value={directoryQuery} placeholder="Search expertise, responsibilities, or skills" /></label>{#each (snapshot?.agents ?? []).filter(agent => { const profile=agent as AgentProfile; const haystack=[agent.name,agent.description,...(profile.expertise??[]),...(profile.responsibilities??[]),...(profile.skills??[])].join(' ').toLowerCase(); return haystack.includes(directoryQuery.trim().toLowerCase()); }) as agent}{@const profile=agent as AgentProfile}<article class:disabled={profile.collaborationEnabled===false}><span class="avatar" style={`--agent-color:${agent.color}`}>{#if avatarSrc(agent)}<img src={avatarSrc(agent)!} alt="" />{:else}{agent.name.slice(0,1).toUpperCase()}{/if}</span><div><b>{agent.name}</b><small>{agent.provider} · {snapshot?.hosts.find(host=>host.id===agent.hostId)?.name ?? 'Unknown host'} · {profile.collaborationEnabled===false?'Collaboration off':'Collaboration on'}</small>{#if (profile.expertise??[]).length}<p>{(profile.expertise??[]).join(' · ')}</p>{/if}</div><button class="secondary" onclick={()=>{modal=null;openTaskComposer(null,agent.id)}}>New chat</button><button class="icon" aria-label={`Edit ${agent.name}`} onclick={()=>{agentDraft={...agent};modal='agent'}}><MoreHorizontal size={15}/></button></article>{:else}<p class="hint">No saved agents match this search.</p>{/each}</div></Modal>
<CommandPalette open={palette !== null} title={palette === "switch" ? "Switch to" : "Controls"} placeholder={palette === "switch" ? "Find a channel, chat or agent…" : "Find a control or setting…"} items={palette === "switch" ? switchItems : controlItems} onselect={selectPalette} onclose={()=>palette=null}/>
{#if modal === 'archived' && snapshot}<ArchivedChats {snapshot} onclose={()=>modal=null}
  onRestore={async taskId=>applySnapshot(await bridge.setTaskArchived(taskId,false),++snapshotIssued)}
  onDelete={deleteArchivedTask} previewDeletion={taskId=>bridge.previewTaskDeletion(taskId)}/>{/if}

<Modal title={projectDraft?.id ? 'Edit project' : 'New project'} open={modal === 'project'} onclose={()=>modal=null}>
  {#if projectDraft}<form class="form" onsubmit={event=>{event.preventDefault();void saveProject();}}>
    <label>Name<input data-autofocus required bind:value={projectDraft.name} placeholder="Project name" /></label>
    <label>Description<input bind:value={projectDraft.description} placeholder="What you're working on together" /></label>
    <div class="project-workspaces"><h3>Working folders <span class="optional">Optional</span></h3>
      <p class="modal-copy">New chats use the project folder on their agent's host. Leave a folder blank to use the agent's default.</p>
      {#each projectDraft.workspaces as workspace (workspace.hostId)}{@const host = snapshot?.hosts.find(host=>host.id===workspace.hostId)}
        <label>{host?.name ?? 'Host'}<input aria-label={`Folder on ${host?.name ?? 'Host'}`} bind:value={workspace.cwd} placeholder={host?.kind === 'ssh' ? '~/projects/my-project' : '/path/to/project'} /></label>
      {/each}
    </div>
    <footer>
      {#if projectDraft.id}<button type="button" class="danger-text" disabled={busy} onclick={()=>modal='deleteProject'}><Trash2 size={15}/>Delete project</button>{/if}
      <span></span><button type="button" class="secondary" onclick={()=>modal=null}>Cancel</button>
      <button class="primary" disabled={busy || !projectDraft.name.trim()}><Save size={15}/>Save project</button>
    </footer>
  </form>{/if}
</Modal>
<Modal title="Delete project" open={modal === 'deleteProject'} onclose={()=>modal='project'}>
  {#if projectDraft}<div class="form"><p>Delete “{projectDraft.name}”?</p><p class="modal-copy">Its chats will move to No project. Their messages, working folders and native sessions are kept.</p>
    <footer><button class="secondary" onclick={()=>modal='project'}>Cancel</button><span></span><button class="danger" disabled={busy} onclick={deleteProject}>Delete project</button></footer>
  </div>{/if}
</Modal>

<Modal
  title={agentDraft?.id ? "Edit agent" : "Create agent"}
  open={modal === "agent"}
  onclose={() => (modal = null)}
  >{#if agentDraft}<form
      class="form"
      onsubmit={(event) => {
        event.preventDefault();
        saveAgent();
      }}
    >
      <label
        >Name<input
          required
          bind:value={agentDraft.name}
          placeholder="e.g. Product work"
        /></label
      ><label
        >Description<input
          bind:value={agentDraft.description}
          placeholder="What this agent is responsible for"
        /></label
      ><label>Avatar <span class="optional">Optional</span><input aria-label="Avatar image" type="file" accept="image/png,image/jpeg,image/webp" onchange={(event)=>chooseAvatar(event.currentTarget.files?.[0])}/>{#if avatarSrc(agentDraft)}<span class="avatar-preview"><img src={avatarSrc(agentDraft)!} alt="Current avatar"/><button type="button" onclick={()=>agentDraft && (agentDraft.avatar=null)}>Remove avatar</button></span>{/if}<small>PNG, JPEG, or WebP up to 2 MiB.</small></label
      ><details class="agent-profile"><summary>Collaboration profile</summary><p>Saved profile details help other agents discover when to involve this agent.</p><label>Expertise<textarea value={profileList((agentDraft as AgentProfile).expertise)} oninput={event => agentDraft && ((agentDraft as AgentProfile).expertise = parseProfileList(event.currentTarget.value))} placeholder="One area per line"></textarea></label><label>Responsibilities<textarea value={profileList((agentDraft as AgentProfile).responsibilities)} oninput={event => agentDraft && ((agentDraft as AgentProfile).responsibilities = parseProfileList(event.currentTarget.value))} placeholder="One responsibility per line"></textarea></label><label>Skills<textarea value={profileList((agentDraft as AgentProfile).skills)} oninput={event => agentDraft && ((agentDraft as AgentProfile).skills = parseProfileList(event.currentTarget.value))} placeholder="One skill per line"></textarea></label><label class="check-row"><input type="checkbox" role="switch" checked={(agentDraft as AgentProfile).collaborationEnabled !== false} onchange={event => agentDraft && ((agentDraft as AgentProfile).collaborationEnabled = event.currentTarget.checked)} /> Available for collaboration</label></details
      ><label
        >Instructions<textarea
          bind:value={agentDraft.instructions}
          placeholder="Guidance for new tasks"
        ></textarea></label
      >
      <div class="form-grid">
        <label
          >Harness<select aria-label="Harness" bind:value={agentDraft.provider}
            onchange={event => { if (agentDraft) { agentDraft.sandbox = event.currentTarget.value === "codex" ? "read-only" : "harness-configured"; agentDraft.model = ""; } }}
            ><option value="codex">Codex</option><option value="claude">Claude Code</option>
            <option value="opencode">OpenCode</option><option value="hermes">Hermes</option></select
          ></label
        ><label
          >Model<input
            bind:value={agentDraft.model}
            placeholder="Harness default"
          /></label
        ><label
          >Host<select bind:value={agentDraft.hostId}
            >{#each snapshot?.hosts ?? [] as host}<option value={host.id}
                >{host.name} · {host.kind}</option
              >{/each}</select
          ></label
        ><label
          >Folder<input
            bind:value={agentDraft.cwd}
            placeholder="/path/to/project"
          /></label
        ><label
          >Permissions<select aria-label="Permissions" bind:value={agentDraft.sandbox}
            >{#if agentDraft.provider === "codex"}<option value="read-only">Read only</option><option
              value="workspace-write">Workspace write</option>
            {:else}<option value="harness-configured">Use harness permissions</option>{/if}</select
          >{#if agentDraft.provider !== "codex"}<small>Uses this harness's permissions on the selected host. Requests for extra approval are declined.</small>{/if}</label
        ><label
          >Colour<input type="color" bind:value={agentDraft.color} /></label
        >
      </div>
      <footer>
        <button
          type="button"
          class="danger-text"
          disabled={!agentDraft.id || busy}
          onclick={() =>
            agentDraft?.id &&
            run(
              () => bridge.deleteAgent(agentDraft!.id),
              "Agent removed.",
            ).then((ok) => ok && (modal = null))}
          ><Trash2 size={15} /> Delete</button
        ><span></span><button
          type="button"
          class="secondary"
          onclick={() => (modal = null)}>Cancel</button
        ><button class="primary" disabled={busy}
          ><Save size={15} /> Save agent</button
        >
      </footer>
    </form>{/if}</Modal
>
<Modal title="Hosts" open={modal === "hosts"} onclose={() => (modal = null)}
  ><div class="host-list">
    {#each snapshot?.hosts ?? [] as host}<button
        class="host-card"
        onclick={() => {
          hostDraft = { ...host };
          probe = null;
          modal = "host";
        }}
        ><span class="host-card-icon"
          >{#if host.kind === "ssh"}<Cloud size={17} />{:else}<HardDrive
              size={17}
            />{/if}</span
        ><span
          ><b>{host.name}</b><small
            >{host.kind === "ssh"
              ? `${host.user ? `${host.user}@` : ""}${host.address}${host.port ? `:${host.port}` : ""}`
              : host.defaultCwd || "This Mac"}</small
          ></span
        ><ChevronDown size={15} /></button
      >{/each}
    {#if !snapshot?.hosts.length}<p class="modal-copy">
        No hosts configured yet.
      </p>{/if}
    <button
      class="secondary add-host"
      onclick={() => {
        hostDraft = blankHost();
        probe = null;
        modal = "host";
      }}><Plus size={15} /> Add host</button
    >
  </div></Modal
>
<Modal
  title={hostDraft?.id ? "Host settings" : "Add host"}
  open={modal === "host"}
  onclose={() => (modal = null)}
  >{#if hostDraft}<form
      class="form"
      onsubmit={(event) => {
        event.preventDefault();
        saveHost();
      }}
    >
      <div class="segmented">
        <button
          type="button"
          class:chosen={hostDraft.kind === "local"}
          onclick={() => hostDraft && (hostDraft.kind = "local")}
          ><HardDrive size={15} /> This Mac</button
        ><button
          type="button"
          class:chosen={hostDraft.kind === "ssh"}
          onclick={() => hostDraft && (hostDraft.kind = "ssh")}
          ><Cloud size={15} /> SSH host</button
        >
      </div>
      <label
        >Name<input
          required
          bind:value={hostDraft.name}
          placeholder={hostDraft.kind === "local" ? "My Mac" : "e.g. Mira"}
        /></label
      >{#if hostDraft.kind === "ssh"}<div class="form-grid">
          <label
            >Address<input
              required
              bind:value={hostDraft.address}
              placeholder="server.example.com"
            /></label
          ><label
            >User<input
              bind:value={hostDraft.user}
              placeholder="Use SSH config"
            /></label
          ><label
            >Port<input
              type="number"
              min="0"
              bind:value={hostDraft.port}
              placeholder="SSH config"
            /><small>0 uses your SSH config or its normal default.</small
            ></label
          ><label
            >Identity file<input
              bind:value={hostDraft.identityFile}
              placeholder="Use SSH config"
            /></label
          >
        </div>{/if}<label
        >Default folder<input
          bind:value={hostDraft.defaultCwd}
          placeholder="/path/to/workspace"
        /></label
      ><label
        >Codex CLI path<input
          bind:value={hostDraft.codexPath}
          placeholder="Leave blank to find Codex automatically"
        /></label
      >
      <details>
        <summary>Other harness paths</summary><label>Claude Code CLI<input bind:value={hostDraft.claudePath} /></label><label
          >OpenCode CLI<input bind:value={hostDraft.opencodePath} /></label
        ><label>Hermes CLI<input bind:value={hostDraft.hermesPath} /></label>
      </details>
      {#if probe}<div class:bad={!probe.ok} class="probe">
          <b>{probe.ok ? "Connection ready" : "Could not connect"}</b>
          <p>{probe.message}</p>
          {#each Object.entries(probe.versions) as [name, version]}<code
              >{name} {version}</code
            >{/each}
        </div>{/if}
      <footer>
        <button
          type="button"
          class="danger-text"
          disabled={!hostDraft.id || busy}
          onclick={() =>
            hostDraft?.id &&
            run(() => bridge.deleteHost(hostDraft!.id), "Host removed.").then(
              (ok) => ok && (modal = null),
            )}><Trash2 size={15} /> Delete</button
        ><span></span><button
          type="button"
          class="secondary"
          disabled={busy}
          onclick={checkHost}><Wifi size={15} /> Probe</button
        ><button class="primary" disabled={busy}
          ><Save size={15} /> Save host</button
        >
      </footer>
    </form>{/if}</Modal
>
<Modal
  title="Task settings"
  open={modal === "taskSettings"}
  onclose={() => (modal = null)}
  >{#if selectedTask}<form
      class="form"
      onsubmit={(event) => {
        event.preventDefault();
        renameTask();
      }}
    >
      <label
        >Task title<input
          data-autofocus
          required
          bind:value={renameTitle}
      /></label>
      <label>Project<select aria-label="Project" value={selectedTask.projectId ?? ''} disabled={busy} onchange={event=>moveTaskProject(selectedTask.id,event.currentTarget.value)}>
        <option value="">No project</option>{#each projects as project}<option value={project.id}>{project.name}</option>{/each}
      </select><small>Moving a chat keeps its current working folder and native session.</small></label>
      <footer>
        <button
          type="button"
          class="secondary"
          disabled={busy || selectedTask.status === "running"}
          onclick={async()=>{await archiveTask(selectedTask);modal=null}}><Archive size={15} /> Archive chat</button
        ><span></span><button
          type="button"
          class="secondary"
          onclick={() => (modal = null)}>Cancel</button
        ><button class="primary" disabled={busy || !renameTitle.trim()}
          ><Save size={15} /> Save title</button
        >
      </footer>
    </form>{/if}</Modal
>
<Modal
  title={channelDraft?.id ? "Edit channel" : "Create channel"}
  open={modal === "channel"}
  onclose={() => (modal = null)}
  >{#if channelDraft}<form
      class="form"
      onsubmit={(event) => {
        event.preventDefault();
        saveChannel();
      }}
    >
      <label
        >Name<input
          required
          bind:value={channelDraft.name}
          placeholder="e.g. Release work"
        /></label
      ><label
        >Description<input
          bind:value={channelDraft.description}
          placeholder="What this channel is for"
        /></label
      >
      <fieldset>
        <legend>Agents in this channel</legend
        >{#each snapshot?.agents ?? [] as agent}<label class="check-row"
            ><input
              type="checkbox" role="switch"
              checked={channelDraft.agentIds.includes(agent.id)}
              onchange={() =>
                channelDraft &&
                (channelDraft.agentIds = channelDraft.agentIds.includes(
                  agent.id,
                )
                  ? channelDraft.agentIds.filter((id) => id !== agent.id)
                  : [...channelDraft.agentIds, agent.id])}
            /><span class="avatar small" style={`--agent-color:${agent.color}`}
              >{agent.name.slice(0, 1)}</span
            >{agent.name}<small>{agent.provider}</small></label
          >{/each}
      </fieldset>
      <footer>
        <span></span><button
          type="button"
          class="secondary"
          onclick={() => (modal = null)}>Cancel</button
        ><button class="primary" disabled={busy}
          ><Save size={15} /> Save channel</button
        >
      </footer>
    </form>{/if}</Modal
>
<Modal
  title="Appearance"
  open={modal === "appearance"}
  onclose={() => (modal = null)}
  >{#if snapshot}<div class="form">
      <fieldset>
        <legend>Theme</legend>
        <div class="segmented">
          {#each ["system", "light", "dark"] as theme}<button
              class:chosen={snapshot.settings.theme === theme}
              disabled={busy}
              onclick={() =>
                run(() =>
                  bridge.saveSettings({
                    ...snapshot!.settings,
                    theme: theme as "system" | "light" | "dark",
                  }),
                )}>{theme}</button
            >{/each}
        </div>
      </fieldset>
      <fieldset>
        <legend>Accent colour</legend>
        <div class="swatches">
          {#each accents as accent}<button
              aria-label={accent}
              class:chosen={snapshot.settings.accent === accent}
              disabled={busy}
              style={`--swatch:${accent}`}
              onclick={() =>
                run(() =>
                  bridge.saveSettings({ ...snapshot!.settings, accent }),
                )}
            ></button>{/each}<input
            aria-label="Custom accent colour"
            type="color"
            disabled={busy}
            value={snapshot.settings.accent}
            onchange={(event) =>
              run(() =>
                bridge.saveSettings({
                  ...snapshot!.settings,
                  accent: event.currentTarget.value,
                }),
              )}
          />
        </div>
      </fieldset>
      <div class="scale-control"><label for="interface-scale">Interface scale</label>
        <div class="scale-value"><span>{snapshot.settings.interfaceScale ?? 125}%</span>
          <button type="button" disabled={busy} onclick={() => run(() => bridge.saveSettings({
            ...snapshot!.settings, interfaceScale: 125,
          }))}>Reset to default · 125%</button>
        </div>
        <input id="interface-scale" type="range" aria-label="Interface scale" min="80" max="200" step="5" disabled={busy}
          value={snapshot.settings.interfaceScale ?? 125}
          onchange={event => run(() => bridge.saveSettings({
            ...snapshot!.settings, interfaceScale: Number(event.currentTarget.value),
          }))} />
        <div class="scale-limits"><span>80%</span><span>200%</span></div>
        <small>Resize text and controls together. Default: 125%.</small>
      </div>
      <fieldset>
        <legend>Pane appearance</legend>
        <label class="check-row"><input type="checkbox" role="switch" aria-label="Dim inactive panes" checked={snapshot.settings.dimInactivePanes ?? true} disabled={busy} onchange={event=>run(()=>bridge.saveSettings({...snapshot!.settings,dimInactivePanes:event.currentTarget.checked}))}/>Dim inactive panes</label>
        <label class="opacity-setting">Inactive pane opacity <span>{Math.round((snapshot.settings.inactivePaneOpacity ?? .6)*100)}%</span><input type="range" aria-label="Inactive pane opacity" min="10" max="90" step="5" value={(snapshot.settings.inactivePaneOpacity ?? .6)*100} disabled={busy || snapshot.settings.dimInactivePanes===false} onchange={event=>run(()=>bridge.saveSettings({...snapshot!.settings,inactivePaneOpacity:Number(event.currentTarget.value)/100}))}/></label>
        <p class="hint">Lower opacity makes inactive panes dimmer.</p>
      </fieldset>
      <fieldset>
        <legend>Conversation activity</legend>
        <label class="check-row"><input type="checkbox" role="switch" disabled={busy}
          checked={snapshot.settings.showToolActivity !== false}
          onchange={event => run(() => bridge.saveSettings({
            ...snapshot!.settings, showToolActivity: event.currentTarget.checked,
          }))} />Show tool activity</label>
        <label class="check-row"><input type="checkbox" role="switch" disabled={busy}
          checked={snapshot.settings.showReasoningSummaries !== false}
          onchange={event => run(() => bridge.saveSettings({
            ...snapshot!.settings, showReasoningSummaries: event.currentTarget.checked,
          }))} />Show reasoning summaries</label>
        <p class="hint">Collapsible blocks show the activity and summaries supplied by the harness.</p>
      </fieldset>
      <fieldset>
        <legend>Messages</legend>
        <label class="check-row"><input type="checkbox" role="switch" disabled={busy}
          checked={snapshot.settings.sendWithEnter ?? false}
          onchange={event => run(() => bridge.saveSettings({
            ...snapshot!.settings, sendWithEnter: event.currentTarget.checked,
          }))} />Enter to send</label>
        <p class="hint">{snapshot.settings.sendWithEnter
          ? "Enter sends. Shift+Enter adds a new line."
          : `${modifierLabel}Enter sends. Enter adds a new line.`}</p>
      </fieldset>
      <p class="hint">Saved on this device and applied throughout Monitter.</p>
    </div>{/if}</Modal
>

<style>
  :global(*) {
    box-sizing: border-box;
  }
  :global(:root) {
    --paper: #fbf8f2;
    --sidebar: #f2ede3;
    --panel: #fffdf8;
    --ink: #28231c;
    --muted: #756b5c;
    --line: rgba(52, 43, 30, 0.13);
    --soft: #f5f1e8;
    --code: #eee9df;
    --accent: #3f9d6a;
    --accent-light-ink: #28764d;
    --accent-dark-ink: #72d69d;
    --accent-ink: var(--accent-light-ink);
    --on-accent: #fff;
    --mono: "IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
    font-family:
      "IBM Plex Sans",
      ui-sans-serif,
      system-ui,
      -apple-system,
      BlinkMacSystemFont,
      "Segoe UI",
      sans-serif;
    color: var(--ink);
    background: #e9e3d8;
    font-synthesis: none;
  }
  :global(:root[data-theme="dark"]) {
    --paper: #191918;
    --sidebar: #121211;
    --panel: #20201e;
    --ink: #eeece7;
    --muted: #aaa59b;
    --line: rgba(255, 255, 255, 0.105);
    --soft: #272725;
    --code: #292926;
    --accent-ink: var(--accent-dark-ink);
    background: #0d0d0c;
  }
  :global(html),
  :global(body) {
    margin: 0;
    width: 100%;
    height: 100%;
    min-width: 0;
    overflow: hidden;
    overscroll-behavior: none;
    background: var(--paper);
  }
  :global(*) { scrollbar-width: thin; scrollbar-color: var(--muted) transparent; }
  :global(svg.lucide) { stroke-width: 1.35; }
  :global(button),
  :global(input),
  :global(textarea),
  :global(select) {
    font: inherit;
  }
  :global(button) {
    border: 0;
    color: inherit;
    background: transparent;
    cursor: pointer;
  }
  :global(button:focus-visible),
  :global(input:focus-visible),
  :global(textarea:focus-visible),
  :global(select:focus-visible) {
    outline: 2px solid var(--accent-ink);
    outline-offset: 2px;
  }
  :global(button:disabled) {
    cursor: not-allowed;
    opacity: 0.5;
  }
  .app-shell {
    position: relative;
    height: 100vh;
    min-height: 0;
    overflow: hidden;
    display: grid;
    grid-template-columns: 252px minmax(0, 1fr);
    background: var(--paper);
  }
  .sidebar {
    display: flex;
    min-height: 0;
    flex-direction: column;
    border-right: 1px solid var(--line);
    background: var(--sidebar);
  }
  .brand {
    height: 38px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 13px;
  }
  .brand strong {
    font-size: 14px;
    letter-spacing: -0.02em;
  }
  .brand .icon {
    margin-left: auto;
  }
  .sidebar-window-space { height: 52px; flex-shrink: 0; }
  .window-toolbar { position: absolute; left: 13px; top: 11px; z-index: 15; }
  .native-mac .window-toolbar { left: calc(88px / var(--interface-scale, 1)); top: calc(25px / var(--interface-scale, 1)); }
  .native-mac .window-toolbar .icon { width: calc(30px / var(--interface-scale, 1)); height: calc(30px / var(--interface-scale, 1)); }
  .native-mac .window-toolbar :global(svg) { width: calc(17px / var(--interface-scale, 1)); height: calc(17px / var(--interface-scale, 1)); }
  .native-mac .sidebar-window-space { height: calc(68px / var(--interface-scale, 1)); }
  .native-mac { --pane-tabbar-height: max(36px, calc(68px / var(--interface-scale, 1))); }
  .native-mac .topbar {
    height: var(--pane-tabbar-height);
    box-sizing: border-box;
    flex-shrink: 0;
    user-select: none;
    -webkit-user-select: none;
  }
  .native-mac .brand strong,
  .native-mac .environment {
    pointer-events: none;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 6px;
  }
  .icon:hover,
  .quiet:hover {
    background: var(--soft);
  }
  .environment i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .side-scroll {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: auto;
    overscroll-behavior: contain;
    padding: 11px 8px;
  }
  .sidebar-views { flex: none; padding: 2px 12px 5px; }
  .view-selector { display: flex; align-items: center; gap: 7px; padding: 6px 5px; color: var(--muted); font-size: 11px; border-radius: 5px; }
  .view-selector:hover { background: var(--soft); color: var(--ink); }
  .view-hint { margin: 5px 7px 10px; color: var(--muted); font-size: 10px; line-height: 1.5; }
  .project-group { margin: 5px 0 12px; }
  .project-row { display: flex; align-items: center; gap: 1px; min-width: 0; border-radius: 5px; }
  .project-row.current { background: var(--paper); }
  .folder-toggle { display: grid; place-items: center; flex: none; width: 20px; height: 30px; color: var(--muted); }
  .project-name { display: flex; align-items: center; flex: 1; min-width: 0; gap: 6px; padding: 7px 0; text-align: left; font-size: 12px; }
  .project-name > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .project-name :global(svg) { flex: none; color: var(--accent-ink); }
  .project-name small, .unassigned-folder small { margin-left: auto; padding-right: 3px; font: 9px var(--mono); color: var(--muted); }
  .project-row .quiet { flex: none; width: 21px; }
  .unassigned-folder { display: flex; align-items: center; gap: 5px; width: 100%; padding: 7px 3px; color: var(--muted); font-size: 11.5px; text-align: left; }
  .project-overview-actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .project-folders { display: grid; gap: 10px; margin: 20px 0; padding: 14px; border: 1px solid var(--line); border-radius: 8px; }
  .project-folders dt { display: flex; align-items: center; gap: 5px; color: var(--muted); font: 10px var(--mono); }
  .project-folders dd { margin: 5px 0 0; overflow-wrap: anywhere; font: 12px var(--mono); }
  .project-workspaces { display: grid; gap: 12px; }
  .project-workspaces h3 { margin: 0; font-size: 12px; }
  .project-workspaces p { margin: 0; }
  .task-workspace-preview { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 10px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted); }
  .task-workspace-preview > span { display: grid; gap: 5px; min-width: 0; }
  .task-workspace-preview b { font-size: 11px; }
  .task-workspace-preview code { overflow-wrap: anywhere; font: 11px var(--mono); }
  .section-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 7px;
    color: var(--muted);
    font: 10px var(--mono);
    letter-spacing: 0.1em;
  }
  .section-label button {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 4px;
  }
  .section-label button:hover {
    background: var(--soft);
  }
  .agent-group {
    margin: 4px 0 9px;
  }
  .agent-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px;
  }
  .avatar {
    display: grid;
    flex: none;
    place-items: center;
    width: 23px;
    height: 23px;
    border-radius: 6px;
    color: white;
    background: var(--agent-color, var(--accent));
    font: 11px var(--mono);
  }
  .avatar.small {
    width: 20px;
    height: 20px;
  }
  .agent-name {
    display: grid;
    flex: 1;
    min-width: 0;
    text-align: left;
  }
  .agent-name b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    font-weight: 600;
  }
  .agent-name small {
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 9.5px var(--mono);
  }
  .quiet {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 5px;
    color: var(--muted);
  }
  .task-tree {
    margin-left: 10px;
    border-left: 1px solid var(--line);
    padding-left: 7px;
  }
  .task-row,
  .channel-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    min-width: 0;
    padding: 6px 7px;
    border-radius: 5px;
    text-align: left;
    font-size: 11.5px;
  }
  .task-row:hover,
  .task-row.current,
  .channel-row:hover {
    background: var(--paper);
  }
  .task-select { display:flex; align-items:center; gap:7px; min-width:0; flex:1; padding:0; text-align:left; }
  .task-select > span:last-child { flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .chat-copy { display: grid; gap: 3px; }
  .chat-copy > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chat-meta { font-size: 9.5px; color: var(--muted); }
  .chat-actions { display:flex; align-items:center; flex:none; gap:1px; opacity:0; pointer-events:none; }
  .task-row:hover .chat-actions, .task-row:focus-within .chat-actions { opacity:1; pointer-events:auto; }
  .chat-actions button { display:grid; place-items:center; width:20px; height:22px; padding:0; color:var(--muted); border-radius:4px; }
  .chat-actions button:hover { background:var(--soft); color:var(--ink); }
  .task-row small { display:none; }
  .task-row:has(.dot.running) small { display:block; }
  .agent-chats { margin-top:24px; display:grid; gap:6px; }
  .channel-row span {
    overflow: hidden;
    flex: 1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task-row small,
  .channel-row small {
    color: var(--muted);
    font: 9px var(--mono);
  }
  .empty-tree {
    margin: 5px 7px;
    color: var(--muted);
    font-size: 11px;
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #aaa092;
  }
  .dot.running {
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 17%, transparent);
  }
  .dot.error {
    background: #c44c4c;
  }
  .dot.completed {
    background: #72957a;
  }
  .channels-label {
    margin-top: 10px;
  }
  .channel-row {
    color: var(--muted);
  }
  .side-empty {
    padding: 19px 10px;
    color: var(--muted);
    text-align: center;
    font-size: 12px;
  }
  .side-empty :global(svg) {
    opacity: 0.6;
  }
  .text-button {
    color: var(--accent-ink);
    font-size: 12px;
  }
  .pane-grid { display: flex; min-width: 0; min-height: 0; overflow: hidden; }
  .workspace {
    flex: 1; width: 100%; height: 100%;
    position: relative;
    display: flex;
    min-width: 0;
    min-height: 0;
    flex-direction: column;
  }
  .topbar {
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    height: var(--pane-tabbar-height,52px);
    padding: 0.5em 0.5em 0;
    gap: 10px;
    flex-shrink: 0;
    border-bottom: 0;
    background: linear-gradient(var(--line), var(--line)) left bottom / 100% 1px no-repeat, var(--sidebar);
  }
  .top-actions {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-shrink: 0;
    padding-bottom: 0.5em;
  }
  .environment {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--muted);
    font: 10px var(--mono);
  }
  .new-task,
  .primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border-radius: 6px;
    padding: 8px 11px;
    color: var(--on-accent);
    background: var(--accent);
    font-size: 12px;
    font-weight: 600;
    box-shadow: inset 0 -1px rgba(0, 0, 0, 0.14);
  }
  .secondary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 8px 11px;
    background: var(--panel);
    font-size: 12px;
  }
  .secondary:hover {
    background: var(--soft);
  }
  .compact {
    padding: 7px 9px;
  }
  .danger {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid color-mix(in srgb, #bd4c43 34%, var(--line));
    border-radius: 6px;
    padding: 7px 9px;
    color: #b54b43;
    font-size: 12px;
  }
  .alert,
  .preview-banner {
    position: absolute;
    z-index: 10;
    top: 64px;
    right: 20px;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: calc(100% - 40px);
    padding: 9px 10px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--panel);
    box-shadow: 0 7px 18px rgba(32, 26, 18, 0.14);
    font-size: 12px;
  }
  .alert {
    z-index: 50;
  }
  .alert button {
    display: grid;
    margin-left: 4px;
    color: var(--muted);
  }
  .alert.error {
    border-color: color-mix(in srgb, #bd4c43 45%, var(--line));
    color: #b84c44;
  }
  .alert.notice {
    color: var(--accent-ink);
  }
  .preview-banner {
    left: 50%;
    right: auto;
    transform: translateX(-50%);
    color: var(--muted);
    white-space: nowrap;
  }
  .loading {
    display: grid;
    flex: 1;
    place-content: center;
    justify-items: center;
    gap: 10px;
    color: var(--muted);
    font-size: 13px;
  }
  .loading :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .overview {
    flex: 1;
    min-height: 0;
    overflow: auto;
    overscroll-behavior: contain;
    padding: 52px clamp(30px, 6vw, 82px);
  }
  .overview-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 30px;
    max-width: 1040px;
    margin: auto;
  }
  .eyebrow {
    margin: 0 0 8px;
    color: var(--muted);
    font: 10px var(--mono);
    letter-spacing: 0.12em;
  }
  .overview h1,
  .conversation h1 {
    margin: 0;
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.045em;
  }
  .overview-head > div > p:last-child,
  .conversation-head > div > p:last-child {
    max-width: 600px;
    margin: 9px 0 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.55;
  }
  .onboarding {
    display: flex;
    gap: 20px;
    max-width: 1040px;
    margin: 40px auto 30px;
    padding: 24px;
    border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--line));
    border-radius: 11px;
    background: color-mix(in srgb, var(--accent) 5%, var(--panel));
  }
  .onboard-number {
    color: var(--accent-ink);
    font: 600 13px var(--mono);
  }
  .onboarding h2 {
    margin: 0;
    font-size: 15px;
  }
  .onboarding p {
    max-width: 600px;
    margin: 6px 0 14px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.5;
  }
  .overview-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 18px;
    max-width: 1040px;
    margin: 30px auto;
  }
  .status-group {
    min-height: 154px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--panel);
  }
  .status-group header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 15px 10px;
    border-bottom: 1px solid var(--line);
  }
  .status-group h2 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  .status-group header span {
    padding: 2px 5px;
    border-radius: 3px;
    color: var(--muted);
    background: var(--soft);
    font: 10px var(--mono);
  }
  .status-group > p {
    margin: 17px 15px;
    color: var(--muted);
    font-size: 12px;
  }
  .overview-task {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 10px 13px;
    border-bottom: 1px solid var(--line);
    text-align: left;
  }
  .overview-task:hover {
    background: var(--soft);
  }
  .overview-task div {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .overview-task b {
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .overview-task small,
  .overview-task > span:last-child {
    color: var(--muted);
    font: 10px var(--mono);
  }
  .overview-task > span:last-child {
    white-space: nowrap;
  }
  .task-layout {
    position: relative;
    display: grid;
    flex: 1;
    min-height: 0;
    grid-template-columns: minmax(0, 1fr) 292px;
  }
  .task-layout.detail-hidden { grid-template-columns: minmax(0, 1fr); }
  .conversation {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex: 1;
    flex-direction: column;
  }
  .tabs {
    display: flex;
    align-items: stretch;
    gap: 3px;
    min-width: 0;
    flex: 1;
    overflow-x: auto;
    overflow-y: hidden;
    overscroll-behavior: contain;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
    max-width: 260px;
    min-height: 28px;
    padding: 0 9px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 6px 6px 0 0;
    color: var(--muted);
    font-size: 11.5px;
  }
  .tab.active {
    color: var(--ink);
    border-color: var(--line);
    background: var(--paper);
  }
  .tab-entry { display: flex; align-items: stretch; flex-shrink: 0; border: 1px solid transparent; border-bottom: 0; border-radius: 6px 6px 0 0; }
  .tab-entry.active { color: var(--ink); border-color: var(--line); background: var(--paper); }
  .tab-entry.active .tab { color: var(--ink); }
  .tab span:last-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tab-entry .tab { max-width: 210px; }
  .close-tab { display: grid; place-items: center; align-self: center; width: 22px; height: 24px; margin-right: 3px; color: var(--muted); border-radius: 4px; }
  .close-tab:hover, .tab:hover { background: var(--soft); }
  .tab.active:hover, .tab-entry.active .tab:hover { background: var(--paper); }
  .conversation-head {
    display: flex;
    flex-shrink: 0;
    max-height: 30%;
    overflow: visible;
    align-items: flex-start;
    justify-content: space-between;
    gap: 20px;
    padding: 25px clamp(25px, 4vw, 50px) 17px;
    border-bottom: 1px solid var(--line);
  }
  .task-heading {
    padding-top: 20px;
  }
  .conversation-head h1 {
    font-size: 22px;
  }
  .conversation-head p :global(svg) {
    vertical-align: -2px;
    margin-right: 4px;
  }
  .task-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: 7px;
  }
  .message {
    max-width: 72ch;
    margin: 0 0 24px;
  }
  .message.user {
    margin-left: auto;
    padding: 12px 14px;
    border-radius: 10px 10px 3px 10px;
    background: var(--soft);
  }
  .message.system {
    padding-left: 12px;
    border-left: 2px solid var(--line);
    color: var(--muted);
  }
  .message-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 7px;
    font-size: 11.5px;
    font-weight: 600;
  }
  .message-meta time {
    color: var(--muted);
    font: 10px var(--mono);
    font-weight: 400;
  }
  .message :global(.markdown) {
    font-size: 13px;
    line-height: 1.65;
  }
  .blank-conversation {
    display: grid;
    min-height: 260px;
    place-content: center;
    justify-items: center;
    max-width: 360px;
    margin: auto;
    color: var(--muted);
    text-align: center;
  }
  .blank-conversation :global(svg) {
    color: var(--accent);
  }
  .blank-conversation h2 {
    margin: 10px 0 5px;
    color: var(--ink);
    font-size: 15px;
  }
  .blank-conversation p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.55;
  }
  .draft-layout { flex: 1; min-height: 0; min-width: 0; overflow: auto; overscroll-behavior: contain; padding: clamp(18px, 5vh, 60px) clamp(16px, 5vw, 64px); }
  .draft-content { width: 100%; max-width: 700px; margin: 0 auto; }
  .draft-intro { margin-bottom: 22px; }
  .draft-intro h1 { margin: 8px 0; font-size: clamp(22px, 2.6vw, 32px); font-weight: 500; }
  .draft-intro > p:last-child { color: var(--muted); line-height: 1.6; }
  .draft-options { margin-bottom: 10px; }
  .draft-composer.composer { max-height: none; margin: 16px 0 12px; }
  .draft-composer.composer textarea { min-height: 110px; }
  .suggestions { display: flex; gap: 7px; flex-wrap: wrap; }
  .suggestions button { border: 1px solid var(--line); border-radius: 7px; color: var(--muted); font-size: 12px; padding: 7px 10px; }
  .suggestions button:hover { border-color: var(--accent); color: var(--ink); }
  .draft-advanced { margin-top: 24px; color: var(--muted); font-size: 12px; }
  .draft-advanced summary { cursor: pointer; margin-bottom: 12px; }
  .slash-menu { max-height: min(240px, 38vh); overflow-y: auto; overscroll-behavior: contain; margin: 0 10px; border: 1px solid var(--line); border-radius: 10px; background: var(--panel); box-shadow: 0 10px 28px rgba(0,0,0,.16); }
  .slash-menu .slash-caption { font: 10px var(--mono); text-transform: uppercase; letter-spacing: .06em; }
  .slash-menu button { width: 100%; display: flex; gap: 10px; text-align: left; padding: 9px 11px; }
  .slash-menu button.active, .slash-menu button:hover { background: color-mix(in srgb, var(--accent) 13%, transparent); }
  .slash-menu b { min-width: 78px; font: 12px var(--mono); }
  .slash-menu span, .slash-menu p { color: var(--muted); font-size: 12px; margin: 0; padding: 9px 11px; }
  .composer {
    flex-shrink: 0;
    max-height: 40%;
    overflow: auto;
    overscroll-behavior: contain;
    margin: 0 clamp(25px, 4vw, 50px) 20px;
    padding: 11px 12px 9px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--panel);
  }
  .composer:focus-within {
    border-color: color-mix(in srgb, var(--accent) 56%, var(--line));
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .composer textarea {
    display: block;
    width: 100%;
    min-height: 52px;
    max-height: 25vh;
    resize: vertical;
    border: 0;
    outline: 0;
    color: var(--ink);
    background: transparent;
    font-size: 13px;
    line-height: 1.5;
  }
  .composer textarea::placeholder {
    color: var(--muted);
  }
  .composer-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--muted);
    font: 10px var(--mono);
  }
  .composer-control { width: 30px; min-width: 30px; height: 30px; padding: 0; display: inline-grid; place-items: center; border-radius: 50%; }
  .spin { animation: composer-spin .8s linear infinite; }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
  @keyframes composer-spin { to { transform: rotate(360deg); } }
  .composer-footer > div {
    display: flex;
    gap: 6px;
  }
  .recipient-picker {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 5px;
    max-height: 72px;
    overflow: auto;
  }
  .recipient-picker > span {
    margin-right: 2px;
  }
  .recipient-picker button {
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 5px;
    color: var(--muted);
    font: 10px var(--mono);
  }
  .recipient-picker button.selected {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--line));
    color: var(--accent-ink);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .run-detail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    border-left: 1px solid var(--line);
    background: var(--sidebar);
  }
  .detail-tabs {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 38px;
    padding: 0 12px;
    border-bottom: 1px solid var(--line);
  }
  .detail-tabs b,
  .detail-section h3 {
    font: 10px var(--mono);
    letter-spacing: 0.08em;
  }
  .detail-tabs button {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--accent-ink);
    font-size: 10.5px;
  }
  .detail-tabs button.active { color: var(--ink); }
  .detail-tabs button:last-child { margin-left: auto; }
  .run-detail.closed, .hidden { display: none; }
  .git-slot { flex: 1; min-height: 0; overflow: hidden; }
  .git-probe-error { font-size: 11px; color: var(--muted); }
  .git-probe-error button { color: var(--accent-ink); text-decoration: underline; }
  .git-probe-error small { display: block; overflow-wrap: anywhere; margin-top: 5px; }
  .detail-scroll {
    overflow: auto;
    flex: 1;
    min-height: 0;
    padding: 17px 14px;
  }
  .run-detail dl {
    display: grid;
    gap: 8px;
    margin: 0;
  }
  .run-detail dl div {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    font: 10.5px var(--mono);
  }
  .run-detail dt {
    color: var(--muted);
  }
  .run-detail dd {
    max-width: 155px;
    margin: 0;
    overflow: hidden;
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session-id {
    color: var(--accent-ink);
  }
  .run-detail dl div.task-folder { align-items: flex-start; }
  .run-detail dd code { display: block; overflow-wrap: anywhere; white-space: normal; }
  .run-detail .task-folder dd { max-width: 155px; white-space: normal; }
  .detail-section {
    margin-top: 21px;
    padding-top: 15px;
    border-top: 1px solid var(--line);
  }
  .detail-section h3 {
    display: flex;
    gap: 5px;
    margin: 0 0 11px;
    color: var(--muted);
  }
  .detail-section h3 span {
    padding: 1px 4px;
    border-radius: 3px;
    background: var(--soft);
  }
  .event {
    display: grid;
    grid-template-columns: 39px 1fr;
    gap: 7px;
    padding: 0 0 10px;
    font-size: 11px;
  }
  .event time {
    color: var(--muted);
    font: 9.5px var(--mono);
  }
  .event b {
    font: 500 10.5px var(--mono);
  }
  .event > div {
    min-width: 0;
  }
  .event p {
    margin: 3px 0 0;
    color: var(--muted);
    font-size: 11px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
  .event-preview {
    overflow-wrap: anywhere;
  }
  .event-overflow {
    max-width: 100%;
    margin-top: 6px;
    color: var(--muted);
  }
  .event-overflow summary {
    width: fit-content;
    cursor: pointer;
    color: var(--accent-ink);
    font: 10px var(--mono);
  }
  .event-overflow pre {
    max-width: 100%;
    max-height: 240px;
    overflow: auto;
    margin: 7px 0 0;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: 5px;
    color: var(--muted);
    background: var(--panel);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 10px/1.45 var(--mono);
  }
  .event.error b {
    color: #b84c44;
  }
  .event.tool b {
    color: var(--accent-ink);
  }
  .detail-empty {
    color: var(--muted);
    font-size: 11.5px;
    line-height: 1.5;
  }
  .delegated {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 8px 0;
    text-align: left;
  }
  .delegated:hover b {
    color: var(--accent-ink);
  }
  .delegated div {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .delegated b {
    overflow: hidden;
    font-size: 11.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .delegated small {
    color: var(--muted);
    font: 10px var(--mono);
  }
  .delegate-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    width: 100%;
    margin-top: 7px;
    padding: 7px;
    border: 1px dashed var(--line);
    border-radius: 6px;
    color: var(--muted);
    font-size: 11.5px;
  }
  .delegate-button:hover {
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .form {
    display: grid;
    gap: 14px;
  }
  .draft-content label,
  .form label,
  .form fieldset {
    display: grid;
    gap: 6px;
    color: var(--muted);
    font-size: 11.5px;
  }
  .form label small {
    line-height: 1.45;
  }
  .optional {
    justify-self: end;
    margin-top: -18px;
    color: var(--muted);
    font: 9.5px var(--mono);
    text-transform: uppercase;
  }
  .draft-content input,
  .draft-content select,
  .form input,
  .form textarea,
  .form select {
    width: 100%;
    padding: 8px 9px;
    border: 1px solid var(--line);
    border-radius: 6px;
    outline: none;
    color: var(--ink);
    background: var(--paper);
    font-size: 12.5px;
  }
  .form input[type="range"] { padding: 0; accent-color: var(--accent); cursor: pointer; }
  .scale-control { display: grid; gap: 8px; }
  .scale-value, .scale-limits { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .scale-value button { color: var(--accent-ink); font-size: 11px; }
  .scale-limits { color: var(--muted); font: 10px var(--mono); }
  .form textarea {
    min-height: 86px;
    resize: vertical;
  }
  .form-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }
  .form footer {
    display: flex;
    align-items: center;
    gap: 7px;
    margin-top: 7px;
  }
  .form footer > span {
    flex: 1;
  }
  .danger-text {
    display: flex;
    align-items: center;
    gap: 5px;
    color: #b84c44;
    font-size: 12px;
  }
  .modal-copy {
    margin: 0;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.55;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 0;
    color: var(--muted);
    font-size: 11.5px;
  }
  .segmented {
    display: flex;
    padding: 3px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--soft);
  }
  .segmented button {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 7px 8px;
    border-radius: 4px;
    color: var(--muted);
    font-size: 11.5px;
    text-transform: capitalize;
  }
  .segmented button.chosen {
    color: var(--ink);
    background: var(--panel);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  }
  .form details {
    color: var(--muted);
    font-size: 11.5px;
  }
  .form details[open] {
    display: grid;
    gap: 9px;
  }
  .form summary {
    cursor: pointer;
  }
  .probe {
    padding: 10px;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, var(--line));
    border-radius: 7px;
    color: var(--accent-ink);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
    font-size: 11.5px;
  }
  .probe.bad {
    border-color: color-mix(in srgb, #bd4c43 40%, var(--line));
    color: #b84c44;
  }
  .probe p {
    margin: 4px 0;
    color: var(--muted);
  }
  .probe code {
    margin-right: 6px;
    font: 10px var(--mono);
  }
  .form fieldset {
    border: 0;
    padding: 0;
    margin: 0;
  }
  .form legend {
    margin-bottom: 7px;
    color: var(--muted);
  }
  .check-row {
    display: flex !important;
    align-items: center;
    grid-template-columns: none !important;
    gap: 7px !important;
    padding: 5px 0;
    color: var(--ink) !important;
  }
  .check-row input {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    flex: none;
    width: 34px !important;
    height: 20px;
    padding: 0;
    margin: 0;
    border: 1px solid var(--line);
    border-radius: 20px;
    background: var(--muted);
    cursor: pointer;
    transition: background 140ms;
  }
  .check-row input::before {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px #0003;
    transition: transform 140ms;
  }
  .check-row input:checked { background: var(--accent); border-color: var(--accent); }
  .check-row input:checked::before { transform: translateX(14px); }
  .check-row input:disabled { opacity: 0.5; cursor: wait; }
  @media (prefers-reduced-motion: reduce) {
    .check-row input, .check-row input::before { transition: none; }
  }
  @media (forced-colors: active) {
    .check-row input { forced-color-adjust: none; background: Canvas; border-color: ButtonText; }
    .check-row input::before { background: ButtonText; }
    .check-row input:checked { background: Highlight; border-color: Highlight; }
    .check-row input:checked::before { background: HighlightText; }
  }
  .check-row small {
    margin-left: auto;
    color: var(--muted);
    font: 10px var(--mono);
  }
  .host-list {
    display: grid;
    gap: 8px;
  }
  .host-card {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 11px;
    border: 1px solid var(--line);
    border-radius: 8px;
    text-align: left;
    background: var(--paper);
  }
  .host-card:hover {
    border-color: color-mix(in srgb, var(--accent) 48%, var(--line));
  }
  .host-card-icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 6px;
    color: var(--accent-ink);
    background: color-mix(in srgb, var(--accent) 9%, var(--panel));
  }
  .host-card > span:nth-child(2) {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .host-card b {
    font-size: 12.5px;
  }
  .host-card small {
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 10px var(--mono);
  }
  .add-host {
    justify-self: start;
    margin-top: 4px;
  }
  .swatches {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .swatches button {
    width: 25px;
    height: 25px;
    border: 2px solid transparent;
    border-radius: 50%;
    background: var(--swatch);
  }
  .swatches button.chosen {
    outline: 2px solid var(--ink);
    outline-offset: 2px;
  }
  .swatches input {
    width: 29px !important;
    height: 29px;
    padding: 1px !important;
    border-radius: 50% !important;
  }
  @media (max-width: 1100px) {
    .task-layout {
      grid-template-columns: minmax(0, 1fr) 252px;
    }
    .overview {
      padding: 40px;
    }
  }
  @media (prefers-color-scheme: dark) {
    :global(:root[data-theme="system"]) {
      --paper: #191918;
      --sidebar: #121211;
      --panel: #20201e;
      --ink: #eeece7;
      --muted: #aaa59b;
      --line: rgba(255, 255, 255, 0.105);
      --soft: #272725;
      --code: #292926;
      --accent-ink: var(--accent-dark-ink);
      background: #0d0d0c;
    }
  }
  @media (max-width: 900px) {
    :global(body) {
      min-width: 0;
    }
    .app-shell {
      grid-template-columns: 210px minmax(0, 1fr);
    }
    .sidebar {
      min-width: 210px;
    }
    .overview-grid {
      grid-template-columns: 1fr;
    }
    .task-layout { grid-template-columns: minmax(0, 1fr); }
    .run-detail { position: absolute; right: 0; top: 0; bottom: 0; width: min(292px, 100%); z-index: 10; box-shadow: -10px 0 25px #0002; }
    .environment { display: none; }
    .conversation-head, .overview-head { flex-wrap: wrap; }
    .conversation-head { padding: 20px; }
    .composer-footer { flex-wrap: wrap; }
  }
  @media (max-width: 640px) {
    .app-shell { grid-template-columns: 150px minmax(0, 1fr); }
    .sidebar { min-width: 0; }
    .brand { padding-right: 5px; gap: 3px; }
    .brand strong { font-size: 13px; }
    .brand .icon { width: 24px; }
    .conversation-head, .overview { padding: 12px; }
    .conversation-head { gap: 8px; }
    .conversation-head h1 { font-size: 18px; }
    .composer { margin: 0 10px 10px; }
    .task-actions { flex-wrap: wrap; }
    .form-grid { grid-template-columns: minmax(0, 1fr); }
  }
  @media (max-height: 500px) {
    .conversation-head { padding-top: 10px; padding-bottom: 10px; gap: 6px; }
    .conversation-head p { margin-bottom: 0; }
    .task-heading { padding-top: 10px; }
    .composer { padding: 7px; margin-bottom: 8px; }
    .composer textarea { min-height: 36px; }
  }

  .avatar img { width: 100%; height: 100%; object-fit: cover; border-radius: inherit; }
  .task-heading { align-items: center; padding-top: 13px; padding-bottom: 13px; }
  .task-heading > h1 { margin: 0; }
  .task-overflow { position: relative; }
  .task-menu { display: grid; min-width: 155px; }
  .task-menu button { display: flex; gap: 7px; align-items: center; padding: 7px; text-align: left; }
  .agent-identity { margin: 0 0 14px; border-bottom: 1px solid var(--line); padding-bottom: 12px; }
  .agent-identity summary { display: flex; gap: 9px; align-items: center; cursor: pointer; list-style: none; }
  .agent-identity summary::-webkit-details-marker { display: none; }
  .identity-avatar { width: 32px; height: 32px; }
  .agent-identity b, .agent-identity small { display: block; }
  .agent-identity small { color: var(--muted); font: 10px var(--mono); margin-top: 2px; }
  .identity-actions { padding: 9px 0 0 41px; }
  .identity-actions button { color: var(--accent-ink); font-size: 11px; }
  .identity-actions p { margin: 6px 0 0; color: var(--muted); font-size: 11px; }
  .avatar-preview { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
  .avatar-preview img { width: 34px; height: 34px; border-radius: 7px; object-fit: cover; }

  .monitter-menu { position: relative; }
  .monitter-dropdown { display: grid; min-width: 150px; }
  .monitter-dropdown button { display: flex; align-items: center; gap: 8px; padding: 8px; text-align: left; border-radius: 5px; }
  .monitter-dropdown button:hover, .monitter-dropdown button:focus-visible { background: var(--soft); }
  .floating-panel { position: fixed; inset: auto; z-index: 50; margin: 0; box-sizing: border-box; overflow: auto; overscroll-behavior: contain; padding: 5px; border: 1px solid var(--line); border-radius: 9px; color: var(--ink); background: var(--panel); box-shadow: 0 12px 30px #0003; }
  .view-menu { display: grid; min-width: 155px; }
  .view-menu button { display: flex; align-items: center; gap: 8px; padding: 8px; text-align: left; font-size: 12px; }
  .view-menu button span { flex: 1; }
  .view-menu button:hover { background: var(--soft); border-radius: 5px; }
  .app-shell.sidebar-collapsed { grid-template-columns: 56px minmax(0,1fr); }
  .sidebar-collapsed .sidebar { min-width: 0; }
  .sidebar-collapsed .brand { justify-content: center; padding: 0; height: 30px; }
  .sidebar-collapsed .brand .icon { margin: 0; }
  .native-mac.sidebar-collapsed { grid-template-columns: max(56px,calc(124px / var(--interface-scale,1))) minmax(0,1fr); }
  .agent-rail { display: flex; align-items: center; gap: 8px; flex-direction: column; flex: 1; min-height: 0; overflow-y: auto; padding: 10px 4px; }
  .rail-avatar { position: relative; flex: none; padding: 4px; border: 1px solid transparent; border-radius: 9px; }
  .rail-avatar.current, .rail-avatar:hover { border-color: var(--line); background: var(--soft); }
  .rail-avatar .avatar { width: 30px; height: 30px; font-size: 12px; }
  .rail-running { position: absolute; width: 6px; height: 6px; border: 2px solid var(--sidebar); border-radius: 50%; background: var(--accent); right: 0; bottom: 0; }
  .rail-chats { width: 320px; }
  .rail-chats header { display: flex; align-items: center; justify-content: space-between; padding: 4px 8px; font-size: 13px; }
  .rail-chat-list { max-height: min(50vh,420px); overflow: auto; }
  .rail-new-chat { display: flex; gap: 7px; align-items: center; width: 100%; padding: 10px; color: var(--accent-ink); font-size: 12px; }

  .agent-profile { display: grid; gap: 9px; margin: 4px 0; padding: 10px; border: 1px solid var(--line); border-radius: 7px; }
  .agent-profile summary { cursor: pointer; font-weight: 600; }
  .agent-profile p { margin: 0; color: var(--muted); font-size: 11px; }
  .collaboration-row { display: flex; width: 100%; gap: 7px; padding: 7px 0; text-align: left; border-bottom: 1px solid var(--line); }
  .collaboration-row > span:last-child { display: grid; min-width: 0; gap: 2px; }
  .collaboration-row small, .collaboration-row em { overflow: hidden; color: var(--muted); text-overflow: ellipsis; white-space: nowrap; font-size: 10px; font-style: normal; }
  .collaboration-row .collaboration-error { color: var(--danger, #c44c79); }
  .agent-directory article { display: flex; gap: 8px; align-items: center; padding: 9px 0; border-bottom: 1px solid var(--line); }
  .agent-directory article > div { display: grid; flex: 1; min-width: 0; gap: 2px; }
  .agent-directory small, .agent-directory p { margin: 0; color: var(--muted); font-size: 10px; }
  .agent-directory article.disabled { opacity: .58; }
  .app-shell.embedded { height: 100%; width: 100%; grid-template-columns: minmax(0,1fr); }
  .embedded .topbar { height: var(--pane-tabbar-height,52px); min-height: 32px; padding: 0.5em 0.5em 0; }
  .layout-control { position: relative; }
  .compact-detail .run-detail { position: absolute; right: 0; top: 0; bottom: 0; width: min(340px,calc(100% - 24px)); z-index: 12; box-shadow: -10px 0 30px #0003; animation: detail-enter .18s ease-out; }
  .detail-backdrop { position: absolute; inset: 0; z-index: 11; background: #0002; }
  @keyframes detail-enter { from { transform: translateX(100%); } to { transform: translateX(0); } }
  @media (prefers-reduced-motion: reduce) { .compact-detail .run-detail { animation: none; } }
  .composer-right { display:flex;align-items:center;gap:8px;min-width:0; }
  .opacity-setting { margin-top:12px;display:grid;grid-template-columns:1fr auto;gap:8px; }
  .opacity-setting input { grid-column:1/-1;width:100%;accent-color:var(--accent); }
  .composer-left { display:flex;align-items:center;gap:8px;min-width:0; }
  .message-avatar { width:20px;height:20px;flex-shrink:0;border-radius:5px;font-size:10px; }
  .attachment-tools { display: flex; align-items: center; gap: 7px; color: var(--muted); }
  .attachment-tools .icon { width: 24px; height: 24px; }
  .attachment-tools small { font-size: 10px; }
  .attachment-input { display: none; }
  .composer.drop-files { outline: 2px solid var(--accent); background: color-mix(in srgb,var(--accent) 8%,var(--panel)); }
</style>
