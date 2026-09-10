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
  Attachment,
  AttachmentTarget,
  AttachmentFileData,
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
  renameTask(id: string, title: string): Promise<Snapshot>;
  deleteTask(id: string): Promise<Snapshot>;
  setTaskArchived(taskId: string, archived: boolean): Promise<Snapshot>;
  saveProject(project: Project): Promise<Snapshot>;
  deleteProject(id: string): Promise<Snapshot>;
  setTaskProject(taskId: string, projectId: string | null): Promise<Snapshot>;
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot>;
  cancelTask(taskId: string): Promise<Snapshot>;
  saveSettings(settings: Settings): Promise<Snapshot>;
  saveChannel(channel: Channel): Promise<Snapshot>;
  sendChannelMessage(
    channelId: string,
    text: string,
    agentIds: string[],
    attachmentIds?: string[],
  ): Promise<Snapshot>;
  resumeTask(taskId: string): Promise<Snapshot>;
  getTaskGoal(taskId: string): Promise<Goal | null>;
  getTaskGitStatus(taskId: string): Promise<TaskGitStatus>;
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
  renameTask: (id, title) => invoke<Snapshot>("rename_task", { id, title }),
  deleteTask: (id) => invoke<Snapshot>("delete_task", { id }),
  setTaskArchived: (taskId, archived) => invoke<Snapshot>("set_task_archived", {taskId, archived}),
  saveProject: project => invoke<Snapshot>("save_project", {project}),
  deleteProject: id => invoke<Snapshot>("delete_project", {id}),
  setTaskProject: (taskId, projectId) => invoke<Snapshot>("set_task_project", {taskId, projectId}),
  sendMessage: (taskId, text, attachmentIds = []) =>
    invoke<Snapshot>("send_message", { taskId, text, attachmentIds }),
  cancelTask: (taskId) => invoke<Snapshot>("cancel_task", { taskId }),
  saveSettings: (settings) => invoke<Snapshot>("save_settings", { settings }),
  saveChannel: (channel) => invoke<Snapshot>("save_channel", { channel }),
  sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) =>
    invoke<Snapshot>("send_channel_message", { channelId, text, agentIds, attachmentIds }),
  resumeTask: (taskId) => invoke<Snapshot>("resume_task", { taskId }),
  getTaskGoal: taskId => invoke<Goal | null>("get_task_goal", { taskId }),
  getTaskGitStatus: taskId => invoke<TaskGitStatus>("get_task_git_status", { taskId }),
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
  settings: { accent: "#3f9d6a", theme: "system", interfaceScale: 125,
    showToolActivity: true, showReasoningSummaries: true, sendWithEnter: false, sidebarView: 'standard' },
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
  renameTask: () => desktopOnly(),
  deleteTask: () => desktopOnly(),
  setTaskArchived: () => desktopOnly(),
  saveProject: () => desktopOnly(),
  deleteProject: () => desktopOnly(),
  setTaskProject: () => desktopOnly(),
  sendMessage: () => desktopOnly(),
  cancelTask: () => desktopOnly(),
  saveSettings: () => desktopOnly(),
  saveChannel: () => desktopOnly(),
  sendChannelMessage: () => desktopOnly(),
  resumeTask: () => desktopOnly(),
  getTaskGoal: async () => null,
  getTaskGitStatus: () => desktopOnly(),
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
      renameTask: (id, title) =>
        test.invoke("rename_task", { id, title }) as Promise<Snapshot>,
      deleteTask: (id) =>
        test.invoke("delete_task", { id }) as Promise<Snapshot>,
      setTaskArchived: (taskId, archived) => test.invoke("set_task_archived", {taskId, archived}) as Promise<Snapshot>,
      saveProject: project => test.invoke("save_project", {project}) as Promise<Snapshot>,
      deleteProject: id => test.invoke("delete_project", {id}) as Promise<Snapshot>,
      setTaskProject: (taskId, projectId) => test.invoke("set_task_project", {taskId, projectId}) as Promise<Snapshot>,
      sendMessage: (taskId, text, attachmentIds = []) =>
        test.invoke("send_message", { taskId, text, attachmentIds }) as Promise<Snapshot>,
      cancelTask: (taskId) =>
        test.invoke("cancel_task", { taskId }) as Promise<Snapshot>,
      saveSettings: (settings) =>
        test.invoke("save_settings", { settings }) as Promise<Snapshot>,
      saveChannel: (channel) =>
        test.invoke("save_channel", { channel }) as Promise<Snapshot>,
      sendChannelMessage: (channelId, text, agentIds, attachmentIds = []) =>
        test.invoke("send_channel_message", {
          channelId,
          text,
          agentIds,
          attachmentIds,
        }) as Promise<Snapshot>,
      resumeTask: (taskId) => test.invoke("resume_task", { taskId }) as Promise<Snapshot>,
      getTaskGoal: taskId => test.invoke("get_task_goal", {taskId}) as Promise<Goal | null>,
      getTaskGitStatus: taskId => test.invoke("get_task_git_status", {taskId}) as Promise<TaskGitStatus>,
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
