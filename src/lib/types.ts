export type Provider = 'codex' | 'claude' | 'opencode' | 'hermes' | 'acp';
/** Explicit stdio launcher, never a shell command. Copied into each new task. */
export interface AcpLaunch { command: string; args: string[]; }
/** Finding an executable does not prove protocol support or authentication. */
export interface AcpCandidate {
  id: string; name: string; description: string; sourceUrl: string;
  integration: 'native' | 'bridge'; launch: AcpLaunch; detected: boolean;
}
export interface AcpProbeResult {
  protocolVersion: number; agentName: string | null; agentVersion: string | null;
  loadSession: boolean; resumeSession: boolean;
  image: boolean; audio: boolean; embeddedContext: boolean;
}
export type Sandbox = 'read-only' | 'workspace-write' | 'harness-configured' | 'yolo';
export type TaskStatus = 'idle' | 'running' | 'completed' | 'error' | 'interrupted';
export type SidebarView = 'standard' | 'activity' | 'projects';
export interface Project {
  id: string; name: string; description: string; icon: string; color: string;
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
  acp?: AcpLaunch | null;
  color: string; sandbox: Sandbox;
  expertise: string[]; responsibilities: string[]; skills: string[];
  collaborationEnabled: boolean;
}
export interface Task {
  id: string; agentId: string; title: string; nativeSessionId: string | null;
  archived: boolean; status: TaskStatus; createdAt: number; updatedAt: number;
  parentTaskId: string | null; channelId: string | null; projectId: string | null;
  hostId: string; cwd: string; provider: Provider; model: string; sandbox: Sandbox;
  acp?: AcpLaunch | null;
  modelSettings?: ModelSettings | null;
}
export interface Message {
  streamStatus?: 'streaming' | 'complete' | 'interrupted';
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
/** A revision-aware, compact UI projection. A null snapshot means unchanged. */
export interface UiSnapshotResponse { revision: string; snapshot: Snapshot | null; }
/** Fast-send acknowledgement: acceptance is durable, but no snapshot is implied. */
export interface SendAccepted { accepted: true; }
/** Full diagnostic activity is deliberately loaded only when its pane is opened. */
export interface TaskEventsPage { events: RunEvent[]; nextBefore: number | null; }
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
export interface ApprovalRequest {
  id: string; taskId: string; provider: Provider; runId: string; tool: string;
  summary: string; detail: string;
  risk: 'low' | 'medium' | 'high' | 'unknown';
  status: 'pending' | 'approved' | 'denied' | 'expired' | 'unsupported';
  createdAt: number; resolvedAt: number | null;
  decision: 'approve_once' | 'approve_always' | 'deny' | null;
  input?: InteractionInput | null;
  response?: unknown;
  /** Missing means an older runtime that cannot create a remembered rule. */
  rememberable?: boolean;
  ruleId?: string | null;
}
/** An app-owned, exact-scope approval rule. Native harness permissions remain one-shot. */
export interface ApprovalRule {
  id: string; agentId: string; hostId: string; provider: Provider; cwd: string; tool: string;
  summary: string; detail: string; createdAt: number; lastUsedAt: number | null; useCount: number;
  scopeDescription?: string;
}
export interface InteractionInput {
  kind: 'questions' | 'form' | 'url';
  questions: { id: string; header: string; question: string; isSecret: boolean; options: { label: string; description: string }[] }[];
  schema: Record<string, unknown> | null;
  url: string | null;
}
export interface Settings {
  /** Optional user identity supplied to agents for chats started after saving. */
  userName?: string;
  shortcutMode?: 'standard' | 'vim';
  showTabCloseButtons?: boolean;
  autoHideTabs?: boolean;
  tabStyle?: 'classic' | 'modern';
  interfaceDensity?: 'tight' | 'normal' | 'spacious';
  tintUserMessages?: boolean;
  compressToolCalls?: boolean;
  terminalFontSize?: number; chatFontSize?: number; interfaceFontSize?: number;
  chatLineHeight?: number; terminalLineHeight?: number;
  terminalFont?: string; chatFont?: string; interfaceFont?: string;
  dimInactivePanes?: boolean; inactivePaneOpacity?: number; focusFollowsMouse?: boolean;
  accent: string; theme: 'light' | 'dark' | 'system'; interfaceScale: number;
  showToolActivity: boolean; showReasoningSummaries: boolean; sendWithEnter: boolean;
  /** Legacy migration seed; active sidebar selection is client-local UI state. */
  sidebarView: SidebarView; busyMessageMode?: 'queue' | 'steer';
}
/** Native-owner configuration, deliberately not part of any workspace snapshot. */
export interface McpServerConfig {
  id: string; name: string; enabled: boolean; agentIds: string[];
  transport: 'stdio' | 'http'; command: string; args: string[];
  env: Record<string, string>; url: string; headers: Record<string, string>;
}
/** Portable markdown instructions, not an executable plugin bundle. */
export interface ManagedSkill {
  id: string; name: string; description: string; enabled: boolean;
  agentIds: string[]; content: string;
}
export interface ExtensionConfig {
  /** Opaque edit revision; stale saves are rejected rather than overwriting newer edits. */
  revision?: string;
  mcpServers: McpServerConfig[]; skills: ManagedSkill[];
}
export interface Snapshot {
  hosts: Host[]; agents: Agent[]; tasks: Task[]; messages: Message[];
  events: RunEvent[]; channels: Channel[]; projects: Project[]; settings: Settings;
  collaborations: Collaboration[]; queuedMessages: QueuedMessage[];
  approvalRequests: ApprovalRequest[];
  /** Omitted by older runtimes and deliberately absent from visitor projections. */
  approvalRules?: ApprovalRule[];
}
export interface ProbeResult { ok: boolean; versions: Record<string, string>; message: string; }
export interface CreateTaskInput {
  agentId: string; title: string; nativeSessionId?: string | null;
  parentTaskId?: string | null; channelId?: string | null; projectId?: string | null; cwd?: string | null;
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
/** `title` is the display title. A custom title takes precedence over `autoTitle`. */
export interface TerminalSession { id: string; title: string; autoTitle?: string | null; customTitle?: string | null; hostId: string; cwd: string; status: 'running' | 'exited'; exitCode: number | null; }
/** `session` is optional for an older LAN desktop; normal native reads include it. */
export interface TerminalRead { chunks: { seq: number; data: number[] }[]; nextSeq: number; status: 'running' | 'exited'; exitCode: number | null; truncated: boolean; session?: TerminalSession; }
export interface AutonameTarget { taskId?: string; channelId?: string; terminalId?: string; content?: string; }
