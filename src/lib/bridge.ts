import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Agent,
  Goal,
  Channel,
  CreateTaskInput,
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
  Sandbox,
} from "./types";

export interface MonitterBridge {
  available: boolean;
  getSnapshot(): Promise<Snapshot>;
  saveHost(host: Host): Promise<Snapshot>;
  deleteHost(id: string): Promise<Snapshot>;
  probeHost(host: Host): Promise<ProbeResult>;
  saveAgent(agent: Agent): Promise<Snapshot>;
  deleteAgent(id: string): Promise<Snapshot>;
  createTask(input: CreateTaskInput): Promise<Task>;
  chooseLocalFolder(initial?: string): Promise<string | null>;
  renameTask(id: string, title: string): Promise<Snapshot>;
  autoname(target: AutonameTarget): Promise<Snapshot>;
  deleteTask(id: string): Promise<Snapshot>;
  setTaskArchived(taskId: string, archived: boolean): Promise<Snapshot>;
  saveProject(project: Project): Promise<Snapshot>;
  deleteProject(id: string): Promise<Snapshot>;
  setTaskProject(taskId: string, projectId: string | null): Promise<Snapshot>;
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot>;
  cancelQueuedMessage(id: string): Promise<Snapshot>;
  editQueuedMessage(id: string, text: string): Promise<Snapshot>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resolveApproval(approvalId: string, decision: 'approve_once' | 'deny'): Promise<Snapshot>;
  saveSettings(settings: Settings): Promise<Snapshot>;
  saveChannel(channel: Channel): Promise<Snapshot>;
  setChannelAgentConversation(channelId: string, enabled: boolean, turnLimit: number): Promise<Snapshot>;
  stopChannelAgentConversation(channelId: string): Promise<Snapshot>;
  setChannelMembership(channelId: string, agentId: string, member: boolean): Promise<Snapshot>;
  sendChannelMessage(
    channelId: string,
    text: string,
    agentIds: string[],
    attachmentIds?: string[],
  ): Promise<Snapshot>;
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
  getTaskGitStatus(taskId: string, detectorSession?: string): Promise<TaskGitStatus>;
  waitForTaskGitMarker(taskId: string, detectorSession?: string): Promise<'found' | 'timeout' | 'already-present'>;
  getTaskGitDiff(taskId: string, path: string, scope: GitDiffScope): Promise<TaskGitDiff>;
  previewTaskDeletion(taskId: string): Promise<TaskDeletionPreview>;
  deleteArchivedTask(taskId: string, removeNativeFiles: boolean): Promise<Snapshot>;
  storeAttachment(target: AttachmentTarget, file: AttachmentFileData, previewDataUrl?: string | null, sourceId?: string): Promise<Attachment>;
  readAttachmentFile(sourcePath: string): Promise<AttachmentFileData>;
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

const nativeBridge: MonitterBridge = {
  available:
    typeof window !== "undefined" &&
    Boolean((window as any).__TAURI_INTERNALS__),
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  saveHost: (host) => invoke<Snapshot>("save_host", { host }),
  deleteHost: (id) => invoke<Snapshot>("delete_host", { id }),
  probeHost: (host) => invoke<ProbeResult>("probe_host", { host }),
  saveAgent: (agent) => invoke<Snapshot>("save_agent", { agent }),
  deleteAgent: (id) => invoke<Snapshot>("delete_agent", { id }),
  createTask: (input) => invoke<Task>("create_task", { input }),
  chooseLocalFolder: (initial = '') => invoke<string | null>("choose_local_folder", { initial }),
  renameTask: (id, title) => invoke<Snapshot>("rename_task", { id, title }),
  autoname: target => invoke<Snapshot>("autoname", { target }),
  deleteTask: (id) => invoke<Snapshot>("delete_task", { id }),
  setTaskArchived: (taskId, archived) => invoke<Snapshot>("set_task_archived", {taskId, archived}),
  saveProject: project => invoke<Snapshot>("save_project", {project}),
  deleteProject: id => invoke<Snapshot>("delete_project", {id}),
  setTaskProject: (taskId, projectId) => invoke<Snapshot>("set_task_project", {taskId, projectId}),
  sendMessage: (taskId, text, attachmentIds = []) =>
    invoke<Snapshot>("send_message", { taskId, text, attachmentIds }),
  cancelQueuedMessage: (id) => invoke<Snapshot>("cancel_queued_message", { id }),
  editQueuedMessage: (id, text) => invoke<Snapshot>("edit_queued_message", { id, text }),
  cancelTask: (taskId) => invoke<Snapshot>("cancel_task", { taskId }),
  resolveApproval: (approvalId, decision) => invoke<Snapshot>("resolve_approval", { approvalId, decision }),
  saveSettings: (settings) => invoke<Snapshot>("save_settings", { settings }),
  saveChannel: (channel) => invoke<Snapshot>("save_channel", { channel }),
  setChannelAgentConversation: (channelId, enabled, turnLimit) => invoke<Snapshot>("set_channel_agent_conversation", {channelId, enabled, turnLimit}),
  stopChannelAgentConversation: (channelId) => invoke<Snapshot>("stop_channel_agent_conversation", {channelId}),
  setChannelMembership: (channelId, agentId, member) =>
    invoke<Snapshot>("set_channel_membership", { channelId, agentId, member }),
  sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) =>
    invoke<Snapshot>("send_channel_message", { channelId, text, agentIds, attachmentIds }),
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
  getTaskGitStatus: (taskId, detectorSession = '') => invoke<TaskGitStatus>("get_task_git_status", { taskId, detectorSession }),
  waitForTaskGitMarker: (taskId, detectorSession = '') => invoke<'found' | 'timeout' | 'already-present'>("wait_for_task_git_marker", { taskId, detectorSession }),
  getTaskGitDiff: (taskId, path, scope) => invoke<TaskGitDiff>("get_task_git_diff", { taskId, path, scope }),
  previewTaskDeletion: taskId => invoke<TaskDeletionPreview>("preview_task_deletion", { taskId }),
  deleteArchivedTask: (taskId, removeNativeFiles) => invoke<Snapshot>("delete_archived_task", { taskId, removeNativeFiles }),
  storeAttachment: (target, file, previewDataUrl = null, sourceId) => invoke<Attachment>("store_attachment", {target, ...file, previewDataUrl, sourceId}),
  readAttachmentFile: sourcePath => invoke<AttachmentFileData>("read_attachment_file", {sourcePath}),
  onChanged: async (handler) => listen("monitter:changed", handler),
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
  settings: { accent: "#3f9d6a", theme: "system", interfaceScale: 125,
    showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard', busyMessageMode: 'queue' },
});

async function desktopOnly<T>(): Promise<T> {
  throw new Error(
    "Open the Monitter desktop app to use hosts, agents, tasks, and settings.",
  );
}

const previewBridge: MonitterBridge = {
  available: false,
  getSnapshot: async () => emptyPreviewSnapshot(),
  saveHost: () => desktopOnly(),
  deleteHost: () => desktopOnly(),
  probeHost: () => desktopOnly(),
  saveAgent: () => desktopOnly(),
  deleteAgent: () => desktopOnly(),
  createTask: () => desktopOnly(),
  chooseLocalFolder: () => desktopOnly(),
  renameTask: () => desktopOnly(),
  autoname: () => desktopOnly(),
  deleteTask: () => desktopOnly(),
  setTaskArchived: () => desktopOnly(),
  saveProject: () => desktopOnly(),
  deleteProject: () => desktopOnly(),
  setTaskProject: () => desktopOnly(),
  sendMessage: () => desktopOnly(),
  cancelQueuedMessage: () => desktopOnly(),
  editQueuedMessage: () => desktopOnly(),
  cancelTask: () => desktopOnly(),
  resolveApproval: () => desktopOnly(),
  saveSettings: () => desktopOnly(),
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
  getTaskGitStatus: () => desktopOnly(),
  waitForTaskGitMarker: () => desktopOnly(),
  getTaskGitDiff: () => desktopOnly(),
  previewTaskDeletion: () => desktopOnly(),
  deleteArchivedTask: () => desktopOnly(),
  storeAttachment: () => desktopOnly(),
  readAttachmentFile: () => desktopOnly(),
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
      saveHost: (host) =>
        test.invoke("save_host", { host }) as Promise<Snapshot>,
      deleteHost: (id) =>
        test.invoke("delete_host", { id }) as Promise<Snapshot>,
      probeHost: (host) =>
        test.invoke("probe_host", { host }) as Promise<ProbeResult>,
      saveAgent: (agent) =>
        test.invoke("save_agent", { agent }) as Promise<Snapshot>,
      deleteAgent: (id) =>
        test.invoke("delete_agent", { id }) as Promise<Snapshot>,
      createTask: (input) =>
        test.invoke("create_task", { input }) as Promise<Task>,
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
      cancelQueuedMessage: (id) => test.invoke("cancel_queued_message", { id }) as Promise<Snapshot>,
      editQueuedMessage: (id, text) => test.invoke("edit_queued_message", { id, text }) as Promise<Snapshot>,
      cancelTask: (taskId) =>
        test.invoke("cancel_task", { taskId }) as Promise<Snapshot>,
      resolveApproval: (approvalId, decision) =>
        test.invoke("resolve_approval", { approvalId, decision }) as Promise<Snapshot>,
      saveSettings: (settings) =>
        test.invoke("save_settings", { settings }) as Promise<Snapshot>,
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
      getTaskGitStatus: (taskId, detectorSession = '') => test.invoke("get_task_git_status", {taskId, detectorSession}) as Promise<TaskGitStatus>,
      waitForTaskGitMarker: (taskId, detectorSession = '') => test.invoke("wait_for_task_git_marker", {taskId, detectorSession}) as Promise<'found' | 'timeout' | 'already-present'>,
      getTaskGitDiff: (taskId, path, scope) => test.invoke("get_task_git_diff", {taskId, path, scope}) as Promise<TaskGitDiff>,
      previewTaskDeletion: taskId => test.invoke("preview_task_deletion", {taskId}) as Promise<TaskDeletionPreview>,
      deleteArchivedTask: (taskId, removeNativeFiles) => test.invoke("delete_archived_task", {taskId, removeNativeFiles}) as Promise<Snapshot>,
      storeAttachment: (target, file, previewDataUrl = null, sourceId) => test.invoke("store_attachment", {target, ...file, previewDataUrl, sourceId}) as Promise<Attachment>,
      readAttachmentFile: sourcePath => test.invoke("read_attachment_file", {sourcePath}) as Promise<AttachmentFileData>,
      onChanged: (handler) => test.listen("monitter:changed", handler),
    };
  }
  return nativeBridge.available ? nativeBridge : previewBridge;
}
