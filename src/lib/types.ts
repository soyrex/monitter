export type Provider = 'codex' | 'claude' | 'opencode' | 'hermes' | 'acp';
export type UsageProvider = Provider | 'minimax' | 'opencode-go';
export type UsageRefreshPolicy = 'cache-only' | 'if-stale' | 'refresh';
export interface AllowanceWindow {
  key: string; label: string; metric: 'requests' | 'tokens' | 'spend' | 'combined' | 'unknown';
  usedPercent: number | null; used: number | null; limit: number | null;
  unit: 'requests' | 'tokens' | 'usd' | 'credits' | 'unknown'; resetsAt: number | null;
}
export interface AllowanceBalance {
  key: string; label: string; unit: 'usd' | 'credits' | 'unknown';
  remaining: number | null; limit: number | null; resetsAt: number | null;
}
export interface SubscriptionUsageSource {
  provider: UsageProvider; hostId: string; source: string;
  /** Owner-only local Codex profile identity; omitted from shared projections. */
  codexHome?: string | null; accountLabel?: string | null;
  state: 'available' | 'unsupported' | 'not-applicable' | 'not-authenticated' | 'error';
  planType: string | null; fetchedAt: number | null; staleAfter: number | null;
  lastAttemptAt: number; windows: AllowanceWindow[]; balances: AllowanceBalance[]; error: string | null;
}
export interface RunUsageSummary {
  runId: string; taskId: string; provider: Provider; configuredModel: string | null;
  startedAt: number; finishedAt: number | null; final: boolean;
  tokens: { input: number | null; output: number | null; cacheRead: number | null; cacheWrite: number | null; reasoning: number | null; total: number | null };
  costUsd: number | null; durationMs: number | null; apiDurationMs: number | null; providerTurns: number | null;
  context: { used: number; size: number } | null;
}
export interface RunUsageAggregate {
  provider: Provider; runs: number; finalRuns: number;
  tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning: number; total: number };
  costUsd: number | null; durationMs: number | null;
}
export interface UsageOverview {
  generatedAt: number; capturedSince: number | null; subscriptions: SubscriptionUsageSource[];
  providerTotals: RunUsageAggregate[]; recentRuns: RunUsageSummary[];
}
/** Explicit stdio launcher, never a shell command. Copied into each new task. */
export interface AcpLaunch { command: string; args: string[]; }
export interface SlashCommand {
  /** Command name without the leading slash. */
  name: string; description: string; inputHint?: string | null;
  source: 'monitter' | 'codex' | 'acp'; provider: Provider;
}
export interface SlashCommandExecution {
  effect: 'sent' | 'notice' | 'openModel' | 'refreshGoal'; message?: string | null;
}
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
/** Per-harness opt-in. Jev recommends model settings only; it never grants authority. */
export type JevRoutingMode = 'off' | 'recommend' | 'safe_auto';
export interface JevModelTiers { fast: string; balanced: string; strong: string; frontier: string; }
export interface JevRoutingDecision {
  task_kind: 'answer' | 'investigate' | 'localized_edit' | 'bug_fix' | 'refactor' | 'architecture' | 'production_sensitive';
  model_tier: 'fast' | 'balanced' | 'strong' | 'frontier';
  reasoning_level: 'low' | 'medium' | 'high' | 'xhigh';
  execution_mode: 'answer' | 'inspect' | 'edit';
  permission_tier: 'read_only' | 'workspace_write' | 'shell_and_tests' | 'human_review_required';
  confidence: number; rationale: string; escalation_conditions: string[];
}
export interface JevRoutePlan {
  traceId: string; promptFingerprint: string; decision: JevRoutingDecision;
  classifierEvidence: { provider: string; model: string; latencyMs: number; inputTokens: number | null; outputTokens: number | null; costUsd: number | null; };
}
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
export interface CodexAccount { home: string; label: string; }
export interface Agent {
  id: string; name: string; description: string; instructions: string;
  avatar: string | null;
  provider: Provider; model: string; hostId: string; cwd: string;
  /** Local Codex account home. Applies to chats created after saving. */
  codexHome?: string | null;
  acp?: AcpLaunch | null;
  color: string; sandbox: Sandbox;
  expertise: string[]; responsibilities: string[]; skills: string[];
  collaborationEnabled: boolean;
  jevRouting?: JevRoutingMode;
  jevModelTiers?: JevModelTiers;
  /** Internal agents are configurable in Settings but hidden from conversation surfaces. */
  internal?: boolean;
}
export interface Task {
  id: string; agentId: string; title: string; nativeSessionId: string | null;
  archived: boolean; status: TaskStatus; createdAt: number; updatedAt: number;
  parentTaskId: string | null; channelId: string | null; projectId: string | null;
  hostId: string; cwd: string; provider: Provider; model: string; sandbox: Sandbox;
  /** Account home pinned when this chat was created. */
  codexHome?: string | null;
  acp?: AcpLaunch | null;
  modelSettings?: ModelSettings | null;
  /** Captured when archived by removing the owning agent; null otherwise. */
  archivedAgentName?: string | null;
}
export interface Message {
  streamStatus?: 'streaming' | 'complete' | 'interrupted';
  phase?: 'commentary' | 'final_answer';
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
/** Durable, provider-normalized delegated work. `source` is diagnostic and is never a UI label. */
export interface SubagentSession {
  id: string;
  source: 'codex' | 'acp' | 'native' | 'collaboration';
  parentTaskId: string;
  parentThreadId?: string | null;
  collaborationId?: string | null;
  agentPath?: string | null;
  agentThreadId?: string | null;
  prompt?: string | null;
  model?: string | null;
  reasoningEffort?: string | null;
  status: 'queued' | 'running' | 'completed' | 'error' | 'interrupted';
  result?: string | null;
  error?: string | null;
  createdAt: number;
  updatedAt: number;
}
export interface SubagentTranscriptEntry {
  id: string;
  role: 'user' | 'assistant' | 'reasoning' | 'activity';
  text: string;
  createdAt: number;
}
export interface RunEvent {
  id: string; taskId: string; kind: 'status' | 'tool' | 'reasoning' | 'usage' | 'error' | 'output' | 'computer' | 'goal' | 'log' | 'collaboration' | 'subagent';
  title: string; detail: string; createdAt: number;
}
/** A revision-aware, compact UI projection. A null snapshot means unchanged. */
export interface UiSnapshotResponse { revision: string; snapshot: Snapshot | null; }
/** Fast-send acknowledgement: acceptance is durable, but no snapshot is implied. */
export interface SendAccepted { accepted: true; }
/** Full diagnostic activity is deliberately loaded only when its pane is opened. */
export interface TaskEventsPage { events: RunEvent[]; nextBefore: number | null; }
export interface ProcessMetricsProcess { pid: number; parentPid: number; name: string; startedAt: number; cpuTimeMs: number; residentMemoryBytes: number; }
export interface ProcessMetricsSample { cpuTimeMs: number; residentMemoryBytes: number; sampledAt: number; rootPid: number; processes: ProcessMetricsProcess[]; }
export interface EventDetailChunk { chunk: string; nextOffset: number | null; totalBytes: number; }
/** A local Markdown document explicitly scoped to one local task folder. */
export interface MarkdownDocument { path: string; title: string; content: string; }
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
  decision: ApprovalDecision | null;
  input?: InteractionInput | null;
  response?: unknown;
  /** Missing means an older runtime that cannot create a remembered rule. */
  rememberable?: boolean;
  /** Runtime-only approval category offered by the live harness session. */
  sessionScope?: 'file_changes';
  ruleId?: string | null;
}
export type ApprovalDecision = 'approve_once' | 'approve_session' | 'approve_always' | 'deny';
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
  windowSurface?: 'opaque' | 'translucent' | 'glass';
  windowTransparency?: number;
  showActivePaneBorder?: boolean; dimInactivePanes?: boolean; inactivePaneOpacity?: number; focusFollowsMouse?: boolean;
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
  agentIds: string[]; allAgents?: boolean; sourceUrl?: string; content: string;
}
export interface ExtensionConfig {
  /** Opaque edit revision; stale saves are rejected rather than overwriting newer edits. */
  revision?: string;
  mcpServers: McpServerConfig[]; skills: ManagedSkill[];
}
/** Redacted desktop-only metadata. Secret values are never returned to the renderer. */
export interface EnvironmentSecretMetadata {
  name: string;
  description: string;
  updatedAt: number;
}
export interface EnvironmentSecretsConfig {
  revision: string;
  entries: EnvironmentSecretMetadata[];
}
export interface Snapshot {
  hosts: Host[]; agents: Agent[]; tasks: Task[]; messages: Message[];
  events: RunEvent[]; channels: Channel[]; projects: Project[]; settings: Settings;
  collaborations: Collaboration[]; queuedMessages: QueuedMessage[];
  /** Omitted by older runtimes; current runtimes always provide this durable projection. */
  subagentSessions?: SubagentSession[];
  /** Inline transcript for sources with no re-queryable native thread (`acp`), keyed by subagent id. */
  subagentTranscripts?: Record<string, SubagentTranscriptEntry[]>;
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
/** Start a new harness with a bounded, visible continuation brief from an idle chat. */
export interface HandoffTaskInput {
  sourceTaskId: string; agentId: string; note?: string | null;
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
export interface ModelTarget { taskId?: string; agentId?: string; projectId?: string | null; codexHome?: string | null; refresh?: boolean; }
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


export interface TerminalTarget { cwd?: string; taskId?: string; agentId?: string; hostId?: string; projectId?: string | null; command?: string; }
/** `title` is the display title. A custom title takes precedence over `autoTitle`. */
export interface TerminalSession { id: string; title: string; autoTitle?: string | null; customTitle?: string | null; hostId: string; cwd: string; status: 'running' | 'exited'; exitCode: number | null; }
/** `session` is optional for an older LAN desktop; normal native reads include it. */
export interface TerminalRead { chunks: { seq: number; data: number[] }[]; nextSeq: number; status: 'running' | 'exited'; exitCode: number | null; truncated: boolean; session?: TerminalSession; }
export interface AutonameTarget { taskId?: string; channelId?: string; terminalId?: string; content?: string; }
