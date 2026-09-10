export type Provider = 'codex' | 'claude' | 'opencode' | 'hermes';
export type Sandbox = 'read-only' | 'workspace-write' | 'harness-configured' | 'yolo';
export type TaskStatus = 'idle' | 'running' | 'completed' | 'error' | 'interrupted';
export type SidebarView = 'standard' | 'activity' | 'projects';
export interface Project {
  id: string; name: string; description: string;
  workspaces: { hostId: string; cwd: string }[];
}
export interface Host {
  id: string; name: string; kind: 'local' | 'ssh'; address: string;
  user: string; port: number; identityFile: string; defaultCwd: string;
  codexPath: string; claudePath: string; opencodePath: string; hermesPath: string;
}
export interface Agent {
  id: string; name: string; description: string; instructions: string;
  avatar: string | null;
  provider: Provider; model: string; hostId: string; cwd: string;
  color: string; sandbox: Sandbox;
  expertise: string[]; responsibilities: string[]; skills: string[];
  collaborationEnabled: boolean;
}
export interface Task {
  id: string; agentId: string; title: string; nativeSessionId: string | null;
  archived: boolean; status: TaskStatus; createdAt: number; updatedAt: number;
  parentTaskId: string | null; channelId: string | null; projectId: string | null;
  hostId: string; cwd: string; provider: Provider; model: string; sandbox: Sandbox;
  modelSettings?: ModelSettings | null;
}
export interface Message {
  id: string; taskId: string; role: 'user' | 'assistant' | 'system';
  text: string; createdAt: number;
  senderAgentId?: string | null; collaborationId?: string | null;
  attachments?: Attachment[];
}
export interface Collaboration {
  id: string; kind: 'message' | 'delegation';
  fromAgentId: string; fromTaskId: string; toAgentId: string; toTaskId: string;
  text: string; requestId: string;
  status: 'queued' | 'running' | 'completed' | 'error' | 'interrupted';
  result: string | null; error: string | null; createdAt: number; updatedAt: number;
}
export interface RunEvent {
  id: string; taskId: string; kind: 'status' | 'tool' | 'reasoning' | 'usage' | 'error' | 'output' | 'computer' | 'goal' | 'log' | 'collaboration';
  title: string; detail: string; createdAt: number;
}
export interface ChannelMessage {
  id: string; role: 'user' | 'assistant'; agentId: string | null;
  text: string; createdAt: number; taskId: string | null;
  attachments?: Attachment[];
}
export interface Channel {
  agentConversationEnabled?: boolean; agentConversationTurnLimit?: number;
  agentConversationTurnsUsed?: number; agentConversationPaused?: boolean;
  id: string; name: string; description: string; agentIds: string[];
  messages: ChannelMessage[];
}
export interface QueuedMessage {
  id: string; taskId: string; channelId: string | null; text: string;
  attachmentIds: string[]; createdAt: number; status: 'queued' | 'sending' | 'error'; error?: string | null;
  senderAgentId?: string | null; origin?: string | null;
}
export interface Settings {
  shortcutMode?: 'standard' | 'vim';
  showTabCloseButtons?: boolean;
  tintUserMessages?: boolean;
  compressToolCalls?: boolean;
  terminalFontSize?: number; chatFontSize?: number; interfaceFontSize?: number;
  terminalFont?: string; chatFont?: string; interfaceFont?: string;
  dimInactivePanes?: boolean; inactivePaneOpacity?: number; focusFollowsMouse?: boolean;
  accent: string; theme: 'light' | 'dark' | 'system'; interfaceScale: number;
  showToolActivity: boolean; showReasoningSummaries: boolean; sendWithEnter: boolean;
  sidebarView: SidebarView; busyMessageMode?: 'queue' | 'steer';
}
export interface Snapshot {
  hosts: Host[]; agents: Agent[]; tasks: Task[]; messages: Message[];
  events: RunEvent[]; channels: Channel[]; projects: Project[]; settings: Settings;
  collaborations: Collaboration[]; queuedMessages: QueuedMessage[];
}
export interface ProbeResult { ok: boolean; versions: Record<string, string>; message: string; }
export interface CreateTaskInput {
  agentId: string; title: string; nativeSessionId?: string | null;
  parentTaskId?: string | null; channelId?: string | null; projectId?: string | null;
  modelSettings?: ModelSettings | null;
  sandbox?: Sandbox | null;
}

export interface Goal {
  objective: string; status: string; tokenBudget?: number | null;
  tokensUsed?: number; timeUsedSeconds?: number;
}
export interface ComputerActivity { id: string; tool: string; summary: string; }

export type GitDiffScope = 'staged' | 'unstaged' | 'untracked';
export interface TaskDeletionPreview { supported: boolean; reason: string; files: string[]; }
export interface Attachment {
  id: string; name: string; mimeType: string; size: number; path: string;
  previewDataUrl?: string | null; sourceId?: string | null;
}
export interface AttachmentTarget { taskId?: string; agentId?: string; projectId?: string | null; }
export interface AttachmentFileData { filename: string; mimeType: string; dataBase64: string; }
export interface GitFileStatus {
  path: string;
  originalPath: string | null;
  indexStatus: string;
  worktreeStatus: string;
  untracked: boolean;
}
export type TaskGitStatus = { repository: false } | {
  repository: true;
  root: string;
  branch: string | null;
  files: GitFileStatus[];
  truncated: boolean;
};
export type TaskGitDiff = { repository: false } | {
  repository: true;
  path: string;
  scope: GitDiffScope;
  text: string;
  truncated: boolean;
  binary: boolean;
};

export interface ModelSettings {
  model: string;
  reasoningEffort: string | null;
  fastMode: boolean | null;
}
export interface ModelTarget { taskId?: string; agentId?: string; projectId?: string | null; }
export interface HarnessModel {
  id: string; name: string; description: string;
  reasoningEfforts: { id: string; description: string }[];
  defaultEffort: string | null;
  supportsFast: boolean; fastDescription: string | null;
}
export interface ModelCatalog {
  models: HarnessModel[]; current: ModelSettings;
  source: string; warning: string | null;
}


export interface TerminalTarget { cwd?: string; taskId?: string; agentId?: string; hostId?: string; projectId?: string | null; }
export interface TerminalSession { id: string; title: string; hostId: string; cwd: string; status: 'running' | 'exited'; exitCode: number | null; }
export interface TerminalRead { chunks: { seq: number; data: number[] }[]; nextSeq: number; status: 'running' | 'exited'; exitCode: number | null; truncated: boolean; }
export interface AutonameTarget { taskId?: string; channelId?: string; terminalId?: string; content?: string; }
