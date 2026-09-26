import { invokeCommand } from './command-invoke';
import { COMMAND_CONTRACT_PROTOCOL_VERSION, type CommandArgs, type CommandName, type CommandResults } from './generated-command-contract';
import { isLanBrowser, lanInvoke } from './lan';
import { applyUiDelta, mergeTaskMessagesPage } from './ui-sync';
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
  ForkTaskInput,
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
  UiDeltaResponse,
  TaskMessagesPage,
  TaskEventsPage,
  ProcessMetricsSample,
  SystemFontFamily,
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
  JevRoutePlan,
  JevCommandCandidate,
  JevCommandPlan,
  MailDetail,
  MailDetailRequestResult,
  BrowserBounds,
  BrowserState,
  BrowserExtensionLoadResult,
  VoiceTranscription,
  JsonValue,
} from "./types";

export interface MonitterBridge {
  available: boolean;
  getSnapshot(): Promise<Snapshot>;
  /** Optional for legacy/test bridges; native and LAN bridges provide paging. */
  getTaskMessages?(taskId: string, beforeId?: string, limit?: number): Promise<TaskMessagesPage>;
  planJevRoute(agentId: string, prompt: string): Promise<JevRoutePlan>;
  recordJevRoute(taskId: string, traceId: string): Promise<void>;
  planJevCommand(query: string, candidates: JevCommandCandidate[]): Promise<JevCommandPlan>;
  getTaskEvents(taskId: string, before?: number, limit?: number): Promise<TaskEventsPage>;
  getUsageOverview(policy?: UsageRefreshPolicy): Promise<UsageOverview>;
  listCodexAccounts(): Promise<CodexAccount[]>;
  getProcessMetrics(): Promise<ProcessMetricsSample>;
  listSystemFonts(): Promise<SystemFontFamily[]>;
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
  forkTask(input: ForkTaskInput): Promise<Task>;
  chooseLocalFolder(initial?: string): Promise<string | null>;
  renameTask(id: string, title: string): Promise<Snapshot>;
  autoname(target: AutonameTarget): Promise<Snapshot>;
  deleteTask(id: string): Promise<Snapshot>;
  setTaskArchived(taskId: string, archived: boolean): Promise<Snapshot>;
  saveProject(project: Project): Promise<Snapshot>;
  postProjectBoardNote(projectId: string, text: string, requestId: string): Promise<Snapshot>;
  deleteProject(id: string): Promise<Snapshot>;
  setTaskProject(taskId: string, projectId: string | null): Promise<Snapshot>;
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot | SendAccepted>;
  requestMailDetail(taskId: string, mailId: string): Promise<MailDetailRequestResult>;
  getMailDetail(taskId: string, mailId: string): Promise<MailDetail | null>;
  clearTaskContext(taskId: string): Promise<Snapshot>;
  cancelQueuedMessage(id: string): Promise<Snapshot>;
  editQueuedMessage(id: string, text: string): Promise<Snapshot>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resolveApproval(approvalId: string, decision: ApprovalDecision): Promise<Snapshot>;
  revokeApprovalRule(ruleId: string): Promise<Snapshot>;
  resolveInput(approvalId: string, response: JsonValue): Promise<Snapshot>;
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
  browserOpen(tabId: string, url: string, bounds: BrowserBounds): Promise<BrowserState>;
  browserSetLayout(tabId: string, bounds: BrowserBounds, visible: boolean): Promise<BrowserState>;
  browserNavigate(tabId: string, url: string): Promise<BrowserState>;
  browserBack(tabId: string): Promise<BrowserState>;
  browserForward(tabId: string): Promise<BrowserState>;
  browserReload(tabId: string): Promise<BrowserState>;
  browserLoadUnpackedExtension(path: string): Promise<BrowserExtensionLoadResult>;
  browserClose(tabId: string): Promise<void>;
  browserGetState(tabId: string): Promise<BrowserState>;
  onBrowserState(handler: (state: BrowserState) => void): Promise<UnlistenFn>;
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
  transcribeVoiceMessage(audioBase64: string): Promise<VoiceTranscription>;
  readAttachmentFile(sourcePath: string): Promise<AttachmentFileData>;
  readAttachmentAudio(attachmentId: string): Promise<AttachmentFileData>;
  readAttachmentImage(attachmentId: string): Promise<AttachmentFileData>;
  onChanged(handler: () => void): Promise<UnlistenFn>;
}

export interface TestBridge {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
  getUiDelta?(revision?: string): Promise<UiDeltaResponse>;
  getTaskMessages?(taskId: string, beforeId?: string, limit?: number): Promise<TaskMessagesPage>;
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

function invoke<C extends CommandName>(
  command: C,
  ...args: keyof CommandArgs[C] extends never ? [args?: CommandArgs[C]] : [args: CommandArgs[C]]
): Promise<CommandResults[C]> {
  const payload = (args[0] ?? {}) as CommandArgs[C];
  return isLanBrowser()
    ? lanInvoke<CommandResults[C]>(command, payload as Record<string, unknown>)
    : invokeCommand(command, payload as CommandArgs[C]);
}

function isUnknownCommand(error: unknown) {
  const value = String(error).toLowerCase();
  return value.includes('unknown command') || value.includes('unknown invoke command') ||
    value.includes('command not found') || value.includes('command `get_ui_delta`') || value.includes('command get_ui_delta') ||
    value.includes('command `get_task_messages`') || value.includes('command get_task_messages') ||
    value.includes("lan command 'get_ui_delta' is not available") || value.includes("lan command 'get_task_messages' is not available") ||
    value.includes('command `get_command_capabilities`') ||
    value.includes('command get_command_capabilities') || value.includes("lan command 'get_command_capabilities' is not available") || value.includes('command `get_ui_snapshot`') ||
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
let capabilityRequest: Promise<Set<string> | null> | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | undefined;
const changedSubscribers = new Set<() => void>();

function invokeUiDelta(revision?: string): Promise<UiDeltaResponse> {
  return invoke('get_ui_delta', revision === undefined ? {} : { revision });
}

function invokeTaskMessages(taskId: string, beforeId?: string, limit?: number): Promise<TaskMessagesPage> {
  return invoke('get_task_messages', { taskId, ...(beforeId === undefined ? {} : { beforeId }), ...(limit === undefined ? {} : { limit }) });
}

function rememberSnapshot(snapshot: Snapshot, revision?: string) {
  cachedSnapshot = snapshot;
  if (revision !== undefined) cachedRevision = revision;
  return snapshot;
}

function goalFromJson(value: JsonValue | null): Goal | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as Record<string, JsonValue>;
  if (typeof record.objective !== 'string' || typeof record.status !== 'string') return null;
  return {
    objective: record.objective,
    status: record.status,
    tokenBudget: typeof record.tokenBudget === 'number' || record.tokenBudget === null ? record.tokenBudget : null,
    ...(typeof record.tokensUsed === 'number' ? { tokensUsed: record.tokensUsed } : {}),
    ...(typeof record.timeUsedSeconds === 'number' ? { timeUsedSeconds: record.timeUsedSeconds } : {}),
  };
}

async function getRevisionedSnapshot(): Promise<Snapshot> {
  const result = await invoke('get_ui_snapshot', cachedRevision ? { revision: cachedRevision } : {});
  return result.snapshot ? rememberSnapshot(result.snapshot, result.revision) : (cachedSnapshot ?? (() => { throw new Error('Monitter reported an unchanged snapshot before a snapshot was loaded.'); })());
}

async function getDeltaSnapshot(): Promise<Snapshot> {
  const response = await invokeUiDelta(cachedRevision);
  const applied = applyUiDelta(cachedSnapshot, cachedRevision, response);
  if (applied.kind !== 'gap') {
    cachedSnapshot = applied.snapshot;
    cachedRevision = applied.revision;
    uiProtocol = 'revisioned';
    return applied.snapshot;
  }

  // A gap may mean an out-of-order read or a new backend epoch. Reset with an
  // unbased read; a delta without its base cannot safely repair the cache.
  const reset = applyUiDelta(null, undefined, await invokeUiDelta());
  if (reset.kind !== 'snapshot') throw new Error('Monitter could not reset the transcript cache after a revision gap.');
  cachedSnapshot = reset.snapshot;
  cachedRevision = reset.revision;
  uiProtocol = 'revisioned';
  return reset.snapshot;
}

function getCommandCapabilities(): Promise<Set<string> | null> {
  if (capabilityRequest) return capabilityRequest;
  capabilityRequest = invoke('get_command_capabilities')
    .then(result => {
      if (result.protocolVersion !== COMMAND_CONTRACT_PROTOCOL_VERSION) {
        throw new Error(`Unsupported Monitter command protocol ${result.protocolVersion}; this client supports ${COMMAND_CONTRACT_PROTOCOL_VERSION}.`);
      }
      return new Set(result.commands);
    })
    .catch(reason => {
      if (isUnknownCommand(reason)) return null;
      throw reason;
    })
    .finally(() => { capabilityRequest = null; });
  return capabilityRequest;
}

function getCachedSnapshot(): Promise<Snapshot> {
  if (snapshotRequest) return snapshotRequest;
  snapshotRequest = (async () => {
    const commands = await getCommandCapabilities();
    if (commands?.has('get_ui_delta')) {
      try { return await getDeltaSnapshot(); }
      catch (reason) {
        if (!isUnknownCommand(reason)) throw reason;
        uiProtocol = 'legacy';
      }
    }
    if (!commands?.has('get_ui_snapshot')) return rememberSnapshot(await invoke('get_snapshot'));
    try { return await getRevisionedSnapshot(); }
    catch (reason) {
      if (!isUnknownCommand(reason)) throw reason;
      return rememberSnapshot(await invoke('get_snapshot'));
    }
  })().finally(() => { snapshotRequest = null; });
  return snapshotRequest;
}

async function getTaskMessages(taskId: string, beforeId?: string, limit?: number): Promise<TaskMessagesPage> {
  const commands = await getCommandCapabilities();
  if (!commands?.has('get_task_messages')) throw new Error('Earlier transcript history is unavailable on this desktop version.');
  if (!cachedSnapshot || !cachedRevision) await getCachedSnapshot();
  let page = await invokeTaskMessages(taskId, beforeId, limit);
  if (page.messages.some(message => message.taskId !== taskId)) throw new Error('Monitter returned a message for a different task.');
  if (page.revision !== cachedRevision) {
    await getCachedSnapshot();
    page = await invokeTaskMessages(taskId, beforeId, limit);
    if (page.messages.some(message => message.taskId !== taskId)) throw new Error('Monitter returned a message for a different task.');
    if (page.revision !== cachedRevision) throw new Error('Transcript history changed while loading. Please try again.');
  }
  if (cachedSnapshot) cachedSnapshot = mergeTaskMessagesPage(cachedSnapshot, page);
  scheduleSnapshotRefresh();
  return page;
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
  // Negotiate before any mutation. Old desktop backends lack this read-only
  // handshake, so only the known legacy send commands remain available there.
  const commands = await getCommandCapabilities();
  return commands?.has('send_message_fast') && commands.has('send_channel_message_fast') ? 'fast' : 'legacy';
}

async function fastSendMessage(taskId: string, text: string, attachmentIds: string[]): Promise<SendAccepted> {
  const receipt = await invoke('send_message_fast', { taskId, text, attachmentIds });
  scheduleSnapshotRefresh();
  return receipt;
}

async function fastSendChannelMessage(channelId: string, text: string, agentIds: string[], attachmentIds: string[]): Promise<SendAccepted> {
  const receipt = await invoke('send_channel_message_fast', { channelId, text, agentIds, attachmentIds });
  scheduleSnapshotRefresh();
  return receipt;
}

async function sendMessageWithProtocol(taskId: string, text: string, attachmentIds: string[]): Promise<Snapshot | SendAccepted> {
  const protocol = await chooseSendProtocol();
  if (protocol === 'legacy') return rememberSnapshot(await invoke('send_message', { taskId, text, attachmentIds }));
  // No catch here: after invoking a mutation, even a command-looking error is
  // ambiguous and must never cause a duplicate legacy send.
  return fastSendMessage(taskId, text, attachmentIds);
}

async function sendChannelMessageWithProtocol(channelId: string, text: string, agentIds: string[], attachmentIds: string[]): Promise<Snapshot | SendAccepted> {
  const protocol = await chooseSendProtocol();
  if (protocol === 'legacy') return rememberSnapshot(await invoke('send_channel_message', { channelId, text, agentIds, attachmentIds }));
  // See direct-send equivalent: mutation errors are surfaced as-is.
  return fastSendChannelMessage(channelId, text, agentIds, attachmentIds);
}

const nativeBridge: MonitterBridge = {
  available:
    typeof window !== "undefined" &&
    (Boolean((window as any).__TAURI_INTERNALS__) || isLanBrowser()),
  getSnapshot: () => getCachedSnapshot(),
  getTaskMessages,
  planJevRoute: (agentId, prompt) => isLanBrowser() ? desktopOnly() : invoke('plan_jev_route', { agentId, prompt }),
  recordJevRoute: (taskId, traceId) => isLanBrowser() ? desktopOnly() : invoke('record_jev_route', { taskId, traceId }),
  planJevCommand: (query, candidates) => isLanBrowser() ? desktopOnly() : invoke('plan_jev_command', { query, candidates }),
  getTaskEvents: (taskId, before, limit) => invoke('get_task_events', { taskId, ...(before === undefined ? {} : { before }), ...(limit === undefined ? {} : { limit }) }),
  getUsageOverview: (policy) => invoke('get_usage_overview', policy === undefined ? {} : { policy }),
  listCodexAccounts: () => invoke('list_codex_accounts'),
  getProcessMetrics: () => invoke('get_process_metrics'),
  listSystemFonts: () => isLanBrowser() ? Promise.resolve([]) : invoke('list_system_fonts'),
  getTaskEventDetail: (taskId, eventId, offset, limit) => invoke('get_task_event_detail', { taskId, eventId, ...(offset === undefined ? {} : { offset }), ...(limit === undefined ? {} : { limit }) }),
  readMarkdownFile: (taskId, href, basePath) => isLanBrowser() ? desktopOnly() : invoke('read_markdown_file', { taskId, href, ...(basePath === undefined ? {} : { basePath }) }),
  getSubagentTranscript: (taskId, subagentId) => invoke('get_subagent_transcript', { taskId, subagentId }),
  saveHost: (host) => invoke("save_host", { host }),
  deleteHost: (id) => invoke("delete_host", { id }),
  probeHost: (host) => invoke("probe_host", { host }),
  discoverAcpAgents: (hostId) => invoke('discover_acp_agents', { hostId }),
  verifyAcpAgent: (hostId, launch) => invoke('verify_acp_agent', { hostId, launch }),
  saveAgent: (agent) => invoke("save_agent", { agent }),
  deleteAgent: (id, chatHandling: 'archive' | 'delete' = 'archive') =>
    invoke("delete_agent", { id, chatHandling }),
  createTask: (input) => invoke("create_task", { input }),
  handoffTask: (input) => invoke("handoff_task", { input }),
  forkTask: (input) => invoke("fork_task", { input }),
  chooseLocalFolder: (initial = '') => isLanBrowser() ? desktopOnly() : invoke("choose_local_folder", { initial }),
  renameTask: (id, title) => invoke("rename_task", { id, title }),
  autoname: target => invoke("autoname", { target }),
  deleteTask: (id) => invoke("delete_task", { id }),
  setTaskArchived: (taskId, archived) => invoke("set_task_archived", {taskId, archived}),
  saveProject: project => invoke("save_project", {project}),
  postProjectBoardNote: (projectId, text, requestId) => invoke('post_project_board_note', { projectId, text, requestId }),
  deleteProject: id => invoke("delete_project", {id}),
  setTaskProject: (taskId, projectId) => invoke("set_task_project", {taskId, projectId}),
  sendMessage: (taskId, text, attachmentIds = []) => sendMessageWithProtocol(taskId, text, attachmentIds),
  requestMailDetail: (taskId, mailId) => isLanBrowser() ? desktopOnly() : invoke('request_mail_detail', { taskId, mailId }),
  getMailDetail: (taskId, mailId) => isLanBrowser() ? desktopOnly() : invoke('get_mail_detail', { taskId, mailId }),
  clearTaskContext: (taskId) => invoke('clear_task_context', { taskId }),
  cancelQueuedMessage: (id) => invoke("cancel_queued_message", { id }),
  editQueuedMessage: (id, text) => invoke("edit_queued_message", { id, text }),
  cancelTask: (taskId) => invoke("cancel_task", { taskId }),
  resolveApproval: (approvalId, decision) => invoke("resolve_approval", { approvalId, decision }),
  revokeApprovalRule: (ruleId) => invoke("revoke_approval_rule", { ruleId }),
  resolveInput: (approvalId, response) => invoke("resolve_input", { approvalId, response }),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  getExtensionConfig: () => isLanBrowser() ? desktopOnly() : invoke('get_extension_config'),
  saveExtensionConfig: (config) => isLanBrowser() ? desktopOnly() : invoke('save_extension_config', { config }),
  listEnvironmentSecrets: () => isLanBrowser() ? desktopOnly() : invoke('list_environment_secrets'),
  setEnvironmentSecret: (revision, name, value, description) => isLanBrowser() ? desktopOnly() : invoke('set_environment_secret', { revision, name, value, description }),
  deleteEnvironmentSecret: (revision, name) => isLanBrowser() ? desktopOnly() : invoke('delete_environment_secret', { revision, name }),
  saveChannel: (channel) => invoke("save_channel", { channel }),
  setChannelAgentConversation: (channelId, enabled, turnLimit) => invoke("set_channel_agent_conversation", {channelId, enabled, turnLimit}),
  stopChannelAgentConversation: (channelId) => invoke("stop_channel_agent_conversation", {channelId}),
  setChannelMembership: (channelId, agentId, member) =>
    invoke("set_channel_membership", { channelId, agentId, member }),
  sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) => sendChannelMessageWithProtocol(channelId, text, agentIds, attachmentIds),
  resumeTask: (taskId) => invoke("resume_task", { taskId }),
  listTerminals: () => invoke('list_terminals'),
  openTerminal: (target, cols, rows) => invoke('open_terminal', {target,cols,rows}),
  writeTerminal: (id, data) => invoke('write_terminal', {id,data}),
  resizeTerminal: (id, cols, rows) => invoke('resize_terminal', {id,cols,rows}),
  readTerminal: (id, afterSeq) => invoke('read_terminal', {id,afterSeq}),
  closeTerminal: id => invoke('close_terminal', {id}),
  browserOpen: (tabId, url, bounds) => isLanBrowser() ? desktopOnly() : invoke('browser_open', {tabId, url, bounds}),
  browserSetLayout: (tabId, bounds, visible) => isLanBrowser() ? desktopOnly() : invoke('browser_set_layout', {tabId, bounds, visible}),
  browserNavigate: (tabId, url) => isLanBrowser() ? desktopOnly() : invoke('browser_navigate', {tabId, url}),
  browserBack: tabId => isLanBrowser() ? desktopOnly() : invoke('browser_back', {tabId}),
  browserForward: tabId => isLanBrowser() ? desktopOnly() : invoke('browser_forward', {tabId}),
  browserReload: tabId => isLanBrowser() ? desktopOnly() : invoke('browser_reload', {tabId}),
  browserLoadUnpackedExtension: path => isLanBrowser() ? desktopOnly() : invoke('browser_load_unpacked_extension', {path}),
  browserClose: tabId => isLanBrowser() ? desktopOnly() : invoke('browser_close', {tabId}),
  browserGetState: tabId => isLanBrowser() ? desktopOnly() : invoke('browser_get_state', {tabId}),
  onBrowserState: handler => isLanBrowser() ? desktopOnly() : listen<BrowserState>('monitter:browser-state', event => handler(event.payload)),
  getModelCatalog: target => invoke("get_model_catalog", {target}),
  setTaskModelSettings: (taskId, settings) => invoke("set_task_model_settings", {taskId,settings}),
  setTaskSandbox: (taskId, sandbox) => invoke('set_task_sandbox', {taskId,sandbox}),
  getTaskGoal: async taskId => goalFromJson(await invoke("get_task_goal", { taskId })),
  getTaskSlashCommands: taskId => invoke("get_task_slash_commands", { taskId }),
  executeTaskSlashCommand: (taskId, command) => invoke("execute_task_slash_command", { taskId, command }),
  clearTaskGoal: taskId => invoke("clear_task_goal", { taskId }),
  getTaskGitStatus: (taskId, detectorSession = '') => invoke("get_task_git_status", { taskId, detectorSession }),
  waitForTaskGitMarker: async (taskId, detectorSession = '') => {
    const result = await invoke("wait_for_task_git_marker", { taskId, detectorSession });
    if (result === 'found' || result === 'timeout' || result === 'already-present') return result;
    throw new Error('Monitter returned an unknown git marker state.');
  },
  getTaskGitDiff: (taskId, path, scope) => invoke("get_task_git_diff", { taskId, path, scope }),
  previewTaskDeletion: taskId => invoke("preview_task_deletion", { taskId }),
  deleteArchivedTask: (taskId, removeNativeFiles) => invoke("delete_archived_task", { taskId, removeNativeFiles }),
  storeAttachment: (target, file, previewDataUrl = null, sourceId) => invoke("store_attachment", {target, ...file, ...(previewDataUrl == null ? {} : {previewDataUrl}), sourceId}),
  transcribeVoiceMessage: audioBase64 => isLanBrowser() ? desktopOnly() : invoke('transcribe_voice_message', { audioBase64 }),
  readAttachmentFile: sourcePath => isLanBrowser() ? desktopOnly() : invoke("read_attachment_file", {sourcePath}),
  readAttachmentAudio: attachmentId => isLanBrowser() ? desktopOnly() : invoke('read_attachment_audio', { attachmentId }),
  readAttachmentImage: attachmentId => invoke('read_attachment_image', {attachmentId}),
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
  mailBatches: [],
  events: [],
  channels: [],
  projects: [],
  collaborations: [],
  queuedMessages: [],
  approvalRequests: [],
  approvalRules: [],
  settings: { accent: "#3f9d6a", theme: "system", interfaceScale: 125, interfaceDensity: 'normal', windowSurface: 'opaque', windowTransparency: 18,
    interfaceFontWeight: 400, chatFontWeight: 400, terminalFontWeight: 400,
    showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard', busyMessageMode: 'queue' },
});
const emptyUsageOverview = (): UsageOverview => ({ generatedAt: Date.now(), capturedSince: null, subscriptions: [], providerTotals: [], recentRuns: [] });

const testSnapshotCaches = new WeakMap<TestBridge, { snapshot: Snapshot | null; revision?: string }>();
async function readTestSnapshot(test: TestBridge): Promise<Snapshot> {
  if (!test.getUiDelta) return test.invoke('get_snapshot') as Promise<Snapshot>;
  let cache = testSnapshotCaches.get(test);
  if (!cache) { cache = { snapshot: null }; testSnapshotCaches.set(test, cache); }
  const response = await test.getUiDelta(cache.revision);
  let applied = applyUiDelta(cache.snapshot, cache.revision, response);
  if (applied.kind === 'gap') {
    applied = applyUiDelta(null, undefined, await test.getUiDelta());
    if (applied.kind !== 'snapshot') throw new Error('Test bridge must return a full snapshot after a revision gap.');
  }
  cache.snapshot = applied.snapshot;
  cache.revision = applied.revision;
  return applied.snapshot;
}

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
  planJevRoute: () => desktopOnly(),
  recordJevRoute: () => desktopOnly(),
  planJevCommand: () => desktopOnly(),
  listCodexAccounts: () => desktopOnly(),
  getProcessMetrics: () => desktopOnly(),
  listSystemFonts: async () => [],
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
  forkTask: () => desktopOnly(),
  chooseLocalFolder: () => desktopOnly(),
  renameTask: () => desktopOnly(),
  autoname: () => desktopOnly(),
  deleteTask: () => desktopOnly(),
  setTaskArchived: () => desktopOnly(),
  saveProject: () => desktopOnly(),
  postProjectBoardNote: () => desktopOnly(),
  deleteProject: () => desktopOnly(),
  setTaskProject: () => desktopOnly(),
  sendMessage: () => desktopOnly(),
  requestMailDetail: () => desktopOnly(),
  getMailDetail: () => desktopOnly(),
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
  browserOpen: () => desktopOnly(),
  browserSetLayout: () => desktopOnly(),
  browserNavigate: () => desktopOnly(),
  browserBack: () => desktopOnly(),
  browserForward: () => desktopOnly(),
  browserReload: () => desktopOnly(),
  browserLoadUnpackedExtension: () => desktopOnly(),
  browserClose: () => desktopOnly(),
  browserGetState: () => desktopOnly(),
  onBrowserState: () => desktopOnly(),
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
  transcribeVoiceMessage: () => desktopOnly(),
  readAttachmentFile: () => desktopOnly(),
  readAttachmentAudio: () => desktopOnly(),
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
      getSnapshot: () => readTestSnapshot(test),
      ...(test.getTaskMessages ? { getTaskMessages: async (taskId: string, beforeId?: string, limit?: number) => {
        let page = await test.getTaskMessages!(taskId, beforeId, limit);
        let cache = testSnapshotCaches.get(test);
        if (cache?.snapshot && cache.revision !== page.revision) {
          await readTestSnapshot(test);
          page = await test.getTaskMessages!(taskId, beforeId, limit);
          cache = testSnapshotCaches.get(test);
          if (cache?.revision !== page.revision) throw new Error('Test bridge history changed while loading.');
        }
        if (cache?.snapshot && cache.revision === page.revision) cache.snapshot = mergeTaskMessagesPage(cache.snapshot, page);
        return page;
      } } : {}),
      getTaskEvents: (taskId, before, limit) => test.invoke('get_task_events', { taskId, ...(before === undefined ? {} : { before }), ...(limit === undefined ? {} : { limit }) }) as Promise<TaskEventsPage>,
      getUsageOverview: (policy) => test.invoke('get_usage_overview', policy === undefined ? {} : { policy }) as Promise<UsageOverview>,
      planJevRoute: (agentId, prompt) => test.invoke('plan_jev_route', { agentId, prompt }) as Promise<JevRoutePlan>,
      recordJevRoute: (taskId, traceId) => test.invoke('record_jev_route', { taskId, traceId }) as Promise<void>,
      planJevCommand: (query, candidates) => test.invoke('plan_jev_command', { query, candidates }) as Promise<JevCommandPlan>,
      listCodexAccounts: () => test.invoke('list_codex_accounts') as Promise<CodexAccount[]>,
      getProcessMetrics: () => test.invoke('get_process_metrics') as Promise<ProcessMetricsSample>,
      listSystemFonts: () => test.invoke('list_system_fonts') as Promise<SystemFontFamily[]>,
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
      forkTask: (input) => test.invoke("fork_task", { input }) as Promise<Task>,
      chooseLocalFolder: (initial = '') => test.invoke("choose_local_folder", { initial }) as Promise<string | null>,
      renameTask: (id, title) =>
        test.invoke("rename_task", { id, title }) as Promise<Snapshot>,
      autoname: target => test.invoke("autoname", { target }) as Promise<Snapshot>,
      deleteTask: (id) =>
        test.invoke("delete_task", { id }) as Promise<Snapshot>,
      setTaskArchived: (taskId, archived) => test.invoke("set_task_archived", {taskId, archived}) as Promise<Snapshot>,
      saveProject: project => test.invoke("save_project", {project}) as Promise<Snapshot>,
      postProjectBoardNote: (projectId, text, requestId) => test.invoke('post_project_board_note', { projectId, text, requestId }) as Promise<Snapshot>,
      deleteProject: id => test.invoke("delete_project", {id}) as Promise<Snapshot>,
      setTaskProject: (taskId, projectId) => test.invoke("set_task_project", {taskId, projectId}) as Promise<Snapshot>,
      sendMessage: (taskId, text, attachmentIds = []) =>
        test.invoke("send_message", { taskId, text, attachmentIds }) as Promise<Snapshot>,
      requestMailDetail: (taskId, mailId) => test.invoke('request_mail_detail', { taskId, mailId }) as Promise<MailDetailRequestResult>,
      getMailDetail: (taskId, mailId) => test.invoke('get_mail_detail', { taskId, mailId }) as Promise<MailDetail | null>,
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
      browserOpen: (tabId, url, bounds) => test.invoke('browser_open', {tabId, url, bounds}) as Promise<BrowserState>,
      browserSetLayout: (tabId, bounds, visible) => test.invoke('browser_set_layout', {tabId, bounds, visible}) as Promise<BrowserState>,
      browserNavigate: (tabId, url) => test.invoke('browser_navigate', {tabId, url}) as Promise<BrowserState>,
      browserBack: tabId => test.invoke('browser_back', {tabId}) as Promise<BrowserState>,
      browserForward: tabId => test.invoke('browser_forward', {tabId}) as Promise<BrowserState>,
      browserReload: tabId => test.invoke('browser_reload', {tabId}) as Promise<BrowserState>,
      browserLoadUnpackedExtension: path => test.invoke('browser_load_unpacked_extension', {path}) as Promise<BrowserExtensionLoadResult>,
      browserClose: tabId => test.invoke('browser_close', {tabId}) as Promise<void>,
      browserGetState: tabId => test.invoke('browser_get_state', {tabId}) as Promise<BrowserState>,
      onBrowserState: async () => () => {},
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
      storeAttachment: (target, file, previewDataUrl = null, sourceId) => test.invoke("store_attachment", {target, ...file, ...(previewDataUrl == null ? {} : {previewDataUrl}), sourceId}) as Promise<Attachment>,
      transcribeVoiceMessage: audioBase64 => test.invoke('transcribe_voice_message', { audioBase64 }) as Promise<VoiceTranscription>,
      readAttachmentFile: sourcePath => test.invoke("read_attachment_file", {sourcePath}) as Promise<AttachmentFileData>,
      readAttachmentAudio: attachmentId => test.invoke('read_attachment_audio', { attachmentId }) as Promise<AttachmentFileData>,
      readAttachmentImage: attachmentId => test.invoke('read_attachment_image', {attachmentId}) as Promise<AttachmentFileData>,
      onChanged: (handler) => test.listen("monitter:changed", handler),
    };
  }
  return nativeBridge.available ? nativeBridge : previewBridge;
}
