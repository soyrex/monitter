import { invoke as nativeInvoke } from "@tauri-apps/api/core";
import { isLanBrowser, lanInvoke } from './lan';
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Agent,
  AcpCandidate,
  AcpLaunch,
  AcpProbeResult,
  Goal,
  Channel,
  CreateTaskInput,
  HandoffTaskInput,
  Host,
  ProbeResult,
  Project,
  Settings,
  Snapshot,
  Task,
  GitDiffScope,
  TaskGitStatus,
  TaskGitDiff,
  TaskDeletionPreview,
  ModelSettings,
  ModelTarget,
  ModelCatalog,
  TerminalTarget,
  TerminalSession,
  TerminalRead,
  AutonameTarget,
  Attachment,
  AttachmentTarget,
  AttachmentFileData,
  CodexAccount,
  Sandbox,
  RunEvent,
  UiSnapshotResponse,
  TaskEventsPage,
  ProcessMetricsSample,
  EventDetailChunk,
  MarkdownDocument,
  SendAccepted,
  ExtensionConfig,
  EnvironmentSecretsConfig,
  ApprovalDecision,
  UsageOverview,
  UsageRefreshPolicy,
  SubagentTranscriptEntry,
  SlashCommand,
  SlashCommandExecution,
} from "./types";

export interface MonitterBridge {
  available: boolean;
  getSnapshot(): Promise<Snapshot>;
  getTaskEvents(taskId: string, before?: number, limit?: number): Promise<TaskEventsPage>;
  getUsageOverview(policy?: UsageRefreshPolicy): Promise<UsageOverview>;
  listCodexAccounts(): Promise<CodexAccount[]>;
  getProcessMetrics(): Promise<ProcessMetricsSample>;
  getTaskEventDetail(taskId: string, eventId: string, offset?: number, limit?: number): Promise<EventDetailChunk>;
  readMarkdownFile(taskId: string, href: string, basePath?: string): Promise<MarkdownDocument>;
  getSubagentTranscript(taskId: string, subagentId: string): Promise<SubagentTranscriptEntry[]>;
  saveHost(host: Host): Promise<Snapshot>;
  deleteHost(id: string): Promise<Snapshot>;
  probeHost(host: Host): Promise<ProbeResult>;
  discoverAcpAgents(hostId: string): Promise<AcpCandidate[]>;
  verifyAcpAgent(hostId: string, launch: AcpLaunch): Promise<AcpProbeResult>;
  saveAgent(agent: Agent): Promise<Snapshot>;
  deleteAgent(id: string, chatHandling?: 'archive' | 'delete'): Promise<Snapshot>;
  createTask(input: CreateTaskInput): Promise<Task>;
  handoffTask(input: HandoffTaskInput): Promise<Task>;
  chooseLocalFolder(initial?: string): Promise<string | null>;
  renameTask(id: string, title: string): Promise<Snapshot>;
  autoname(target: AutonameTarget): Promise<Snapshot>;
  deleteTask(id: string): Promise<Snapshot>;
  setTaskArchived(taskId: string, archived: boolean): Promise<Snapshot>;
  saveProject(project: Project): Promise<Snapshot>;
  deleteProject(id: string): Promise<Snapshot>;
  setTaskProject(taskId: string, projectId: string | null): Promise<Snapshot>;
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot | SendAccepted>;
  clearTaskContext(taskId: string): Promise<Snapshot>;
  cancelQueuedMessage(id: string): Promise<Snapshot>;
  editQueuedMessage(id: string, text: string): Promise<Snapshot>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resolveApproval(approvalId: string, decision: ApprovalDecision): Promise<Snapshot>;
  revokeApprovalRule(ruleId: string): Promise<Snapshot>;
  resolveInput(approvalId: string, response: unknown): Promise<Snapshot>;
  saveSettings(settings: Settings): Promise<Snapshot>;
  getExtensionConfig(): Promise<ExtensionConfig>;
  saveExtensionConfig(config: ExtensionConfig): Promise<ExtensionConfig>;
  listEnvironmentSecrets(): Promise<EnvironmentSecretsConfig>;
  setEnvironmentSecret(revision: string, name: string, value: string, description: string): Promise<EnvironmentSecretsConfig>;
  deleteEnvironmentSecret(revision: string, name: string): Promise<EnvironmentSecretsConfig>;
  saveChannel(channel: Channel): Promise<Snapshot>;
  setChannelAgentConversation(channelId: string, enabled: boolean, turnLimit: number): Promise<Snapshot>;
  stopChannelAgentConversation(channelId: string): Promise<Snapshot>;
  setChannelMembership(channelId: string, agentId: string, member: boolean): Promise<Snapshot>;
  sendChannelMessage(
    channelId: string,
    text: string,
    agentIds: string[],
    attachmentIds?: string[],
  ): Promise<Snapshot | SendAccepted>;
  resumeTask(taskId: string): Promise<Snapshot>;
  listTerminals(): Promise<TerminalSession[]>;
  openTerminal(target: TerminalTarget, cols: number, rows: number): Promise<TerminalSession>;
  writeTerminal(id: string, data: string): Promise<void>;
  resizeTerminal(id: string, cols: number, rows: number): Promise<void>;
  readTerminal(id: string, afterSeq: number): Promise<TerminalRead>;
  closeTerminal(id: string): Promise<void>;
  getModelCatalog(target: ModelTarget): Promise<ModelCatalog>;
  setTaskModelSettings(taskId: string, settings: ModelSettings): Promise<Snapshot>;
  setTaskSandbox(taskId: string, sandbox: Sandbox): Promise<Snapshot>;
  getTaskGoal(taskId: string): Promise<Goal | null>;
  getTaskSlashCommands(taskId: string): Promise<SlashCommand[]>;
  executeTaskSlashCommand(taskId: string, command: string): Promise<SlashCommandExecution>;
  clearTaskGoal(taskId: string): Promise<void>;
  getTaskGitStatus(taskId: string, detectorSession?: string): Promise<TaskGitStatus>;
  waitForTaskGitMarker(taskId: string, detectorSession?: string): Promise<'found' | 'timeout' | 'already-present'>;
  getTaskGitDiff(taskId: string, path: string, scope: GitDiffScope): Promise<TaskGitDiff>;
  previewTaskDeletion(taskId: string): Promise<TaskDeletionPreview>;
  deleteArchivedTask(taskId: string, removeNativeFiles: boolean): Promise<Snapshot>;
  storeAttachment(target: AttachmentTarget, file: AttachmentFileData, previewDataUrl?: string | null, sourceId?: string): Promise<Attachment>;
  readAttachmentFile(sourcePath: string): Promise<AttachmentFileData>;
  readAttachmentImage(attachmentId: string): Promise<AttachmentFileData>;
  onChanged(handler: () => void): Promise<UnlistenFn>;
}

export interface TestBridge {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
  listen(event: string, handler: () => void): Promise<UnlistenFn>;
}

declare global {
  interface Window {
    /** Test-only browser bridge. The native application never supplies this. */
    __MONITTER_TEST_BRIDGE__?: TestBridge;
    /** Legacy test fixture alias, retained for browser QA compatibility. */
    __MONITTER_BRIDGE__?: MonitterBridge;
  }
}

function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return isLanBrowser() ? lanInvoke<T>(command, args) : nativeInvoke<T>(command, args);
}

function isUnknownCommand(error: unknown) {
  const value = String(error).toLowerCase();
  return value.includes('unknown command') || value.includes('unknown invoke command') ||
    value.includes('command not found') || value.includes('command `get_ui_snapshot`') ||
    value.includes('command get_ui_snapshot') || value.includes('command `send_message_fast`') ||
    value.includes('command send_message_fast') || value.includes('command `send_channel_message_fast`') ||
    value.includes('command send_channel_message_fast') ||
    value.includes("lan command 'get_ui_snapshot' is not available");
}

// The installed desktop app can lag the web bundle. Only an explicit missing
// command is safe to retry with the legacy protocol: mutations must never be
// replayed after an arbitrary transport or server failure.
let uiProtocol: 'unknown' | 'revisioned' | 'legacy' = 'unknown';
let cachedSnapshot: Snapshot | null = null;
let cachedRevision: string | undefined;
let snapshotRequest: Promise<Snapshot> | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | undefined;
const changedSubscribers = new Set<() => void>();

function rememberSnapshot(snapshot: Snapshot, revision?: string) {
  cachedSnapshot = snapshot;
  if (revision !== undefined) cachedRevision = revision;
  return snapshot;
}

async function getRevisionedSnapshot(): Promise<Snapshot> {
  const result = await invoke<UiSnapshotResponse>('get_ui_snapshot', cachedRevision ? { revision: cachedRevision } : {});
  uiProtocol = 'revisioned';
  return result.snapshot ? rememberSnapshot(result.snapshot, result.revision) : (cachedSnapshot ?? (() => { throw new Error('Monitter reported an unchanged snapshot before a snapshot was loaded.'); })());
}

function getCachedSnapshot(): Promise<Snapshot> {
  if (snapshotRequest) return snapshotRequest;
  snapshotRequest = (async () => {
    if (uiProtocol === 'legacy') return rememberSnapshot(await invoke<Snapshot>('get_snapshot'));
    try { return await getRevisionedSnapshot(); }
    catch (reason) {
      if (!isUnknownCommand(reason)) throw reason;
      uiProtocol = 'legacy';
      return rememberSnapshot(await invoke<Snapshot>('get_snapshot'));
    }
  })().finally(() => { snapshotRequest = null; });
  return snapshotRequest;
}

function scheduleSnapshotRefresh() {
  // Coalesce bursts without continually pushing the refresh out forever.
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined;
    // The subscribed UI owns the one coalesced read. Fetching here and then
    // asking it to reload would double every LAN/native change notification.
    changedSubscribers.forEach(handler => handler());
  }, 100);
}

async function chooseSendProtocol(): Promise<'fast' | 'legacy'> {
  // Probe once, before any mutation. Established revisioned sessions already
  // have the capability and cache, so a send never waits for a snapshot read.
  if (uiProtocol === 'legacy') return 'legacy';
  if (uiProtocol === 'revisioned' && cachedSnapshot) return 'fast';
  await getCachedSnapshot();
  return uiProtocol === 'revisioned' && cachedSnapshot ? 'fast' : 'legacy';
}

async function fastSend(command: 'send_message_fast' | 'send_channel_message_fast', args: Record<string, unknown>): Promise<SendAccepted> {
  const receipt = await invoke<SendAccepted>(command, args);
  scheduleSnapshotRefresh();
  return receipt;
}

async function sendMessageWithProtocol(taskId: string, text: string, attachmentIds: string[]): Promise<Snapshot | SendAccepted> {
  const protocol = await chooseSendProtocol();
  if (protocol === 'legacy') return rememberSnapshot(await invoke<Snapshot>('send_message', { taskId, text, attachmentIds }));
  // No catch here: after invoking a mutation, even a command-looking error is
  // ambiguous and must never cause a duplicate legacy send.
  return fastSend('send_message_fast', { taskId, text, attachmentIds });
}

async function sendChannelMessageWithProtocol(channelId: string, text: string, agentIds: string[], attachmentIds: string[]): Promise<Snapshot | SendAccepted> {
  const protocol = await chooseSendProtocol();
  if (protocol === 'legacy') return rememberSnapshot(await invoke<Snapshot>('send_channel_message', { channelId, text, agentIds, attachmentIds }));
  // See direct-send equivalent: mutation errors are surfaced as-is.
  return fastSend('send_channel_message_fast', { channelId, text, agentIds, attachmentIds });
}

const nativeBridge: MonitterBridge = {
  available:
    typeof window !== "undefined" &&
    (Boolean((window as any).__TAURI_INTERNALS__) || isLanBrowser()),
  getSnapshot: () => getCachedSnapshot(),
  getTaskEvents: (taskId, before, limit) => invoke<TaskEventsPage>('get_task_events', { taskId, ...(before === undefined ? {} : { before }), ...(limit === undefined ? {} : { limit }) }),
  getUsageOverview: (policy) => invoke<UsageOverview>('get_usage_overview', policy === undefined ? {} : { policy }),
  listCodexAccounts: () => invoke<CodexAccount[]>('list_codex_accounts'),
  getProcessMetrics: () => invoke<ProcessMetricsSample>('get_process_metrics'),
  getTaskEventDetail: (taskId, eventId, offset, limit) => invoke<EventDetailChunk>('get_task_event_detail', { taskId, eventId, ...(offset === undefined ? {} : { offset }), ...(limit === undefined ? {} : { limit }) }),
  readMarkdownFile: (taskId, href, basePath) => isLanBrowser() ? desktopOnly() : invoke<MarkdownDocument>('read_markdown_file', { taskId, href, ...(basePath === undefined ? {} : { basePath }) }),
  getSubagentTranscript: (taskId, subagentId) => invoke<SubagentTranscriptEntry[]>('get_subagent_transcript', { taskId, subagentId }),
  saveHost: (host) => invoke<Snapshot>("save_host", { host }),
  deleteHost: (id) => invoke<Snapshot>("delete_host", { id }),
  probeHost: (host) => invoke<ProbeResult>("probe_host", { host }),
  discoverAcpAgents: (hostId) => invoke<AcpCandidate[]>('discover_acp_agents', { hostId }),
  verifyAcpAgent: (hostId, launch) => invoke<AcpProbeResult>('verify_acp_agent', { hostId, launch }),
  saveAgent: (agent) => invoke<Snapshot>("save_agent", { agent }),
  deleteAgent: (id, chatHandling: 'archive' | 'delete' = 'archive') =>
    invoke<Snapshot>("delete_agent", { id, chatHandling }),
  createTask: (input) => invoke<Task>("create_task", { input }),
  handoffTask: (input) => invoke<Task>("handoff_task", { input }),
  chooseLocalFolder: (initial = '') => isLanBrowser() ? desktopOnly() : invoke<string | null>("choose_local_folder", { initial }),
  renameTask: (id, title) => invoke<Snapshot>("rename_task", { id, title }),
  autoname: target => invoke<Snapshot>("autoname", { target }),
  deleteTask: (id) => invoke<Snapshot>("delete_task", { id }),
  setTaskArchived: (taskId, archived) => invoke<Snapshot>("set_task_archived", {taskId, archived}),
  saveProject: project => invoke<Snapshot>("save_project", {project}),
  deleteProject: id => invoke<Snapshot>("delete_project", {id}),
  setTaskProject: (taskId, projectId) => invoke<Snapshot>("set_task_project", {taskId, projectId}),
  sendMessage: (taskId, text, attachmentIds = []) => sendMessageWithProtocol(taskId, text, attachmentIds),
  clearTaskContext: (taskId) => invoke<Snapshot>('clear_task_context', { taskId }),
  cancelQueuedMessage: (id) => invoke<Snapshot>("cancel_queued_message", { id }),
  editQueuedMessage: (id, text) => invoke<Snapshot>("edit_queued_message", { id, text }),
  cancelTask: (taskId) => invoke<Snapshot>("cancel_task", { taskId }),
  resolveApproval: (approvalId, decision) => invoke<Snapshot>("resolve_approval", { approvalId, decision }),
  revokeApprovalRule: (ruleId) => invoke<Snapshot>("revoke_approval_rule", { ruleId }),
  resolveInput: (approvalId, response) => invoke<Snapshot>("resolve_input", { approvalId, response }),
  saveSettings: (settings) => invoke<Snapshot>("save_settings", { settings }),
  getExtensionConfig: () => isLanBrowser() ? desktopOnly() : invoke<ExtensionConfig>('get_extension_config'),
  saveExtensionConfig: (config) => isLanBrowser() ? desktopOnly() : invoke<ExtensionConfig>('save_extension_config', { config }),
  listEnvironmentSecrets: () => isLanBrowser() ? desktopOnly() : invoke<EnvironmentSecretsConfig>('list_environment_secrets'),
  setEnvironmentSecret: (revision, name, value, description) => isLanBrowser() ? desktopOnly() : invoke<EnvironmentSecretsConfig>('set_environment_secret', { revision, name, value, description }),
  deleteEnvironmentSecret: (revision, name) => isLanBrowser() ? desktopOnly() : invoke<EnvironmentSecretsConfig>('delete_environment_secret', { revision, name }),
  saveChannel: (channel) => invoke<Snapshot>("save_channel", { channel }),
  setChannelAgentConversation: (channelId, enabled, turnLimit) => invoke<Snapshot>("set_channel_agent_conversation", {channelId, enabled, turnLimit}),
  stopChannelAgentConversation: (channelId) => invoke<Snapshot>("stop_channel_agent_conversation", {channelId}),
  setChannelMembership: (channelId, agentId, member) =>
    invoke<Snapshot>("set_channel_membership", { channelId, agentId, member }),
  sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) => sendChannelMessageWithProtocol(channelId, text, agentIds, attachmentIds),
  resumeTask: (taskId) => invoke<Snapshot>("resume_task", { taskId }),
  listTerminals: () => invoke<TerminalSession[]>('list_terminals'),
  openTerminal: (target, cols, rows) => invoke<TerminalSession>('open_terminal', {target,cols,rows}),
  writeTerminal: (id, data) => invoke<void>('write_terminal', {id,data}),
  resizeTerminal: (id, cols, rows) => invoke<void>('resize_terminal', {id,cols,rows}),
  readTerminal: (id, afterSeq) => invoke<TerminalRead>('read_terminal', {id,afterSeq}),
  closeTerminal: id => invoke<void>('close_terminal', {id}),
  getModelCatalog: target => invoke<ModelCatalog>("get_model_catalog", {target}),
  setTaskModelSettings: (taskId, settings) => invoke<Snapshot>("set_task_model_settings", {taskId,settings}),
  setTaskSandbox: (taskId, sandbox) => invoke<Snapshot>('set_task_sandbox', {taskId,sandbox}),
  getTaskGoal: taskId => invoke<Goal | null>("get_task_goal", { taskId }),
  getTaskSlashCommands: taskId => invoke<SlashCommand[]>("get_task_slash_commands", { taskId }),
  executeTaskSlashCommand: (taskId, command) => invoke<SlashCommandExecution>("execute_task_slash_command", { taskId, command }),
  clearTaskGoal: taskId => invoke<void>("clear_task_goal", { taskId }),
  getTaskGitStatus: (taskId, detectorSession = '') => invoke<TaskGitStatus>("get_task_git_status", { taskId, detectorSession }),
  waitForTaskGitMarker: (taskId, detectorSession = '') => invoke<'found' | 'timeout' | 'already-present'>("wait_for_task_git_marker", { taskId, detectorSession }),
  getTaskGitDiff: (taskId, path, scope) => invoke<TaskGitDiff>("get_task_git_diff", { taskId, path, scope }),
  previewTaskDeletion: taskId => invoke<TaskDeletionPreview>("preview_task_deletion", { taskId }),
  deleteArchivedTask: (taskId, removeNativeFiles) => invoke<Snapshot>("delete_archived_task", { taskId, removeNativeFiles }),
  storeAttachment: (target, file, previewDataUrl = null, sourceId) => invoke<Attachment>("store_attachment", {target, ...file, previewDataUrl, sourceId}),
  readAttachmentFile: sourcePath => isLanBrowser() ? desktopOnly() : invoke<AttachmentFileData>("read_attachment_file", {sourcePath}),
  readAttachmentImage: attachmentId => invoke<AttachmentFileData>('read_attachment_image', {attachmentId}),
  onChanged: async (handler) => {
    changedSubscribers.add(handler);
    const changed = () => scheduleSnapshotRefresh();
    if (!isLanBrowser()) {
      const unlisten = await listen("monitter:changed", changed);
      return () => { changedSubscribers.delete(handler); unlisten(); };
    }
    const timer = setInterval(changed, 1500);
    return () => { changedSubscribers.delete(handler); clearInterval(timer); };
  },
};

const emptyPreviewSnapshot = (): Snapshot => ({
  hosts: [],
  agents: [],
  tasks: [],
  messages: [],
  events: [],
  channels: [],
  projects: [],
  collaborations: [],
  queuedMessages: [],
  approvalRequests: [],
  approvalRules: [],
  settings: { accent: "#3f9d6a", theme: "system", interfaceScale: 125, interfaceDensity: 'normal', windowSurface: 'opaque', windowTransparency: 18,
    showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard', busyMessageMode: 'queue' },
});
const emptyUsageOverview = (): UsageOverview => ({ generatedAt: Date.now(), capturedSince: null, subscriptions: [], providerTotals: [], recentRuns: [] });

async function desktopOnly<T>(): Promise<T> {
  throw new Error(
    "Open the Monitter desktop app to use hosts, agents, tasks, and settings.",
  );
}

const previewBridge: MonitterBridge = {
  available: false,
  getSnapshot: async () => emptyPreviewSnapshot(),
  getTaskEvents: async () => ({ events: [], nextBefore: null }),
  getUsageOverview: async () => emptyUsageOverview(),
  listCodexAccounts: () => desktopOnly(),
  getProcessMetrics: () => desktopOnly(),
  getTaskEventDetail: () => desktopOnly(),
  readMarkdownFile: () => desktopOnly(),
  getSubagentTranscript: () => desktopOnly(),
  saveHost: () => desktopOnly(),
  deleteHost: () => desktopOnly(),
  probeHost: () => desktopOnly(),
  discoverAcpAgents: () => desktopOnly(),
  verifyAcpAgent: () => desktopOnly(),
  saveAgent: () => desktopOnly(),
  deleteAgent: () => desktopOnly(),
  createTask: () => desktopOnly(),
  handoffTask: () => desktopOnly(),
  chooseLocalFolder: () => desktopOnly(),
  renameTask: () => desktopOnly(),
  autoname: () => desktopOnly(),
  deleteTask: () => desktopOnly(),
  setTaskArchived: () => desktopOnly(),
  saveProject: () => desktopOnly(),
  deleteProject: () => desktopOnly(),
  setTaskProject: () => desktopOnly(),
  sendMessage: () => desktopOnly(),
  clearTaskContext: () => desktopOnly(),
  cancelQueuedMessage: () => desktopOnly(),
  editQueuedMessage: () => desktopOnly(),
  cancelTask: () => desktopOnly(),
  resolveApproval: () => desktopOnly(),
  revokeApprovalRule: () => desktopOnly(),
  resolveInput: () => desktopOnly(),
  saveSettings: () => desktopOnly(),
  getExtensionConfig: () => desktopOnly(),
  saveExtensionConfig: () => desktopOnly(),
  listEnvironmentSecrets: () => desktopOnly(),
  setEnvironmentSecret: () => desktopOnly(),
  deleteEnvironmentSecret: () => desktopOnly(),
  saveChannel: () => desktopOnly(),
  setChannelAgentConversation: () => desktopOnly(),
  stopChannelAgentConversation: () => desktopOnly(),
  setChannelMembership: () => desktopOnly(),
  sendChannelMessage: () => desktopOnly(),
  resumeTask: () => desktopOnly(),
  listTerminals: async () => [],
  openTerminal: () => desktopOnly(),
  writeTerminal: () => desktopOnly(),
  resizeTerminal: () => desktopOnly(),
  readTerminal: () => desktopOnly(),
  closeTerminal: () => desktopOnly(),
  getModelCatalog: () => desktopOnly(),
  setTaskModelSettings: () => desktopOnly(),
  setTaskSandbox: () => desktopOnly(),
  getTaskGoal: async () => null,
  getTaskSlashCommands: async () => [],
  executeTaskSlashCommand: () => desktopOnly(),
  clearTaskGoal: async () => { throw new Error("Goal clearing requires a connected harness."); },
  getTaskGitStatus: () => desktopOnly(),
  waitForTaskGitMarker: () => desktopOnly(),
  getTaskGitDiff: () => desktopOnly(),
  previewTaskDeletion: () => desktopOnly(),
  deleteArchivedTask: () => desktopOnly(),
  storeAttachment: () => desktopOnly(),
  readAttachmentFile: () => desktopOnly(),
  readAttachmentImage: () => desktopOnly(),
  onChanged: async () => () => {},
};

export function getBridge(): MonitterBridge {
  if (typeof window !== "undefined" && window.__MONITTER_BRIDGE__)
    return window.__MONITTER_BRIDGE__;
  if (typeof window !== "undefined" && window.__MONITTER_TEST_BRIDGE__) {
    const test = window.__MONITTER_TEST_BRIDGE__;
    return {
      available: true,
      getSnapshot: () => test.invoke("get_snapshot") as Promise<Snapshot>,
      getTaskEvents: (taskId, before, limit) => test.invoke('get_task_events', { taskId, ...(before === undefined ? {} : { before }), ...(limit === undefined ? {} : { limit }) }) as Promise<TaskEventsPage>,
      getUsageOverview: (policy) => test.invoke('get_usage_overview', policy === undefined ? {} : { policy }) as Promise<UsageOverview>,
      listCodexAccounts: () => test.invoke('list_codex_accounts') as Promise<CodexAccount[]>,
      getProcessMetrics: () => test.invoke('get_process_metrics') as Promise<ProcessMetricsSample>,
      getTaskEventDetail: (taskId, eventId, offset, limit) => test.invoke('get_task_event_detail', { taskId, eventId, ...(offset === undefined ? {} : { offset }), ...(limit === undefined ? {} : { limit }) }) as Promise<EventDetailChunk>,
      readMarkdownFile: (taskId, href, basePath) => test.invoke('read_markdown_file', { taskId, href, ...(basePath === undefined ? {} : { basePath }) }) as Promise<MarkdownDocument>,
      getSubagentTranscript: (taskId, subagentId) => test.invoke('get_subagent_transcript', { taskId, subagentId }) as Promise<SubagentTranscriptEntry[]>,
      saveHost: (host) =>
        test.invoke("save_host", { host }) as Promise<Snapshot>,
      deleteHost: (id) =>
        test.invoke("delete_host", { id }) as Promise<Snapshot>,
      probeHost: (host) =>
        test.invoke("probe_host", { host }) as Promise<ProbeResult>,
      discoverAcpAgents: (hostId) => test.invoke('discover_acp_agents', { hostId }) as Promise<AcpCandidate[]>,
      verifyAcpAgent: (hostId, launch) => test.invoke('verify_acp_agent', { hostId, launch }) as Promise<AcpProbeResult>,
      saveAgent: (agent) =>
        test.invoke("save_agent", { agent }) as Promise<Snapshot>,
      deleteAgent: (id, chatHandling: 'archive' | 'delete' = 'archive') =>
        test.invoke("delete_agent", { id, chatHandling }) as Promise<Snapshot>,
      createTask: (input) =>
        test.invoke("create_task", { input }) as Promise<Task>,
      handoffTask: (input) => test.invoke("handoff_task", { input }) as Promise<Task>,
      chooseLocalFolder: (initial = '') => test.invoke("choose_local_folder", { initial }) as Promise<string | null>,
      renameTask: (id, title) =>
        test.invoke("rename_task", { id, title }) as Promise<Snapshot>,
      autoname: target => test.invoke("autoname", { target }) as Promise<Snapshot>,
      deleteTask: (id) =>
        test.invoke("delete_task", { id }) as Promise<Snapshot>,
      setTaskArchived: (taskId, archived) => test.invoke("set_task_archived", {taskId, archived}) as Promise<Snapshot>,
      saveProject: project => test.invoke("save_project", {project}) as Promise<Snapshot>,
      deleteProject: id => test.invoke("delete_project", {id}) as Promise<Snapshot>,
      setTaskProject: (taskId, projectId) => test.invoke("set_task_project", {taskId, projectId}) as Promise<Snapshot>,
      sendMessage: (taskId, text, attachmentIds = []) =>
        test.invoke("send_message", { taskId, text, attachmentIds }) as Promise<Snapshot>,
      clearTaskContext: (taskId) => test.invoke('clear_task_context', { taskId }) as Promise<Snapshot>,
      cancelQueuedMessage: (id) => test.invoke("cancel_queued_message", { id }) as Promise<Snapshot>,
      editQueuedMessage: (id, text) => test.invoke("edit_queued_message", { id, text }) as Promise<Snapshot>,
      cancelTask: (taskId) =>
        test.invoke("cancel_task", { taskId }) as Promise<Snapshot>,
      resolveApproval: (approvalId, decision) =>
        test.invoke("resolve_approval", { approvalId, decision }) as Promise<Snapshot>,
      revokeApprovalRule: (ruleId) => test.invoke('revoke_approval_rule', { ruleId }) as Promise<Snapshot>,
      resolveInput: (approvalId, response) => test.invoke("resolve_input", { approvalId, response }) as Promise<Snapshot>,
      saveSettings: (settings) =>
        test.invoke("save_settings", { settings }) as Promise<Snapshot>,
      getExtensionConfig: () => test.invoke('get_extension_config') as Promise<ExtensionConfig>,
      saveExtensionConfig: (config) => test.invoke('save_extension_config', { config }) as Promise<ExtensionConfig>,
      listEnvironmentSecrets: () => test.invoke('list_environment_secrets') as Promise<EnvironmentSecretsConfig>,
      setEnvironmentSecret: (revision, name, value, description) => test.invoke('set_environment_secret', { revision, name, value, description }) as Promise<EnvironmentSecretsConfig>,
      deleteEnvironmentSecret: (revision, name) => test.invoke('delete_environment_secret', { revision, name }) as Promise<EnvironmentSecretsConfig>,
      saveChannel: (channel) =>
        test.invoke("save_channel", { channel }) as Promise<Snapshot>,
      setChannelAgentConversation: (channelId, enabled, turnLimit) => test.invoke("set_channel_agent_conversation", {channelId,enabled,turnLimit}) as Promise<Snapshot>,
      stopChannelAgentConversation: (channelId) => test.invoke("stop_channel_agent_conversation", {channelId}) as Promise<Snapshot>,
      setChannelMembership: (channelId, agentId, member) =>
        test.invoke("set_channel_membership", { channelId, agentId, member }) as Promise<Snapshot>,
      sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) =>
        test.invoke("send_channel_message", {
          channelId,
          text,
          agentIds,
          attachmentIds,
        }) as Promise<Snapshot>,
      resumeTask: (taskId) => test.invoke("resume_task", { taskId }) as Promise<Snapshot>,
      listTerminals: () => test.invoke('list_terminals', {}) as Promise<TerminalSession[]>,
      openTerminal: (target, cols, rows) => test.invoke('open_terminal', {target,cols,rows}) as Promise<TerminalSession>,
      writeTerminal: (id, data) => test.invoke('write_terminal', {id,data}) as Promise<void>,
      resizeTerminal: (id, cols, rows) => test.invoke('resize_terminal', {id,cols,rows}) as Promise<void>,
      readTerminal: (id, afterSeq) => test.invoke('read_terminal', {id,afterSeq}) as Promise<TerminalRead>,
      closeTerminal: id => test.invoke('close_terminal', {id}) as Promise<void>,
      getModelCatalog: target => test.invoke("get_model_catalog", {target}) as Promise<ModelCatalog>,
      setTaskModelSettings: (taskId, settings) => test.invoke("set_task_model_settings", {taskId,settings}) as Promise<Snapshot>,
      setTaskSandbox: (taskId, sandbox) => test.invoke('set_task_sandbox', {taskId,sandbox}) as Promise<Snapshot>,
      getTaskGoal: taskId => test.invoke("get_task_goal", {taskId}) as Promise<Goal | null>,
      getTaskSlashCommands: taskId => test.invoke("get_task_slash_commands", {taskId}) as Promise<SlashCommand[]>,
      executeTaskSlashCommand: (taskId, command) => test.invoke("execute_task_slash_command", {taskId, command}) as Promise<SlashCommandExecution>,
      clearTaskGoal: taskId => test.invoke("clear_task_goal", {taskId}) as Promise<void>,
      getTaskGitStatus: (taskId, detectorSession = '') => test.invoke("get_task_git_status", {taskId, detectorSession}) as Promise<TaskGitStatus>,
      waitForTaskGitMarker: (taskId, detectorSession = '') => test.invoke("wait_for_task_git_marker", {taskId, detectorSession}) as Promise<'found' | 'timeout' | 'already-present'>,
      getTaskGitDiff: (taskId, path, scope) => test.invoke("get_task_git_diff", {taskId, path, scope}) as Promise<TaskGitDiff>,
      previewTaskDeletion: taskId => test.invoke("preview_task_deletion", {taskId}) as Promise<TaskDeletionPreview>,
      deleteArchivedTask: (taskId, removeNativeFiles) => test.invoke("delete_archived_task", {taskId, removeNativeFiles}) as Promise<Snapshot>,
      storeAttachment: (target, file, previewDataUrl = null, sourceId) => test.invoke("store_attachment", {target, ...file, previewDataUrl, sourceId}) as Promise<Attachment>,
      readAttachmentFile: sourcePath => test.invoke("read_attachment_file", {sourcePath}) as Promise<AttachmentFileData>,
      readAttachmentImage: attachmentId => test.invoke('read_attachment_image', {attachmentId}) as Promise<AttachmentFileData>,
      onChanged: (handler) => test.listen("monitter:changed", handler),
    };
  }
  return nativeBridge.available ? nativeBridge : previewBridge;
}
