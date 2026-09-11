<script lang="ts">
  import "../../app.css";
  import AnimatedTitle from "./AnimatedTitle.svelte";
  import { autonaming } from "$lib/autoname-state";
  import { getContext, setContext, onMount, tick, untrack } from "svelte";
  import { sidebarReorder } from "$lib/sidebar-reorder";
  import SidebarResize from "./SidebarResize.svelte";
  import PaneNotice from "./PaneNotice.svelte";
  import SettingsPane from "./SettingsPane.svelte";
  import { saveSettingsPatch } from "$lib/settings-save";
  import MentionComposer from "./MentionComposer.svelte";
  import ChannelMembers from "./ChannelMembers.svelte";
  import QueuedMessages from "./QueuedMessages.svelte";
  import { channelCommands, parseChannelCommand, resolveChannelAgent } from "$lib/channel-commands";
  import { completeVimCommand, parseVimCommand, parseVimWindowKey, vimCommandHelp, type VimCommand, type VimTabTarget } from "$lib/vim-commands";
  import { mentionedAgentIds } from "$lib/mentions";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    Bot,
    Archive,
    Activity,
    ArrowUp,
    Search,
    ChevronDown,
    ChevronRight,
    RotateCw,
    Cloud,
    Command,
    Folder,
    Code,
    Rocket,
    Globe,
    Palette,
    Database,
    Wrench,
    Layers,
    Briefcase,
    HardDrive,
    LoaderCircle,
    LayoutDashboard,
    MessageSquare,
    MoreHorizontal,
    MoveDiagonal,
    Minimize2,
    Network,
    PanelRight,
    Paperclip,
    Pencil,
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
    Settings,
    Task,
    ModelSettings,
    TerminalTarget,
    TaskGitStatus,
    Attachment,
    AttachmentTarget,
    AttachmentFileData,
    ApprovalRequest,
    Sandbox,
  } from "$lib/types";
  import { getBridge } from "$lib/bridge";
  import { activeOperatorShare, formatOperatorMessage, splitOperatorMessage } from '$lib/operator-sharing';
  import Modal from "$lib/components/Modal.svelte";
  import Markdown from "$lib/components/Markdown.svelte";
  import CommandPalette from "$lib/components/CommandPalette.svelte";
  import TaskActivity from "$lib/components/TaskActivity.svelte";
  import { activeComputerTools } from "$lib/activity";
  import { groupConversationActivity } from '$lib/activity-grouping';
  import RunActivity from "$lib/components/RunActivity.svelte";
  import MessagePane from "$lib/components/MessagePane.svelte";
  import GitPane from "$lib/components/GitPane.svelte";
  import RunSummary from "$lib/components/RunSummary.svelte";
  import TimelinePane from "$lib/components/TimelinePane.svelte";
  import PaneGrid from '$lib/components/PaneGrid.svelte';
  import AppSurface from './AppSurface.svelte';
  import type { PaneLayout, PaneTabTransfer } from '$lib/panes';
  import { paneIds } from '$lib/panes';
  import { insertTab, normalizeTabOrder, type TabKey } from '$lib/tab-order';
  import ArchivedChats from "$lib/components/ArchivedChats.svelte";
  import ModelPicker from '$lib/components/ModelPicker.svelte';
  import AccessPicker from '$lib/components/AccessPicker.svelte';
  import TerminalPane from '$lib/components/TerminalPane.svelte';
  import { terminalSessions, registerTerminal, closeTerminalSession, recentTerminalOutput } from '$lib/terminal-runtime';
  import AttachmentList from '$lib/components/AttachmentList.svelte';
  import ApprovalRequestCard from '$lib/components/ApprovalRequestCard.svelte';
  import {readBrowserFile,thumbnail,nativeBlob} from '$lib/attachment-files';
  import { floating } from "$lib/floating";
  import { loadWorkspaceSet, remapTerminalIds, saveWorkspaceSet, taskBelongsToWorkspace, workspaceForTask, type PersistedWorkspace, type PersistedWorkspaceSet, type WorkspaceKey } from '$lib/workspace-persistence';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';

  let { embedded = false, paneId = 'main', active = true, parentSnapshot = null, workspaceKey = 'all', onSnapshot, onTabDrop, onLayout, onSelection, onTerminalSelect, onWorkspaceChange, onSettingsSelect, onTabPointerStart, onClosePane, onAgentSettingsSelect, onExpandPane, onVimSplit, onVimWorkspace, onExistingChat, parentExpandedPaneId=null }:
    { embedded?: boolean; paneId?: string; active?: boolean; parentSnapshot?: Snapshot | null; workspaceKey?: WorkspaceKey;
      onSnapshot?: (value: Snapshot) => void; onTabDrop?: (id: string, edge: DropEdge, data: PaneTabTransfer, before?: TabKey) => void;
      parentExpandedPaneId?:string|null; onExpandPane?:(id:string|null)=>void; onAgentSettingsSelect?:(draft:Agent)=>void; onClosePane?:(id:string)=>void; onLayout?: (mode: 'single' | 'columns' | 'grid') => void; onSelection?: (taskId: string | null) => void; onTerminalSelect?: (id:string)=>void; onWorkspaceChange?:()=>void; onSettingsSelect?:(category?:string)=>void; onTabPointerStart?:(event:PointerEvent,tab:PaneTabTransfer)=>void; onVimSplit?:(id:string,axis:'horizontal'|'vertical')=>void; onVimWorkspace?:(id:string,command:VimCommand)=>Promise<void>; onExistingChat?:(kind:'task'|'channel',id:string,requester:string)=>boolean } = $props();
  type DropEdge = 'center' | 'left' | 'right' | 'top' | 'bottom';
  type AgentEditorState={draft:Agent|null;edits:Record<string,Agent>};
  type TabPayload = { settingsEditor?:AgentEditorState;settingsCategory?:string; tab: PaneTabTransfer; draft?: TaskDraft; text?: string; attachments?:Attachment[]; attachmentContext?:string;recipients?:string[] };
  type PaneState = { settingsEditor?:AgentEditorState;overviewOpen:boolean;settingsOpen:boolean;settingsCategory:string;openTerminalIds:string[];selectedTerminalId:string|null;openEmptyIds:string[];selectedEmptyId:string|null;openTaskIds:string[];openDraftIds:string[];openChannelIds:string[];tabOrder:TabKey[];taskDrafts:Record<string,TaskDraft>;drafts:Record<string,string>;selectedTaskId:string|null;currentDraftId:string|null;selectedChannelId:string|null;pane:typeof pane;focusedAgentId:string|null;focusedProjectId:string|null;showDetail:boolean;detailTab:'run'|'git'|'timeline';queuedAttachments:Record<string,Attachment[]>;attachmentContexts:Record<string,string>;channelRecipients:Record<string,string[]> };
  let layout = $state<PaneLayout>({id:'main'}), activePaneId = $state('main');
  let tabOrder = $state<TabKey[]>([]);
  let expandedPaneId=$state<string|null>(null), focusStep=$state<0|1|2>(0), focusTarget=$state('');
  const workspaceExpansion=$derived(embedded?parentExpandedPaneId:expandedPaneId);
  const contentKey=$derived.by(()=>`${pane}:${pane==='task'?currentDraftId??selectedTaskId:pane==='channel'?selectedChannelId:pane==='terminal'?selectedTerminalId:pane==='agent'?focusedAgentId:pane==='project'?focusedProjectId:''}`);
  function setPaneExpansion(id:string|null){if(embedded)onExpandPane?.(id);else{expandedPaneId=id;if(id)activePaneId=id;}}
  function resetTabExpansion(){focusStep=0;focusTarget='';if(workspaceExpansion===paneId)setPaneExpansion(null);}
  function expandTab(workspace=false){
    if(focusStep){resetTabExpansion();return;}
    focusTarget=contentKey;
    focusStep=workspace?2:1;
    if(workspace)setPaneExpansion(paneId);
  }
  $effect(()=>{if(focusStep && (contentKey!==focusTarget || (focusStep===2 && workspaceExpansion!==paneId)))untrack(resetTabExpansion);});
  $effect(()=>{if(!embedded && expandedPaneId && (activePaneId!==expandedPaneId || !paneIds(layout).includes(expandedPaneId)))expandedPaneId=null;});

  let pointerTabDrag = $state<{tab:PaneTabTransfer;pointerId:number;startX:number;startY:number}|null>(null);
  let paneRefs = $state<Record<string, { openAgentSettings:(draft:Agent)=>void;openSettings:(category?:string)=>void;openTerminalTab:(id:string)=>void;newTerminal:()=>Promise<void>;openEmptyTab:()=>void;openTask: (task: Task) => void; openChannel: (channel: Channel) => void; openTaskComposer: (parentId?: string | null, agentId?: string | null, projectId?: string | null) => void; takeTab: (tab: PaneTabTransfer) => TabPayload | null; receiveTab: (payload: TabPayload, before?: TabKey) => void; reorderTab:(tab:PaneTabTransfer,before?:TabKey)=>void; allTabs: () => PaneTabTransfer[]; captureState:()=>PaneState; restoreState:(value:PaneState)=>void; closeActiveTab:()=>void; swapActiveTab:(direction:1|-1)=>void; toggleDetail:()=>void; hasPending:()=>boolean;attachNativeFiles:(paths:string[])=>Promise<void> }>>({});
  let paneSelections = $state<Record<string,string|null>>({});
  let sidebarScrolled = $state(false);
  let workspaceReady = $state(false);
  let workspacePersistenceError = $state('');
  let workspacePersistenceDisabled = $state(false);
  let workspaceSet = $state<PersistedWorkspaceSet | null>(null);
  const unavailableTerminals = new Set<string>();
  let activeWorkspaceKey = $state<WorkspaceKey>('all');
  type SharedComposer = { text: string; attachments: Attachment[]; context?: string };
  const sharedComposers = getContext<Map<string, SharedComposer>>('monitter-task-composers') ?? new Map<string, SharedComposer>();
  setContext('monitter-task-composers', sharedComposers);
  const workspaceNavigation = getContext<{ task: (task: Task) => void; channel: (channel: Channel) => void; draft: (payload: TabPayload) => Promise<void> }>('monitter-workspace-navigation')
    ?? { task: routeTaskWorkspace, channel: routeChannel, draft: moveDraftToWorkspace };
  setContext('monitter-workspace-navigation', workspaceNavigation);
  let compactDetail = $state(false);
  let queuedAttachments=$state<Record<string,Attachment[]>>({}), attachmentContexts=$state<Record<string,string>>({}), pendingUploads=$state<Record<string,boolean>>({});
  let filePicker=$state<HTMLInputElement>();
  const currentAttachments=$derived(queuedAttachments[currentDraftKey() ?? ''] ?? []);
  const filesBusy=$derived(pendingUploads[currentDraftKey() ?? ''] ?? false);
  let overviewOpen = $state(untrack(()=>!embedded));
  let settingsOpen = $state(false), settingsCategory = $state('appearance');
  let openChannelIds = $state<string[]>([]);
  let channelRecipients = $state<Record<string,string[]>>({});
  $effect(()=>{ if(embedded && parentSnapshot) snapshot=parentSnapshot; });
  $effect(()=>{ if(embedded) activeWorkspaceKey=workspaceKey; });
  $effect(()=>{ onSelection?.(selectedTaskId); });

  const bridge = getBridge();
  const macPlatform = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
  let tabIndexModifier = $state(false);
  const modifierLabel = macPlatform ? '⌘' : 'Ctrl+';
  const nativeMac = $derived(
    !embedded && typeof navigator !== "undefined" && isTauri() && /Mac/.test(navigator.userAgent));
  let nativeFullscreen = $state(false);
  onMount(() => {
    if (!nativeMac) return;
    let mounted = true, revision = 0;
    let timer: ReturnType<typeof setTimeout>;
    const updateFullscreen = () => {
      const request = ++revision;
      void getCurrentWindow().isFullscreen().then(value => {
        if (mounted && request === revision) nativeFullscreen = value;
      }).catch(() => { /* Keep the control clearance if native state is unavailable. */ });
    };
    const resized = () => { updateFullscreen(); clearTimeout(timer); timer = setTimeout(updateFullscreen, 200); };
    window.addEventListener('resize', resized);
    updateFullscreen();
    return () => { mounted = false; clearTimeout(timer); window.removeEventListener('resize', resized); };
  });
  let snapshot = $state<Snapshot | null>(null),
    selectedTaskId = $state<string | null>(null),
    selectedChannelId = $state<string | null>(null),
    pane = $state<"empty" | "overview" | "task" | "channel" | "agent" | "project" | "terminal" | "settings">(untrack(()=>embedded?"empty":"overview"));
  let openTerminalIds=$state<string[]>([]), selectedTerminalId=$state<string|null>(null), terminalBusy=$state(false);
  let openEmptyIds=$state<string[]>([]), selectedEmptyId=$state<string|null>(null);
  const selectedTerminal=$derived(selectedTerminalId ? $terminalSessions[selectedTerminalId] ?? null : null);
  const openTerminals=$derived(openTerminalIds.flatMap(id=>$terminalSessions[id]?[$terminalSessions[id]]:[]));
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
  let taskMenu = $state(false);
  let sidebarCollapsed = $state(false);
  let collapsedAgents = $state<Record<string, boolean>>({});
  let railAgentId = $state<string | null>(null);
  let railAnchor = $state<HTMLButtonElement>();
  let taskMenuAnchor = $state<HTMLButtonElement>();
  let detailTab = $state<'run' | 'git' | 'timeline'>('run');
  let gitState = $state<{ repository: boolean | null; error: string; loading: boolean; status:TaskGitStatus|null }>({ repository: null, error: '', loading: false, status:null });
  let gitPane = $state<GitPane>();
  const railAgent = $derived(snapshot?.agents.find(agent => agent.id === railAgentId) ?? null);
  const sidebarViews = [{ id: 'standard', label: 'Standard', icon: Bot }, { id: 'activity', label: 'Activity', icon: Activity }, { id: 'projects', label: 'Projects', icon: Folder }] as const;
  const projectIcons = [
    { id: 'folder', label: 'Folder', icon: Folder }, { id: 'briefcase', label: 'Briefcase', icon: Briefcase }, { id: 'code', label: 'Code', icon: Code },
    { id: 'rocket', label: 'Rocket', icon: Rocket }, { id: 'globe', label: 'Globe', icon: Globe }, { id: 'palette', label: 'Palette', icon: Palette },
    { id: 'database', label: 'Database', icon: Database }, { id: 'wrench', label: 'Wrench', icon: Wrench }, { id: 'layers', label: 'Layers', icon: Layers },
  ];
  const projectColours = ['#3f9d6a', '#3978d4', '#8755c7', '#c44c79', '#c27524'];
  function projectIconComponent(id: string | undefined) { return projectIcons.find(option => option.id === id)?.icon ?? Folder; }

  let modal = $state<
      | "agent"
      | "hosts"
      | "host"
      | "taskSettings"
      | "channel"
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
  type TaskDraft = { modelSettings?:ModelSettings;modelAgentId?:string;sandbox?:Sandbox;sandboxAgentId?:string; id: string; text: string; title: string; agentId: string; projectId: string; parentId: string | null; nativeSessionId: string; cwd: string; createdTaskId?: string };
  type CollaborationRecord = Collaboration;
  type AgentProfile = Agent & { expertise?: string[]; responsibilities?: string[]; skills?: string[]; collaborationEnabled?: boolean };
  let taskDrafts = $state<Record<string, TaskDraft>>({});
  let openDraftIds = $state<string[]>([]);
  let currentDraftId = $state<string | null>(null);
  let taskAgentId = $state(""),
    taskProjectId = $state(""),
    taskParentId = $state<string | null>(null),
    taskNativeSessionId = $state(""),
    taskCwd = $state(""),
    renameTitle = $state("");
  let palette = $state<"switch" | "controls" | null>(null);
  let vimCommandOpen = $state(false), vimCommandText = $state(''), vimCommandError = $state(''), vimHelpOpen = $state(false);
  let vimCommandInput = $state<HTMLInputElement>();
  let vimArmed = $state(false);
  let vimCompletionSeed='',vimCompletionValue='',vimCompletionIndex=-1;
  let paneFocusChord = $state(false);
  const vimShortcuts = $derived(snapshot?.settings.shortcutMode === 'vim');
  $effect(() => { vimShortcuts; paneFocusChord=false; vimArmed=false; vimCommandOpen=false; });
  let directoryQuery = $state("");
  let focusedAgentId = $state<string | null>(null);
  let focusedProjectId = $state<string | null>(null);
  let collapsedProjects = $state<Record<string, boolean>>({});
  let sidebarOrder = $state<Record<string,string[]>>({});
  function sidebarSorted<T extends {id:string}>(items:T[],group:string):T[] {
    const order=sidebarOrder[group]??[];
    return [...items].sort((a,b)=>(order.indexOf(a.id)<0?Infinity:order.indexOf(a.id))-(order.indexOf(b.id)<0?Infinity:order.indexOf(b.id)));
  }
  function moveSidebar(group:string,id:string,target:string,after:boolean) {
    const visible=Array.from(document.querySelectorAll<HTMLElement>('[data-sidebar-sort-group]')).filter(node=>node.dataset.sidebarSortGroup===group).map(node=>node.dataset.sidebarSortId!).filter(Boolean);
    const order=[...new Set(visible)].filter(value=>value!==id);
    const index=order.indexOf(target);if(index<0)return;
    order.splice(index+(after?1:0),0,id);sidebarOrder={...sidebarOrder,[group]:order};
    try{localStorage.setItem('monitter.sidebar-order.v1',JSON.stringify(sidebarOrder));}catch{error='Could not save sidebar order.';}
  }

  const projects = $derived(snapshot?.projects ?? []);
  const focusedProject = $derived(projects.find(project => project.id === focusedProjectId) ?? null);
  const sidebarView = $derived(snapshot?.settings.sidebarView ?? 'standard');
  const activeTasks = $derived(snapshot?.tasks.filter(task => !task.archived) ?? []);
  const scopedTasks = $derived(activeTasks.filter(task => taskBelongsToWorkspace(task, activeWorkspaceKey)));
  const activityTasks = $derived(activeTasks.filter(task=>!task.channelId).sort((a,b) =>
    Number(b.status === 'running') - Number(a.status === 'running') || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id)));
  const scopedActivityTasks = $derived(scopedTasks.filter(task=>!task.channelId).sort((a,b) =>
    Number(b.status === 'running') - Number(a.status === 'running') || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id)));
  const workspaceLabel = $derived(activeWorkspaceKey === 'all' ? 'All activity' : activeWorkspaceKey.startsWith('agent:')
    ? snapshot?.agents.find(agent => agent.id === activeWorkspaceKey.slice(6))?.name ?? 'Deleted agent'
    : activeWorkspaceKey.slice(8) === 'unassigned' ? 'No project' : projects.find(project => project.id === activeWorkspaceKey.slice(8))?.name ?? 'Deleted project');
  function suggestedTaskCwd(agentId: string, projectId: string) {
    const agent = snapshot?.agents.find(item => item.id === agentId);
    const project = projects.find(item => item.id === projectId);
    return project?.workspaces.find(workspace => workspace.hostId === agent?.hostId)?.cwd || agent?.cwd || snapshot?.hosts.find(host => host.id === agent?.hostId)?.defaultCwd || '';
  }
  const taskFormAgent = $derived(snapshot?.agents.find(agent => agent.id === taskAgentId));
  const taskFormProject = $derived(projects.find(project => project.id === taskProjectId));
  const inheritedTaskCwd = $derived(taskFormAgent?.cwd || snapshot?.hosts.find(host => host.id === taskFormAgent?.hostId)?.defaultCwd || '');
  const taskFormCwd = $derived(taskFormProject?.workspaces.find(workspace => workspace.hostId === taskFormAgent?.hostId)?.cwd || taskCwd || inheritedTaskCwd);
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
    snapshot?.hosts.find((h) => h.id === (pane==='terminal' ? selectedTerminal?.hostId : selectedTask?.hostId)) ??
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
  const timelineEvents = $derived([...visibleEvents].sort((a, b) => b.createdAt - a.createdAt));
  const conversationItems = $derived(groupConversationActivity(
    messages,
    visibleEvents.filter(event => event.kind === "tool" ||
      (event.kind === "reasoning" && event.detail.trim())),
    snapshot?.settings.compressToolCalls === true,
  ));
  const selectedApprovalRequests = $derived(
    selectedTask ? (snapshot?.approvalRequests ?? []).filter(request => request.taskId === selectedTask.id) : [],
  );
  const pendingApprovalRequests = $derived(selectedApprovalRequests
    .filter(request => request.status === 'pending')
    .sort((left, right) => left.createdAt - right.createdAt));
  const resolvedApprovalRequests = $derived(selectedApprovalRequests
    .filter(request => request.status !== 'pending')
    .sort((left, right) => (left.resolvedAt ?? left.createdAt) - (right.resolvedAt ?? right.createdAt)));
  let resolvingApprovalId = $state<string | null>(null);
  const globalPendingApprovals = $derived((snapshot?.approvalRequests ?? []).filter(request => request.status === 'pending')
    .map(request => ({ request, task: snapshot?.tasks.find(task => task.id === request.taskId) ?? null }))
    .filter((item): item is { request: ApprovalRequest; task: Task } => !!item.task)
    .sort((left, right) => left.request.createdAt - right.request.createdAt));
  function approvalCount(scope: WorkspaceKey) { return globalPendingApprovals.filter(item => taskBelongsToWorkspace(item.task, scope)).length; }
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
  function focusExistingChat(kind: 'task' | 'channel', id: string, requester: string): boolean {
    if (embedded) return onExistingChat?.(kind, id, requester) ?? false;
    const candidates = paneIds(layout);
    const owner = [activePaneId, ...candidates.filter(candidate => candidate !== activePaneId)]
      .find(candidate => candidates.includes(candidate) && (candidate === 'main' ? allTabs() : paneRefs[candidate]?.allTabs() ?? [])
        .some(tab => tab.kind === kind && tab.id === id));
    if (!owner || owner === requester) return false;
    activePaneId = owner;
    const target = owner === 'main' ? { openTask, openChannel } : paneRefs[owner];
    if (kind === 'task') {
      const task = snapshot?.tasks.find(item => item.id === id);
      if (task) target?.openTask(task);
    } else {
      const channel = snapshot?.channels.find(item => item.id === id);
      if (channel) target?.openChannel(channel);
    }
    void tick().then(() => document.querySelector<HTMLElement>(`.pane-leaf[data-pane-id="${CSS.escape(owner)}"]`)?.focus({ preventScroll: true }));
    return true;
  }
  function routeTask(task: Task) {
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openTask(task); else openTask(task);
  }
  function routeChannel(channel: Channel) {
    if (embedded) { workspaceNavigation.channel(channel); return; }
    if (activeWorkspaceKey !== 'all') { void switchWorkspace('all').then(changed => { if (changed) routeChannel(channel); }); return; }
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openChannel(channel); else openChannel(channel);
  }
  function routeDraft(agentId: string) {
    const scope = `agent:${agentId}` as WorkspaceKey;
    if (!embedded && scope !== activeWorkspaceKey) { void switchWorkspace(scope).then(changed => { if (changed) routeDraft(agentId); }); return; }
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openTaskComposer(null, agentId); else openTaskComposer(null, agentId);
  }
  function routeProjectDraft(projectId: string) {
    const scope = `project:${projectId}` as WorkspaceKey;
    if (!embedded && scope !== activeWorkspaceKey) { void switchWorkspace(scope).then(changed => { if (changed) routeProjectDraft(projectId); }); return; }
    const target = !embedded && activePaneId !== 'main' ? paneRefs[activePaneId] : null;
    if (target) target.openTaskComposer(null, null, projectId); else openTaskComposer(null, null, projectId);
  }
  function resizeSplit(id: string, ratio: number) {
    function resize(node: PaneLayout): PaneLayout {
      if (!('axis' in node)) return node;
      return node.id === id ? {...node,ratio:Math.max(.15,Math.min(.85,ratio))}
        : {...node,first:resize(node.first),second:resize(node.second)};
    }
    layout = resize(layout);
  }
  function availableTabs(): TabKey[] {
    return [
      ...openTaskIds.map(id=>({kind:'task' as const,id})),
      ...openDraftIds.map(id=>({kind:'draft' as const,id})),
      ...openChannelIds.map(id=>({kind:'channel' as const,id})),
      ...openTerminalIds.map(id=>({kind:'terminal' as const,id})),
      ...openEmptyIds.map(id=>({kind:'empty' as const,id})),
      ...(settingsOpen ? [{kind:'settings' as const,id:'settings'}] : []),
    ];
  }
  function orderedTabs(): TabKey[] { return normalizeTabOrder(tabOrder, availableTabs()); }
  function rememberTab(tab: TabKey, before?: TabKey) {
    const current=orderedTabs();
    if (!before && current.some(item=>item.kind===tab.kind && item.id===tab.id)) { tabOrder=current; return; }
    tabOrder = insertTab(current, tab, before);
  }
  function forgetTab(tab: TabKey) { tabOrder = tabOrder.filter(current=>current.kind!==tab.kind || current.id!==tab.id); }
  export function allTabs(): PaneTabTransfer[] {
    return orderedTabs().map(tab=>({sourcePaneId:paneId,...tab}));
  }
  export function reorderTab(tab: PaneTabTransfer, before?: TabKey) { tabOrder=insertTab(orderedTabs(),tab,before); }
  export function swapActiveTab(direction: 1 | -1) {
    const current = currentVimTab(), tabs = orderedTabs();
    if (!current || tabs.length < 2) return;
    const index = tabs.findIndex(tab => tab.kind === current.kind && tab.id === current.id);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= tabs.length) return;
    [tabs[index], tabs[target]] = [tabs[target], tabs[index]];
    tabOrder = tabs;
  }
  export function toggleDetail() { showDetail = !showDetail; }
  export function hasPending() { return terminalBusy || Object.values(composerPending).some(Boolean) || Object.values(pendingUploads).some(Boolean); }
  export function captureState():PaneState {
    // Persistence must only read reactive state: writing here can recursively trigger itself.
    const captured:PaneState=JSON.parse(JSON.stringify({settingsEditor:{draft:agentDraft,edits:agentEdits},overviewOpen,settingsOpen,settingsCategory,openTerminalIds,selectedTerminalId,openEmptyIds,selectedEmptyId,openTaskIds,openDraftIds,openChannelIds,tabOrder:orderedTabs(),taskDrafts,drafts,selectedTaskId,currentDraftId,selectedChannelId,pane,focusedAgentId,focusedProjectId,showDetail,detailTab,queuedAttachments,attachmentContexts,channelRecipients}));
    const key=currentDraftKey();
    if(key)captured.drafts[key]=composer;
    if(pane==='channel' && selectedChannelId)captured.channelRecipients[selectedChannelId]=[...recipients];
    if(pane==='task' && currentDraftId && captured.taskDrafts[currentDraftId]) {
      captured.taskDrafts[currentDraftId]={...captured.taskDrafts[currentDraftId],text:composer,title:taskTitle,agentId:taskAgentId,projectId:taskProjectId,parentId:taskParentId,nativeSessionId:taskNativeSessionId,cwd:taskCwd};
    }
    return captured;
  }
  export function restoreState(value:PaneState) {
    value = applySharedComposers(value as unknown as Record<string, unknown>) as unknown as PaneState;
    agentDraft=value.settingsEditor?.draft??null;agentEdits=value.settingsEditor?.edits??{};
    ({overviewOpen,settingsOpen,settingsCategory,openTerminalIds,selectedTerminalId,openEmptyIds,selectedEmptyId,openTaskIds,openDraftIds,openChannelIds,tabOrder,taskDrafts,drafts,selectedTaskId,currentDraftId,selectedChannelId,pane,focusedAgentId,focusedProjectId,showDetail,detailTab,queuedAttachments,attachmentContexts,channelRecipients}=value);
    tabOrder = normalizeTabOrder(Array.isArray(tabOrder) ? tabOrder : [], availableTabs());
    composer=drafts[currentDraftKey() ?? ''] ?? '';
    recipients=selectedChannelId?channelRecipients[selectedChannelId]??[]:[];
    const draft=currentDraftId?taskDrafts[currentDraftId]:null;
    if(draft) {composer=draft.text;taskTitle=draft.title;taskAgentId=draft.agentId;taskProjectId=draft.projectId;taskParentId=draft.parentId;taskNativeSessionId=draft.nativeSessionId;taskCwd=draft.cwd??'';}
  }
  function captureChildren() {
    return Object.fromEntries(paneIds(layout).filter(id=>id!=='main').flatMap(id=>paneRefs[id]?[[id,paneRefs[id].captureState()]]:[]));
  }
  function captureWorkspace(): PersistedWorkspace {
    const main = captureState() as unknown as Record<string, unknown>;
    const panes = captureChildren() as unknown as Record<string, Record<string, unknown>>;
    const terminalIds = new Set<string>([
      ...(main.openTerminalIds as string[]),
      ...Object.values(panes).flatMap(state => Array.isArray(state.openTerminalIds) ? state.openTerminalIds as string[] : []),
    ]);
    const terminals = [...terminalIds].flatMap(id => {
      const session = $terminalSessions[id];
      return session ? [{ id, hostId: session.hostId, cwd: session.cwd }] : [];
    });
    for (const terminal of workspaceSet?.workspaces[activeWorkspaceKey]?.terminals ?? []) {
      if (unavailableTerminals.has(terminal.id) && !terminals.some(item => item.id === terminal.id)) terminals.push(terminal);
    }
    return { version: 1, layout, activePaneId, main, panes, sidebarCollapsed, collapsedAgents, collapsedProjects, terminals };
  }
  let workspaceTransition = $state(false);
  function syncSharedTaskDrafts(source: PersistedWorkspace) {
    if (!workspaceSet) return;
    // Each chat has one owning pane in the current layout. Only that pane's
    // visible composer is authoritative; cached copies in other panes are not.
    for (const state of [source.main, ...Object.values(source.panes)]) {
      if (state.pane === 'task' && typeof state.selectedTaskId === 'string' && !state.currentDraftId) {
        const key = `task:${state.selectedTaskId}`;
        sharedComposers.set(key, {
          text: (state.drafts as Record<string, string>)?.[key] ?? '',
          attachments: (state.queuedAttachments as Record<string, Attachment[]>)?.[key] ?? [],
          context: (state.attachmentContexts as Record<string, string>)?.[key],
        });
      }
    }
    for (const workspace of Object.values(workspaceSet.workspaces)) {
      workspace.main = applySharedComposers(workspace.main);
      workspace.panes = Object.fromEntries(Object.entries(workspace.panes).map(([id, state]) => [id, applySharedComposers(state)]));
    }
  }
  function applySharedComposers(state: Record<string, unknown>): Record<string, unknown> {
    const copy = { ...state, drafts: { ...(state.drafts as Record<string, string>) }, queuedAttachments: { ...(state.queuedAttachments as Record<string, Attachment[]>) }, attachmentContexts: { ...(state.attachmentContexts as Record<string, string>) } };
    for (const [key, value] of sharedComposers) {
      copy.drafts[key] = value.text;
      copy.queuedAttachments[key] = [...value.attachments];
      if (value.context) copy.attachmentContexts[key] = value.context;
      else delete copy.attachmentContexts[key];
    }
    return copy;
  }
  function seedSharedComposers(set: PersistedWorkspaceSet) {
    const workspaces = Object.entries(set.workspaces).sort(([left], [right]) => Number(left === set.activeWorkspaceKey) - Number(right === set.activeWorkspaceKey));
    for (const [, workspace] of workspaces) for (const state of [workspace.main, ...Object.values(workspace.panes)]) {
      for (const key of Object.keys((state.drafts as object) ?? {}).filter(key => key.startsWith('task:'))) {
        sharedComposers.set(key, { text: (state.drafts as Record<string, string>)[key], attachments: (state.queuedAttachments as Record<string, Attachment[]>)?.[key] ?? [], context: (state.attachmentContexts as Record<string, string>)?.[key] });
      }
    }
  }
  function publishComposer(key: string, text: string) {
    if (key.startsWith('task:')) sharedComposers.set(key, { text, attachments: [...(queuedAttachments[key] ?? [])], context: attachmentContexts[key] });
  }
  function persistWorkspace() {
    if (workspaceTransition) return true;
    if (!workspaceReady) return true;
    const captured = captureWorkspace();
    if (!workspaceSet) workspaceSet = { version: 2, activeWorkspaceKey, workspaces: {} };
    workspaceSet.workspaces[activeWorkspaceKey] = captured;
    workspaceSet.activeWorkspaceKey = activeWorkspaceKey;
    syncSharedTaskDrafts(captured);
    // Keep this window's workspaces usable in memory without touching a corrupt
    // recovery record. The visible restore error remains until a later restart.
    if (workspacePersistenceDisabled) return true;
    const failure = saveWorkspaceSet(workspaceSet);
    if (failure && !workspacePersistenceError) {
      workspacePersistenceError = failure;
      notice = `Could not save workspace state: ${failure}`;
    }
    if (!failure) workspacePersistenceError = '';
    return !failure;
  }
  function isWorkspaceLayout(value: PaneLayout) {
    const ids = paneIds(value);
    return ids.includes('main') && ids.length <= 4 && new Set(ids).size === ids.length;
  }
  function sanitizePaneState(value: Record<string, unknown>, terminalIds: Record<string, string>, scope = activeWorkspaceKey): PaneState {
    const saved = remapTerminalIds(value, terminalIds) as unknown as Partial<PaneState>;
    const fallback = emptyWorkspace();
    const state: PaneState = {
      ...fallback, ...saved,
      overviewOpen: saved.overviewOpen !== false,
      settingsOpen: saved.settingsOpen === true,
      settingsCategory: ['appearance','typography','behaviour','conversation','agents','directory'].includes(saved.settingsCategory ?? '') ? saved.settingsCategory! : 'appearance',
      openTerminalIds: Array.isArray(saved.openTerminalIds) ? saved.openTerminalIds : fallback.openTerminalIds,
      openEmptyIds: Array.isArray(saved.openEmptyIds) ? saved.openEmptyIds : fallback.openEmptyIds,
      openTaskIds: Array.isArray(saved.openTaskIds) ? saved.openTaskIds : fallback.openTaskIds,
      openDraftIds: Array.isArray(saved.openDraftIds) ? saved.openDraftIds : fallback.openDraftIds,
      openChannelIds: Array.isArray(saved.openChannelIds) ? saved.openChannelIds : fallback.openChannelIds,
      tabOrder: Array.isArray(saved.tabOrder) ? saved.tabOrder.filter((tab): tab is TabKey => !!tab && typeof tab === 'object' && ['task','draft','channel','terminal','settings','empty'].includes((tab as TabKey).kind) && typeof (tab as TabKey).id === 'string') : fallback.tabOrder,
      taskDrafts: saved.taskDrafts && typeof saved.taskDrafts === 'object' ? saved.taskDrafts : fallback.taskDrafts,
      drafts: saved.drafts && typeof saved.drafts === 'object' ? saved.drafts : fallback.drafts,
      queuedAttachments: saved.queuedAttachments && typeof saved.queuedAttachments === 'object' ? saved.queuedAttachments : fallback.queuedAttachments,
      attachmentContexts: saved.attachmentContexts && typeof saved.attachmentContexts === 'object' ? saved.attachmentContexts : fallback.attachmentContexts,
      channelRecipients: saved.channelRecipients && typeof saved.channelRecipients === 'object' ? saved.channelRecipients : fallback.channelRecipients,
      pane: ['empty', 'overview', 'task', 'channel', 'agent', 'project', 'terminal', 'settings'].includes(saved.pane as string) ? saved.pane! : fallback.pane,
      detailTab: ['run', 'git', 'timeline'].includes(saved.detailTab as string) ? saved.detailTab! : fallback.detailTab,
    };
    const taskIds = new Set(snapshot?.tasks.filter(task => !task.archived && taskBelongsToWorkspace(task, scope)).map(task => task.id) ?? []);
    const channelIds = new Set(snapshot?.channels.map(channel => channel.id) ?? []);
    const agentIds = new Set(snapshot?.agents.map(agent => agent.id) ?? []);
    const projectIds = new Set(snapshot?.projects.map(project => project.id) ?? []);
    state.openTaskIds = state.openTaskIds.filter(id => taskIds.has(id));
    state.openChannelIds = state.openChannelIds.filter(id => channelIds.has(id));
    state.openDraftIds = state.openDraftIds.filter(id => {
      const draft = state.taskDrafts[id];
      return !!draft && agentIds.has(draft.agentId) && taskBelongsToWorkspace(draft, scope);
    });
    // Closed draft tabs remain recoverable after restart; only their tab is closed.
    state.taskDrafts = Object.fromEntries(Object.entries(state.taskDrafts).filter(([, draft]) => {
      const value = draft as TaskDraft;
      return !!value;
    }));
    state.tabOrder = normalizeTabOrder(state.tabOrder, [
      ...state.openTaskIds.map(id=>({kind:'task' as const,id})), ...state.openDraftIds.map(id=>({kind:'draft' as const,id})),
      ...state.openChannelIds.map(id=>({kind:'channel' as const,id})), ...state.openTerminalIds.map(id=>({kind:'terminal' as const,id})),
      ...(state.openEmptyIds ?? []).map(id=>({kind:'empty' as const,id})),
      ...(state.settingsOpen ? [{kind:'settings' as const,id:'settings'}] : []),
    ]);
    state.selectedTaskId = state.selectedTaskId && state.openTaskIds.includes(state.selectedTaskId) ? state.selectedTaskId : null;
    state.selectedChannelId = state.selectedChannelId && channelIds.has(state.selectedChannelId) ? state.selectedChannelId : null;
    state.currentDraftId = state.currentDraftId && state.openDraftIds.includes(state.currentDraftId) ? state.currentDraftId : null;
    state.focusedAgentId = state.focusedAgentId && agentIds.has(state.focusedAgentId) ? state.focusedAgentId : null;
    state.focusedProjectId = state.focusedProjectId && projectIds.has(state.focusedProjectId) ? state.focusedProjectId : null;
    if (state.pane === 'task' && !state.selectedTaskId && !state.currentDraftId) state.pane = 'overview';
    if (state.pane === 'channel' && !state.selectedChannelId) state.pane = 'overview';
    if (state.pane === 'settings' && !state.settingsOpen) state.pane = 'overview';
    if (state.pane === 'terminal' && !state.selectedTerminalId) state.pane = 'overview';
    if (state.pane === 'overview' && !state.overviewOpen) state.pane = 'empty';
    return state;
  }
  function pruneWorkspaceScope() {
    if (embedded || !workspaceReady || workspaceTransition) return;
    persistWorkspace();
    const terminalIds = Object.fromEntries(Object.keys($terminalSessions).map(id => [id, id]));
    restoreState(sanitizePaneState(captureState() as unknown as Record<string, unknown>, terminalIds, activeWorkspaceKey));
    for (const id of paneIds(layout)) if (id !== 'main' && paneRefs[id]) {
      paneRefs[id].restoreState(sanitizePaneState(paneRefs[id].captureState() as unknown as Record<string, unknown>, terminalIds, activeWorkspaceKey));
    }
  }
  async function restoreWorkspace(saved = workspaceSet?.workspaces[activeWorkspaceKey], terminalIds: Record<string, string> = {}) {
    if (!saved || !snapshot || !isWorkspaceLayout(saved.layout)) return;
    layout = saved.layout;
    sidebarCollapsed = saved.sidebarCollapsed;
    collapsedAgents = saved.collapsedAgents;
    collapsedProjects = saved.collapsedProjects;
    await tick();
    // Shell restoration happens once at startup, never as a side effect of
    // navigating between workspaces.
    const replacements: Record<string, string> = { ...Object.fromEntries(saved.terminals.map(terminal => [terminal.id, terminal.id])), ...terminalIds };
    restoreState(sanitizePaneState(saved.main, replacements));
    for (const id of paneIds(layout)) if (id !== 'main' && saved.panes[id] && paneRefs[id]) {
      paneRefs[id].restoreState(sanitizePaneState(saved.panes[id], replacements));
    }
    activePaneId = paneIds(layout).includes(saved.activePaneId) ? saved.activePaneId : 'main';
  }
  async function restoreWorkspaceTerminals(set: PersistedWorkspaceSet) {
    const wanted = new Map<string, { id: string; hostId: string; cwd: string }>();
    for (const workspace of Object.values(set.workspaces)) for (const terminal of workspace.terminals) wanted.set(terminal.id, terminal);
    const replacements: Record<string, string> = {};
    const live = await bridge.listTerminals();
    await Promise.all([...wanted.values()].map(async terminal => {
      if (!snapshot?.hosts.some(host => host.id === terminal.hostId) || !terminal.cwd.trim()) { unavailableTerminals.add(terminal.id); return; }
      const existing = live.find(session => session.id === terminal.id);
      if (existing) { registerTerminal(existing); replacements[terminal.id] = existing.id; return; }
      try { const fresh = await bridge.openTerminal({ hostId: terminal.hostId, cwd: terminal.cwd }, 80, 24); registerTerminal(fresh); replacements[terminal.id] = fresh.id; }
      catch (reason) { unavailableTerminals.add(terminal.id); error = `Could not restore terminal in ${terminal.cwd}: ${text(reason)}. Its saved location has been preserved.`; }
    }));
    for (const workspace of Object.values(set.workspaces)) {
      const preserve = Object.fromEntries(workspace.terminals.map(terminal => [terminal.id, replacements[terminal.id] ?? terminal.id]));
      workspace.main = remapTerminalIds(workspace.main, preserve);
      workspace.panes = Object.fromEntries(Object.entries(workspace.panes).map(([id, state]) => [id, remapTerminalIds(state, preserve)]));
      workspace.terminals = workspace.terminals.map(terminal => ({ ...terminal, id: replacements[terminal.id] ?? terminal.id }));
    }
    if (unavailableTerminals.size) notice = `${unavailableTerminals.size} saved terminal${unavailableTerminals.size === 1 ? '' : 's'} unavailable. Their locations are preserved for the next restart.`;
    return replacements;
  }
  function emptyWorkspace() {
    const state = captureState();
    return { ...state, settingsEditor: { draft: null, edits: {} }, overviewOpen: true, settingsOpen: false, openTerminalIds: [], selectedTerminalId: null, openEmptyIds: [], selectedEmptyId: null, openTaskIds: [], openDraftIds: [], openChannelIds: [], tabOrder: [], taskDrafts: {}, drafts: {}, selectedTaskId: null, currentDraftId: null, selectedChannelId: null, focusedAgentId: null, focusedProjectId: null, pane: 'overview' as const, queuedAttachments: {}, attachmentContexts: {}, channelRecipients: {} };
  }
  async function switchWorkspace(next: WorkspaceKey): Promise<boolean> {
    if (next === activeWorkspaceKey) return true;
    if (workspaceTransition || layoutPending()) { notice = 'Wait for the current send or upload before switching workspaces.'; return false; }
    if (!persistWorkspace()) return false;
    workspaceTransition = true;
    try {
      activeWorkspaceKey = next;
      const saved = workspaceSet?.workspaces[next];
      if (saved) await restoreWorkspace(saved);
      else { layout = { id: 'main' }; await tick(); restoreState(emptyWorkspace()); activePaneId = 'main'; }
    } finally { workspaceTransition = false; persistWorkspace(); }
    return activeWorkspaceKey === next;
  }
  async function chooseWorkspace(event: Event) {
    const select = event.currentTarget as HTMLSelectElement;
    await switchWorkspace(select.value as WorkspaceKey);
    select.value = activeWorkspaceKey;
  }
  function routeTaskWorkspace(task: Task) {
    if (embedded) { workspaceNavigation.task(task); return; }
    const target = workspaceForTask(task, activeWorkspaceKey);
    if (target !== activeWorkspaceKey) void switchWorkspace(target).then(changed => { if (changed) routeTask(task); });
    else routeTask(task);
  }
  $effect(() => {
    // Stringifying tracks pane-local edits, including drafts, without mutating state from captureState.
    JSON.stringify({ agentDraft, agentEdits, overviewOpen, settingsOpen, settingsCategory, openTerminalIds, selectedTerminalId, openEmptyIds, selectedEmptyId, openTaskIds, openDraftIds, openChannelIds, tabOrder, taskDrafts, drafts, selectedTaskId, currentDraftId, selectedChannelId, pane, composer, taskTitle, taskAgentId, taskProjectId, taskParentId, taskNativeSessionId, taskCwd, focusedAgentId, focusedProjectId, showDetail, detailTab, queuedAttachments, attachmentContexts, channelRecipients, recipients, layout, activePaneId, sidebarCollapsed, collapsedAgents, collapsedProjects });
    workspaceReady;
    untrack(() => { if (embedded) onWorkspaceChange?.(); else persistWorkspace(); });
  });
  function layoutPending() { return hasPending() || paneIds(layout).some(id=>paneRefs[id]?.hasPending()); }
  export function takeTab(tab: PaneTabTransfer): TabPayload | null {
    saveCurrentDraft();
    if (composerPending[`${tab.kind}:${tab.id}`] || pendingUploads[`${tab.kind}:${tab.id}`]) return null;
    if(tab.kind==='empty') {
      if (!openEmptyIds.includes(tab.id)) return null;
      openEmptyIds = openEmptyIds.filter(id => id !== tab.id);
      if (pane === 'empty' && selectedEmptyId === tab.id) { selectedEmptyId = null; openOverview(); }
      forgetTab(tab); return { tab };
    }
    if(tab.kind==='settings') {
      if(!settingsOpen)return null;
      const category=settingsCategory,settingsEditor:AgentEditorState=JSON.parse(JSON.stringify({draft:agentDraft,edits:agentEdits}));closeSettings();forgetTab(tab);return {tab,settingsCategory:category,settingsEditor};
    }
    if(tab.kind==='terminal') {
      if(terminalBusy || !openTerminalIds.includes(tab.id))return null;
      openTerminalIds=openTerminalIds.filter(id=>id!==tab.id);
      if(pane==='terminal' && selectedTerminalId===tab.id)openOverview();
      forgetTab(tab);
      return {tab};
    }
    const attachments=queuedAttachments[`${tab.kind}:${tab.id}`], attachmentContext=attachmentContexts[`${tab.kind}:${tab.id}`];
    if (tab.kind === 'draft') {
      const draft = taskDrafts[tab.id];
      if (!draft) return null;
      const payload = {tab,draft:{...draft},text:draft.text,attachments,attachmentContext};
      closeTaskDraft(tab.id); delete taskDrafts[tab.id]; delete drafts[`draft:${tab.id}`];
      forgetTab(tab);
      return payload;
    }
    const key = `${tab.kind}:${tab.id}`, text = drafts[key] ?? '';
    if (tab.kind === 'task') closeTaskTab(tab.id);
    else {
      openChannelIds = openChannelIds.filter(id=>id!==tab.id);
      if (selectedChannelId === tab.id) openOverview();
    }
    delete drafts[key];
    forgetTab(tab);
    return {tab,text,attachments,attachmentContext,recipients:channelRecipients[tab.id]};
  }
  export function receiveTab(payload: TabPayload, before?: TabKey) {
    const {tab} = payload;
    if(tab.kind==='empty') { if (!openEmptyIds.includes(tab.id)) openEmptyIds = [...openEmptyIds, tab.id]; selectedEmptyId=tab.id; selectedTaskId=null; selectedChannelId=null; selectedTerminalId=null; currentDraftId=null; pane='empty'; rememberTab(tab,before); return; }
    if(tab.kind==='settings') {agentDraft=payload.settingsEditor?.draft??null;agentEdits=payload.settingsEditor?.edits??{};settingsCategory=payload.settingsCategory??'appearance';openSettings();rememberTab(tab,before);return;}
    if(tab.kind==='terminal') {openTerminalTab(tab.id);rememberTab(tab,before);return;}
    if(tab.kind==='channel')channelRecipients[tab.id]=payload.recipients ?? [];
    queuedAttachments[`${tab.kind}:${tab.id}`]=payload.attachments ?? [];
    if(payload.attachmentContext) attachmentContexts[`${tab.kind}:${tab.id}`]=payload.attachmentContext;
    if (tab.kind === 'draft' && payload.draft) {
      taskDrafts[tab.id] = payload.draft;
      openTaskDraft(payload.draft); rememberTab(tab,before);
    } else if (tab.kind === 'task') {
      const task = snapshot?.tasks.find(task=>task.id===tab.id);
      if (task) { drafts[`task:${tab.id}`] = payload.text ?? ''; openTask(task); rememberTab(tab,before); }
    } else if (tab.kind === 'channel') {
      const channel = snapshot?.channels.find(channel=>channel.id===tab.id);
      if (channel) { drafts[`channel:${tab.id}`] = payload.text ?? ''; openChannel(channel); rememberTab(tab,before); }
    }
  }
  async function setLayout(mode: 'single' | 'columns' | 'grid') {
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
  async function splitPaneForVim(id: string, axis: 'horizontal' | 'vertical') {
    if (embedded) { onVimSplit?.(paneId, axis); return; }
    if (workspaceTransition || layoutPending()) { notice = 'Wait for the current action before splitting a pane.'; return; }
    if (!paneIds(layout).includes(id)) return;
    if (paneIds(layout).length >= 4) { error = 'Monitter supports up to four panes.'; return; }
    saveCurrentDraft();
    const saved = captureChildren();
    const fresh = crypto.randomUUID();
    const insert = (node: PaneLayout): PaneLayout => {
      if ('axis' in node) return { ...node, first: insert(node.first), second: insert(node.second) };
      return node.id === id ? { id: crypto.randomUUID(), axis, ratio: .5, first: node, second: { id: fresh } } : node;
    };
    persistWorkspace(); workspaceTransition = true;
    try {
      layout = insert(layout); await tick();
      for (const pane of paneIds(layout)) if (saved[pane]) paneRefs[pane]?.restoreState(saved[pane]);
      activePaneId = fresh;
    } finally { workspaceTransition = false; persistWorkspace(); }
  }
  async function dropTab(targetId: string, edge: DropEdge, tab: PaneTabTransfer, before?: TabKey) {
    if(embedded) { onTabDrop?.(targetId,edge,tab,before); return; }
    if (workspaceTransition || composerPending[`${tab.kind}:${tab.id}`] || pendingUploads[`${tab.kind}:${tab.id}`] || terminalBusy) { notice='Wait for the current action before moving this tab.'; return; }
    const ids=paneIds(layout);
    if (!ids.includes(tab.sourcePaneId) || !ids.includes(targetId)) return;
    if (edge==='center' && targetId===tab.sourcePaneId) {
      if(before && before.kind===tab.kind && before.id===tab.id) return;
      if(targetId==='main') reorderTab(tab,before); else paneRefs[targetId]?.reorderTab(tab,before); return;
    }
    if (edge!=='center' && ids.length>=4) { notice='Up to four panes are available. Drop in the centre to move a tab.'; return; }
    if(edge!=='center' && layoutPending()) {notice='Wait for the message to be accepted before splitting a pane.';return;}
    const source=tab.sourcePaneId==='main'?{takeTab,hasPending}:paneRefs[tab.sourcePaneId];
    if(!source || source.hasPending())return;
    const previousLayout=layout, previousMain=captureState(), previousChildren=captureChildren(), previousActive=activePaneId;
    // Keep disk state complete while split rendering temporarily unmounts panes.
    persistWorkspace();workspaceTransition=true;
    let moved=false;
    try {
      const payload=source.takeTab(tab);if(!payload)return;
      const saved=edge!=='center'?captureChildren():{};
      let destination=targetId;
      if(edge!=='center') {
        destination=crypto.randomUUID();
        function insert(node:PaneLayout):PaneLayout {
          if('axis' in node)return {...node,first:insert(node.first),second:insert(node.second)};
          if(node.id!==targetId)return node;
          const fresh={id:destination}, before=edge==='left'||edge==='top';
          return {id:crypto.randomUUID(),axis:edge==='left'||edge==='right'?'horizontal':'vertical',ratio:.5,first:before?fresh:node,second:before?node:fresh};
        }
        layout=insert(layout);
      }
      await tick();
      for(const id of paneIds(layout))if(saved[id])paneRefs[id]?.restoreState(saved[id]);
      const receiver=destination==='main'?{receiveTab,allTabs}:paneRefs[destination];
      if(!receiver)throw new Error('The destination pane could not be opened.');
      receiver.receiveTab(payload,before);
      if(!receiver.allTabs().some(item=>item.kind===tab.kind && item.id===tab.id))throw new Error('The destination could not accept this tab.');
      activePaneId=destination;moved=true;
    } catch(reason) {
      layout=previousLayout;await tick();restoreState(previousMain);
      for(const [id,state] of Object.entries(previousChildren))paneRefs[id]?.restoreState(state);
      activePaneId=previousActive;error=`Could not move tab: ${text(reason)}`;
    } finally {workspaceTransition=false;persistWorkspace();}
    if(moved){
      const remaining=tab.sourcePaneId==='main'?captureState():paneRefs[tab.sourcePaneId]?.captureState();
      if(remaining && !remaining.overviewOpen && !(remaining.openTaskIds.length+remaining.openDraftIds.length+remaining.openChannelIds.length+remaining.openTerminalIds.length+(remaining.settingsOpen?1:0)))await removeEmptyPane(tab.sourcePaneId);
    }
  }

  function dragTab(event:DragEvent,kind:PaneTabTransfer['kind'],id:string) {
    saveCurrentDraft();
    if(terminalBusy || !event.dataTransfer || composerPending[`${kind}:${id}`] || pendingUploads[`${kind}:${id}`]) {event.preventDefault();return;}
    event.dataTransfer.effectAllowed='move';
    event.dataTransfer.setData('application/x-monitter-tab',JSON.stringify({sourcePaneId:paneId,kind,id}));
  }
  function startTabPointer(event: PointerEvent, kind: PaneTabTransfer['kind'], id: string) {
    if (event.button !== 0 || terminalBusy || composerPending[`${kind}:${id}`] || pendingUploads[`${kind}:${id}`]) return;
    event.preventDefault();
    const tab={sourcePaneId:paneId,kind,id};
    if (embedded) { onTabPointerStart?.(event,tab); return; }
    pointerTabDrag={tab,pointerId:event.pointerId,startX:event.clientX,startY:event.clientY};
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
    const agentIds=pane==='channel'?effectiveRecipients:currentDraftId?[taskAgentId]:[];
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
  function clearAttachments(key:string,ids:string[]) {queuedAttachments[key]=(queuedAttachments[key]??[]).filter(item=>!ids.includes(item.id)); publishComposer(key, currentDraftKey() === key ? composer : drafts[key] ?? sharedComposers.get(key)?.text ?? '');}
  async function attachFiles(items:(File|string)[]) {
    const key=currentDraftKey(), targets=attachmentTargets(), scope=attachmentScope;
    if(!key || filesBusy || busy)return;
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
    const clear=()=>node.classList.remove('drop-files');
    const leave=(event:DragEvent)=>{if(!(event.relatedTarget instanceof Node) || !node.contains(event.relatedTarget))clear();};
    const drop=(event:DragEvent)=>{clear();const files=Array.from(event.dataTransfer?.files??[]);if(files.length){event.preventDefault();event.stopPropagation();void attachFiles(files);}};
    const paste=(event:ClipboardEvent)=>{const files=Array.from(event.clipboardData?.files??[]);if(files.length){event.preventDefault();void attachFiles(files);}};
    node.addEventListener('dragover',over);node.addEventListener('dragleave',leave);node.addEventListener('drop',drop);node.addEventListener('paste',paste);
    return {destroy(){node.removeEventListener('dragover',over);node.removeEventListener('dragleave',leave);node.removeEventListener('drop',drop);node.removeEventListener('paste',paste);}};
  }
  const draftModelSettings=$derived(currentTaskDraft?.modelAgentId===taskAgentId ? currentTaskDraft?.modelSettings ?? null : null);
  const draftSandbox=$derived(currentTaskDraft?.sandboxAgentId===taskAgentId ? currentTaskDraft?.sandbox ?? taskFormAgent?.sandbox ?? 'read-only' : taskFormAgent?.sandbox ?? 'read-only');
  async function changeModel(settings:ModelSettings) {
    const draft=currentTaskDraft, agentId=taskAgentId, taskId=selectedTask?.id ?? draft?.createdTaskId;
    if(taskId) {
      const ticket=++snapshotIssued;
      const result=await bridge.setTaskModelSettings(taskId,settings);
      applySnapshot(result,ticket);
    }
    if(draft && taskDrafts[draft.id]) taskDrafts[draft.id]={...taskDrafts[draft.id],modelSettings:settings,modelAgentId:agentId};
  }
  async function changeSandbox(sandbox:Sandbox) {
    const draft=currentTaskDraft, agentId=taskAgentId, taskId=selectedTask?.id ?? draft?.createdTaskId;
    if(taskId) {
      const ticket=++snapshotIssued;
      const result=await bridge.setTaskSandbox(taskId,sandbox);
      applySnapshot(result,ticket);
    }
    if(draft && taskDrafts[draft.id]) taskDrafts[draft.id]={...taskDrafts[draft.id],sandbox,sandboxAgentId:agentId};
  }
  const activeTurnDelivery = "Delivered to the recipient’s active turn via its Monitter inbox.";
  const collaborationStatus = (item: CollaborationRecord) => item.result === activeTurnDelivery ? "Delivered" : item.status;
  const profileList = (value: string[] | undefined) => (value ?? []).join("\n");
  const parseProfileList = (value: string) => value.split(/\n/);
  const senderName = (message: (typeof messages)[number]) => {
    if (message.role === 'user') return splitOperatorMessage(message.text.replace(/^\[Two human operators are collaborating[^\n]*\]\n/, '')).name;
    const sender = (message as typeof message & { senderAgentId?: string | null }).senderAgentId;
    return sender ? snapshot?.agents.find(agent => agent.id === sender)?.name ?? "Agent" : null;
  };
  const operatorMessageText = (value: string) => splitOperatorMessage(value.replace(/^\[Two human operators are collaborating[^\n]*\]\n/, '')).text;
  const operatorShareFor = (taskId: string) => {
    const share = $activeOperatorShare;
    return share?.taskIds.includes(taskId) ? share : null;
  };
  const operatorPrompt = (taskId: string, value: string) => {
    const share = operatorShareFor(taskId);
    return share ? formatOperatorMessage([share.primary, share.visitor], share.primary, value) : value;
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
  const agentAvatarIcons = [Bot, Terminal, Code, Rocket, Wrench, Layers, Briefcase, Database, Globe, Network];
  const agentAvatarIcon = (agent: Agent | null | undefined) => {
    const seed = agent?.id || agent?.name || 'agent';
    let hash = 0;
    for (let index = 0; index < seed.length; index += 1) hash = (hash * 31 + seed.charCodeAt(index)) >>> 0;
    return agentAvatarIcons[hash % agentAvatarIcons.length];
  };
  async function chooseAvatar(file?: File) {
    if (!agentDraft || !file) return;
    if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type) || file.size > 2 * 1024 * 1024) { error = 'Choose a PNG, JPEG, or WebP image up to 2 MiB.'; return; }
    const target = agentDraft;
    const reader = new FileReader();
    reader.onerror = () => { if (agentDraft === target) error = 'Could not read this image.'; };
    reader.onload = () => {
      if (agentDraft !== target || settingsCategory !== 'agents') return;
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
    root.style.setProperty('--interface-font-ratio', String((settings.interfaceFontSize ?? 14) / 14));
    root.style.setProperty('--chat-font-ratio', String((settings.chatFontSize ?? 13) / 13));
    root.style.setProperty('--chat-font-size', `${settings.chatFontSize ?? 13}px`);
    root.style.setProperty('--terminal-font-size', String(settings.terminalFontSize ?? 14));
    root.style.setProperty('--chat-line-height', String(settings.chatLineHeight ?? 1.65));
    root.style.setProperty('--terminal-line-height', String(settings.terminalLineHeight ?? 1));
    const fontStack = (name: string | undefined, fallback: string) => name?.trim() ? `${JSON.stringify(name.trim())}, ${fallback}` : fallback;
    root.style.setProperty('--interface-font', fontStack(settings.interfaceFont, '"IBM Plex Sans", system-ui, sans-serif'));
    root.style.setProperty('--chat-font', fontStack(settings.chatFont, '"IBM Plex Sans", system-ui, sans-serif'));
    root.style.setProperty('--terminal-font', fontStack(settings.terminalFont, '"IBM Plex Mono", Menlo, monospace'));
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
    if(embedded) onSnapshot?.(next); else { applyAppearance(next.settings); untrack(pruneWorkspaceScope); }
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
  async function resolveApproval(request: ApprovalRequest, decision: 'approve_once' | 'deny') {
    if (busy || request.status !== 'pending') return;
    resolvingApprovalId = request.id;
    try {
      await run(() => bridge.resolveApproval(request.id, decision));
    } finally {
      resolvingApprovalId = null;
    }
  }
  function debouncedReload() {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(reload, 125);
  }
  onMount(() => {
    if(!embedded)try{const saved=JSON.parse(localStorage.getItem('monitter.sidebar-order.v1')??'{}');if(saved && typeof saved==='object' && !Array.isArray(saved))sidebarOrder=Object.fromEntries(Object.entries(saved).filter(([,ids])=>Array.isArray(ids)&&ids.every(id=>typeof id==='string')) as [string,string[]][]);}catch{/* Use original order if storage is unavailable. */}

    if (embedded) return;
    let unlisten: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;
    let unlistenCloseTab: UnlistenFn | undefined;
    let unlistenBeforeQuit: UnlistenFn | undefined;
    let mounted = true;
    let dragGeneration = 0;
    const clearNativeDrop = () => document.querySelectorAll('.composer.drop-files').forEach(node=>node.classList.remove('drop-files'));
    const persistOnPageHide = () => persistWorkspace();
    window.addEventListener('pagehide', persistOnPageHide);
    void (async () => {
      try {
        if (isTauri()) {
          const stopBeforeQuit = await listen('monitter-before-quit', async () => {
            if (workspaceReady && !persistWorkspace()) { error = workspacePersistenceError || 'Could not save workspace state before quitting.'; return; }
            try { await invoke('finish_quit'); } catch (reason) { error = `Could not quit: ${text(reason)}`; }
          });
          if (mounted) unlistenBeforeQuit = stopBeforeQuit; else { stopBeforeQuit(); return; }
        }
        const stopListening = await bridge.onChanged(debouncedReload);
        if (!mounted) {
          stopListening();
          return;
        }
        unlisten = stopListening;
        await reload();
        try {
          workspaceSet = loadWorkspaceSet();
          if (workspaceSet) {
            seedSharedComposers(workspaceSet);
            activeWorkspaceKey = workspaceSet.activeWorkspaceKey;
            const terminalIds = await restoreWorkspaceTerminals(workspaceSet);
            const remappedIds = Object.fromEntries(Object.values(terminalIds).map(id => [id, id]));
            await restoreWorkspace(workspaceSet.workspaces[activeWorkspaceKey], remappedIds);
          }
        } catch (reason) {
          workspacePersistenceDisabled = true;
          workspacePersistenceError = text(reason);
          error = `Could not restore workspace state: ${workspacePersistenceError}`;
        }
        workspaceReady = true;
        if (!workspacePersistenceDisabled) persistWorkspace();
        if(isTauri()) {
          const stopCloseTab = await listen('monitter-close-tab', () => {
            if (mounted) {
              cancelPaneFocusChord();
              closeFocusedTab();
            }
          });
          if (mounted) unlistenCloseTab = stopCloseTab; else stopCloseTab();
          const stopDrop=await getCurrentWebview().onDragDropEvent(event=>{
            const payload=event.payload, generation=++dragGeneration;
            if(payload.type==='leave') { clearNativeDrop(); return; }
            if(payload.type==='drop') clearNativeDrop();
            void (async()=>{
              const physical=await getCurrentWindow().innerSize();
              if(!mounted || (payload.type!=='drop' && generation!==dragGeneration))return;
              const x=payload.position.x/(physical.width/window.innerWidth), y=payload.position.y/(physical.height/window.innerHeight);
              const composerNode=document.elementFromPoint(x,y)?.closest('.composer');
              if(payload.type!=='drop') {
                clearNativeDrop();
                composerNode?.classList.add('drop-files');
                return;
              }
              if(!composerNode)return;
              const id=composerNode.closest<HTMLElement>('[data-pane-id]')?.dataset.paneId ?? 'main';
              if(id==='main') await attachNativeFiles(payload.paths); else await paneRefs[id]?.attachNativeFiles(payload.paths);
            })().catch(reason=>{clearNativeDrop();error=text(reason)});
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
      unlistenCloseTab?.();
      unlistenBeforeQuit?.();
      cancelPaneFocusChord();
      persistWorkspace();
      window.removeEventListener('pagehide', persistOnPageHide);
      clearNativeDrop();
    };
  });
  function slashFloating(node: HTMLElement) {
    const anchor = node.closest<HTMLElement>('.composer');
    if (!anchor) return;
    const size = () => { node.style.width = `${Math.max(0, anchor.getBoundingClientRect().width - 20)}px`; };
    size();
    const popup = floating(node, {anchor, side:'above', focus:false});
    const observer = new ResizeObserver(size);
    observer.observe(anchor);
    return {destroy() { observer.disconnect(); popup.destroy(); }};
  }
  function currentDraftKey() {
    if (pane === "task" && currentDraftId) return `draft:${currentDraftId}`;
    if (pane === "task" && selectedTaskId) return `task:${selectedTaskId}`;
    if (pane === "channel" && selectedChannelId)
      return `channel:${selectedChannelId}`;
    return null;
  }
  function saveCurrentDraft() {
    taskMenu = false;
    railAgentId = null;
    const key = currentDraftKey();
    if(pane==='channel' && selectedChannelId)channelRecipients[selectedChannelId]=[...recipients];
    if (key && drafts[key] !== composer) drafts[key] = composer;
    if (key) publishComposer(key, composer);
    if (pane === "task" && currentDraftId && taskDrafts[currentDraftId]) {
      const current = taskDrafts[currentDraftId];
      if (current.text !== composer || current.title !== taskTitle || current.agentId !== taskAgentId || current.projectId !== taskProjectId || current.parentId !== taskParentId || current.nativeSessionId !== taskNativeSessionId || current.cwd !== taskCwd) {
        taskDrafts[currentDraftId] = { ...current, text: composer, title: taskTitle, agentId: taskAgentId, projectId: taskProjectId, parentId: taskParentId, nativeSessionId: taskNativeSessionId, cwd: taskCwd };
      }
    }
  }
  function terminalTarget():TerminalTarget {
    if(pane==='task' && selectedTask && !currentDraftId)return {taskId:selectedTask.id};
    if(currentTaskDraft && taskAgentId)return {agentId:taskAgentId,projectId:taskProjectId||null};
    if(pane==='agent' && focusedAgent)return {agentId:focusedAgent.id};
    if(pane==='project' && focusedProject)return {projectId:focusedProject.id};
    if(pane==='terminal' && selectedTerminal)return {hostId:selectedTerminal.hostId};
    if(activeWorkspaceKey.startsWith('agent:'))return {agentId:activeWorkspaceKey.slice(6)};
    if(activeWorkspaceKey.startsWith('project:'))return {projectId:activeWorkspaceKey.slice(8)==='unassigned'?null:activeWorkspaceKey.slice(8)};
    return {};
  }
  export async function newTerminal() {
    if(terminalBusy)return;
    const target=terminalTarget();terminalBusy=true;error='';
    try {const session=await bridge.openTerminal(target,80,24);registerTerminal(session);openTerminalTab(session.id);}
    catch(reason){error=`Could not open terminal: ${text(reason)}`;}
    finally{terminalBusy=false;}
  }
  export function openTerminalTab(id:string) {
    if(!$terminalSessions[id])return;
    saveCurrentDraft();if(!openTerminalIds.includes(id))openTerminalIds=[...openTerminalIds,id];
    rememberTab({kind:'terminal',id});
    selectedTerminalId=id;selectedEmptyId=null;selectedTaskId=null;selectedChannelId=null;currentDraftId=null;
    focusedAgentId=null;focusedProjectId=null;composer='';pane='terminal';
  }
  $effect(() => {
    const sessions=$terminalSessions, ids=openTerminalIds;
    if(!ids.some(id=>!sessions[id]))return;
    untrack(()=>{
      openTerminalIds=ids.filter(id=>sessions[id]);
      if(pane!=='terminal'||!selectedTerminalId||sessions[selectedTerminalId])return;
      const terminal=openTerminalIds.at(-1);
      const task=snapshot?.tasks.find(task=>task.id===openTaskIds.at(-1));
      const channel=snapshot?.channels.find(channel=>channel.id===openChannelIds.at(-1));
      const draft=taskDrafts[openDraftIds.at(-1)??''];
      if(terminal)openTerminalTab(terminal);
      else if(task)openTask(task);
      else if(channel)openChannel(channel);
      else if(draft)openTaskDraft(draft);
      else openOverview();
    });
  });
  async function closeTerminalTab(id:string) {
    if(terminalBusy)return;
    terminalBusy=true;
    try {await closeTerminalSession(id);openTerminalIds=openTerminalIds.filter(value=>value!==id);forgetTab({kind:'terminal',id});
      if(pane==='terminal' && selectedTerminalId===id) {const next=openTerminalIds.at(-1);if(next)openTerminalTab(next);else openOverview();}
    } catch(reason){error=`Could not close terminal: ${text(reason)}`;}
    finally{terminalBusy=false;}
  }
  function routeTerminal(id:string) {
    if(embedded){onTerminalSelect?.(id);return;}
    const owner=paneIds(layout).find(candidate=>(candidate==='main'?allTabs():paneRefs[candidate]?.allTabs()??[]).some(tab=>tab.kind==='terminal'&&tab.id===id));
    if (!owner) {
      const scope = Object.entries(workspaceSet?.workspaces ?? {}).find(([key, workspace]) => key !== activeWorkspaceKey && workspace.terminals.some(terminal => terminal.id === id))?.[0] as WorkspaceKey | undefined;
      if (scope) { void switchWorkspace(scope).then(changed => { if (changed) routeTerminal(id); }); return; }
    }
    if(owner && owner!=='main'){activePaneId=owner;paneRefs[owner]?.openTerminalTab(id);}
    else{activePaneId='main';openTerminalTab(id);}
  }
  function closeOverview() {
    const next=orderedTabs().at(-1);
    if(!next){if(embedded)onClosePane?.(paneId);else void removeEmptyPane(paneId);return;}
    overviewOpen=false;
    if(pane!=='overview')return;
    if(next.kind==='settings')openSettings();
    else if(next.kind==='terminal')openTerminalTab(next.id);
    else if(next.kind==='draft')openTaskDraft(taskDrafts[next.id]);
    else if(next.kind==='task'){const task=snapshot?.tasks.find(item=>item.id===next.id);if(task)openTask(task);}
    else {const channel=snapshot?.channels.find(item=>item.id===next.id);if(channel)openChannel(channel);}
  }
  async function removeEmptyPane(id:string) {
    if(embedded){onClosePane?.(id);return;}
    const ids=paneIds(layout);
    if(ids.length===1 || !ids.includes(id))return;
    if(workspaceTransition || layoutPending()){notice='Wait for the current action before closing this pane.';return;}
    if((id==='main'?allTabs():paneRefs[id]?.allTabs()??[]).length)return;
    const states:Record<string,PaneState>={main:captureState(),...captureChildren()};
    function prune(node:PaneLayout):PaneLayout|null {
      if(!('axis' in node))return node.id===id?null:node;
      const first=prune(node.first),second=prune(node.second);
      return first && second?{...node,first,second}:first??second;
    }
    let next=prune(layout)!;
    const promoted=id==='main'?paneIds(next)[0]:null;
    if(promoted){
      const rename=(node:PaneLayout):PaneLayout=>'axis' in node?{...node,first:rename(node.first),second:rename(node.second)}:node.id===promoted?{id:'main'}:node;
      next=rename(next);
    }
    persistWorkspace();workspaceTransition=true;
    try {
      layout=next;await tick();
      if(promoted)restoreState(states[promoted]);
      for(const remaining of paneIds(next))if(remaining!=='main' && states[remaining])paneRefs[remaining]?.restoreState(states[remaining]);
      if(activePaneId===id)activePaneId=promoted?'main':paneIds(next)[0];
      else if(activePaneId===promoted)activePaneId='main';
    }finally{workspaceTransition=false;persistWorkspace();}
  }
  function openOverview() {
    saveCurrentDraft();
    selectedTaskId = null;
    currentDraftId = null;
    selectedChannelId = null;
    composer = "";
    focusedAgentId = null;
    focusedProjectId = null;
    pane = overviewOpen ? "overview" : "empty";
  }
  export function openTask(task: Task) {
    if (!taskBelongsToWorkspace(task, activeWorkspaceKey)) { workspaceNavigation.task(task); return; }
    if (focusExistingChat('task', task.id, paneId)) return;
    saveCurrentDraft();
    if (!openTaskIds.includes(task.id)) openTaskIds = [...openTaskIds, task.id];
    rememberTab({kind:'task',id:task.id});
    selectedTaskId = task.id;
    currentDraftId = null;
    selectedChannelId = null;
    const key = `task:${task.id}`, shared = sharedComposers.get(key);
    if (shared) { drafts[key] = shared.text; queuedAttachments[key] = [...shared.attachments]; if (shared.context) attachmentContexts[key] = shared.context; else delete attachmentContexts[key]; }
    composer = shared?.text ?? drafts[key] ?? "";
    focusedAgentId = null;
    focusedProjectId = null;
    pane = "task";
    scrollRevision += 1;
  }
  function closeTaskTab(id: string) {
    openTaskIds = openTaskIds.filter(openId => openId !== id);
    forgetTab({kind:'task',id});
    if (selectedTaskId === id) {
      const next = openTasks.at(-1);
      if (next) openTask(next); else openOverview();
    }
  }
  export function openChannel(channel: Channel) {
    if (activeWorkspaceKey !== 'all') { workspaceNavigation.channel(channel); return; }
    if (focusExistingChat('channel', channel.id, paneId)) return;
    if(!openChannelIds.includes(channel.id)) openChannelIds=[...openChannelIds,channel.id];
    rememberTab({kind:'channel',id:channel.id});
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
  export function openEmptyTab() {
    saveCurrentDraft();
    const id = crypto.randomUUID();
    openEmptyIds = [...openEmptyIds, id];
    selectedEmptyId = id; selectedTaskId = null; selectedChannelId = null; selectedTerminalId = null; currentDraftId = null;
    focusedAgentId = null; focusedProjectId = null; composer = ''; pane = 'empty';
    rememberTab({ kind: 'empty', id });
  }
  function closeEmptyTab(id: string) {
    openEmptyIds = openEmptyIds.filter(item => item !== id); forgetTab({ kind: 'empty', id });
    if (selectedEmptyId === id) { selectedEmptyId = null; openOverview(); }
  }
  export function openTaskComposer(parentId: string | null = null, agentId: string | null = null, projectId?: string | null) {
    saveCurrentDraft();
    const id = crypto.randomUUID();
    const scopedProject = activeWorkspaceKey.startsWith('project:') ? activeWorkspaceKey.slice(8) === 'unassigned' ? '' : activeWorkspaceKey.slice(8) : '';
    const scopedAgent = activeWorkspaceKey.startsWith('agent:') ? activeWorkspaceKey.slice(6) : '';
    const project = projectId === undefined ? (parentId ? snapshot?.tasks.find(task=>task.id===parentId)?.projectId ?? '' : focusedProjectId ?? selectedTask?.projectId ?? scopedProject) : projectId ?? '';
    const agent = agentId ?? (parentId ? selectedTask?.agentId ?? defaultAgent?.id ?? '' : (scopedAgent || focusedAgent?.id || defaultAgent?.id || ''));
    taskDrafts[id] = { id, text: '', title: '', agentId: agent, projectId: project, parentId, nativeSessionId: '', cwd: suggestedTaskCwd(agent, project) };
    openDraftIds = [...openDraftIds, id]; currentDraftId = id; selectedTaskId = null; selectedEmptyId = null; selectedChannelId = null; pane = 'task';
    rememberTab({kind:'draft',id});
    composer = ''; taskTitle = ''; taskAgentId = agent; taskProjectId = project; taskParentId = parentId; taskNativeSessionId = ''; taskCwd = suggestedTaskCwd(agent, project); scrollRevision += 1;
  }
  function openTaskDraft(draft: TaskDraft) {
    saveCurrentDraft();
    if (!openDraftIds.includes(draft.id)) openDraftIds = [...openDraftIds, draft.id];
    rememberTab({kind:'draft',id:draft.id});
    currentDraftId = draft.id; selectedTaskId = null; selectedEmptyId = null; selectedChannelId = null; pane = 'task';
    composer = draft.text; taskTitle = draft.title; taskAgentId = draft.agentId; taskProjectId = draft.projectId; taskParentId = draft.parentId; taskNativeSessionId = draft.nativeSessionId; taskCwd = draft.cwd ?? ''; scrollRevision += 1;
  }
  async function moveDraftToWorkspace(payload: TabPayload) {
    if (!payload.draft) return;
    const scope = workspaceForTask(payload.draft, activeWorkspaceKey);
    if (!await switchWorkspace(scope)) return;
    const target = activePaneId === 'main' ? { receiveTab } : paneRefs[activePaneId];
    target?.receiveTab(payload);
  }
  async function routeChangedDraft() {
    await tick();
    if (hasPending()) return;
    saveCurrentDraft();
    const draft = currentDraftId ? taskDrafts[currentDraftId] : null;
    if (!draft || taskBelongsToWorkspace(draft, activeWorkspaceKey)) return;
    const key = `draft:${draft.id}`;
    await workspaceNavigation.draft({ tab: { kind: 'draft', id: draft.id, sourcePaneId: paneId }, draft: { ...draft }, text: composer, attachments: queuedAttachments[key], attachmentContext: attachmentContexts[key] });
  }
  function closeTaskDraft(id: string) { saveCurrentDraft(); openDraftIds = openDraftIds.filter(item => item !== id); forgetTab({kind:'draft',id}); if (currentDraftId === id) { currentDraftId = null; const next = openDrafts.at(-1); if (next) openTaskDraft(next); else openOverview(); } }
  export function openSettings(category?:string) {
    if(category)settingsCategory=category;
    saveCurrentDraft(); settingsOpen=true; rememberTab({kind:'settings',id:'settings'}); pane='settings';
    selectedTaskId=null;selectedEmptyId=null;selectedChannelId=null;selectedTerminalId=null;currentDraftId=null;
    focusedAgentId=null;focusedProjectId=null;composer='';modal=null;palette=null;
  }
  function routeSettings(category?:string) {
    if(embedded){onSettingsSelect?.(category);return;}
    const owner=paneIds(layout).find(id=>(id==='main'?allTabs():paneRefs[id]?.allTabs()??[]).some(tab=>tab.kind==='settings')) ?? activePaneId;
    activePaneId=owner;
    if(owner==='main')openSettings(category);else paneRefs[owner]?.openSettings(category);
  }
  function closeSettings() {
    settingsOpen=false;forgetTab({kind:'settings',id:'settings'});if(pane!=='settings')return;
    const task=openTasks.at(-1), draft=openDrafts.at(-1);
    const channel=snapshot?.channels.find(item=>item.id===openChannelIds.at(-1));
    const terminal=openTerminalIds.at(-1);
    if(task)openTask(task);else if(draft)openTaskDraft(draft);else if(channel)openChannel(channel);
    else if(terminal)openTerminalTab(terminal);else openOverview();
  }
  async function savePreference(patch:Partial<Settings>) {
    error='';
    try {
      const next=await saveSettingsPatch(patch);
      applySnapshot(next,++snapshotIssued);
    } catch(reason) {
      error=`Could not save settings: ${text(reason)}`;
      throw reason;
    }
  }
  export function closeActiveTab() {
    if(pane==='overview'){closeOverview();return;}
    if(pane==='empty'){if(selectedEmptyId)closeEmptyTab(selectedEmptyId);else closeOverview();return;}
    if(pane==='settings'){closeSettings();return;}
    if (pane === 'terminal' && selectedTerminalId) { void closeTerminalTab(selectedTerminalId); return; }
    if (pane === 'task' && currentDraftId) { closeTaskDraft(currentDraftId); return; }
    if (pane === 'task' && selectedTaskId) { closeTaskTab(selectedTaskId); return; }
    if (pane === 'channel' && selectedChannelId) {
      saveCurrentDraft();
      openChannelIds = openChannelIds.filter(id => id !== selectedChannelId);
      forgetTab({kind:'channel',id:selectedChannelId});
      openOverview();
    }
  }

  function openVimCommand() {
    vimCompletionSeed='';vimCompletionValue='';vimCompletionIndex=-1;
    vimArmed = false; vimCommandOpen = true; vimCommandError = ''; vimHelpOpen = false; vimCommandText = '';
    void tick().then(() => vimCommandInput?.focus());
  }
  function closeVimCommand() { vimCommandOpen = false; vimCommandError = ''; vimHelpOpen = false; }
  function selectRelativeTab(direction: 1 | -1) {
    const tabs = orderedTabs();
    if (!tabs.length) return;
    const current = pane === 'task' ? (currentDraftId ? { kind: 'draft' as const, id: currentDraftId } : selectedTaskId ? { kind: 'task' as const, id: selectedTaskId } : null)
      : pane === 'channel' && selectedChannelId ? { kind: 'channel' as const, id: selectedChannelId }
      : pane === 'terminal' && selectedTerminalId ? { kind: 'terminal' as const, id: selectedTerminalId }
      : pane === 'settings' ? { kind: 'settings' as const, id: 'settings' } : null;
    const index = current ? tabs.findIndex(tab => tab.kind === current.kind && tab.id === current.id) : -1;
    const next = tabs[(index + direction + tabs.length) % tabs.length];
    if (next.kind === 'task') { const task = snapshot?.tasks.find(item => item.id === next.id); if (task) openTask(task); }
    else if (next.kind === 'draft') { const draft = taskDrafts[next.id]; if (draft) openTaskDraft(draft); }
    else if (next.kind === 'channel') { const channel = snapshot?.channels.find(item => item.id === next.id); if (channel) openChannel(channel); }
    else if (next.kind === 'terminal') openTerminalTab(next.id);
    else if (next.kind === 'empty') { selectedEmptyId=next.id; pane='empty'; }
    else openSettings();
  }
  function currentVimTab(): TabKey | null {
    if (pane === 'task') return currentDraftId ? {kind:'draft',id:currentDraftId} : selectedTaskId ? {kind:'task',id:selectedTaskId} : null;
    if (pane === 'channel' && selectedChannelId) return {kind:'channel',id:selectedChannelId};
    if (pane === 'terminal' && selectedTerminalId) return {kind:'terminal',id:selectedTerminalId};
    if (pane === 'empty' && selectedEmptyId) return {kind:'empty',id:selectedEmptyId};
    return pane === 'settings' ? {kind:'settings',id:'settings'} : null;
  }
  function selectVimTab(target: VimTabTarget): boolean {
    const tabs=orderedTabs(); if(!tabs.length)return false;
    const current=currentVimTab(), index=current?tabs.findIndex(tab=>tab.kind===current.kind&&tab.id===current.id):-1;
    const targetIndex=target.kind==='last'?tabs.length-1:target.kind==='index'?target.index-1:((index+target.offset)%tabs.length+tabs.length)%tabs.length;
    const tab=tabs[targetIndex]; if(!tab)return false;
    if(tab.kind==='task'){const item=snapshot?.tasks.find(item=>item.id===tab.id);if(item)openTask(item);}
    else if(tab.kind==='draft'){const item=taskDrafts[tab.id];if(item)openTaskDraft(item);}
    else if(tab.kind==='channel'){const item=snapshot?.channels.find(item=>item.id===tab.id);if(item)openChannel(item);}
    else if(tab.kind==='terminal')openTerminalTab(tab.id); else if(tab.kind==='empty'){selectedEmptyId=tab.id;pane='empty';} else openSettings(); return true;
  }
  async function executeWorkspaceVim(command: VimCommand) {
    if (embedded) { await onVimWorkspace?.(paneId,command); return; }
    const ids=paneIds(layout), current=activePaneId;
    if(command.kind==='focus-pane') { const t=command.target; if(typeof t==='object'&&'direction'in t)focusAdjacentPane(t.direction); else { const index=t==='first'?0:t==='last'?ids.length-1:t==='next'?(ids.indexOf(current)+1)%ids.length:t==='previous'?(ids.indexOf(current)-1+ids.length)%ids.length:t.index-1; if(ids[index])activePaneId=ids[index]; } return; }
    if(command.kind==='only-pane') {
      const kept=current==='main'?captureState():paneRefs[current]?.captureState();
      await setLayout('single'); activePaneId='main';
      if(kept){
        if(kept.pane==='task'&&kept.currentDraftId&&taskDrafts[kept.currentDraftId])openTaskDraft(taskDrafts[kept.currentDraftId]);
        else if(kept.pane==='task'){const task=snapshot?.tasks.find(t=>t.id===kept.selectedTaskId);if(task)openTask(task);}
        else if(kept.pane==='channel'){const channel=snapshot?.channels.find(c=>c.id===kept.selectedChannelId);if(channel)openChannel(channel);}
        else if(kept.pane==='terminal'&&kept.selectedTerminalId)openTerminalTab(kept.selectedTerminalId);
        else if(kept.pane==='settings')openSettings(kept.settingsCategory);
        else openOverview();
      }
      return;
    }
    if(command.kind==='close-pane') { const id=command.target?ids[command.target-1]:current; if(id&&ids.length>1) { const destination=ids.find(item=>item!==id)!; const tabs=id==='main'?allTabs():paneRefs[id]?.allTabs()??[]; for(const tab of tabs)await dropTab(destination,'center',tab); await removeEmptyPane(id); } return; }
    if(command.kind==='equalize-panes') { const equal=(node:PaneLayout):PaneLayout=>'axis'in node?{...node,ratio:.5,first:equal(node.first),second:equal(node.second)}:node; layout=equal(layout);persistWorkspace();return; }
    if(command.kind==='split') { const previous=activePaneId;await splitPaneForVim(current,command.axis);if(command.size&&activePaneId!==previous)await executeWorkspaceVim({kind:'resize-pane',axis:command.axis,size:command.size});return; }
    const restoreLayout=async(next:PaneLayout)=>{const states:Record<string,PaneState>={main:captureState(),...captureChildren()};persistWorkspace();workspaceTransition=true;try{layout=next;await tick();for(const id of paneIds(next))if(id==='main')restoreState(states.main);else if(states[id])paneRefs[id]?.restoreState(states[id]);}finally{workspaceTransition=false;persistWorkspace();}};
    const exchangeContents = async (sources: string[]) => {
      const states: Record<string, PaneState> = {main: captureState(), ...captureChildren()};
      workspaceTransition = true;
      try {
        for (const [index, id] of ids.entries()) {
          const state = states[sources[index]];
          if (id === 'main') restoreState(state);
          else paneRefs[id]?.restoreState(state);
        }
        activePaneId = ids[sources.indexOf(current)];
        await tick();
      } finally { workspaceTransition = false; persistWorkspace(); }
    };
    if(command.kind==='rotate-panes') {
      await exchangeContents(command.direction===1?[ids.at(-1)!,...ids.slice(0,-1)]:[...ids.slice(1),ids[0]]);return;
    }
    if(command.kind==='exchange-pane') {
      const target=command.target?ids[command.target-1]:ids[(ids.indexOf(current)+1)%ids.length];
      if(!target)throw new Error('That pane number is not open.');
      await exchangeContents(ids.map(id=>id===current?target:id===target?current:id));return;
    }
    if(command.kind==='move-pane') {
      if(ids.length<2)return;
      const prune=(node:PaneLayout):PaneLayout|null=>{if(!('axis'in node))return node.id===current?null:node;const first=prune(node.first),second=prune(node.second);return first&&second?{...node,first,second}:first??second;};
      const rest=prune(layout)!;const leading=command.edge==='left'||command.edge==='top';
      await restoreLayout({id:crypto.randomUUID(),axis:command.edge==='left'||command.edge==='right'?'horizontal':'vertical',ratio:.5,first:leading?{id:current}:rest,second:leading?rest:{id:current}});return;
    }
    if(command.kind==='resize-pane') {
      let nearest:{node:Extract<PaneLayout,{axis:string}>;first:boolean}|undefined;
      const search=(node:PaneLayout)=>{if(!('axis'in node))return;const first=paneIds(node.first).includes(current),second=paneIds(node.second).includes(current);if(!first&&!second)return;if(node.axis===command.axis)nearest={node,first};search(first?node.first:node.second);};search(layout);
      if(!nearest)return;
      const match=nearest as {node:Extract<PaneLayout,{axis:string}>;first:boolean};
      const box=document.querySelector<HTMLElement>(`.pane-split[data-split-id="${CSS.escape(match.node.id)}"]`)?.getBoundingClientRect();
      if(!box)return;
      const extent=Math.max(1,(command.axis==='horizontal'?box.width:box.height)-1);
      const font=parseFloat(getComputedStyle(document.documentElement).fontSize)||16;
      const unit=command.axis==='horizontal'?font*.6:font*1.5;
      const currentFraction=match.first?match.node.ratio:1-match.node.ratio;
      const requested=command.maximize ? .85:command.size!==undefined?command.size*unit/extent:currentFraction+(command.delta??0)*unit/extent;
      const fraction=Math.max(.15,Math.min(.85,requested));
      resizeSplit(match.node.id,match.first?fraction:1-fraction);persistWorkspace();return;
    }
  }

  async function executeVimCommand(command: VimCommand) {
    if (command.kind === 'help') { vimCommandError = ''; vimHelpOpen = true; return; }
    if (command.kind === 'tabnew') {
      const tabs=orderedTabs(),current=currentVimTab(),currentIndex=current?tabs.findIndex(t=>t.kind===current.kind&&t.id===current.id):-1;
      let before:TabKey|undefined;
      if(command.after){const target=command.after;const at=target.kind==='last'?tabs.length-1:target.kind==='index'?target.index-1:currentIndex+target.offset;if(at<0||at>=tabs.length){vimCommandError='That tab target does not exist.';return;}before=tabs[at+1];}
      openTaskComposer();if(currentDraftId&&command.after)reorderTab({sourcePaneId:paneId,kind:'draft',id:currentDraftId},before);
    }
    else if (command.kind === 'tabselect') selectVimTab(command.target);
    else if (command.kind === 'tabclose') { if(command.target&&!selectVimTab(command.target)){vimCommandError='That tab target does not exist.';return;} closeActiveTab(); }
    else if (command.kind === 'tabonly') {
      if(command.target&&!selectVimTab(command.target)){vimCommandError='That tab target does not exist.';return;}
      if(hasPending()){vimCommandError='Wait for the current action before closing tabs.';return;}
      const keep=currentVimTab();
      for(const tab of [...orderedTabs()]){
        if(keep&&tab.kind===keep.kind&&tab.id===keep.id)continue;
        if(tab.kind==='terminal')await closeTerminalTab(tab.id);
        else if(tab.kind==='task')closeTaskTab(tab.id);
        else if(tab.kind==='draft')closeTaskDraft(tab.id);
        else if(tab.kind==='channel'){openChannelIds=openChannelIds.filter(id=>id!==tab.id);forgetTab(tab);}
        else closeSettings();
      }
      if(keep)overviewOpen=false;
    }
    else if (command.kind === 'tabmove') { const current=currentVimTab(); if(current){const ordered=orderedTabs(), currentIndex=ordered.findIndex(tab=>tab.kind===current.kind&&tab.id===current.id),tabs=ordered.filter(tab=>tab.kind!==current.kind||tab.id!==current.id);let before:TabKey|undefined; if(command.target==='first')before=tabs[0]; else if(command.target==='last')before=undefined; else if(command.target.kind==='after')before=tabs[command.target.index]; else {const index=Math.max(0,Math.min(tabs.length,currentIndex+command.target.offset));before=tabs[index];} reorderTab({sourcePaneId:paneId,...current},before); } }
    else if (command.kind === 'terminal') await newTerminal();
    else if (command.kind === 'split' || command.kind === 'close-pane' || command.kind === 'only-pane' || command.kind === 'focus-pane' || command.kind === 'resize-pane' || command.kind === 'equalize-panes' || command.kind === 'rotate-panes' || command.kind === 'exchange-pane' || command.kind === 'move-pane') { await executeWorkspaceVim(command); }
    closeVimCommand();
  }
  function submitVimCommand() {
    const result = parseVimCommand(vimCommandText);
    if ('error' in result) { vimCommandError = result.error; return; }
    void executeVimCommand(result.command).catch(reason=>{vimCommandError=text(reason);});
  }
  function handleVimCommandKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeVimCommand(); }
    else if (event.key === 'Tab') {
      event.preventDefault();event.stopPropagation();
      if(vimCommandText!==vimCompletionValue){vimCompletionSeed=vimCommandText;vimCompletionIndex=-1;}
      const choices=completeVimCommand(vimCompletionSeed);
      if(choices.length){vimCompletionIndex=(vimCompletionIndex<0?(event.shiftKey?choices.length-1:0):(vimCompletionIndex+(event.shiftKey?-1:1)+choices.length)%choices.length);vimCommandText=choices[vimCompletionIndex].value;vimCompletionValue=vimCommandText;}
    }
    else if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); submitVimCommand(); }
  }

  function isSendKey(event: KeyboardEvent) {
    return event.key === "Enter" && !event.isComposing && !event.shiftKey && !event.altKey &&
      (snapshot?.settings.sendWithEnter || event.metaKey || event.ctrlKey);
  }
  async function send() {
    if (busy || handleSlashSubmit()) return;
    if (currentDraftId) { await createTask(); return; }
    if (!selectedTask || !canSend || busy) return;
    const taskId = selectedTask.id;
    const sentDraft = composer;
    const key = `task:${taskId}`, attachmentIds=currentAttachments.map(item=>item.id);
    setComposerPending(key, true);
    try { const result = await run(() => bridge.sendMessage(taskId, operatorPrompt(taskId, promptText(sentDraft)),attachmentIds)); if (result) {clearSentDraft(key, sentDraft);clearAttachments(key,attachmentIds);}  }
    finally { setComposerPending(key, false); }
  }
  function clearSentDraft(key: string, sentDraft: string) {
    if (currentDraftKey() === key) scrollRevision += 1;
    if (currentDraftKey() === key && composer === sentDraft) composer = "";
    if (drafts[key] === sentDraft) { delete drafts[key]; publishComposer(key, ''); }
  }
  async function browseTaskFolder() {
    if (!taskFormAgent || snapshot?.hosts.find(host => host.id === taskFormAgent.hostId)?.kind !== 'local') return;
    try {
      const folder = await bridge.chooseLocalFolder(taskCwd || inheritedTaskCwd);
      if (folder) taskCwd = folder;
    } catch (reason) { error = `Could not choose folder: ${text(reason)}`; }
  }
  async function createTask() {
    if (busy) return;
    const draftId = currentDraftId;
    const draft = draftId ? taskDrafts[draftId] : null;
    if (!draftId || !draft || !canSend || !taskAgentId) { error = "Choose an agent and write a message."; return; }
    const textToSend = promptText(composer), attachmentIds=currentAttachments.map(item=>item.id);
    const captured = { text: composer, title: taskTitle, agentId: taskAgentId, projectId: taskProjectId, parentId: taskParentId, nativeSessionId: taskNativeSessionId, cwd: taskCwd };
    const values = { modelSettings:draftModelSettings, sandbox:draftSandbox, agentId: captured.agentId, title: captured.title.trim() || textToSend.slice(0, 72) || 'New chat', nativeSessionId: captured.nativeSessionId.trim() || null, parentTaskId: captured.parentId, channelId: null, projectId: captured.projectId || null, cwd: captured.projectId ? null : captured.cwd.trim() || null };
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
      tabOrder = tabOrder.map(tab=>tab.kind==='draft' && tab.id===draftId?{kind:'task' as const,id:taskId!}:tab);
      if (currentDraftId === draftId) {
        currentDraftId = null;
        const created = snapshot?.tasks.find(task => task.id === taskId);
        if (created) openTask(created);
      }
    } catch (reason) { error = text(reason); }
    finally { busy = false; setComposerPending(`draft:${draftId}`, false); }
  }

  let agentEdits=$state<Record<string,Agent>>({});
  function selectAgentEditor(id:string) {
    if(agentDraft)agentEdits[agentDraft.id]=JSON.parse(JSON.stringify(agentDraft));
    agentDraft=JSON.parse(JSON.stringify(agentEdits[id]??snapshot?.agents.find(agent=>agent.id===id)??blankAgent()));
  }
  export function openAgentSettings(draft:Agent) {
    if(agentDraft)agentEdits[agentDraft.id]=JSON.parse(JSON.stringify(agentDraft));
    agentDraft=JSON.parse(JSON.stringify(agentEdits[draft.id]??draft));
    openSettings();settingsCategory='agents';
  }
  function routeAgentSettings(draft:Agent) {
    modal=null;
    if(embedded){onAgentSettingsSelect?.(draft);return;}
    const owner=paneIds(layout).find(id=>(id==='main'?allTabs():paneRefs[id]?.allTabs()??[]).some(tab=>tab.kind==='settings'))??activePaneId;
    activePaneId=owner;
    if(owner==='main')openAgentSettings(draft);else paneRefs[owner]?.openAgentSettings(draft);
  }
  $effect(()=>{if(settingsOpen && settingsCategory==='agents' && !agentDraft && snapshot)untrack(()=>selectAgentEditor(snapshot!.agents[0]?.id??''));});
  function discardAgentEdits(){if(!agentDraft)return;delete agentEdits[agentDraft.id];agentDraft=JSON.parse(JSON.stringify(snapshot?.agents.find(agent=>agent.id===agentDraft?.id)??blankAgent()));}
  async function deleteEditedAgent(){if(!agentDraft?.id)return;const id=agentDraft.id;if(await run(()=>bridge.deleteAgent(id),'Agent removed.')){delete agentEdits[id];agentDraft=null;selectAgentEditor(snapshot?.agents[0]?.id??'');}}
  async function saveAgent() {
    if(!agentDraft)return;
    const oldId=agentDraft.id;
    const submitted={...agentDraft,id:oldId||crypto.randomUUID(),expertise:(agentDraft.expertise??[]).map(value=>value.trim()).filter(Boolean),responsibilities:(agentDraft.responsibilities??[]).map(value=>value.trim()).filter(Boolean),skills:(agentDraft.skills??[]).map(value=>value.trim()).filter(Boolean)};
    if(await run(()=>bridge.saveAgent(submitted),'Agent saved.')){delete agentEdits[oldId];agentDraft=JSON.parse(JSON.stringify(snapshot?.agents.find(agent=>agent.id===submitted.id)??submitted));}
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
      id: project?.id ?? '', name: project?.name ?? '', description: project?.description ?? '', icon: project?.icon ?? 'folder', color: project?.color ?? '#3f9d6a',
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
    if (view === 'activity' && !await switchWorkspace('all')) return;
    await run(()=>saveSettingsPatch({sidebarView:view}));
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
  async function autonameCurrentPane() {
    if (busy || terminalBusy) return;
    const terminalId = pane === 'terminal' ? selectedTerminal?.id ?? null : null;
    const target = pane === 'task' && selectedTask ? { taskId: selectedTask.id }
      : pane === 'channel' && activeChannel ? { channelId: activeChannel.id }
      : pane === 'terminal' && selectedTerminal ? { terminalId: selectedTerminal.id, content: recentTerminalOutput(selectedTerminal.id) }
      : null;
    if (!target) { error = 'Open a chat, channel, or terminal with recent content to auto-name it.'; return; }
    const namingKey = 'taskId' in target ? `task:${target.taskId}` : 'channelId' in target ? `channel:${target.channelId}` : `terminal:${target.terminalId}`;
    if ($autonaming[namingKey]) return;
    autonaming.update(value=>({...value,[namingKey]:true}));
    try {
    const result = await run(() => bridge.autoname(target), 'Title updated.');
    if (result && terminalId) {
      const refreshed = (await bridge.listTerminals()).find(session => session.id === terminalId);
      if (refreshed) registerTerminal(refreshed);
    }
    } finally { autonaming.update(value=>{const next={...value};delete next[namingKey];return next;}); }
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
    ...(pane === "channel" ? channelCommands : []),
    { id: "new", label: "/new", detail: "Open a local New chat draft" },
    { id: "settings", label: "/settings", detail: "Open Monitter preferences" },
    ...(pane === "task" ? [{ id: "project", label: "/project", detail: "Choose the project for this chat" }] : []),
    ...((pane === "task" && selectedTask) || (pane === "channel" && activeChannel) ? [{ id: "autoname", label: "/autoname", detail: "Generate a title from recent content" }] : []),
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
    const channelCommand = pane === 'channel' ? parseChannelCommand(composer) : null;
    if(channelCommand) { void executeChannelCommand(channelCommand.name,channelCommand.args,composer); return true; }
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
    if(item.id.startsWith('channel:')) {
      const name=item.id.slice(8);
      if(['invite','kick','topic'].includes(name)) { composer=`/${name} `; slashOpen=false; return; }
      await executeChannelCommand(name,'',composer); return;
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
    else if (item.id === "settings") routeSettings();
    else if (item.id === "project") {
      if (currentTaskDraft) document.getElementById("task-project")?.focus();
      else if (task) { renameTitle = task.title; modal = "taskSettings"; }
    } else if (item.id === "autoname") await autonameCurrentPane();
    else if (item.id === "stop" && task) await run(()=>bridge.cancelTask(task.id), "Stopping task…");
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
    const typedChannelCommand = pane==='channel' ? parseChannelCommand(composer) : null;
    if(event.key==='Enter' && !event.shiftKey && typedChannelCommand && (typedChannelCommand.args || !['invite','kick'].includes(typedChannelCommand.name))) { event.preventDefault(); handleSlashSubmit(); return; }
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
        const result = await saveSettingsPatch({interfaceScale: target});
        if (isSnapshot(result)) applySnapshot(result, ++snapshotIssued);
      }
    } catch (reason) { error = text(reason); }
    finally { scaleInFlight = null; scaleSaving = false; }
  }
  function dismissMonitterMenu(event: PointerEvent) {
    if (!(event.target instanceof Element) || !event.target.closest('.task-overflow')) taskMenu = false;
    if (!(event.target instanceof Element) || !event.target.closest('.agent-rail, .rail-chats')) railAgentId = null;
  }
  function focusAdjacentPane(direction: 'left' | 'right' | 'up' | 'down') {
    if (embedded) return false;
    const current = document.querySelector<HTMLElement>(`.pane-leaf[data-pane-id="${CSS.escape(activePaneId)}"]`);
    if (!current) return false;
    const origin = current.getBoundingClientRect(), horizontal = direction === 'left' || direction === 'right';
    const originCenter = horizontal ? origin.top + origin.height / 2 : origin.left + origin.width / 2;
    const candidates = [...document.querySelectorAll<HTMLElement>('.pane-leaf[data-pane-id]')]
      .filter(node => node !== current)
      .map(node => ({ node, rect: node.getBoundingClientRect() }))
      .filter(({ rect }) => direction === 'left' ? rect.right <= origin.left + 1 : direction === 'right' ? rect.left >= origin.right - 1 : direction === 'up' ? rect.bottom <= origin.top + 1 : rect.top >= origin.bottom - 1)
      .sort((a, b) => {
        const distanceA = horizontal ? Math.abs((direction === 'left' ? origin.left - a.rect.right : a.rect.left - origin.right)) : Math.abs((direction === 'up' ? origin.top - a.rect.bottom : a.rect.top - origin.bottom));
        const distanceB = horizontal ? Math.abs((direction === 'left' ? origin.left - b.rect.right : b.rect.left - origin.right)) : Math.abs((direction === 'up' ? origin.top - b.rect.bottom : b.rect.top - origin.bottom));
        const offsetA = Math.abs((horizontal ? a.rect.top + a.rect.height / 2 : a.rect.left + a.rect.width / 2) - originCenter);
        const offsetB = Math.abs((horizontal ? b.rect.top + b.rect.height / 2 : b.rect.left + b.rect.width / 2) - originCenter);
        return distanceA - distanceB || offsetA - offsetB;
      });
    const next = candidates[0]?.node;
    if (!next) return false;
    activePaneId = next.dataset.paneId!;
    const target = next.querySelector<HTMLElement>('.terminal-pane .xterm-helper-textarea')
      ?? next.querySelector<HTMLElement>('textarea[aria-label="Task message"], textarea[aria-label="Channel message"]')
      ?? next.querySelector<HTMLElement>('.messages') ?? next;
    target.focus({ preventScroll: true });
    return true;
  }
  $effect(() => {
    if (embedded || !isTauri()) return;
    const enabled = vimShortcuts;
    let disposed=false, stop:UnlistenFn|undefined;
    const syncShield=()=>{void invoke('set_native_escape_shield',{enabled:enabled && !(document.activeElement instanceof Element && document.activeElement.closest('.terminal-pane'))}).catch(reason=>{error=`Could not apply keyboard mode: ${text(reason)}`;});};
    document.addEventListener('focusin',syncShield);
    syncShield();
    void listen('monitter-native-escape',()=>{
      if(!enabled||disposed)return;
      const target=document.activeElement ?? window;
      target.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',code:'Escape',bubbles:true,cancelable:true}));
    }).then(unlisten=>{if(disposed)unlisten();else stop=unlisten;});
    return()=>{disposed=true;stop?.();document.removeEventListener('focusin',syncShield);void invoke('set_native_escape_shield',{enabled:false}).catch(()=>{});};
  });
  function cancelPaneFocusChord() { paneFocusChord=false; }
  function closeFocusedTab() {
    const target = activePaneId === 'main' ? { closeActiveTab } : paneRefs[activePaneId];
    target?.closeActiveTab();
  }
  function openFocusedTaskComposer() {
    if (activePaneId === 'main') openTaskComposer();
    else paneRefs[activePaneId]?.openTaskComposer();
  }
  function openFocusedEmptyTab() {
    if (activePaneId === 'main') openEmptyTab();
    else paneRefs[activePaneId]?.openEmptyTab();
  }
  function openFocusedTerminal() {
    if (activePaneId === 'main') void newTerminal();
    else void paneRefs[activePaneId]?.newTerminal();
  }
  function swapFocusedTab(direction: 1 | -1) {
    if (activePaneId === 'main') swapActiveTab(direction);
    else paneRefs[activePaneId]?.swapActiveTab(direction);
  }
  function toggleFocusedDetail() {
    if (activePaneId === 'main') toggleDetail();
    else paneRefs[activePaneId]?.toggleDetail();
  }
  function handleShortcuts(event: KeyboardEvent) {
    tabIndexModifier = macPlatform ? event.metaKey : event.ctrlKey;
    if (!embedded && tabIndexModifier && !event.altKey && !event.shiftKey && !event.isComposing && /^[1-9]$/.test(event.key) && !document.querySelector('[role="dialog"]')) {
      event.preventDefault();
      const buttons=document.querySelectorAll<HTMLButtonElement>(`.pane-leaf[data-pane-id="${CSS.escape(activePaneId)}"] .tabs > .tab-entry > button.tab`);
      buttons[Number(event.key)-1]?.click();
      return;
    }
    const inTerminal=event.target instanceof Element && !!event.target.closest('.terminal-pane');
    const commandModifier = macPlatform ? event.metaKey : event.ctrlKey;
    if (!embedded && !vimShortcuts && commandModifier && !event.altKey && !event.isComposing && !modal && !palette && !taskMenu && !railAgentId) {
      const key = event.key.toLowerCase();
      if (event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
        event.preventDefault();
        swapFocusedTab(event.key === 'ArrowLeft' ? -1 : 1);
        return;
      }
      if (key === 't') {
        event.preventDefault();
        if (event.shiftKey) openFocusedTerminal(); else openFocusedEmptyTab();
        return;
      }
      if (event.shiftKey && key === 'c') {
        event.preventDefault(); openFocusedTaskComposer(); return;
      }
      if (key === 'b') {
        event.preventDefault();
        if (event.shiftKey) toggleFocusedDetail();
        else sidebarCollapsed = !sidebarCollapsed;
        return;
      }
    }
    if (!embedded && !modal && !palette && !taskMenu && !railAgentId && !vimCommandOpen) {
      if (paneFocusChord) {
        cancelPaneFocusChord(); event.preventDefault(); event.stopPropagation();
        if(event.key==='Escape')return;
        const aliases:Record<string,string>={ArrowLeft:'h',ArrowRight:'l',ArrowUp:'k',ArrowDown:'j'};
        const parsed=parseVimWindowKey(aliases[event.key]??event.key);
        if('error'in parsed){notice=parsed.error;return;}
        void executeWorkspaceVim(parsed.command).catch(reason=>{error=text(reason);});return;
      }
      if (!(inTerminal && event.ctrlKey && !event.metaKey) && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && !event.isComposing && event.key.toLowerCase() === 'w') {
        event.preventDefault();
        if (vimShortcuts) paneFocusChord=true;
        else closeFocusedTab();
        return;
      }
    }
    if(embedded ? !active : activePaneId !== 'main' && !modal && !palette) return;
    if (vimCommandOpen) return;
    if(inTerminal && event.ctrlKey && !event.metaKey) {
      if(event.shiftKey && ['p','k'].includes(event.key.toLowerCase())) {event.preventDefault();palette=event.key.toLowerCase()==='p'?'controls':'switch';return;}
      if(!event.shiftKey)return;
    }
    if(!vimShortcuts && event.key==='Escape' && compactDetail && showDetail && !modal && !palette && !taskMenu && !railAgentId) {event.preventDefault();showDetail=false;return;}
    if (event.key === 'Escape' && (taskMenu || railAgentId)) { event.preventDefault(); taskMenu = false; railAgentId = null; return; }
    if (vimShortcuts && !inTerminal && !modal && !palette && !taskMenu && !railAgentId) {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); vimArmed = true; return; }
      if (vimArmed && event.key === ':' && !event.metaKey && !event.ctrlKey && !event.altKey) { event.preventDefault(); openVimCommand(); return; }
      if (!event.metaKey && !event.ctrlKey && !event.altKey && event.key.length === 1) vimArmed = false;
    }

    if ((event.metaKey || event.ctrlKey) && event.key === ',' && !event.altKey && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      taskMenu = false; railAgentId = null;
      palette = null; routeSettings();
      return;
    }

    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && !event.isComposing && (event.key==='0' || event.code==='Numpad0')) {
      event.preventDefault();scaleQueued=125;void flushScale();return;
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
    {id:"settings:settings",label:"Settings",group:"Workspace",detail:"Appearance, typography and behaviour"},
    ...Object.values($terminalSessions).map(session=>({id:`terminal:${session.id}`,label:session.title,group:'Terminals',detail:`${snapshot?.hosts.find(host=>host.id===session.hostId)?.name??'Host'} · ${session.cwd}`})),
    ...Object.values(taskDrafts).map(draft => ({id:`draft:${draft.id}`,label:draft.title || 'New chat',group:'Draft chats',detail:snapshot?.agents.find(agent=>agent.id===draft.agentId)?.name ?? 'Agent'})),
    ...(snapshot?.tasks ?? []).filter(task=>!task.channelId).toSorted((a,b)=>b.updatedAt-a.updatedAt).map(task=>({
      id:`task:${task.id}`,label:task.title,group:task.archived ? "Archived chats" : "Chats",
      detail:`${task.archived ? "Select to restore · " : ""}${snapshot?.agents.find(agent=>agent.id===task.agentId)?.name ?? "Agent"}`,
      keywords:`${task.provider} ${task.cwd}`,
    })),
    ...(snapshot?.channels ?? []).map(channel=>({id:`channel:${channel.id}`,label:channel.name,group:"Channels",detail:channel.description})),
    ...(snapshot?.agents ?? []).map(agent=>({id:`agent:${agent.id}`,label:agent.name,group:"Agents",detail:`${agent.provider} · ${agent.description}`})),
    ...projects.map(project=>({id:`project:${project.id}`,label:project.name,group:'Projects',detail:project.description || `${activityTasks.filter(task=>task.projectId===project.id).length} chats`})),
  ]);
  const controlItems = $derived([
    ...([{id:'single',label:'One pane'},{id:'columns',label:'Two columns'},{id:'grid',label:'2 × 2 grid'}] as const).map(item=>({id:`layout:${item.id}`,label:item.label,group:'Layout'})),
    {id:"new-task",label:"New chat",group:"Create"},
    {id:"vim-command",label:"Vim command",detail:"Open : command mode for workspace controls",keywords:"vim ex command tabnew split terminal quit",group:"Workspace"},
    {id:"new-terminal",label:"New terminal",detail:"Open a shell in this host and folder",group:"Create",disabled:terminalBusy},
    ...(selectedTask || activeChannel || selectedTerminal ? [{id:"autoname",label:"Auto-name current pane",detail:"Generate a title from recent visible content",keywords:"/autoname rename title",group:"Current pane",disabled:terminalBusy}] : []),
    {id:"new-agent",label:"New agent",group:"Create"},
    {id:"agent-directory",label:"Agent directory",detail:"Find agents by expertise, responsibility, or skill",group:"Collaborate"},
    {id:"new-channel",label:"New channel",group:"Create"},
    {id:"new-project",label:"New project",group:"Create"},
    ...(['standard','activity','projects'] as SidebarView[]).map(view=>({id:`sidebar:${view}`,label:`${view[0].toUpperCase()+view.slice(1)} sidebar view`,group:'Sidebar',checked:sidebarView===view})),
    {id:"appearance",label:"Settings",detail:"Accent colour, scale, theme and conversation settings",group:"Settings"},
    {id:"hosts",label:"Manage hosts",detail:"Local and SSH connections",group:"Settings"},
    {id:"archived",label:"Archived chats",detail:"Restore or permanently delete archived chats",group:"Workspace"},
    {id:"tools",label:"Show tool activity",checked:snapshot?.settings.showToolActivity !== false,group:"Toggles"},
    {id:"reasoning",label:"Show reasoning summaries",checked:snapshot?.settings.showReasoningSummaries !== false,group:"Toggles"},
    {id:"steer-busy",label:"Steer busy agents when supported",checked:snapshot?.settings.busyMessageMode==='steer',group:"Toggles"},
    {id:"focus-mouse",label:"Focus follows mouse",checked:snapshot?.settings.focusFollowsMouse ?? false,group:"Toggles"},
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
      if (kind === "settings") { routeSettings();
      } else if (kind === "terminal") { routeTerminal(itemId);
      } else if (kind === "draft") {
        const draft = taskDrafts[itemId]; if (draft) openTaskDraft(draft);
      } else if (kind === "task") {
        const task = snapshot?.tasks.find(task=>task.id===itemId);
        if (task?.archived) await archiveTask(task,false); else if (task) routeTaskWorkspace(task);
      } else if (kind === "channel") {
        const channel = snapshot?.channels.find(channel=>channel.id===itemId);
        if (channel) { if (activeWorkspaceKey !== 'all') { if (await switchWorkspace('all')) routeChannel(channel); } else routeChannel(channel); }
      } else if (kind === 'project') {
        const project = projects.find(project=>project.id===itemId);
        if (project) { const scope=`project:${project.id}` as WorkspaceKey; if(scope===activeWorkspaceKey)openProject(project);else await switchWorkspace(scope); }
      } else {
        const agent = snapshot?.agents.find(agent=>agent.id===itemId);
        if (agent) { const scope=`agent:${agent.id}` as WorkspaceKey; if(scope===activeWorkspaceKey)openAgent(agent);else await switchWorkspace(scope); }
      }
      return;
    }
    const settings = snapshot?.settings;
    if (!settings || busy) return;
    if (id === "vim-command") { palette=null; openVimCommand(); }
    else if (id === "new-terminal") {palette=null;await newTerminal();}
    else if (id === "autoname") { palette=null; await autonameCurrentPane(); }
    else if (id.startsWith('layout:')) {palette=null;setLayout(id.slice(7) as 'single'|'columns'|'grid');}
    else if (id === "tools") await run(()=>saveSettingsPatch({showToolActivity:!settings.showToolActivity}));
    else if (id === "reasoning") await run(()=>saveSettingsPatch({showReasoningSummaries:!settings.showReasoningSummaries}));
    else if (id === "steer-busy") await run(()=>saveSettingsPatch({busyMessageMode:settings.busyMessageMode==='steer'?'queue':'steer'}));
    else if (id === "focus-mouse") await run(()=>saveSettingsPatch({focusFollowsMouse:!settings.focusFollowsMouse}));
    else if (id === "dim-panes") await run(()=>saveSettingsPatch({dimInactivePanes:!(settings.dimInactivePanes ?? true)}));
    else if (id === "enter") await run(()=>saveSettingsPatch({sendWithEnter:!settings.sendWithEnter}));
    else if (id === "detail") showDetail = !showDetail;
    else if (id.startsWith("scale-")) { if (id === "scale-reset") { scaleQueued = 125; void flushScale(); } else queueScale(id === "scale-up" ? 5 : -5); }
    else if (id.startsWith("theme:")) await run(()=>saveSettingsPatch({theme:id.slice(6) as "light"|"dark"|"system"}));
    else if (id.startsWith('sidebar:')) await setSidebarView(id.slice(8) as SidebarView);
    else {
      palette = null;
      if (id === "new-task") openTaskComposer();
      if (id === "new-agent") { routeAgentSettings(blankAgent()); }
      if (id === "agent-directory") { directoryQuery='';routeSettings('directory'); }
      if (id === "new-channel") { channelDraft=blankChannel(); modal="channel"; }
      if (id === 'new-project') editProject();
      if (id === "hosts") modal=id;
      if (id === "appearance") routeSettings();
      if (id === "archive" && selectedTask) await archiveTask(selectedTask);
      if (id === "archived") modal = 'archived';
      if (id === "stop" && selectedTask) await run(()=>bridge.cancelTask(selectedTask.id));
    }
  }
  function editActiveChannel() {
    if(!activeChannel)return;
    taskMenu=false;channelDraft={...activeChannel,agentIds:[...activeChannel.agentIds],messages:activeChannel.messages};modal='channel';
  }
  async function changeChannelMembership(agentId:string,member:boolean) {
    const channel=activeChannel;if(!channel)return false;
    const agent=snapshot?.agents.find(agent=>agent.id===agentId);
    const result=await run(()=>bridge.setChannelMembership(channel.id,agentId,member),`${agent?.name??'Agent'} ${member?'joined':'left'} ${channel.name}.`);
    if(result && !member) { channelRecipients[channel.id]=(channelRecipients[channel.id]??[]).filter(id=>id!==agentId);if(selectedChannelId===channel.id)recipients=recipients.filter(id=>id!==agentId); }
    return !!result;
  }
  async function executeChannelCommand(name:string,args:string,original:string) {
    const channel=activeChannel,key=currentDraftKey();if(!channel||!key||busy)return;
    let success=true;
    error="";notice="";
    try {
      if(['members','names','help','admin'].includes(name) && args)throw Error(`/${name} does not take arguments.`);
      if(name==='members'||name==='names') {showDetail=true;notice=`Members: ${channelMentionAgents.map(agent=>agent.name).join(', ')||'None'}`;}
      else if(name==='help') {showDetail=true;notice='Channel commands: /members, /invite @name, /kick @name, /topic text, /admin. Use // to send a literal slash message.';}
      else if(name==='admin')editActiveChannel();
      else if(name==='topic') {if(!args)notice=`Topic: ${channel.description||'No topic set'}`;else success=!!await run(()=>bridge.saveChannel({...channel,description:args==='-'?'':args}),'Channel topic updated.');}
      else if(name==='invite'||name==='kick') {
        if(!args)throw Error(`Usage: /${name} @agent-name`);
        const agent=resolveChannelAgent(args,name==='kick'?channelMentionAgents:snapshot?.agents??[]);
        success=await changeChannelMembership(agent.id,name==='invite');
      }
      if(success) {clearSentDraft(key,original);slashOpen=false;}
    }catch(reason){error=text(reason);}
  }
  function toggleRecipient(id: string) {
    recipients = recipients.includes(id)
      ? recipients.filter((entry) => entry !== id)
      : [...recipients, id];
  }
  async function stopChannel() {
    if(activeChannel) await run(()=>bridge.stopChannelAgentConversation(activeChannel.id), "Channel stopped.");
  }
  async function configureChannelConversation(enabled:boolean,turnLimit:number) { if(activeChannel) await run(()=>bridge.setChannelAgentConversation(activeChannel.id,enabled,turnLimit)); }
  const channelMentionAgents = $derived(snapshot?.agents.filter(agent=>activeChannel?.agentIds.includes(agent.id)) ?? []);
  const channelMentionIds = $derived(mentionedAgentIds(composer, channelMentionAgents));
  const effectiveRecipients = $derived([...new Set([...recipients, ...channelMentionIds])].filter(id=>activeChannel?.agentIds.includes(id)));
  const currentQueuedMessages = $derived((snapshot?.queuedMessages??[]).filter(message=>pane==='channel'?message.channelId===selectedChannelId:message.taskId===selectedTaskId));
  async function editQueuedMessage(id:string,text:string) { return Boolean(await run(()=>bridge.editQueuedMessage(id,text))); }
  async function removeQueuedMessage(id:string) { await run(()=>bridge.cancelQueuedMessage(id)); }
  async function sendChannel() {
    if (busy || handleSlashSubmit()) return;
    if (!activeChannel || !canSend || !effectiveRecipients.length) { error = "Choose at least one agent to receive this channel message."; return; }
    const channelId = activeChannel.id, sentDraft = composer, agentIds = [...effectiveRecipients], key = `channel:${channelId}`, attachmentIds=currentAttachments.map(item=>item.id);
    setComposerPending(key, true);
    try { if (await run(() => bridge.sendChannelMessage(channelId, promptText(sentDraft), agentIds,attachmentIds))) {clearSentDraft(key, sentDraft);clearAttachments(key,attachmentIds);}  }
    finally { setComposerPending(key, false); }
  }
</script>

<svelte:head><meta name="theme-color" content="#e9e3d8" /></svelte:head>
<svelte:window onkeydown={handleShortcuts} onkeyup={event=>tabIndexModifier=macPlatform?event.metaKey:event.ctrlKey} onblur={()=>{tabIndexModifier=false;cancelPaneFocusChord()}} onpointerdown={dismissMonitterMenu} />

{#if vimCommandOpen}
  <div class="vim-commandbar" role="dialog" aria-label="Monitter Vim command">
    <form onsubmit={event=>{event.preventDefault();submitVimCommand();}}>
      <label><span aria-hidden="true">:</span><input aria-label="Monitter command" bind:this={vimCommandInput} bind:value={vimCommandText} onkeydown={handleVimCommandKeydown} autocomplete="off" spellcheck="false" /></label>
    </form>
    {#if vimCommandError}<p role="alert">{vimCommandError}</p>{/if}
    {#if vimHelpOpen}<ul aria-label="Available Monitter commands">{#each vimCommandHelp as item}<li>{item}</li>{/each}</ul>{/if}
  </div>
{/if}

{#snippet slashMenu()}
  {#if slashOpen && isSlashCommand(composer)}
    <div class="slash-menu" use:slashFloating role="menu" aria-label="Monitter commands"><div class="slash-options">
      <p class="slash-caption">Monitter commands</p>
      {#each slashVisibleItems as item, index}
        <button type="button" role="menuitem" class:active={index===slashIndex} disabled={busy} onclick={()=>selectSlash(item)}><b>{item.label}</b><span>{item.detail}</span></button>
      {:else}
        <p>Use the native terminal for harness commands. Start with // to send a literal slash message.</p>
      {/each}
    </div></div>
  {/if}
{/snippet}

{#snippet sidebarChat(task: Task, detail = false)}
  {@const sortGroup=sidebarView==='activity'?'':sidebarView==='projects'?`project-chats:${task.projectId??'unassigned'}`:`agent-chats:${task.agentId}`}
  {@const agent = snapshot?.agents.find(item=>item.id===task.agentId)}
  <div use:sidebarReorder={{group:sortGroup,id:task.id,move:moveSidebar}} class="task-row" class:current={task.id === (activePaneId==='main'?selectedTaskId:paneSelections[activePaneId])} data-task-id={task.id}>
    <button class="task-select" onclick={() => {
      const target: WorkspaceKey = sidebarView === 'standard' ? `agent:${task.agentId}` : sidebarView === 'projects' ? `project:${task.projectId ?? 'unassigned'}` : activeWorkspaceKey;
      if (target !== activeWorkspaceKey) void switchWorkspace(target).then(changed => { if (changed) routeTask(task); }); else routeTask(task);
    }} title={task.title}>
      {#if detail}<span class="avatar small" title={agent?.name ?? 'Agent'} aria-label={agent?.name ?? 'Agent'}>{@render avatarVisual(agent, 12)}</span>{/if}
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
  <div class="attachment-tools"><button class="icon" aria-label="Attach files" title="Attach files, or drop or paste them here" disabled={busy || filesBusy} onclick={()=>filePicker?.click()}>{#if filesBusy}<LoaderCircle class="spin" size={16}/>{:else}<Paperclip size={16}/>{/if}</button>{#if filesBusy}<small role="status">Saving attachments…</small>{/if}</div>
  <input class="attachment-input" bind:this={filePicker} type="file" multiple aria-label="Choose attachments" onchange={event=>{const files=Array.from(event.currentTarget.files??[]);event.currentTarget.value='';void attachFiles(files)}}/>
{/snippet}

{#snippet avatarVisual(agent: Agent | null | undefined, size = 13)}
  {#if avatarSrc(agent)}
    <img src={avatarSrc(agent)!} alt="" />
  {:else}
    {@const Icon = agentAvatarIcon(agent)}
    <Icon {size} strokeWidth={1.8} aria-hidden="true" />
  {/if}
{/snippet}

{#snippet messageAvatar(agent: Agent | null | undefined)}
  {#if agent}<span class="avatar message-avatar" title={agent.name}>{@render avatarVisual(agent, 12)}</span>{/if}
{/snippet}

{#snippet agentWaiting(agent: Agent | null | undefined, starting = false)}
  <div class="agent-waiting" role="status" aria-live="polite" aria-label={`${agent?.name ?? 'Agent'} ${starting ? 'is getting ready…' : 'is pondering…'}`}>
    {@render messageAvatar(agent)}
    <span class="waiting-spinner" aria-hidden="true"><LoaderCircle size={14}/></span>
  </div>
{/snippet}

{#snippet paneExpandControl()}
  {@const label=focusStep===0?'Expand pane':'Restore pane layout'}
  <button class="icon pane-expand-control" aria-label={label} aria-pressed={focusStep>0} title={focusStep?label:`Expand pane (${modifierLabel}click to fill workspace)`} oncontextmenu={event=>{if(event.ctrlKey){event.preventDefault();expandTab(true)}}} onclick={event=>expandTab(event.metaKey||event.ctrlKey)}>{#if focusStep}<Minimize2 size={15}/>{:else}<MoveDiagonal size={15}/>{/if}</button>
{/snippet}

{#snippet rightSidebarControl()}
  <button class="icon right-sidebar-control" aria-label={showDetail ? 'Hide right sidebar' : 'Show right sidebar'} title={showDetail ? 'Hide right sidebar' : 'Show right sidebar'} aria-pressed={showDetail} onclick={()=>showDetail=!showDetail}><PanelRight size={16}/></button>
{/snippet}

{#snippet workspaceView()}
  <section class="workspace" class:tab-expanded={focusStep>0} data-expansion={focusStep} use:watchPane>
    <header class="topbar" data-tauri-drag-region>
      <nav class="tabs" class:hide-tab-close={snapshot?.settings.showTabCloseButtons === false} class:show-tab-index={tabIndexModifier && (embedded ? active : activePaneId === 'main')} aria-label="Open tasks" ondragover={tabBarOver} ondrop={tabBarDrop}>
        {#if overviewOpen}<div class="tab-entry dashboard-tab" class:active={pane==='overview'}>
          <button class="tab" aria-pressed={pane==='overview'} aria-label="Overview" title="Dashboard" onclick={openOverview}><LayoutDashboard size={16}/></button>

          <button class="close-tab" aria-label="Close dashboard tab" title="Close dashboard tab" onclick={closeOverview}><X size={12}/></button>
        </div>{/if}
        {#each orderedTabs() as tab (`${tab.kind}:${tab.id}`)}
          {#if tab.kind === 'draft'}{@const draft=taskDrafts[tab.id]}{#if draft}
            <div class="tab-entry" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={currentDraftId === tab.id}><button class="tab" draggable="false" ondragstart={event=>dragTab(event,'draft',tab.id)} onpointerdown={event=>startTabPointer(event,'draft',tab.id)} onclick={() => openTaskDraft(draft)}><span class="dot idle"></span><span>{draft.title || 'New chat'}</span></button><button class="close-tab" aria-label="Close draft" onclick={() => closeTaskDraft(tab.id)}><X size={12}/></button></div>
          {/if}
          {:else if tab.kind === 'task'}{@const task=snapshot?.tasks.find(item=>item.id===tab.id)}{#if task}
            <div class="tab-entry" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={selectedTaskId === tab.id}>
              <button class="tab" draggable="false" ondragstart={event=>dragTab(event,'task',tab.id)} onpointerdown={event=>startTabPointer(event,'task',tab.id)} aria-pressed={selectedTaskId === tab.id} onclick={() => openTask(task)} title={task.title}><span class={`dot ${task.status}`}></span><span><AnimatedTitle text={task.title} active={$autonaming[`task:${task.id}`]}/></span></button>

              <button class="close-tab" aria-label={`Close tab ${task.title}`} onclick={() => closeTaskTab(tab.id)}><X size={12} /></button>
            </div>
          {/if}
          {:else if tab.kind === 'channel'}{@const channel=snapshot?.channels.find(item=>item.id===tab.id)}{#if channel}
            <div class="tab-entry" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={selectedChannelId===tab.id}><button class="tab" aria-pressed={selectedChannelId===tab.id} draggable="false" ondragstart={event=>dragTab(event,'channel',tab.id)} onpointerdown={event=>startTabPointer(event,'channel',tab.id)} onclick={()=>openChannel(channel)}><Radio size={13}/><span><AnimatedTitle text={channel.name} active={$autonaming[`channel:${channel.id}`]}/></span></button><button class="close-tab" aria-label={`Close channel tab ${channel.name}`} onclick={()=>{saveCurrentDraft();openChannelIds=openChannelIds.filter(id=>id!==tab.id);forgetTab(tab);if(selectedChannelId===tab.id)openOverview()}}><X size={12}/></button></div>
          {/if}
          {:else if tab.kind === 'terminal'}{@const session=$terminalSessions[tab.id]}{#if session}
            <div class="tab-entry terminal-tab" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={pane==='terminal' && selectedTerminalId===tab.id}><button class="tab" draggable="false" ondragstart={event=>dragTab(event,'terminal',tab.id)} onpointerdown={event=>startTabPointer(event,'terminal',tab.id)} aria-pressed={pane==='terminal'&&selectedTerminalId===tab.id} onclick={()=>openTerminalTab(tab.id)} title={session.cwd}><Terminal size={13}/><span><AnimatedTitle text={session.title} active={$autonaming[`terminal:${session.id}`]}/>{session.status==='exited'?' · exited':''}</span></button><button class="close-tab" aria-label={`Close terminal ${session.title}`} title="Close terminal and end its session" disabled={terminalBusy} onclick={()=>closeTerminalTab(tab.id)}><X size={12}/></button></div>
          {/if}
          {:else if tab.kind === 'empty'}
            <div class="tab-entry" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={pane==='empty' && selectedEmptyId===tab.id}><button class="tab" aria-pressed={pane==='empty' && selectedEmptyId===tab.id} draggable="false" ondragstart={event=>dragTab(event,'empty',tab.id)} onpointerdown={event=>startTabPointer(event,'empty',tab.id)} onclick={()=>{saveCurrentDraft();selectedEmptyId=tab.id;selectedTaskId=null;selectedChannelId=null;selectedTerminalId=null;currentDraftId=null;pane='empty'}}><Plus size={13}/><span>New tab</span></button><button class="close-tab" aria-label="Close empty tab" onclick={()=>closeEmptyTab(tab.id)}><X size={12}/></button></div>
          {:else if tab.kind === 'settings'}
            <div class="tab-entry settings-tab" data-tab-kind={tab.kind} data-tab-id={tab.id} class:active={pane==='settings'}><button class="tab" aria-pressed={pane==='settings'} draggable="false" ondragstart={event=>dragTab(event,'settings','settings')} onpointerdown={event=>startTabPointer(event,'settings','settings')} onclick={()=>openSettings()}><Settings2 size={13}/><span>Settings</span></button><button class="close-tab" aria-label="Close Settings tab" onclick={closeSettings}><X size={12}/></button></div>
          {/if}
        {/each}
        {#if pane === "agent" && focusedAgent}<div class="tab-entry active"><button class="tab active" aria-pressed="true"><Bot size={13}/>{focusedAgent.name}</button></div>{/if}
        {#if pane === 'project' && focusedProject}{@const ProjectIcon = projectIconComponent(focusedProject.icon)}<div class="tab-entry active"><button class="tab active" aria-pressed="true"><ProjectIcon size={13} style={`color:${focusedProject.color}`}/>{focusedProject.name}</button></div>{/if}
      </nav>
      <div class="top-actions" data-tauri-drag-region>
        {#if pane==='empty' && (embedded || paneIds(layout).length>1)}<button class="icon" aria-label="Close empty pane" title="Close pane" onclick={closeOverview}><X size={16}/></button>{/if}
        <button class="icon" aria-label={terminalBusy?'Opening terminal':'Open terminal'} title="Open terminal in this host and folder" disabled={terminalBusy||!snapshot} onclick={newTerminal}>{#if terminalBusy}<LoaderCircle size={16} class="spin"/>{:else}<Terminal size={16}/>{/if}</button>


      </div>
    </header>
    {#if error}<PaneNotice message={error} blocking ondismiss={()=>error=''}/>{/if}
    {#if notice}<PaneNotice message={notice} ondismiss={()=>notice=''}/>{/if}
    {#if !bridge.available}<div class="preview-banner">
        <Command size={14} /> Browser design preview — connect the native app to
        use hosts, agents, and tasks.
      </div>{/if}
    {#if settingsOpen && snapshot}<div class="settings-surface" class:settings-hidden={pane!=='settings'}>
      <SettingsPane settings={snapshot.settings} bind:category={settingsCategory} {agentEditor} {agentDirectory} headerActions={paneExpandControl} onsave={savePreference}/>
    </div>{/if}
    {#if !snapshot}<div class="loading">
        <LoaderCircle size={22} /><span>Loading your workspace…</span
        >{#if error}<button onclick={reload}>Try again</button>{/if}
      </div>
    {:else if pane === 'empty'}<section class="empty-pane" aria-label="Choose pane content"><div class="pane-choices">
      <button class="pane-choice" disabled={busy} onclick={()=>snapshot?.agents.length?openTaskComposer():routeAgentSettings(blankAgent())}><MessageSquare size={22}/><span>New chat</span></button>
      <button class="pane-choice" disabled={terminalBusy} onclick={newTerminal}>{#if terminalBusy}<LoaderCircle size={22} class="spin"/>{:else}<Terminal size={22}/>{/if}<span>Terminal</span></button>
    </div></section>
    {:else if pane === 'terminal' && selectedTerminal}<div class="terminal-surface"><header class="terminal-pane-header">{@render paneExpandControl()}</header><TerminalPane sessionId={selectedTerminal.id} active={(embedded?active:activePaneId==='main') && !modal && !palette}/></div>
    {:else if pane === 'project' && focusedProject}
      {@const ProjectIcon = projectIconComponent(focusedProject.icon)}
      <section class="overview project-overview">
      <div class="overview-head"><div class="overview-expand">{@render paneExpandControl()}</div><div><p class="eyebrow"><ProjectIcon size={13} style={`color:${focusedProject.color}`}/>PROJECT</p><h1>{focusedProject.name}</h1><p>{focusedProject.description || 'A shared project for your agents.'}</p></div>
        <div class="project-overview-actions"><button class="secondary" aria-label={`Edit project ${focusedProject.name}`} onclick={()=>editProject(focusedProject)}><Settings2 size={15}/>Edit project</button><button class="primary" onclick={()=>routeProjectDraft(focusedProject.id)}><Plus size={16}/>New chat</button></div>
      </div>
      {#if focusedProject.workspaces.length}<dl class="project-folders">{#each focusedProject.workspaces as workspace}<div><dt><HardDrive size={13}/>{snapshot.hosts.find(host=>host.id===workspace.hostId)?.name ?? 'Host'}</dt><dd>{workspace.cwd}</dd></div>{/each}</dl>{/if}
      <div class="agent-chats">{#each scopedActivityTasks.filter(task=>task.projectId===focusedProject.id) as task (task.id)}<button class="overview-task" onclick={()=>openTask(task)}><span class={`dot ${task.status}`}></span><div><b>{task.title}</b><small>{snapshot.agents.find(agent=>agent.id===task.agentId)?.name ?? 'Agent'} · {task.provider} · {relative(task.updatedAt)}</small></div></button>{:else}<p class="hint">No chats yet. Choose any agent to start working on this project.</p>{/each}</div>
    </section>
    {:else if pane === "agent" && focusedAgent}<section class="overview">
        <div class="overview-head"><div class="overview-expand">{@render paneExpandControl()}</div><div><p class="eyebrow">AGENT · {focusedAgent.provider}</p><h1>{focusedAgent.name}</h1><p>{focusedAgent.description}</p></div>
          <button class="primary" onclick={()=>openTaskComposer(null,focusedAgent.id)}><Plus size={16}/>New chat</button></div>
        <div class="agent-chats">{#each snapshot.tasks.filter(task=>task.agentId===focusedAgent.id && !task.archived) as task}<button class="overview-task" onclick={()=>openTask(task)}><span class={`dot ${task.status}`}></span><div><b>{task.title}</b><small>{relative(task.updatedAt)}</small></div></button>{:else}<p class="hint">No chats yet. Start one with {focusedAgent.name}.</p>{/each}</div>
      </section>
    {:else if pane === "overview" || (pane === "task" && !selectedTask && !currentTaskDraft) || (pane === "channel" && !activeChannel) || (pane === 'project' && !focusedProject) || (pane==='terminal' && !selectedTerminal)}<section
        class="overview dashboard-overview"
      >
        <div class="overview-head"><div class="overview-expand">{@render paneExpandControl()}</div>
          <div>
            <p class="eyebrow">WORKSPACE · {workspaceLabel}</p>
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
                : routeAgentSettings(blankAgent())}
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
                    : routeAgentSettings(blankAgent())}
                >{snapshot.hosts.length
                  ? "Create Codex agent"
                  : "Add local host"}</button
              >
            </div>
          </section>{/if}
        <div class="overview-grid">
          {#each [["running", "Running"], ["idle", "Ready"], ["completed", "Finished"], ["error", "Needs attention"]] as [status, label]}{@const tasks =
              scopedTasks.filter((task) => task.status === status)}
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
    {:else if pane === "channel" && activeChannel}<section class="task-layout" class:detail-hidden={!showDetail || compactDetail} class:compact-detail={compactDetail}><section class="conversation">
        <MessagePane resetKey={`channel:${activeChannel.id}:${scrollRevision}`}>
        {#snippet header()}<div class="conversation-head task-heading">
          <h1 title={activeChannel.description || undefined}><AnimatedTitle text={activeChannel.name} active={$autonaming[`channel:${activeChannel.id}`]}/></h1>
          <div class="task-actions">
            {#if activeChannelTasks.length}<button class="danger icon" aria-label="Stop channel agents" title="Stop channel agents" disabled={busy} onclick={stopChannel}><Square size={14}/></button>{/if}
            {@render paneExpandControl()}
              {@render rightSidebarControl()}
            <div class="task-overflow">
              <button bind:this={taskMenuAnchor} class="icon" aria-label="Channel actions" aria-haspopup="menu" aria-expanded={taskMenu} onclick={()=>taskMenu=!taskMenu}><MoreHorizontal size={17}/></button>
              {#if taskMenu && taskMenuAnchor}<div use:floating={{anchor:taskMenuAnchor}} class="task-menu floating-panel" role="menu" aria-label="Channel actions">
                <button role="menuitem" onclick={editActiveChannel}><Settings2 size={15}/>Edit channel</button>
              </div>{/if}
            </div>
          </div>
          </div>
        {/snippet}
          {#if activeChannel.messages.length}{#each activeChannel.messages as message}<article
                class:user={message.role === "user"}
                class:tinted={message.role === "user" && snapshot.settings.tintUserMessages}
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
          {#each [...new Set(activeChannelTasks.map(task=>task.agentId))] as agentId}
            {@render agentWaiting(snapshot.agents.find(agent=>agent.id===agentId))}
          {/each}
          {#if composerPending[`channel:${activeChannel.id}`] && !activeChannelTasks.length}
            {#each effectiveRecipients as agentId}{@render agentWaiting(snapshot.agents.find(agent=>agent.id===agentId), true)}{/each}
          {/if}
        </MessagePane>
        <QueuedMessages messages={currentQueuedMessages} agents={snapshot.agents} tasks={snapshot.tasks} {busy} onremove={removeQueuedMessage} onedit={editQueuedMessage}/>
        <div class="composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
          {@render slashMenu()}
          <MentionComposer bind:value={composer} agents={channelMentionAgents} oninput={updateSlash} onkeydown={handleComposerKeydown}/>
          <div class="composer-footer">
            {@render attachmentTools()}
            <div class="recipient-picker">
              <span>Send to</span
              >{#each snapshot.agents.filter( (a) => activeChannel.agentIds.includes(a.id), ) as agent}<button
                  class:selected={effectiveRecipients.includes(agent.id)}
                  aria-pressed={effectiveRecipients.includes(agent.id)}
                  disabled={channelMentionIds.includes(agent.id)}
                  title={channelMentionIds.includes(agent.id) ? "Mentioned in this message. Remove the @mention to deselect." : `Send to ${agent.name}`}
                  onclick={() => toggleRecipient(agent.id)}>{agent.name}</button
                >{/each}
            </div>
            {#if activeChannelTasks.length}<button class="danger composer-control" aria-label="Stop channel tasks" title={activeChannelStarting ? "Starting channel — stop" : "Stop channel tasks"} onclick={stopChannel}>{#if activeChannelStarting}<LoaderCircle class="spin" size={15}/>{:else}<Square size={15}/>{/if}</button>{/if}<button class="primary composer-control" aria-label={composerPending[`channel:${activeChannel.id}`] ? "Starting channel message" : "Send channel message"} title={composerPending[`channel:${activeChannel.id}`] ? "Starting…" : "Send"} disabled={busy || !canSend} onclick={sendChannel}>{#if composerPending[`channel:${activeChannel.id}`]}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button>
          </div>
        </div>
      </section>
      {#if compactDetail && showDetail}<button class="detail-backdrop" aria-label="Dismiss channel members" onclick={()=>showDetail=false}></button>{/if}
      <aside class="run-detail channel-members" class:closed={!showDetail} aria-label="Channel members">
        <SidebarResize side="right"/>
        <ChannelMembers channel={activeChannel} agents={snapshot.agents} hosts={snapshot.hosts} tasks={snapshot.tasks} {busy} onmembership={changeChannelMembership} onadmin={editActiveChannel} onconversation={configureChannelConversation} onstopconversation={stopChannel} onclose={()=>showDetail=false}/>
      </aside>
      </section>
    {:else if currentTaskDraft}
      <section class="draft-layout" aria-label="New chat draft">
        <div class="draft-content">
          <div class="draft-intro"><div class="draft-expand">{@render paneExpandControl()}</div>
            <p class="eyebrow">NEW CHAT</p>
            <h1>What would you like to work on?</h1>
            <p>Ask a question, explore a project, or describe a change. Your agent starts when you send.</p>
          </div>
          <div class="draft-options form-grid">
            <label>Agent<select class="draft-select" aria-label="Agent" bind:value={taskAgentId} onchange={routeChangedDraft} disabled={busy || filesBusy || !!currentTaskDraft.createdTaskId}>{#each snapshot.agents as agent}<option value={agent.id}>{agent.name} · {agent.provider}</option>{/each}</select></label>
            <label>Project<select class="draft-select" id="task-project" aria-label="Project" bind:value={taskProjectId} onchange={routeChangedDraft} disabled={busy || filesBusy || !!currentTaskDraft.createdTaskId}><option value="">No project</option>{#each projects as project}<option value={project.id}>{project.name}</option>{/each}</select></label>
          </div>
          {#if taskFormAgent && !taskProjectId}<label class="task-workspace-editor"><span><Folder size={13}/>Working folder</span><div><input aria-label="Working folder" bind:value={taskCwd} placeholder={inheritedTaskCwd || '/path/to/project'} disabled={busy || !!currentTaskDraft.createdTaskId}/>{#if snapshot.hosts.find(host=>host.id===taskFormAgent.hostId)?.kind === 'local'}<button class="icon" aria-label="Browse working folder" title="Choose folder" disabled={busy || !!currentTaskDraft.createdTaskId} onclick={browseTaskFolder}><Folder size={15}/></button>{/if}</div></label>{:else if taskFormAgent}<p class="task-workspace-preview"><Folder size={13}/><span><b>{snapshot.hosts.find(host=>host.id===taskFormAgent.hostId)?.name ?? 'Host'}</b><code>{taskFormCwd}</code></span></p>{/if}
          <div class="composer draft-composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
            {@render slashMenu()}
            <textarea bind:value={composer} aria-label="Task message" placeholder="Describe what you want this agent to do…" oninput={(event)=>updateSlash(event.currentTarget.value)} onkeydown={handleComposerKeydown}></textarea>
            <div class="composer-footer"><div class="composer-left">{@render attachmentTools()}{#if taskFormAgent}<AccessPicker provider={taskFormAgent.provider} sandbox={draftSandbox} disabled={busy||filesBusy} onchange={changeSandbox}/>{/if}</div><div class="composer-right"><ModelPicker target={currentTaskDraft.createdTaskId?{taskId:currentTaskDraft.createdTaskId}:{agentId:taskAgentId,projectId:taskProjectId||null}} settings={draftModelSettings} fallbackModel={taskFormAgent?.model??''} disabled={busy||filesBusy} onchange={changeModel}/><button class="primary composer-control" aria-label={composerPending[`draft:${currentDraftId}`] ? "Starting task" : "Send task message"} title={composerPending[`draft:${currentDraftId}`] ? "Starting…" : "Send"} disabled={busy || !canSend || !taskAgentId} onclick={send}>{#if composerPending[`draft:${currentDraftId}`]}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button></div></div>
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
        {#snippet detailTabs()}<div class="detail-tabs" role="tablist" aria-label="Run detail views">
          <div class="detail-tab-entry" class:active={detailTab==='run' || (detailTab==='git' && gitState.repository!==true)}><button class="detail-tab" role="tab" aria-selected={detailTab==='run' || (detailTab==='git' && gitState.repository!==true)} onclick={()=>detailTab='run'}>Run detail</button></div>
          <div class="detail-tab-entry" class:active={detailTab==='timeline'}><button class="detail-tab" role="tab" aria-selected={detailTab==='timeline'} onclick={()=>detailTab='timeline'}>Timeline</button></div>
          {#if gitState.repository}<div class="detail-tab-entry" class:active={detailTab==='git'}><button class="detail-tab" role="tab" aria-selected={detailTab==='git'} onclick={()=>detailTab='git'}>Git changes</button></div>{/if}
          <button class="detail-close" aria-label="Close run detail" onclick={() => (showDetail = false)}><X size={14} /></button>
        </div>{/snippet}
        <div class="conversation-head task-heading pane-task-header">
            <h1 class="task-title"><AnimatedTitle text={selectedTask.title} active={$autonaming[`task:${selectedTask.id}`]}/><button class="icon task-title-edit" aria-label="Task settings" title="Edit task" onclick={()=>{renameTitle=selectedTask.title;taskProjectId=selectedTask.projectId??'';modal='taskSettings'}}><Pencil size={14}/></button></h1>
            <div class="task-actions">
              {#if selectedTask.status === "running"}<button class="danger icon" aria-label="Stop" title="Stop" disabled={busy} onclick={() => run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")}><Square size={14}/></button>{/if}
              {#if selectedTask.nativeSessionId && ["interrupted","error"].includes(selectedTask.status)}<button class="icon" aria-label="Resume session" disabled={busy || selectedTask.archived} title="Reconnect and resume this session" onclick={resumeTask}><RotateCw size={15}/></button>{/if}
              {@render paneExpandControl()}
              {@render rightSidebarControl()}
            </div>
          </div>
        {#if showDetail && !compactDetail}{@render detailTabs()}{/if}
        <section class="conversation">
          <TaskActivity {goal} {goalNote} tools={computerTools} onstop={() => selectedTask && run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")} disabled={busy} />
          <MessagePane resetKey={`task:${selectedTask.id}:${scrollRevision}`}>
            {#if pendingApprovalRequests.length}<section class="pending-approvals" aria-label="Pending approval requests">
              {#each pendingApprovalRequests as request (request.id)}<ApprovalRequestCard {request} disabled={busy} resolving={resolvingApprovalId === request.id} onresolve={resolveApproval}/>{/each}
            </section>{/if}
            {#if conversationItems.length}{#each conversationItems as item (item.type === 'tool-group' ? `tool:${item.values[0].id}` : item.value.id)}
              {#if item.type === "activity"}<RunActivity event={item.value} />
              {:else if item.type === "tool-group"}<RunActivity events={item.values} compressed={snapshot.settings.compressToolCalls === true} running={selectedTask.status === "running"} />
              {:else}{@const message = item.value}{@const operator = message.role === 'user' ? splitOperatorMessage(message.text.replace(/^\[Two human operators are collaborating[^\n]*\]\n/, '')) : null}<article
                  class:user={message.role === "user"}
                class:tinted={message.role === "user" && snapshot.settings.tintUserMessages}
                  class:system={message.role === "system"}
                  class="message"
                >
                  <div class="message-meta">
                    {#if operator?.name}<span class="avatar message-avatar human-avatar" title={operator.name}>{operator.name.slice(0, 1).toUpperCase()}</span>{:else}{@render messageAvatar(message.senderAgentId ? snapshot.agents.find(agent=>agent.id===message.senderAgentId) : message.role==='assistant' ? selectedAgent : null)}{/if}
                    <span
                      >{senderName(message) ?? (message.role === "user" ? "You" : message.role === "assistant" ? (selectedAgent?.name ?? "Agent") : "System")}</span
                    ><time>{date(message.createdAt)}</time>
                  </div>
                  <Markdown text={message.role === 'user' ? operatorMessageText(message.text) : message.text} /><AttachmentList attachments={message.attachments ?? []}/>
                </article>{/if}{/each}{:else if !pendingApprovalRequests.length && !resolvedApprovalRequests.length}<div class="blank-conversation">
                <Terminal size={24} />
                <h2>No messages yet</h2>
                <p>
                  Describe what you want this agent to do. Its actual output
                  will appear here.
                </p>
              </div>{/if}
            {#if resolvedApprovalRequests.length}<section class="approval-history" aria-label="Approval history">
              <h2>Approval history</h2>
              {#each resolvedApprovalRequests as request (request.id)}<ApprovalRequestCard {request}/>{/each}
            </section>{/if}
            {#if selectedTask.status === 'running' || selectedTaskStarting}
              {@render agentWaiting(selectedAgent, selectedTask.status !== 'running')}
            {/if}
          </MessagePane>
          <QueuedMessages messages={currentQueuedMessages} agents={snapshot.agents} tasks={snapshot.tasks} {busy} onremove={removeQueuedMessage} onedit={editQueuedMessage}/>
          <div class="composer" use:fileDrop>
            <AttachmentList attachments={currentAttachments} onremove={filesBusy?undefined:removeAttachment}/>
            {@render slashMenu()}
            <textarea
              bind:value={composer}
              aria-label="Task message"
              placeholder={`Message ${selectedAgent?.name ?? "agent"}…`}
              oninput={(event) => updateSlash(event.currentTarget.value)}
              onkeydown={handleComposerKeydown}
            ></textarea>
            <div class="composer-footer">
              <div class="composer-left">{@render attachmentTools()}<AccessPicker provider={selectedTask.provider} sandbox={selectedTask.sandbox} disabled={busy||selectedTask.status==='running'} onchange={changeSandbox}/></div>
              <div class="composer-right">
                <ModelPicker target={{taskId:selectedTask.id}} settings={selectedTask.modelSettings??null} fallbackModel={selectedTask.model} disabled={busy||selectedTask.status==='running'} onchange={changeModel}/>
{#if selectedTask.status === "running"}<button class="danger composer-control" aria-label="Stop current task" title="Stop current task" onclick={() => run(() => bridge.cancelTask(selectedTask.id), "Stopping task…")}><Square size={15}/></button>{/if}
                <button class="primary composer-control" aria-label="Send task message" title={selectedTask.status==='running' ? (snapshot.settings.busyMessageMode==='steer'?'Send follow-up (steer if supported, otherwise queue)':'Queue message') : 'Send'} disabled={busy || !canSend} onclick={send}>{#if composerPending[`task:${selectedTask.id}`]}<LoaderCircle class="spin" size={15}/>{:else}<ArrowUp size={16}/>{/if}</button>
              </div>
            </div>
          </div>
        </section>
        {#if compactDetail && showDetail}<button class="detail-backdrop" aria-label="Dismiss right sidebar" onclick={()=>showDetail=false}></button>{/if}
        <aside class="run-detail" class:closed={!showDetail} aria-label="Right sidebar">
          <SidebarResize side="right"/>
            {#if compactDetail}{@render detailTabs()}{/if}
            <div class="git-slot" class:hidden={detailTab!=='git' || gitState.repository!==true}>
              <GitPane bind:this={gitPane} taskId={selectedTask.id} probeKey={`${selectedTask.hostId}\u001f${selectedTask.cwd}`} active={showDetail} onStatus={value=>{gitState=value}}/>
            </div>
            <div class="detail-scroll" class:hidden={detailTab!=='timeline'}><TimelinePane events={timelineEvents} provider={selectedTask.provider} {goalError}/></div>
            <div class="detail-scroll" class:hidden={detailTab==='timeline' || (detailTab==='git' && gitState.repository===true)}>
              <details class="agent-identity" open aria-label="Agent identity"><summary><button class="avatar identity-avatar identity-avatar-button" aria-label={`Change ${selectedAgent?.name ?? 'agent'} avatar`} title="Change avatar" onclick={event=>{event.preventDefault();event.stopPropagation();if(selectedAgent)routeAgentSettings({...selectedAgent});}}>{@render avatarVisual(selectedAgent, 17)}<span class="avatar-edit-overlay"><Pencil size={13}/></span></button><span><b>{selectedAgent?.name ?? 'Agent'}</b><small>{selectedTask.provider}{selectedTask.model ? ` · ${selectedTask.model}` : ''}</small></span></summary>{#if selectedAgent?.description}<div class="identity-actions"><p>{selectedAgent.description}</p></div>{/if}</details>
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
                  <dd>{selectedTask.sandbox === "yolo" ? "YOLO — skip permissions" : selectedTask.sandbox === "harness-configured" ? "Harness permissions" : selectedTask.sandbox}</dd>
                </div>
                {#if selectedTask.nativeSessionId}<div>
                    <dt>native session</dt>
                    <dd class="session-id" title={selectedTask.nativeSessionId}>
                      {selectedTask.nativeSessionId}
                    </dd>
                  </div>{/if}
              </dl>
              <RunSummary task={selectedTask} gitStatus={gitState.status} tasks={snapshot.tasks} terminalSessions={Object.values($terminalSessions)} onOpenGit={()=>detailTab='git'}/>
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

<main class:preview={!bridge.available} class:native-mac={nativeMac} class:native-fullscreen={nativeFullscreen} class:sidebar-collapsed={sidebarCollapsed} class:embedded class="app-shell">
  {#if !embedded}<aside class="sidebar" aria-label="Agents and tasks">
    <SidebarResize side="left" collapsed={sidebarCollapsed} oncollapse={value=>{sidebarCollapsed=value;sidebarScrolled=false;railAgentId=null}}/>
    <div class="brand" class:scrolled={sidebarScrolled} data-tauri-drag-region>
      {#if !sidebarCollapsed}<strong>monitter</strong>{/if}
      {#if !sidebarCollapsed}<div class="sidebar-views" role="group" aria-label="Sidebar view">
        {#each sidebarViews as view}<button class="view-toggle" aria-label={`${view.label} view`} title={`${view.label} view`} aria-pressed={sidebarView === view.id} disabled={busy || !bridge.available || !snapshot} onclick={()=>{void setSidebarView(view.id)}}><view.icon size={15}/></button>{/each}
      </div>{/if}
    </div>
    {#if !sidebarCollapsed}
    <nav class="side-scroll" onscroll={event=>sidebarScrolled=event.currentTarget.scrollTop>0}>
      <section class="workspace-scopes" aria-label="Desktop workspace" data-workspace-picker>
        <p class="section-label"><span>WORKSPACE</span></p>
        <select aria-label="Switch desktop workspace" value={activeWorkspaceKey} onchange={chooseWorkspace} disabled={workspaceTransition}>
          <option value="all">All activity</option>
          <optgroup label="Agents">{#each snapshot?.agents ?? [] as agent}<option value={`agent:${agent.id}`}>{agent.name}</option>{/each}</optgroup>
          <optgroup label="Projects">{#each projects as project}<option value={`project:${project.id}`}>{project.name}</option>{/each}<option value="project:unassigned">No project</option></optgroup>
        </select>
        {#if globalPendingApprovals.length}<div class="workspace-approval-list" aria-label="Pending approvals across workspaces">
          {#each globalPendingApprovals as item (item.request.id)}<button class="workspace-approval" data-approval-task={item.task.id} onclick={()=>routeTaskWorkspace(item.task)}><span class="dot running"></span><span>Approval · {item.task.title}</span></button>{/each}
        </div>{/if}
      </section>
      {#if sidebarView === 'standard'}
      <div class="section-label">
        <span>AGENTS</span><button
          aria-label="New chat"
          title="New chat"
          disabled={busy || !snapshot?.agents.length}
          onclick={() => openTaskComposer()}><Plus size={15} /></button
        >
      </div>
      {#if snapshot?.agents.length}{#each sidebarSorted(snapshot.agents,'agents') as agent}{@const agentTasks =
            sidebarSorted(snapshot.tasks.filter(
              (task) => task.agentId === agent.id && !task.parentTaskId && !task.channelId && !task.archived && taskBelongsToWorkspace(task, activeWorkspaceKey),
            ),`agent-chats:${agent.id}`)}
          <section class="agent-group" class:has-chats={agentTasks.length > 0 && !collapsedAgents[agent.id]}>
            <div class="agent-row" use:sidebarReorder={{group:'agents',id:agent.id,move:moveSidebar}}>
              <button class="avatar agent-avatar-toggle" aria-label={`${collapsedAgents[agent.id] ? 'Expand' : 'Collapse'} chats for ${agent.name}`} aria-expanded={!collapsedAgents[agent.id]} aria-controls={`agent-chats-${agent.id}`} onclick={()=>collapsedAgents[agent.id]=!collapsedAgents[agent.id]}>
                {@render avatarVisual(agent, 14)}
                <span class="avatar-toggle-overlay" aria-hidden="true">{#if collapsedAgents[agent.id]}<ChevronRight size={16}/>{:else}<ChevronDown size={16}/>{/if}</span>
              </button><button
                class="agent-name"
                aria-label={`Open agent ${agent.name}`}
                aria-pressed={activeWorkspaceKey === `agent:${agent.id}`}
                aria-controls={`agent-chats-${agent.id}`}
                onclick={() => { collapsedAgents[agent.id]=false; void switchWorkspace(`agent:${agent.id}`); }}
                ><b>{agent.name}{#if approvalCount(`agent:${agent.id}`)}<span class="approval-badge" aria-label={`${approvalCount(`agent:${agent.id}`)} pending approvals`}>{approvalCount(`agent:${agent.id}`)}</span>{/if}</b><small
                  >{agent.provider}{agent.model
                    ? ` · ${agent.model}`
                    : ""}</small
                ></button
              >{#if !openTasks.some(task=>task.agentId===agent.id && !task.channelId)}<button class="quiet" aria-label={`New chat with ${agent.name}`} title="New chat" onclick={() => routeDraft(agent.id)}><Plus size={15}/></button>{/if}<button
                class="quiet"
                aria-label={`Edit ${agent.name}`}
                onclick={() => {
                  routeAgentSettings({ ...agent });
                }}><MoreHorizontal size={15} /></button
              >
            </div>
            <div class="task-tree" id={`agent-chats-${agent.id}`} hidden={collapsedAgents[agent.id]}>
              {#each agentTasks as task}{@render sidebarChat(task)}{/each}{#if !agentTasks.length}<p class="empty-tree">No chats yet</p>{/if}
            </div>
          </section>{/each}{:else}<div class="side-empty">
          <Bot size={18} />
          <p>Agents hold their own tasks and settings.</p>
          <button
            class="text-button"
            onclick={() => {
              routeAgentSettings(blankAgent());
            }}>Create first agent</button
          >
        </div>{/if}
      {:else if sidebarView === 'activity'}
        <div class="section-label"><span>ACTIVITY</span><button aria-label="New chat" title="New chat" onclick={()=>openTaskComposer()}><Plus size={15}/></button></div>
        <p class="view-hint">Running first, then most recent.</p>
        <div class="activity-list">{#each scopedActivityTasks as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">No chats yet</p>{/each}</div>
      {:else}
        <div class="section-label"><span>PROJECTS</span><button aria-label="New project" title="New project" onclick={()=>editProject()}><Plus size={15}/></button></div>
        {#each sidebarSorted(projects,'projects') as project (project.id)}
          {@const projectTasks = sidebarSorted(scopedActivityTasks.filter(task=>task.projectId===project.id),`project-chats:${project.id}`)}
          {@const ProjectIcon = projectIconComponent(project.icon)}
          <section class="project-group" aria-label={`Project ${project.name}`}>
            <div use:sidebarReorder={{group:'projects',id:project.id,move:moveSidebar}} class="project-row" class:current={focusedProjectId === project.id || selectedTask?.projectId === project.id}>
              <button class="folder-toggle" aria-label={`${collapsedProjects[project.id] ? 'Expand' : 'Collapse'} project ${project.name}`} aria-expanded={!collapsedProjects[project.id]} onclick={()=>collapsedProjects[project.id]=!collapsedProjects[project.id]}>
                {#if collapsedProjects[project.id]}<ChevronRight size={13}/>{:else}<ChevronDown size={13}/>{/if}
              </button>
              <button class="project-name" aria-label={`Open project ${project.name}`} onclick={()=>{const target=`project:${project.id}` as WorkspaceKey;if(target!==activeWorkspaceKey)void switchWorkspace(target);else openProject(project);}}><ProjectIcon size={14} style={`color:${project.color}`}/><span>{project.name}</span>{#if approvalCount(`project:${project.id}`)}<span class="approval-badge" aria-label={`${approvalCount(`project:${project.id}`)} pending approvals`}>{approvalCount(`project:${project.id}`)}</span>{/if}<small>{projectTasks.length}</small></button>
              <button class="quiet" aria-label={`New chat in ${project.name}`} title="New chat" onclick={()=>routeProjectDraft(project.id)}><Plus size={14}/></button>
              <button class="quiet" aria-label={`Edit project ${project.name}`} title="Edit project" onclick={()=>editProject(project)}><MoreHorizontal size={14}/></button>
            </div>
            {#if !collapsedProjects[project.id]}<div class="task-tree">
              {#each projectTasks as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">No chats yet</p>{/each}
            </div>{/if}
          </section>
        {:else}<p class="view-hint">Group chats from any agent in a project.</p>{/each}
        <section class="project-group" aria-label="No project">
          <button class="unassigned-folder" aria-expanded={!collapsedProjects.unassigned} onclick={()=>collapsedProjects.unassigned=!collapsedProjects.unassigned}>
            {#if collapsedProjects.unassigned}<ChevronRight size={13}/>{:else}<ChevronDown size={13}/>{/if}<Folder size={14}/><span>No project</span><small>{activityTasks.filter(task=>!task.projectId).length}</small>
          </button>
          {#if !collapsedProjects.unassigned}<div class="task-tree">
            {#each sidebarSorted(scopedActivityTasks.filter(task=>!task.projectId),'project-chats:unassigned') as task (task.id)}{@render sidebarChat(task,true)}{:else}<p class="empty-tree">All chats are organised.</p>{/each}
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
      {#each sidebarSorted(snapshot?.channels ?? [],'channels') as channel}<button use:sidebarReorder={{group:'channels',id:channel.id,move:moveSidebar}}
          class:current={channel.id === selectedChannelId}
          class="channel-row"
          onclick={() => routeChannel(channel)}
          ><Radio size={14} /><span>{channel.name}</span><small
            >{channel.agentIds.length}</small
          ></button
        >{/each}
    </nav>
    {:else}<nav class="agent-rail" aria-label="Agents" onscroll={event=>sidebarScrolled=event.currentTarget.scrollTop>0}>
      <select class="rail-workspace-picker" aria-label="Switch desktop workspace" value={activeWorkspaceKey} onchange={chooseWorkspace} disabled={workspaceTransition} title={`Workspace: ${workspaceLabel}`}>
        <option value="all">All activity</option>{#each snapshot?.agents ?? [] as agent}<option value={`agent:${agent.id}`}>{agent.name}</option>{/each}{#each projects as project}<option value={`project:${project.id}`}>{project.name}</option>{/each}<option value="project:unassigned">No project</option>
      </select>
      {#if globalPendingApprovals.length}<div class="rail-approvals" aria-label="Pending approvals across workspaces">{#each globalPendingApprovals as item (item.request.id)}<button class="workspace-approval" data-approval-task={item.task.id} title={`Approval · ${item.task.title}`} onclick={()=>routeTaskWorkspace(item.task)}><span class="dot running"></span></button>{/each}</div>{/if}
      {#each sidebarSorted(snapshot?.agents ?? [],'agents') as agent}<button use:sidebarReorder={{group:'agents',id:agent.id,move:moveSidebar}} class="rail-avatar" class:current={railAgentId === agent.id || selectedAgent?.id === agent.id} aria-label={`Chats with ${agent.name}`} title={agent.name} aria-expanded={railAgentId === agent.id} onclick={(event)=>{railAnchor=event.currentTarget;railAgentId=railAgentId===agent.id?null:agent.id}}>
        <span class="avatar">{@render avatarVisual(agent, 15)}</span>
        {#if activeTasks.some(task=>task.agentId===agent.id && task.status==='running')}<span class="rail-running" aria-label="Running"></span>{/if}
      </button>{/each}
      <button class="icon" aria-label="New chat" title="New chat" disabled={busy || !snapshot?.agents.length} onclick={()=>openTaskComposer()}><Plus size={17}/></button>
      <button class="icon" aria-label="Switch channel, chat or agent" title={`Switch channel, chat or agent (${modifierLabel}K)`} onclick={()=>palette='switch'}><Search size={16}/></button>
    </nav>{/if}
    {#if railAgent && railAnchor}<div class="rail-chats floating-panel" role="dialog" aria-label={`${railAgent.name} chats`} use:floating={{anchor:railAnchor,side:'right'}}>
      <header><strong>{railAgent.name}</strong><button class="icon" aria-label="Close agent chats" onclick={()=>railAgentId=null}><X size={14}/></button></header>
      <div class="rail-chat-list">{#each sidebarSorted(scopedActivityTasks.filter(task=>task.agentId===railAgent.id),`agent-chats:${railAgent.id}`) as task (task.id)}{@render sidebarChat(task)}{:else}<p class="detail-empty">No chats yet.</p>{/each}</div>
      <button class="rail-new-chat" aria-label={`New chat with ${railAgent.name}`} onclick={()=>routeDraft(railAgent!.id)}><Plus size={14}/>New chat</button>
    </div>{/if}
    <footer class="sidebar-footer" aria-label="Workspace controls">
      <button class="icon" aria-label="Preferences" title="Preferences" onclick={()=>routeSettings()}><Settings2 size={16}/></button>
      <button class="icon" aria-label="Hosts" title="Hosts" onclick={()=>modal='hosts'}><Network size={16}/></button>
      <button class="icon" aria-label="Agent directory" title="Agent directory" onclick={()=>{directoryQuery='';routeSettings('directory')}}><Bot size={16}/></button>
      <button class="icon" aria-label="Archived chats" title="Archived chats" onclick={()=>modal='archived'}><Archive size={16}/></button>
    </footer>
  </aside>{/if}
  {#if embedded}{@render workspaceView()}{:else}<div class="pane-grid">
    <PaneGrid {layout} {activePaneId} {expandedPaneId} pointerDrag={pointerTabDrag} onPointerDragEnd={()=>pointerTabDrag=null} focusFollowsMouse={snapshot?.settings.focusFollowsMouse ?? false} dimInactivePanes={snapshot?.settings.dimInactivePanes ?? true} inactivePaneOpacity={snapshot?.settings.inactivePaneOpacity ?? .6} onactivate={id=>activePaneId=id} onresize={resizeSplit} ondropTab={dropTab}>
      {#snippet children(id)}{#if id==='main'}{@render workspaceView()}{:else}
        <AppSurface embedded={true} paneId={id} active={activePaneId===id && !modal && !palette} parentSnapshot={snapshot} workspaceKey={activeWorkspaceKey}
          onSnapshot={value=>applySnapshot(value,++snapshotIssued)} onTabDrop={dropTab} onLayout={setLayout} onVimSplit={splitPaneForVim} onVimWorkspace={(source,command)=>{activePaneId=source;return executeWorkspaceVim(command)}}
          onExistingChat={focusExistingChat} parentExpandedPaneId={expandedPaneId} onExpandPane={setPaneExpansion} onAgentSettingsSelect={routeAgentSettings} onClosePane={removeEmptyPane} onSettingsSelect={routeSettings} onTerminalSelect={routeTerminal} onSelection={taskId=>paneSelections[id]=taskId} onWorkspaceChange={persistWorkspace} onTabPointerStart={(event,tab)=>pointerTabDrag={tab,pointerId:event.pointerId,startX:event.clientX,startY:event.clientY}} bind:this={paneRefs[id]}/>
      {/if}{/snippet}
    </PaneGrid>
  </div>{/if}
</main>


<CommandPalette open={palette !== null} title={palette === "switch" ? "Switch to" : "Controls"} placeholder={palette === "switch" ? "Find a channel, chat or agent…" : "Find a control or setting…"} items={palette === "switch" ? switchItems : controlItems} onselect={selectPalette} onclose={()=>palette=null}/>
{#if modal === 'archived' && snapshot}<ArchivedChats {snapshot} onclose={()=>modal=null}
  onRestore={async taskId=>applySnapshot(await bridge.setTaskArchived(taskId,false),++snapshotIssued)}
  onDelete={deleteArchivedTask} previewDeletion={taskId=>bridge.previewTaskDeletion(taskId)}/>{/if}

<Modal title={projectDraft?.id ? 'Edit project' : 'New project'} open={modal === 'project'} onclose={()=>modal=null}>
  {#if projectDraft}<form class="form" onsubmit={event=>{event.preventDefault();void saveProject();}}>
    <label>Name<input data-autofocus required bind:value={projectDraft.name} placeholder="Project name" /></label>
    <label>Description<input bind:value={projectDraft.description} placeholder="What you're working on together" /></label>
    <fieldset class="project-identity"><legend>Project icon</legend><div class="project-icon-options">{#each projectIcons as option}{@const Icon = option.icon}<button type="button" class:selected={projectDraft.icon===option.id} aria-label={option.label} title={option.label} style={`--project-colour:${projectDraft.color}`} onclick={()=>projectDraft={...projectDraft!,icon:option.id}}><Icon size={17}/></button>{/each}</div></fieldset>
    <fieldset class="project-identity"><legend>Icon colour</legend><div class="project-colour-options">{#each projectColours as colour}<button type="button" class:selected={projectDraft.color===colour} aria-label={`Use ${colour}`} style={`--project-colour:${colour}`} onclick={()=>projectDraft={...projectDraft!,color:colour}}></button>{/each}<label class="project-custom-colour"><span>Custom colour</span><input type="color" aria-label="Custom project icon colour" value={projectDraft.color} onchange={event=>projectDraft={...projectDraft!,color:event.currentTarget.value}}/></label></div></fieldset>
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

{#snippet agentDirectory()}<div class="form agent-directory"><label>Find agents<input aria-label="Find agents" bind:value={directoryQuery} placeholder="Search expertise, responsibilities, or skills" /></label>{#each (snapshot?.agents ?? []).filter(agent => { const profile=agent as AgentProfile; const haystack=[agent.name,agent.description,...(profile.expertise??[]),...(profile.responsibilities??[]),...(profile.skills??[])].join(' ').toLowerCase(); return haystack.includes(directoryQuery.trim().toLowerCase()); }) as agent}{@const profile=agent as AgentProfile}<article class:disabled={profile.collaborationEnabled===false}><span class="avatar">{@render avatarVisual(agent, 13)}</span><div><b>{agent.name}</b><small>{agent.provider} · {snapshot?.hosts.find(host=>host.id===agent.hostId)?.name ?? 'Unknown host'} · {profile.collaborationEnabled===false?'Collaboration off':'Collaboration on'}</small>{#if (profile.expertise??[]).length}<p>{(profile.expertise??[]).join(' · ')}</p>{/if}</div><button class="secondary" onclick={()=>{modal=null;openTaskComposer(null,agent.id)}}>New chat</button><button class="icon" aria-label={`Edit ${agent.name}`} onclick={()=>{routeAgentSettings({...agent})}}><MoreHorizontal size={15}/></button></article>{:else}<p class="hint">No saved agents match this search.</p>{/each}</div>{/snippet}

{#snippet agentEditor()}
  <div class="agent-editor-selector"><label>Agent<select aria-label="Select agent" disabled={busy} value={agentDraft?.id??''} onchange={event=>selectAgentEditor(event.currentTarget.value)}><option value="">New agent</option>{#each snapshot?.agents??[] as agent}<option value={agent.id}>{agent.name}</option>{/each}</select></label><button class="secondary" disabled={busy} onclick={()=>selectAgentEditor('')}><Plus size={15}/>New agent</button></div>
  <p class="hint">Choose an agent to edit its identity, harness, permissions and collaboration profile.</p>
{#if agentDraft}<form
      class="form agent-settings-form"
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
            {:else}<option value="harness-configured">Use harness permissions</option>{/if}{#if ['codex','claude'].includes(agentDraft.provider)}<option value="yolo">YOLO — skip permissions</option>{/if}</select
          >{#if agentDraft.provider !== "codex" && agentDraft.sandbox !== 'yolo'}<small>Uses this harness's permissions on the selected host. Monitter shows approval controls only when this harness exposes a live response channel.</small>{/if}</label
        ><label class="check-row"><input type="checkbox" role="switch" disabled={!['codex','claude'].includes(agentDraft.provider)} checked={agentDraft.sandbox === 'yolo'} onchange={event => { if (agentDraft) agentDraft.sandbox = event.currentTarget.checked ? 'yolo' : (agentDraft.provider === 'codex' ? 'read-only' : 'harness-configured'); }} /> YOLO — skip permissions<small>{['codex','claude'].includes(agentDraft.provider) ? 'Applies to new chats. Existing chats keep their saved permissions.' : 'This harness has no verified skip-permissions mode.'}</small></label
        ><label
          >Colour<input type="color" bind:value={agentDraft.color} /></label
        >
      </div>
      <footer>
        <button
          type="button"
          class="danger-text"
          disabled={!agentDraft.id || busy}
          onclick={deleteEditedAgent}
          ><Trash2 size={15} /> Delete</button
        ><span></span><button
          type="button"
          class="secondary"
          onclick={discardAgentEdits}>Discard changes</button
        ><button class="primary" disabled={busy}
          ><Save size={15} /> Save agent</button
        >
      </footer>
    </form>{/if}
{/snippet}
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
            /><span class="avatar small"
              >{@render avatarVisual(agent, 12)}</span
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

<style>
  :global(*) {
    box-sizing: border-box;
  }
  :global(:root) {
    --terminal-background: #090b0d;
    --terminal-foreground: #e5e7eb;
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
    font-family: var(--interface-font, "IBM Plex Sans", system-ui, sans-serif);
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
  :global(*) { scrollbar-width: thin; scrollbar-color: color-mix(in srgb, var(--muted) 55%, transparent) transparent; }
  :global(*::-webkit-scrollbar) { width:8px; height:8px; }
  :global(*::-webkit-scrollbar-track) { background:transparent; }
  :global(*::-webkit-scrollbar-thumb) { min-height:28px; border:2px solid transparent; border-radius:999px; background:color-mix(in srgb, var(--muted) 42%, transparent); background-clip:padding-box; }
  :global(*:hover::-webkit-scrollbar-thumb) { background:color-mix(in srgb, var(--muted) 68%, transparent); background-clip:padding-box; }
  :global(*::-webkit-scrollbar-corner) { background:transparent; }
  :global(svg.lucide) { stroke-width: .5px; }
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
  /* Monitter keeps focus visually quiet: interaction state comes from the
     surrounding control, never browser-provided focus rings or glows. */
  :global(:focus),
  :global(:focus-visible) {
    outline: none !important;
    box-shadow: none !important;
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
    position: relative;
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
    font-size: calc(14px * var(--interface-font-ratio, 1));
    letter-spacing: -0.02em;
  }
  .brand { height: var(--pane-tabbar-height,52px); box-sizing: border-box; }
  .native-mac .brand { height: var(--pane-tabbar-height); padding-left: calc(92px / var(--interface-scale,1)); padding-right: 8px; padding-top: calc(12px / var(--interface-scale,1)); gap: 4px; }
  .native-mac.native-fullscreen .brand { padding-left: 13px; }
  .native-mac.native-fullscreen.sidebar-collapsed { grid-template-columns: 56px minmax(0,1fr); }
  .native-mac .sidebar-views { gap: 0; }
  .native-mac .view-toggle { width: 22px; }
  .native-mac { --pane-tabbar-height: max(36px, calc(68px / var(--interface-scale, 1))); }
  .native-mac .topbar {
    height: var(--pane-tabbar-height);
    box-sizing: border-box;
    flex-shrink: 0;
    user-select: none;
    -webkit-user-select: none;
  }
  .native-mac .brand strong {
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
  .side-scroll {
    --scroll-fade: 20px;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: auto;
    overscroll-behavior: contain;
    padding: 11px 8px;
  }
  .sidebar-views { display: flex; flex: none; gap: 2px; margin-left: auto; }
  .view-toggle { display: grid; place-items: center; width: 24px; height: 26px; padding: 0; border-radius: 5px; color: var(--muted); }
  .view-toggle:hover { background: var(--soft); color: var(--ink); }
  .view-toggle[aria-pressed="true"] { color: var(--accent-ink); background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .view-hint { margin: 5px 7px 10px; color: var(--muted); font-size: calc(10px * var(--interface-font-ratio, 1)); line-height: 1.5; }
  .project-group { margin: 5px 0 12px; }
  .workspace-scopes { display:grid; gap:7px; margin:4px 0 14px; padding:0 2px 11px; border-bottom:1px solid var(--line); }
  .workspace-scopes .section-label { margin:0; }
  .workspace-scopes select { width:100%; min-width:0; padding:7px 8px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--panel); font:calc(12px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); }
  .workspace-approval-list { display:grid; gap:4px; }
  .workspace-approval { display:flex; align-items:center; gap:6px; min-width:0; padding:5px 4px; border:0; border-radius:5px; color:var(--ink); background:color-mix(in srgb,var(--accent) 8%,transparent); text-align:left; font-size:calc(10px * var(--interface-font-ratio, 1)); }
  .workspace-approval > span:last-child { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .rail-workspace-picker { width:44px; height:28px; padding:0; border:1px solid var(--line); border-radius:5px; color:var(--ink); background:var(--panel); font-size:10px; }
  .approval-badge { display:inline-flex; align-items:center; justify-content:center; margin-left:5px; min-width:16px; padding:1px 4px; border-radius:8px; color:var(--accent-ink); background:color-mix(in srgb,var(--accent) 18%,transparent); font-size:10px; }
  .agent-name[aria-pressed="true"] { color:var(--accent-ink); }
  .rail-approvals { display:grid; gap:4px; }
  .rail-approvals .workspace-approval { display:grid; place-items:center; width:28px; height:22px; padding:0; }
  .project-row { display: flex; align-items: center; gap: 1px; min-width: 0; border-radius: 5px; }
  .project-row.current { background: var(--paper); }
  .folder-toggle { display: grid; place-items: center; flex: none; width: 20px; height: 30px; color: var(--muted); }
  .project-name { display: flex; align-items: center; flex: 1; min-width: 0; gap: 6px; padding: 7px 0; text-align: left; font-size: calc(12px * var(--interface-font-ratio, 1)); }
  .project-name > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .project-name :global(svg) { flex: none; }
  .overview-head .eyebrow { display:flex; align-items:center; gap:6px; }
  .project-identity { display:grid; gap:8px; }.project-identity legend { color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.project-icon-options,.project-colour-options { display:flex; flex-wrap:wrap; gap:7px; }.project-icon-options button { display:grid; place-items:center; width:34px; height:34px; border:1px solid var(--line); border-radius:7px; color:var(--project-colour); background:var(--panel); }.project-icon-options button.selected { border-color:var(--project-colour); background:color-mix(in srgb,var(--project-colour) 14%,var(--panel)); }.project-colour-options > button { width:24px; height:24px; padding:0; border:2px solid transparent; border-radius:50%; background:var(--project-colour); }.project-colour-options > button.selected { border-color:var(--ink); outline:2px solid var(--panel); outline-offset:-4px; }.project-custom-colour { position:relative; display:grid; place-items:center; width:25px; height:25px; overflow:hidden; border:1px solid var(--line); border-radius:50%; }.project-custom-colour span { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); }.project-custom-colour input { position:absolute; inset:-6px; width:38px; height:38px; padding:0; border:0; background:transparent; cursor:pointer; }
  .project-name small, .unassigned-folder small { margin-left: auto; padding-right: 3px; font: calc(9px * var(--interface-font-ratio, 1)) var(--mono); color: var(--muted); }
  .project-row .quiet { flex: none; width: 21px; }
  .unassigned-folder { display: flex; align-items: center; gap: 5px; width: 100%; padding: 7px 3px; color: var(--muted); font-size: calc(11.5px * var(--interface-font-ratio, 1)); text-align: left; }
  .project-overview-actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .project-folders { display: grid; gap: 10px; margin: 20px 0; padding: 14px; border: 1px solid var(--line); border-radius: 8px; }
  .project-folders dt { display: flex; align-items: center; gap: 5px; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .project-folders dd { margin: 5px 0 0; overflow-wrap: anywhere; font: calc(12px * var(--interface-font-ratio, 1)) var(--mono); }
  .project-workspaces { display: grid; gap: 12px; }
  .project-workspaces h3 { margin: 0; font-size: calc(12px * var(--interface-font-ratio, 1)); }
  .project-workspaces p { margin: 0; }
  .task-workspace-preview { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 10px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted); }
  .task-workspace-preview > span { display: grid; gap: 5px; min-width: 0; }
  .task-workspace-preview b { font-size: calc(11px * var(--interface-font-ratio, 1)); }
  .task-workspace-preview code { overflow-wrap: anywhere; font: calc(11px * var(--interface-font-ratio, 1)) var(--mono); }
  .section-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 7px;
    color: var(--muted);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    background: var(--accent);
    font: calc(11px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .agent-avatar-toggle { position:relative; padding:0; border:0; overflow:hidden; cursor:pointer; }
  .avatar-toggle-overlay { position:absolute; inset:0; display:grid; place-items:center; background:rgba(0,0,0,.75); color:#fff; opacity:0; pointer-events:none; border-radius:inherit; }
  .agent-avatar-toggle:hover .avatar-toggle-overlay, .agent-avatar-toggle:focus-visible .avatar-toggle-overlay { opacity:1; }
  .task-tree[hidden] { display:none; }
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
    font-size: calc(12.5px * var(--interface-font-ratio, 1));
    font-weight: 600;
  }
  .agent-name small {
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
    font: calc(9.5px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .quiet {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 5px;
    color: var(--muted);
  }
  .agent-group { --thread-axis: 16.5px; }
  .agent-group > .agent-row { position: relative; }
  .agent-group.has-chats > .agent-row::after {
    content: ""; position: absolute; pointer-events: none;
    left: calc(var(--thread-axis) - .5px); width: 1px;
    top: calc(50% + 11.5px); bottom: 0; background: var(--line);
  }
  .task-tree { margin-left:10px; border-left:1px solid var(--line); padding-left:7px; }
  .agent-group > .task-tree { margin:0; border:0; padding:0; }
  .agent-group.has-chats > .task-tree { padding-bottom: 1em; }
  .agent-group > .task-tree > .task-row { position: relative; padding-left: calc(var(--thread-axis) - 3px); }
  .agent-group > .task-tree > .task-row::before {
    content: ""; position: absolute; pointer-events: none;
    left: calc(var(--thread-axis) - .5px); width: 1px;
    top: 0; bottom: 0; background: var(--line);
  }
  .agent-group > .task-tree > .task-row:last-child::before { bottom: 50%; }
  .agent-group > .task-tree .task-select > .dot { position: relative; z-index: 1; }
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
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
  .chat-meta { font-size: calc(9.5px * var(--interface-font-ratio, 1)); color: var(--muted); }
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
    font: calc(9px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .empty-tree {
    margin: 5px 7px;
    color: var(--muted);
    font-size: calc(11px * var(--interface-font-ratio, 1));
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
  }
  .side-empty :global(svg) {
    opacity: 0.6;
  }
  .text-button {
    color: var(--accent-ink);
    font-size: calc(12px * var(--interface-font-ratio, 1));
  }
  .pane-grid { display: flex; min-width: 0; min-height: 0; overflow: hidden; }
  .settings-surface { container-type:inline-size; flex:1; min-width:0; min-height:0; overflow:hidden; display:flex; }
  .settings-surface.settings-hidden { display:none; }
  .pane-expand-control { flex:none; }
  .overview-expand { order:99; flex:none; }
  .draft-expand { float:right; }
  .terminal-surface { display:flex; flex-direction:column; flex:1; min-height:0; overflow:hidden; background:var(--terminal-background,#090b0d); }
  .terminal-pane-header { display:flex; justify-content:flex-end; flex:none; padding:2px 10px; }
  .terminal-surface :global(.terminal-pane) { flex:1; height:auto; }
  .tab-expanded > .topbar { display:none; }
  .workspace {
    container: workspace-pane / inline-size;
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
  .sidebar-footer { position:absolute; bottom:0; left:0; right:0; z-index:3; display:flex; justify-content:space-around; align-items:center; height:48px; padding:4px 10px; box-sizing:border-box; border-top:1px solid var(--line); background:var(--sidebar); }
  .sidebar .side-scroll { padding-bottom:60px; }
  .sidebar .agent-rail { padding-bottom:156px; }
  .sidebar-collapsed .sidebar-footer { flex-direction:column; height:144px; padding:4px; }
  .brand.scrolled { box-shadow:inset 0 -1px var(--line); }
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
  }
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
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
    font-size: calc(13px * var(--interface-font-ratio, 1));
  }
  .loading :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .empty-pane { flex:1;min-width:0;min-height:0;overflow:auto;display:grid;place-items:center;padding:20px; }
  .pane-choices { display:flex;flex-wrap:wrap;justify-content:center;gap:16px; }
  .pane-choice { display:flex;align-items:center;gap:12px;padding:20px 24px;border:1px solid var(--line);border-radius:10px;background:var(--panel);color:var(--muted); }
  .pane-choice:hover { color:var(--ink);background:var(--soft);border-color:var(--accent); }
  .overview {
    --scroll-fade: 20px;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
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
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
    letter-spacing: 0.12em;
  }
  .overview h1,
  .conversation h1 {
    margin: 0;
    font-size: calc(28px * var(--interface-font-ratio, 1));
    font-weight: 600;
    letter-spacing: -0.045em;
  }
  .overview-head > div > p:last-child {
    max-width: 600px;
    margin: 9px 0 0;
    color: var(--muted);
    font-size: calc(13px * var(--interface-font-ratio, 1));
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
    font: 600 calc(13px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .onboarding h2 {
    margin: 0;
    font-size: calc(15px * var(--interface-font-ratio, 1));
  }
  .onboarding p {
    max-width: 600px;
    margin: 6px 0 14px;
    color: var(--muted);
    font-size: calc(13px * var(--interface-font-ratio, 1));
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
    font-weight: 600;
  }
  .status-group header span {
    padding: 2px 5px;
    border-radius: 3px;
    color: var(--muted);
    background: var(--soft);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .status-group > p {
    margin: 17px 15px;
    color: var(--muted);
    font-size: calc(12px * var(--interface-font-ratio, 1));
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .overview-task small,
  .overview-task > span:last-child {
    color: var(--muted);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    grid-template-rows: auto minmax(0, 1fr);
  }
  .task-layout.detail-hidden { grid-template-columns: minmax(0, 1fr); }
  .pane-task-header { grid-column: 1 / -1; grid-row: 1; }
  .task-layout > .conversation { grid-column: 1; grid-row: 2; }
  .task-layout > .run-detail { grid-column: 2; grid-row: 2; }
  .task-layout > .detail-tabs { grid-column: 2; grid-row: 1; }
  .task-layout:not(.detail-hidden):not(.compact-detail) .pane-task-header { grid-column: 1; }
  .conversation {
    --chat-content-max-width: 900px;
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
  }
  .tab.active {
    color: var(--ink);
    border-color: var(--line);
    background: color-mix(in srgb, var(--paper) 50%, transparent);
  }
  .tab-entry { position:relative; display: flex; align-items: stretch; flex-shrink: 0; border: 1px solid transparent; border-bottom: 0; border-radius: 6px 6px 0 0; }
  .tab-entry.active { color: var(--ink); border-color: var(--line); background: var(--paper); }
  .tab-entry.active .tab { color: var(--ink); }
  .tab span:last-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tab-entry .tab { max-width: 210px; padding-right:31px; }
  .close-tab { position:absolute; z-index:2; right:3px; top:50%; display:grid; place-items:center; width:22px; height:24px; transform:translateY(-50%); color:var(--muted); border-radius:4px; opacity:0; pointer-events:none; transition:opacity .12s ease; }
  .tab-entry:hover .close-tab, .tab-entry:focus-within .close-tab { opacity:1; pointer-events:auto; }
  .close-tab:hover, .tab:hover { background: var(--soft); }
  .tab.active:hover, .tab-entry.active .tab:hover { background: var(--paper); }
  .terminal-tab.active, .terminal-tab.active .tab:hover { background: var(--terminal-background); }
  .terminal-tab.active .tab, .terminal-tab.active .close-tab { color: var(--terminal-foreground); }
  .terminal-tab.active .close-tab:hover { background: #252a31; }
  .conversation-head {
    background: color-mix(in srgb, var(--paper) 50%, transparent);
    -webkit-backdrop-filter: blur(14px);
    backdrop-filter: blur(14px);
    display: flex;
    flex-shrink: 0;
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
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: calc(22px * var(--interface-font-ratio, 1));
  }
  .conversation-head h1 :global(.animated-title) { display: block; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .task-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: 7px;
  }
  .message {
    max-width: 100%;
    margin: 0 0 24px;
  }
  .pending-approvals { margin-bottom: 3px; }
  .approval-history { margin: 14px 0 24px; padding-top: 13px; border-top: 1px solid var(--line); }
  .approval-history h2 { margin: 0 0 7px; color: var(--muted); font: 600 calc(10px * var(--interface-font-ratio, 1)) var(--mono); letter-spacing: .06em; text-transform: uppercase; }
  .approval-history :global(.approval-request) { margin-bottom: 10px; }
  .message.user.tinted { background:color-mix(in srgb, var(--accent) 16%, var(--panel)); }
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
    font-weight: 600;
  }
  .message-meta time {
    color: var(--muted);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
    font-weight: 400;
  }
  .message :global(.markdown) {
    font-size: var(--chat-font-size, 13px);
    line-height: var(--chat-line-height, 1.65);
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
    font-size: calc(15px * var(--interface-font-ratio, 1));
  }
  .blank-conversation p {
    margin: 0;
    font-size: calc(12.5px * var(--interface-font-ratio, 1));
    line-height: 1.55;
  }
  .draft-layout { --scroll-fade:20px; -webkit-mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); flex: 1; min-height: 0; min-width: 0; overflow: auto; overscroll-behavior: contain; display:grid; align-content:center; padding:clamp(24px, 5vh, 60px) clamp(16px, 5vw, 64px); }
  .draft-content { width:100%; max-width:700px; margin:auto; }
  .draft-intro { margin-bottom: 22px; }
  .draft-intro h1 { margin: 8px 0; font-size: clamp(22px, 2.6vw, 32px); font-weight: 500; }
  .draft-intro > p:last-child { color: var(--muted); line-height: 1.6; }
  .draft-options { margin-bottom: 10px; }
  .draft-composer.composer { max-height: none; margin: 16px 0 12px; }
  .draft-composer.composer textarea { min-height:72px; }
  .draft-options label { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }
  .draft-select { appearance:none; -webkit-appearance:none; width:100%; padding:10px 34px 10px 11px; border:1px solid var(--line); border-radius:7px; color:var(--ink); background:var(--panel) linear-gradient(45deg,transparent 50%,var(--muted) 50%) calc(100% - 15px) 52% / 5px 5px no-repeat,linear-gradient(135deg,var(--muted) 50%,transparent 50%) calc(100% - 10px) 52% / 5px 5px no-repeat; font:inherit; }
  .task-workspace-editor { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.task-workspace-editor > span { display:flex; align-items:center; gap:6px; }.task-workspace-editor > div { display:flex; gap:6px; }.task-workspace-editor input { min-width:0; flex:1; padding:10px 11px; border:1px solid var(--line); border-radius:7px; outline:0; color:var(--ink); background:var(--panel); font:calc(12px * var(--interface-font-ratio, 1)) var(--mono); }.task-workspace-editor button { flex:none; }
  .suggestions { display: flex; gap: 7px; flex-wrap: wrap; }
  .suggestions button { border: 1px solid var(--line); border-radius: 7px; color: var(--muted); font-size: calc(12px * var(--interface-font-ratio, 1)); padding: 7px 10px; }
  .suggestions button:hover { border-color: var(--accent); color: var(--ink); }
  .draft-advanced { margin-top: 24px; color: var(--muted); font-size: calc(12px * var(--interface-font-ratio, 1)); }
  .draft-advanced summary { cursor: pointer; margin-bottom: 12px; }
  .slash-menu { position:fixed; inset:auto; margin:0; padding:0; overflow:hidden; border:1px solid var(--line); border-radius:10px; background:var(--panel); color:var(--ink); box-shadow:0 5px 20px #0002; animation:slash-rise 160ms ease-out; }
  .slash-options { max-height:min(240px,38vh); overflow-y:auto; overscroll-behavior:contain; }
  @keyframes slash-rise { from { clip-path:inset(100% -24px -24px); transform:translateY(6px); opacity:0; } to { clip-path:inset(-24px); transform:translateY(0); opacity:1; } }
  @media (prefers-reduced-motion:reduce) { .slash-menu { animation:none; } }

  .slash-menu .slash-caption { font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); text-transform: uppercase; letter-spacing: .06em; }
  .slash-menu button { width: 100%; display: flex; gap: 10px; text-align: left; padding: 9px 11px; }
  .slash-menu button.active, .slash-menu button:hover { background: color-mix(in srgb, var(--accent) 13%, transparent); }
  .slash-menu b { min-width: 78px; font: calc(12px * var(--interface-font-ratio, 1)) var(--mono); }
  .slash-menu span, .slash-menu p { color: var(--muted); font-size: calc(12px * var(--interface-font-ratio, 1)); margin: 0; padding: 9px 11px; }
  .composer {
    container-type: inline-size;
    flex-shrink: 0;
    max-height: 40%;
    overflow: auto;
    overscroll-behavior: contain;
    width: min(var(--chat-content-max-width), calc(100% - 2 * var(--chat-side-padding, clamp(25px, 4vw, 50px))));
    box-sizing: border-box;
    margin: 0 auto 20px;
    padding: 11px 12px 9px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--panel);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.18);
  }
  .composer textarea {
    font-family: var(--chat-font, "IBM Plex Sans", system-ui, sans-serif);
    display: block;
    width: 100%;
    min-height: 52px;
    max-height: 25vh;
    resize: vertical;
    border: 0;
    outline: 0;
    color: var(--ink);
    background: transparent;
    font-size: var(--chat-font-size, 13px);
    line-height: var(--chat-line-height, 1.65);
  }
  .composer :global(textarea:focus-visible) { outline: none; box-shadow: none; }
  .composer textarea::placeholder {
    color: var(--muted);
  }
  .composer-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--muted);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .recipient-picker button.selected {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--line));
    color: var(--accent-ink);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .run-detail {
    position: relative;
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
    align-items: flex-end;
    gap: 3px;
    min-height: var(--pane-tabbar-height, 52px);
    height: auto;
    align-self: stretch;
    padding: 0.5em 0.5em 0;
    /* Same physical rule as the pane header, so both surfaces meet cleanly. */
    border-bottom: 1px solid var(--line);
    background: var(--paper);
  }
  .detail-section h3 {
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
    letter-spacing: 0.08em;
  }
  .detail-tab-entry {
    display: flex;
    align-self: flex-end;
    align-items: stretch;
    flex-shrink: 0;
    margin-bottom: -1px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 6px 6px 0 0;
  }
  .detail-tab {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    min-height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: 5px 5px 0 0;
    color: var(--muted);
    font-size: calc(10.5px * var(--interface-font-ratio, 1));
  }
  .detail-tab:hover { background: var(--soft); color: var(--ink); }
  .detail-tab-entry.active { position:relative; z-index:1; margin-bottom:-1px; color: var(--ink); border-color: var(--line); background: var(--sidebar); }
  .detail-tab-entry.active .detail-tab { color: var(--ink); }
  .detail-tab-entry.active .detail-tab:hover { background: var(--sidebar); }
  .detail-tabs .detail-close {
    width: 30px;
    min-height: 30px;
    margin: 0 0 0 auto;
    padding: 0;
    border: 0;
    border-radius: 6px;
    color: var(--accent-ink);
    align-self: center;
  }
  .channel-members { display:flex;flex-direction:column;overflow:hidden;padding:0; }
  .run-detail.closed, .hidden { display: none; }
  .git-slot { flex: 1; min-height: 0; overflow: hidden; }
  .detail-scroll {
    --scroll-fade: 20px;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0, #000 var(--scroll-fade), #000 calc(100% - var(--scroll-fade)), transparent 100%);
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
    font: calc(10.5px * var(--interface-font-ratio, 1)) var(--mono);
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .delegated small {
    color: var(--muted);
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
  }
  .form label small {
    line-height: 1.45;
  }
  .optional {
    justify-self: end;
    margin-top: -18px;
    color: var(--muted);
    font: calc(9.5px * var(--interface-font-ratio, 1)) var(--mono);
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
    font-size: calc(12.5px * var(--interface-font-ratio, 1));
  }
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
    font-size: calc(12px * var(--interface-font-ratio, 1));
  }
  .modal-copy {
    margin: 0;
    color: var(--muted);
    font-size: calc(12.5px * var(--interface-font-ratio, 1));
    line-height: 1.55;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 0;
    color: var(--muted);
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
    text-transform: capitalize;
  }
  .segmented button.chosen {
    color: var(--ink);
    background: var(--panel);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  }
  .form details {
    color: var(--muted);
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
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
    font-size: calc(11.5px * var(--interface-font-ratio, 1));
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
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
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
    font-size: calc(12.5px * var(--interface-font-ratio, 1));
  }
  .host-card small {
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
    font: calc(10px * var(--interface-font-ratio, 1)) var(--mono);
  }
  .add-host {
    justify-self: start;
    margin-top: 4px;
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
    .overview-head { flex-wrap: wrap; }
    .conversation-head { padding: 20px; }
    .composer-footer { flex-wrap: wrap; }
  }
  @media (max-width: 640px) {
    .app-shell { grid-template-columns: 150px minmax(0, 1fr); }
    .sidebar { min-width: 0; }
    .brand { padding-right: 5px; gap: 3px; }
    .brand strong { font-size: calc(13px * var(--interface-font-ratio, 1)); }
    .conversation-head, .overview { padding: 12px; }
    .conversation-head { gap: 8px; }
    .conversation-head h1 { font-size: calc(18px * var(--interface-font-ratio, 1)); }
    .composer { margin: 0 10px 10px; }
    .task-actions { flex-wrap: wrap; }
    .form-grid { grid-template-columns: minmax(0, 1fr); }
  }
  @container workspace-pane (width < 1000px) {
    .dashboard-overview { padding: 20px; }
    .conversation { --chat-side-padding: 20px; }
    .conversation-head { padding-left: 20px; padding-right: 20px; }
    .dashboard-overview .overview-head, .dashboard-overview .overview-grid { max-width: none; }
  }
  @media (max-height: 500px) {
    .conversation-head { padding-top: 10px; padding-bottom: 10px; gap: 6px; }
    .task-heading { padding-top: 10px; }
    .composer { padding: 7px; margin-bottom: 8px; }
    .composer textarea { min-height: 36px; }
  }

  .avatar img { width: 100%; height: 100%; object-fit: cover; border-radius: inherit; }
  .task-heading { align-items: center; padding-top: 13px; padding-bottom: 13px; }
  .task-heading > h1 { margin: 0; }
  .tabs.hide-tab-close .close-tab { display:none; }
  .tabs { counter-reset: tab-index; }
  .tabs > .tab-entry { counter-increment: tab-index; }
  .tabs.show-tab-index > .tab-entry::after { content: counter(tab-index); position:absolute; right:5px; top:50%; transform:translateY(-50%); z-index:3; min-width:18px; height:18px; display:grid; place-items:center; border-radius:4px; background:var(--panel); color:var(--accent-ink); border:1px solid var(--line); font:11px var(--mono); pointer-events:none; }
  .task-title { display: inline-flex; min-width: 0; align-items: center; gap: 5px; }
  .task-title-edit { flex: none; opacity: 0; color: var(--muted); transition: opacity .12s ease, color .12s ease; }
  .task-heading:hover .task-title-edit, .task-title:focus-within .task-title-edit { opacity: 1; }
  .task-title-edit:hover { color: var(--ink); }
  .task-overflow { position: relative; }
  .task-menu { display: grid; min-width: 155px; }
  .task-menu button { display: flex; gap: 7px; align-items: center; padding: 7px; text-align: left; }
  .agent-identity { margin: 0 0 14px; border-bottom: 1px solid var(--line); padding-bottom: 12px; }
  .agent-identity summary { display: flex; gap: 9px; align-items: center; cursor: pointer; list-style: none; }
  .agent-identity summary::-webkit-details-marker { display: none; }
  .identity-avatar { width: 32px; height: 32px; }.identity-avatar-button { position:relative; padding:0; border:0; cursor:pointer; overflow:hidden; }.avatar-edit-overlay { position:absolute; inset:0; display:grid; place-items:center; border-radius:inherit; color:#fff; background:rgba(0,0,0,.75); opacity:0; transition:opacity .15s ease; }.identity-avatar-button:hover .avatar-edit-overlay { opacity:1; }
  .agent-identity b, .agent-identity small { display: block; }
  .agent-identity small { color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); margin-top: 2px; }
  .identity-actions { padding: 9px 0 0 41px; }
  .identity-actions p { margin: 6px 0 0; color: var(--muted); font-size: calc(11px * var(--interface-font-ratio, 1)); }
  .avatar-preview { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
  .avatar-preview img { width: 34px; height: 34px; border-radius: 7px; object-fit: cover; }

  .floating-panel { position: fixed; inset: auto; z-index: 50; margin: 0; box-sizing: border-box; overflow: auto; overscroll-behavior: contain; padding: 5px; border: 1px solid var(--line); border-radius: 9px; color: var(--ink); background: var(--panel); box-shadow: 0 12px 30px #0003; }
  .app-shell.sidebar-collapsed { grid-template-columns: 56px minmax(0,1fr); }
  .sidebar-collapsed .sidebar { min-width: 0; }
  .sidebar-collapsed .brand { justify-content: center; padding: 0; height: var(--pane-tabbar-height,52px); }
  .native-mac.sidebar-collapsed { grid-template-columns: max(56px,calc(124px / var(--interface-scale,1))) minmax(0,1fr); }
  .agent-rail { display: flex; align-items: center; gap: 8px; flex-direction: column; flex: 1; min-height: 0; overflow-y: auto; padding: 10px 4px; }
  .rail-avatar { position: relative; flex: none; padding: 4px; border: 1px solid transparent; border-radius: 9px; }
  .rail-avatar.current, .rail-avatar:hover { border-color: var(--line); background: var(--soft); }
  .rail-avatar .avatar { width: 30px; height: 30px; font-size: calc(12px * var(--interface-font-ratio, 1)); }
  .rail-running { position: absolute; width: 6px; height: 6px; border: 2px solid var(--sidebar); border-radius: 50%; background: var(--accent); right: 0; bottom: 0; }
  .rail-chats { width: 320px; }
  .rail-chats header { display: flex; align-items: center; justify-content: space-between; padding: 4px 8px; font-size: calc(13px * var(--interface-font-ratio, 1)); }
  .rail-chat-list { max-height: min(50vh,420px); overflow: auto; }
  .rail-new-chat { display: flex; gap: 7px; align-items: center; width: 100%; padding: 10px; color: var(--accent-ink); font-size: calc(12px * var(--interface-font-ratio, 1)); }

  .agent-profile { display: grid; gap: 9px; margin: 4px 0; padding: 10px; border: 1px solid var(--line); border-radius: 7px; }
  .agent-settings-form { max-width:none; }
  .agent-editor-selector { display:flex;align-items:end;gap:12px;flex-wrap:wrap; }
  .agent-editor-selector label { display:grid;gap:6px;flex:1;min-width:140px; }
  .agent-editor-selector select { width:100%; }
  .agent-profile summary { cursor: pointer; font-weight: 600; }
  .agent-profile p { margin: 0; color: var(--muted); font-size: calc(11px * var(--interface-font-ratio, 1)); }
  .collaboration-row { display: flex; width: 100%; gap: 7px; padding: 7px 0; text-align: left; border-bottom: 1px solid var(--line); }
  .collaboration-row > span:last-child { display: grid; min-width: 0; gap: 2px; }
  .collaboration-row small, .collaboration-row em { overflow: hidden; color: var(--muted); text-overflow: ellipsis; white-space: nowrap; font-size: calc(10px * var(--interface-font-ratio, 1)); font-style: normal; }
  .collaboration-row .collaboration-error { color: var(--danger, #c44c79); }
  .agent-directory article { display: flex; gap: 8px; align-items: center; padding: 9px 0; border-bottom: 1px solid var(--line); }
  .agent-directory article > div { display: grid; flex: 1; min-width: 0; gap: 2px; }
  .agent-directory small, .agent-directory p { margin: 0; color: var(--muted); font-size: calc(10px * var(--interface-font-ratio, 1)); }
  .agent-directory article.disabled { opacity: .58; }
  .app-shell.embedded { height: 100%; width: 100%; grid-template-columns: minmax(0,1fr); }
  .embedded .topbar { height: var(--pane-tabbar-height,52px); min-height: 32px; padding: 0.5em 0.5em 0; }
  .compact-detail .run-detail { position: absolute; right: 0; top: 0; bottom: 0; width: min(340px,calc(100% - 24px)); z-index: 12; box-shadow: -10px 0 30px #0003; animation: detail-enter .18s ease-out; }
  .detail-backdrop { position: absolute; inset: 0; z-index: 11; background: #0002; }
  @keyframes detail-enter { from { transform: translateX(100%); } to { transform: translateX(0); } }
  .app-shell:not(.embedded):not(.sidebar-collapsed) { grid-template-columns: min(40vw, max(230px, var(--left-sidebar-width, 252px))) minmax(0, 1fr); }
  .task-layout:not(.detail-hidden) { grid-template-columns: minmax(0, 1fr) min(40vw, max(260px, var(--right-sidebar-width, 292px)), calc(100% - 300px)); }
  .compact-detail .run-detail { width: min(40vw, max(260px, var(--right-sidebar-width, 340px)), calc(100% - 24px)); }
  @media (prefers-reduced-motion: reduce) { .compact-detail .run-detail { animation: none; } }
  .composer-right { display:flex;align-items:center;gap:8px;min-width:0; }
  .composer-left { display:flex;align-items:center;gap:8px;min-width:0; }
  .agent-waiting { display:flex; align-items:center; gap:8px; margin:8px 0 24px; color:var(--muted); font-family:var(--chat-font,"IBM Plex Sans",system-ui,sans-serif); font-size:var(--chat-font-size,13px); }
  .waiting-spinner { display:inline-flex; flex:none; color:var(--accent-ink); animation:waiting-turn 1.4s linear infinite; }
  @keyframes waiting-turn { to { transform:rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .waiting-spinner { animation:none; } }
  .message-avatar { width:20px;height:20px;flex-shrink:0;border-radius:5px;font-size:calc(10px * var(--interface-font-ratio, 1)); }
  .human-avatar { background:var(--accent); color:var(--on-accent); }
  .attachment-tools { display: flex; align-items: center; gap: 7px; color: var(--muted); }
  .attachment-tools .icon { width: 24px; height: 24px; }
  .attachment-tools small { font-size: calc(10px * var(--interface-font-ratio, 1)); }
  .attachment-input { display: none; }
  .composer.drop-files { outline: 2px solid var(--accent); background: color-mix(in srgb,var(--accent) 8%,var(--panel)); }
  .vim-commandbar { position: fixed; z-index: 80; left: 50%; bottom: 20px; width: min(540px,calc(100vw - 32px)); transform: translateX(-50%); padding: 8px; border: 1px solid var(--line); border-radius: 8px; color: var(--ink); background: var(--panel); box-shadow: 0 12px 30px #0004; }
  .vim-commandbar form { display: flex; align-items: center; gap: 6px; }
  .vim-commandbar label { display: flex; min-width: 0; flex: 1; align-items: center; gap: 5px; color: var(--accent-ink); font: 600 calc(14px * var(--interface-font-ratio,1)) var(--mono); }
  .vim-commandbar input { min-width: 0; flex: 1; border: 0; outline: 0; color: var(--ink); background: transparent; font: inherit; }
  .vim-commandbar p { margin: 7px 2px 0; color: var(--danger,#b84c44); font-size: calc(11px * var(--interface-font-ratio,1)); }
  .vim-commandbar ul { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 4px 10px; margin: 8px 2px 1px; padding: 0; list-style: none; color: var(--muted); font: calc(10px * var(--interface-font-ratio,1)) var(--mono); }
  @media (prefers-reduced-motion: reduce) { .vim-commandbar { transition: none; } }
</style>
