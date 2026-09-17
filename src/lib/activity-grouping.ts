import type { ApprovalRequest, Message, RunEvent } from '$lib/types';

type RecordValue = Record<string, unknown>;
const acpRecord = (value: unknown): value is RecordValue => !!value && typeof value === 'object' && !Array.isArray(value);

export interface NativeSubagentActivity {
  type: 'collabAgentToolCall' | 'subAgentActivity';
  id: string;
  action: string;
  phase: string;
  agentPath: string | null;
  agentThreadId: string | null;
  receiverThreadIds: string[];
  prompt: string | null;
  model: string | null;
  reasoningEffort: string | null;
  agentsStates: Record<string, { status: string; message: string }>;
}

/** Structured Codex collaboration items, kept distinct from Monitter-routed delegations. */
export function nativeSubagentActivity(event: RunEvent): NativeSubagentActivity | null {
  if (event.kind !== 'tool' && event.kind !== 'subagent') return null;
  let detail: unknown;
  try { detail = JSON.parse(event.detail); }
  catch { return null; }
  if (!acpRecord(detail)) return null;
  const activity = acpRecord(detail.activity) ? detail.activity : detail;
  const type = ['collabAgentToolCall', 'subAgentActivity'].includes(String(activity.type))
    ? activity.type as NativeSubagentActivity['type']
    : typeof activity.agentThreadId === 'string' || typeof activity.agent_thread_id === 'string' ? 'subAgentActivity' : null;
  if (!type) return null;
  const receivers = activity.receiverThreadIds ?? activity.receiver_thread_ids;
  const states = acpRecord(activity.agentsStates)
    ? Object.fromEntries(Object.entries(activity.agentsStates).flatMap(([id, value]) => acpRecord(value)
      ? [[id, { status: typeof value.status === 'string' ? value.status : '', message: typeof value.message === 'string' ? value.message : '' }]]
      : []))
    : {};
  return {
    type, id: typeof activity.id === 'string' ? activity.id : event.id,
    action: typeof activity.tool === 'string' ? activity.tool : typeof activity.kind === 'string' ? activity.kind : '',
    phase: typeof activity.status === 'string' ? activity.status : '',
    agentPath: typeof activity.agentPath === 'string' ? activity.agentPath : null,
    agentThreadId: typeof activity.agentThreadId === 'string' ? activity.agentThreadId : typeof activity.agent_thread_id === 'string' ? activity.agent_thread_id : null,
    receiverThreadIds: Array.isArray(receivers) ? receivers.filter((value): value is string => typeof value === 'string') : [],
    prompt: typeof activity.prompt === 'string' && activity.prompt.trim() ? activity.prompt.trim() : null,
    model: typeof activity.model === 'string' && activity.model.trim() ? activity.model.trim() : null,
    reasoningEffort: typeof activity.reasoningEffort === 'string' && activity.reasoningEffort.trim() ? activity.reasoningEffort.trim() : null,
    agentsStates: states,
  };
}

function nativeSubagentName(value: NativeSubagentActivity): string {
  const leaf = value.agentPath?.split('/').filter(Boolean).at(-1)?.replaceAll(/[_-]+/g, ' ').trim();
  return leaf ? leaf.replace(/\b\w/g, character => character.toUpperCase()) : 'Subagent';
}

/** Item start/completion envelopes share an item id; retain one semantic action. */
function coalesceNativeSubagentActivity(events: RunEvent[]): RunEvent[] {
  const result: RunEvent[] = [];
  const positions = new Map<string, number>();
  for (const event of events) {
    const activity = nativeSubagentActivity(event);
    if (!activity) { result.push(event); continue; }
    const key = `${event.taskId}:${activity.type}:${activity.id}`;
    const position = positions.get(key);
    if (position === undefined) {
      positions.set(key, result.length);
      result.push(event);
      continue;
    }
    const previous = result[position];
    // Prefer the completion payload because it contains receiver IDs and final
    // agent-state messages, while retaining the first timestamp for ordering.
    result[position] = { ...event, createdAt: previous.createdAt };
  }
  return result;
}

/** ACP envelopes are retained in historical and current diagnostic journals. */
function acpUpdate(event: RunEvent): RecordValue | null {
  try {
    const update = JSON.parse(event.detail)?.update;
    return acpRecord(update) && ['tool_call', 'tool_call_update', 'plan'].includes(String(update.sessionUpdate)) ? update : null;
  } catch { return null; }
}
function acpToolIdentity(event: RunEvent): string | null {
  const update = acpUpdate(event);
  if (!update) return null;
  if (update.sessionUpdate === 'plan') return 'plan';
  const kinds: Record<string, string> = { read: 'read', edit: 'edit', delete: 'edit', move: 'edit', search: 'search_files', execute: 'command_execution', think: 'plan', fetch: 'web_fetch', switch_mode: 'plan' };
  return kinds[String(update.kind)] ?? (typeof update.title === 'string' && update.title.trim() ? update.title.trim().toLowerCase() : 'tool');
}
/** Collapse old lifecycle echoes without rewriting the diagnostic journal. */
function coalesceAcpActivity(events: RunEvent[], messages: Message[]): RunEvent[] {
  const result: RunEvent[] = [];
  const calls = new Map<string, number>();
  const boundaries = messages.filter(message => message.role === 'user' || message.role === 'system');
  for (const event of [...events].sort((a, b) => a.createdAt - b.createdAt)) {
    const update = acpUpdate(event);
    const callId = update?.sessionUpdate === 'plan' ? 'plan' : update?.toolCallId;
    if (!update || typeof callId !== 'string' || !callId) { result.push(event); continue; }
    const envelope = JSON.parse(event.detail);
    const boundary = boundaries.reduce((latest, message) => message.taskId === event.taskId && message.createdAt <= event.createdAt ? Math.max(latest, message.createdAt) : latest, -1);
    const key = JSON.stringify([event.taskId, envelope.sessionId, boundary, update.sessionUpdate === 'plan' ? 'plan' : 'tool', callId]);
    const index = calls.get(key);
    if (index === undefined) { calls.set(key, result.length); result.push(event); continue; }
    const previous = result[index];
    const merged = { ...acpUpdate(previous), ...Object.fromEntries(Object.entries(update).filter(([, value]) => value !== null)) };
    result[index] = { ...event, createdAt: previous.createdAt, title: typeof merged.title === 'string' ? merged.title : previous.title, detail: JSON.stringify({ ...envelope, update: merged }) };
  }
  return result;
}
function acpReadableDetail(event: RunEvent): string | null {
  const update = acpUpdate(event);
  if (!update) return null;
  const lines: string[] = [];
  if (typeof update.title === 'string') lines.push(update.title);
  if (typeof update.status === 'string') lines.push(`Status: ${update.status.replaceAll('_', ' ')}`);
  const text = (value: unknown, depth = 0): string[] => {
    if (depth > 5 || value == null) return [];
    if (typeof value === 'string') return value.trim() ? [value] : [];
    if (Array.isArray(value)) return value.flatMap(item => text(item, depth + 1));
    if (!acpRecord(value)) return [];
    if (value.type === 'diff') return [`Changed ${String(value.path ?? 'file')}`, ...(typeof value.newText === 'string' ? [value.newText] : [])];
    if (value.type === 'terminal') return ['Terminal output is available in the terminal.'];
    return text(value.text ?? value.content ?? value.message ?? value.error, depth + 1);
  };
  if (acpRecord(update.rawInput)) {
    for (const key of ['command', 'cmd', 'file_path', 'path', 'pattern', 'query', 'url']) {
      const value = update.rawInput[key];
      if (typeof value === 'string' && value.trim()) lines.push(`${key}: ${value}`);
    }
  }
  lines.push(...text(update.content), ...text(update.rawOutput), ...text(update.error));
  if (Array.isArray(update.entries)) {
    for (const entry of update.entries) if (acpRecord(entry) && typeof entry.content === 'string') lines.push(`${String(entry.status ?? 'pending').replaceAll('_', ' ')}: ${entry.content}`);
  }
  return [...new Set(lines)].join('\n\n') || 'Tool update; no additional output reported.';
}

/** Durable, user-authored Stop record emitted by the native cancellation path. */
export const CANCELLATION_EVENT_TITLE = 'You cancelled this run.';
export const CONTEXT_CLEARED_TITLE = 'Context Cleared';

/** Only this concise status record belongs in the ordinary chat transcript. */
export function isCancellationEvent(event: RunEvent): boolean {
  return event.kind === 'status' && event.title === CANCELLATION_EVENT_TITLE;
}

/** The transcript companion to the diagnostic cancellation event. */
export function isCancellationMessage(message: Pick<Message, 'role' | 'text' | 'senderAgentId' | 'attachments'>): boolean {
  return message.role === 'system' && message.text === CANCELLATION_EVENT_TITLE &&
    !message.senderAgentId && !message.attachments?.length;
}

/** A durable boundary between independent provider contexts in one visible chat. */
export function isContextClearedMessage(message: Pick<Message, 'role' | 'text' | 'senderAgentId' | 'attachments'>): boolean {
  return message.role === 'system' && message.text === CONTEXT_CLEARED_TITLE &&
    !message.senderAgentId && !message.attachments?.length;
}

/** Extract displayable summary text, never provider IDs or encrypted metadata. */
export function reasoningSummary(detail: string): string {
  const trimmed = detail.trim();
  if (!trimmed) return '';
  const text = (value: unknown, depth = 0): string => {
    if (depth > 8) return '';
    if (typeof value === 'string') return value.trim();
    if (Array.isArray(value)) return value.map(item => text(item, depth + 1)).filter(Boolean).join('\n\n');
    if (!value || typeof value !== 'object') return '';
    const record = value as Record<string, unknown>;
    // Codex sends summary text blocks or strings; other harnesses can supply
    // plain text/content. Prefer the summary when both representations exist.
    return text(record.summary, depth + 1) || text(record.text, depth + 1) || text(record.content, depth + 1);
  };
  try { return text(JSON.parse(trimmed)); }
  catch { return trimmed; }
}

export function isBlankReasoning(event: RunEvent): boolean {
  return event.kind === 'reasoning' && !reasoningSummary(event.detail);
}

/**
 * Codex app-server lifecycle echoes for conversation items are not tool work.
 * Their completed counterparts are persisted as ordinary bubbles by the adapter;
 * only a structured native item type is safe to hide here.
 */
export function isNativeMessageTransportArtifact(event: RunEvent): boolean {
  if (event.kind !== 'tool') return false;
  try {
    const type = JSON.parse(event.detail)?.type;
    return typeof type === 'string' && ['usermessage', 'agentmessage'].includes(type.toLowerCase());
  } catch { return false; }
}

export type ConversationActivityItem =
  | { type: 'message'; value: Message }
  | { type: 'activity'; value: RunEvent }
  | { type: 'approval'; value: ApprovalRequest }
  | { type: 'reasoning-group'; values: RunEvent[] }
  | { type: 'tool-group'; values: RunEvent[] };

/** One waiting indicator per conversation, never alongside a reply or approval. */
export function showThinkingFallback(items: ConversationActivityItem[], working: boolean, awaitingApproval = false): boolean {
  if (!working || awaitingApproval) return false;
  const latest = items.at(-1);
  if (latest?.type === 'reasoning-group') return false;
  return !(latest?.type === 'message' && latest.value.role === 'assistant');
}

function toolIdentity(event: RunEvent) {
  const acpIdentity = acpToolIdentity(event);
  if (acpIdentity) return acpIdentity;
  const normalize = (value: string) => {
    const title = value.trim().toLowerCase();
    const aliases: Record<string, string> = {
      'web search': 'web_search', 'run command': 'command_execution',
      'command execution': 'command_execution', 'file change': 'file_change',
      websearch: 'web_search', commandexecution: 'command_execution', filechange: 'file_change',
    };
    return aliases[title] || title;
  };
  const title = normalize(event.title);
  const specificTitle = title && !['tool activity', 'acp tool activity', 'tool result', 'tool', 'tool_result'].includes(title);
  try {
    const detail = JSON.parse(event.detail);
    const type = typeof detail?.type === 'string' ? normalize(detail.type) : '';
    // Native event kinds remain consistent across structured and plain-text
    // lifecycle updates. Arguments and incidental result names are not identity.
    if (['command_execution', 'web_search', 'file_change'].includes(type)) return type;
    if (specificTitle) return title;
    if (typeof detail?.tool === 'string' && detail.tool.trim()) return normalize(detail.tool);
  } catch { /* Plain-text updates retain the same tool identity. */ }
  return specificTitle ? title : `event:${event.id}`;
}

/** Connector methods (gmail.search_emails / gmail.read_email) share a family. */
export function toolFamily(event: RunEvent) {
  const identity = toolIdentity(event);
  const mcp = identity.match(/^(?:mcp__)?([^_]+(?:_[^_]+)*)__([^_].*)$/);
  if (mcp) return mcp[1];
  const dotted = identity.match(/^([a-z][a-z0-9_-]*)[.\/]([a-z][a-z0-9_-]*)$/);
  return dotted ? dotted[1] : identity;
}

/** Recognize harness shell tools without exposing command arguments in the summary. */
export function isShellActivity(event: RunEvent): boolean {
  const identity = toolIdentity(event);
  if (['command_execution', 'bash', 'shell', 'shell_command', 'exec_command', 'functions.exec_command'].includes(identity)) return true;
  return /^(?:\/[^\s]+\/)?(?:ba|z|fi|k|da)?sh(?:\s|$)/i.test(event.title.trim());
}

/** Context compaction lifecycle updates share an app-server item id. */
export function contextCompactionId(event: RunEvent): string | null {
  const normalize = (value: string) => value.replaceAll(/[\s_-]/g, '').toLowerCase();
  let detail: Record<string, unknown> | null = null;
  try {
    const parsed = JSON.parse(event.detail);
    detail = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null;
  } catch { /* A title-only event has no durable lifecycle identity. */ }
  const type = normalize(String(detail?.type ?? event.title));
  const id = detail?.id;
  return type === 'contextcompaction' && typeof id === 'string' && id.trim() ? id : null;
}

export function isContextCompaction(event: RunEvent): boolean {
  const normalize = (value: string) => value.replaceAll(/[\s_-]/g, '').toLowerCase();
  if (normalize(event.title) === 'contextcompaction') return true;
  try { return normalize(String(JSON.parse(event.detail)?.type ?? '')) === 'contextcompaction'; }
  catch { return false; }
}

/** New app-server records explicitly retain the compaction lifecycle phase. */
export function contextCompactionPhase(event: RunEvent): 'started' | 'completed' | null {
  if (!isContextCompaction(event)) return null;
  try {
    const phase = JSON.parse(event.detail)?.monitterPhase;
    return phase === 'started' || phase === 'completed' ? phase : null;
  } catch { return null; }
}

/** Stable intent for a tool call, independent of which harness reported it. */
export type ToolCategory =
  | 'shell'
  | 'read'
  | 'edit'
  | 'search_files'
  | 'list_files'
  | 'web_search'
  | 'web_fetch'
  | 'image'
  | 'tool_search'
  | 'todo'
  | 'task'
  | 'task_coordination'
  | 'user_input'
  | 'skill'
  | 'computer'
  | 'schedule'
  | 'mcp'
  | 'other';

const READ_IDS = new Set(['read', 'view', 'cat', 'head', 'tail', 'fileread', 'notebookread', 'getfilecontents']);
const EDIT_IDS = new Set(['filechange', 'write', 'edit', 'patch', 'apply', 'applypatch', 'multiedit', 'notebookedit', 'strreplace', 'createfile']);
const LIST_IDS = new Set(['ls', 'list', 'listfiles', 'listdirectory', 'listdir']);
const SEARCH_FILES_IDS = new Set(['grep', 'rg', 'ripgrep', 'glob', 'find', 'filesearch', 'searchfiles', 'codesearch']);
const WEB_SEARCH_IDS = new Set(['websearch', 'searchweb']);
const WEB_FETCH_IDS = new Set(['webfetch', 'fetchurl', 'httpget', 'fetch', 'curl']);
const IMAGE_IDS = new Set(['imageview', 'viewimage']);
const TOOL_SEARCH_IDS = new Set(['toolsearch', 'searchtools']);
const TODO_IDS = new Set(['todowrite', 'updatetodo', 'updateplan', 'todo', 'plan']);
const TASK_IDS = new Set(['task', 'delegatetask', 'delegate', 'spawnagent']);
const TASK_COORDINATION_IDS = new Set([
  'taskstatus', 'taskresult', 'taskmessage', 'taskupdate', 'waitfortask', 'wait',
  'gettaskresult', 'listagents', 'sendmessage', 'closeagent',
]);
const USER_INPUT_IDS = new Set(['question', 'askuser', 'waitforuser', 'userinput', 'requestinput']);
const SKILL_IDS = new Set(['skill', 'loadskill', 'useskill', 'invokeskill']);
const COMPUTER_IDS = new Set(['computeruse', 'computeraction', 'computer', 'cua', 'js']);
const SCHEDULE_IDS = new Set(['schedulewakeup']);
const CONNECTOR_RE = /^(?:mcp__)?([^_]+(?:_[^_]+)*)__([^_].*)$/;
const DOTTED_RE = /^([a-z][a-z0-9_-]*)[.\/]([a-z][a-z0-9_-]*)$/;

function normalizeToolId(value: string): string {
  return value.toLowerCase().replaceAll(/[_-]+/g, '');
}

function friendlyToolName(value: string): string {
  const cleaned = value.replaceAll(/[_-]+/g, ' ').trim();
  return cleaned ? cleaned.replace(/\b\w/g, char => char.toUpperCase()) : '';
}

/** Map raw Codex, Claude and OpenCode names to one user-facing intent. */
export function toolCategory(event: RunEvent): ToolCategory {
  const nativeSubagent = nativeSubagentActivity(event);
  if (nativeSubagent?.type === 'subAgentActivity' || nativeSubagent?.action === 'spawnAgent') return 'task';
  if (nativeSubagent) return 'task_coordination';
  if (isShellActivity(event)) return 'shell';
  const identity = toolIdentity(event);
  const id = normalizeToolId(identity);
  if (COMPUTER_IDS.has(id)) return 'computer';
  if (SCHEDULE_IDS.has(id)) return 'schedule';
  if (IMAGE_IDS.has(id)) return 'image';
  if (TOOL_SEARCH_IDS.has(id)) return 'tool_search';
  if (TODO_IDS.has(id)) return 'todo';
  if (TASK_IDS.has(id)) return 'task';
  if (TASK_COORDINATION_IDS.has(id)) return 'task_coordination';
  if (USER_INPUT_IDS.has(id)) return 'user_input';
  if (SKILL_IDS.has(id)) return 'skill';
  if (WEB_SEARCH_IDS.has(id)) return 'web_search';
  if (WEB_FETCH_IDS.has(id)) return 'web_fetch';
  if (EDIT_IDS.has(id)) return 'edit';
  if (READ_IDS.has(id)) return 'read';
  if (LIST_IDS.has(id)) return 'list_files';
  if (SEARCH_FILES_IDS.has(id)) return 'search_files';
  if (CONNECTOR_RE.test(identity) || DOTTED_RE.test(identity)) return 'mcp';
  return 'other';
}

export interface ToolPresentation {
  icon: 'square-terminal' | 'eye' | 'file-pen' | 'file-search' | 'folder-open'
    | 'globe' | 'download' | 'image' | 'search' | 'list-checks' | 'bot' | 'users'
    | 'message-circle' | 'book-open' | 'monitor' | 'alarm-clock' | 'plug' | 'wrench' | 'terminal';
  label: string;
}

export interface ToolImage {
  /** The provider-approved local path that its image-view tool read. */
  path: string;
  /** A deliberately path-free label for the transcript. */
  name: string;
}

/**
 * Image-view events carry the exact file Codex viewed. Keep that path in the
 * diagnostic payload, but only surface the filename in the compact timeline.
 */
export function toolImage(event: RunEvent): ToolImage | null {
  if (toolCategory(event) !== 'image') return null;
  try {
    const parsed = parseToolDetail(event.detail);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
    const path = (parsed as Record<string, unknown>).path;
    if (typeof path !== 'string' || !path.trim()) return null;
    const cleaned = path.trim();
    const name = cleaned.split(/[\\/]/).filter(Boolean).at(-1) || 'image';
    return { path: cleaned, name };
  } catch {
    return null;
  }
}

/** Friendly label and icon; raw provider tool names stay in the expandable detail. */
export function toolPresentation(event: RunEvent, inProgress: boolean): ToolPresentation {
  const nativeSubagent = nativeSubagentActivity(event);
  if (nativeSubagent) {
    const name = nativeSubagentName(nativeSubagent);
    if (nativeSubagent.type === 'subAgentActivity') {
      if (nativeSubagent.action === 'started') return { icon: 'bot', label: `${name} started` };
      if (nativeSubagent.action === 'interacted') return { icon: 'message-circle', label: `${name} sent an update` };
      if (nativeSubagent.action === 'completed') return { icon: 'bot', label: `${name} finished` };
      return { icon: 'bot', label: `${name} activity` };
    }
    inProgress = nativeSubagent.phase === 'inProgress';
    const count = nativeSubagent.receiverThreadIds.length;
    const target = count > 1 ? `${count} subagents` : 'a subagent';
    if (nativeSubagent.action === 'spawnAgent') return { icon: 'bot', label: inProgress ? 'Starting a subagent' : 'Started a subagent' };
    if (nativeSubagent.action === 'sendInput') return { icon: 'message-circle', label: inProgress ? `Sending an update to ${target}` : `Sent an update to ${target}` };
    if (nativeSubagent.action === 'wait') return { icon: 'users', label: inProgress ? `Waiting for ${target}` : `Received updates from ${target}` };
    if (nativeSubagent.action === 'closeAgent') return { icon: 'users', label: inProgress ? `Finishing with ${target}` : `Finished with ${target}` };
  }
  const acp = acpUpdate(event);
  if (acp?.status === 'failed') return { icon: 'wrench', label: 'Tool failed' };
  if (acp?.status === 'completed') inProgress = false;
  if (acp && acpToolIdentity(event) === 'tool') return { icon: 'wrench', label: inProgress ? 'Using a tool' : 'Used a tool' };
  const tense = (now: string, past: string) => inProgress ? now : past;
  switch (toolCategory(event)) {
    case 'shell': return { icon: 'square-terminal', label: tense('Run a command', 'Ran a command') };
    case 'read': return { icon: 'eye', label: tense('Read a file', 'Read a file') };
    case 'edit': return { icon: 'file-pen', label: tense('Edit a file', 'Edited a file') };
    case 'search_files': return { icon: 'file-search', label: tense('Search files', 'Searched files') };
    case 'list_files': return { icon: 'folder-open', label: tense('List files', 'Listed files') };
    case 'web_search': return { icon: 'globe', label: tense('Search the web', 'Searched the web') };
    case 'web_fetch': return { icon: 'download', label: tense('Fetch a web page', 'Fetched a web page') };
    case 'image': {
      const image = toolImage(event);
      return { icon: 'image', label: tense(`Viewing image${image ? `: ${image.name}` : ''}`, `Viewed image${image ? `: ${image.name}` : ''}`) };
    }
    case 'tool_search': return { icon: 'search', label: tense('Find a tool', 'Found a tool') };
    case 'todo': return { icon: 'list-checks', label: tense('Update the plan', 'Updated the plan') };
    case 'task': return { icon: 'bot', label: tense('Start an agent', 'Started an agent') };
    case 'task_coordination': return { icon: 'users', label: tense('Coordinate agents', 'Coordinated agents') };
    case 'user_input': return { icon: 'message-circle', label: tense('Wait for input', 'Received input') };
    case 'skill': return { icon: 'book-open', label: tense('Load a skill', 'Loaded a skill') };
    case 'computer': return { icon: 'monitor', label: tense('Use the computer', 'Used the computer') };
    case 'schedule': return { icon: 'alarm-clock', label: tense('Schedule a wake-up', 'Scheduled a wake-up') };
    case 'mcp': {
      const name = friendlyToolName(toolFamily(event)) || 'a connector';
      return { icon: 'plug', label: tense(`Work in ${name}`, `Worked in ${name}`) };
    }
    case 'other': {
      const family = toolFamily(event);
      const name = family.startsWith('event:') ? '' : friendlyToolName(family);
      return { icon: name ? 'wrench' : 'terminal', label: tense(`Use ${name || 'a tool'}`, `Used ${name || 'a tool'}`) };
    }
  }
}

const MAX_DETAIL_LENGTH = 6_000;

function truncateDetail(value: string): string {
  return value.length <= MAX_DETAIL_LENGTH ? value : `${value.slice(0, MAX_DETAIL_LENGTH).trimEnd()}\n\n…output truncated`;
}

function displayConnectorTool(value: string): string {
  return value
    .replace(/^mcp__/, '')
    .replace('__', '.')
    .replaceAll('_', ' ');
}

function contentText(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap(item => {
    if (!item || typeof item !== 'object') return [];
    const text = (item as Record<string, unknown>).text;
    return typeof text === 'string' && text.trim() && text.trim() !== 'Action completed.' ? [text.trim()] : [];
  });
}

function structuredSummary(value: unknown): string[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [];
  const record = value as Record<string, unknown>;
  const lines: string[] = [];
  const preferred = ['message', 'summary', 'subject', 'filename', 'email', 'name'];
  for (const key of preferred) {
    const item = record[key];
    if (typeof item === 'string' && item.trim()) lines.push(`${friendlyToolName(key)}: ${item.trim()}`);
  }
  for (const [key, item] of Object.entries(record)) {
    if (!Array.isArray(item)) continue;
    const label = friendlyToolName(key).toLowerCase();
    lines.push(`${item.length} ${label}`);
    if (key === 'emails') {
      for (const email of item.slice(0, 8)) {
        if (!email || typeof email !== 'object') continue;
        const subject = (email as Record<string, unknown>).subject;
        const from = (email as Record<string, unknown>).from_;
        if (typeof subject === 'string') lines.push(`• ${subject}${typeof from === 'string' ? ` — ${from}` : ''}`);
      }
      if (item.length > 8) lines.push(`• …and ${item.length - 8} more`);
    }
  }
  return [...new Set(lines)];
}

export interface ToolFileChange {
  path: string;
  verb: 'Created' | 'Updated' | 'Deleted';
  added: number;
  removed: number;
  diff: string;
}

function parseToolDetail(raw: string): unknown {
  let parsed: unknown = JSON.parse(raw);
  // Some harness adapters retain the provider payload as a JSON string inside
  // the event JSON. Unwrap that transport layer before interpreting the tool.
  for (let depth = 0; depth < 3 && typeof parsed === 'string'; depth += 1) {
    try { parsed = JSON.parse(parsed); }
    catch { break; }
  }
  return parsed;
}

function findFileChangePayload(value: unknown, depth = 0): Record<string, unknown> | null {
  if (depth > 5 || value == null) return null;
  if (typeof value === 'string') {
    try { return findFileChangePayload(parseToolDetail(value), depth + 1); }
    catch { return null; }
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findFileChangePayload(item, depth + 1);
      if (found) return found;
    }
    return null;
  }
  if (typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  if (Array.isArray(record.changes)) return record;
  for (const key of ['result', 'detail', 'content', 'structured_content', 'structuredContent']) {
    const found = findFileChangePayload(record[key], depth + 1);
    if (found) return found;
  }
  if (typeof record.text === 'string') return findFileChangePayload(record.text, depth + 1);
  return null;
}

function salvageTruncatedFileChanges(raw: string): ToolFileChange[] {
  const changes: ToolFileChange[] = [];
  const diffPattern = /"diff":"((?:\\.|[^"\\])*)"/g;
  for (const match of raw.matchAll(diffPattern)) {
    let diff = '';
    try { diff = JSON.parse(`"${match[1]}"`); }
    catch { continue; }
    const remainder = raw.slice((match.index ?? 0) + match[0].length);
    const nextDiff = remainder.search(diffPattern);
    const recordTail = nextDiff >= 0 ? remainder.slice(0, nextDiff) : remainder;
    const pathMatch = recordTail.match(/"path":"([^"\n]*)/);
    const path = pathMatch?.[1]?.replace(/…?\s*\[truncated\].*$/, '').trim() || 'Changed file';
    const lines = diff.split('\n');
    const added = lines.filter(line => line.startsWith('+') && !line.startsWith('+++')).length;
    const removed = lines.filter(line => line.startsWith('-') && !line.startsWith('---')).length;
    changes.push({ path, verb: 'Updated', added, removed, diff });
  }
  return changes;
}

/** Extract file patches without leaking unrelated provider metadata into the UI. */
export function toolFileChanges(event: RunEvent): ToolFileChange[] {
  if (toolCategory(event) !== 'edit') return [];
  let parsed: unknown;
  try { parsed = parseToolDetail(event.detail); }
  catch { return salvageTruncatedFileChanges(event.detail); }
  const payload = findFileChangePayload(parsed);
  if (!payload) return [];
  const changes = payload.changes;
  if (!Array.isArray(changes)) return [];
  return changes.flatMap(change => {
    if (!change || typeof change !== 'object') return [];
    const record = change as Record<string, unknown>;
    const path = typeof record.path === 'string' ? record.path : 'a file';
    const diff = typeof record.diff === 'string' ? record.diff : '';
    const lines = diff.split('\n');
    const added = lines.filter(line => line.startsWith('+') && !line.startsWith('+++')).length;
    const removed = lines.filter(line => line.startsWith('-') && !line.startsWith('---')).length;
    const kind = record.kind && typeof record.kind === 'object'
      ? String((record.kind as Record<string, unknown>).type ?? 'update')
      : 'update';
    const verb = kind === 'add' || kind === 'create' ? 'Created' : kind === 'delete' ? 'Deleted' : 'Updated';
    return [{ path, verb, added, removed, diff } satisfies ToolFileChange];
  });
}

/** Convert provider JSON into the useful human-readable part of a tool event. */
export function readableToolDetail(event: RunEvent): string {
  const nativeSubagent = nativeSubagentActivity(event);
  if (nativeSubagent) {
    const lines: string[] = [];
    if (nativeSubagent.type === 'subAgentActivity') {
      const verb = nativeSubagent.action === 'started' ? 'started working' : nativeSubagent.action === 'completed' ? 'finished' : 'sent an update';
      lines.push(`${nativeSubagentName(nativeSubagent)} ${verb}.`);
    } else {
      if (nativeSubagent.prompt) lines.push(`${nativeSubagent.action === 'sendInput' ? 'Update' : 'Task'}\n${nativeSubagent.prompt}`);
      const configuration = [nativeSubagent.model, nativeSubagent.reasoningEffort].filter(Boolean).join(' · ');
      if (configuration) lines.push(configuration);
      for (const [index, state] of Object.values(nativeSubagent.agentsStates).entries()) {
        const status = state.status ? state.status.replaceAll(/([a-z])([A-Z])/g, '$1 $2').toLowerCase() : 'updated';
        lines.push(`${nativeSubagent.receiverThreadIds.length > 1 ? `Subagent ${index + 1}` : 'Subagent'} · ${status}${state.message ? `\n${state.message}` : ''}`);
      }
    }
    return truncateDetail(lines.join('\n\n') || 'Subagent activity updated.');
  }
  const acp = acpReadableDetail(event);
  if (acp !== null) return truncateDetail(acp);
  const raw = event.detail.trim();
  if (!raw || raw === 'null') return 'No additional details.';
  const category = toolCategory(event);
  if (category === 'edit') {
    const changes = toolFileChanges(event);
    if (changes.length) return truncateDetail(changes.map(change => {
      const count = change.added || change.removed ? ` (+${change.added} −${change.removed})` : '';
      return `${change.verb} ${change.path}${count}`;
    }).join('\n'));
  }
  let parsed: unknown;
  try { parsed = parseToolDetail(raw); }
  catch { return truncateDetail(raw); }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return truncateDetail(String(parsed ?? 'No additional details.'));

  const detail = parsed as Record<string, unknown>;

  if (category === 'tool_search' && typeof detail.query === 'string') {
    const query = detail.query.replace(/^select:/, '').split(',').map(item => displayConnectorTool(item.trim())).filter(Boolean);
    return query.length ? `Tools requested:\n${query.map(item => `• ${item}`).join('\n')}` : 'Searched available tools.';
  }

  if (category === 'image' && typeof detail.path === 'string') return `Viewed ${detail.path}`;

  const lines: string[] = [];
  if (category === 'web_search' && typeof detail.query === 'string' && detail.query.trim()) lines.push(`Query: ${detail.query.trim()}`);
  if (category === 'shell') {
    const command = detail.command ?? detail.cmd;
    if (typeof command === 'string' && command.trim()) lines.push(`Command\n${command.trim()}`);
    else if (Array.isArray(command)) lines.push(`Command\n${command.map(String).join(' ')}`);
  }
  if (typeof detail.error === 'string' && detail.error.trim()) lines.push(`Error\n${detail.error.trim()}`);
  for (const key of ['message', 'summary', 'output', 'result'] as const) {
    const value = detail[key];
    if (typeof value === 'string' && value.trim()) lines.push(`${friendlyToolName(key)}\n${value.trim()}`);
  }
  lines.push(...contentText(detail.content));
  lines.push(...structuredSummary(detail.structured_content ?? detail.structuredContent));
  if (Array.isArray(detail.results)) lines.push(`${detail.results.length} results returned`);
  if (typeof detail.path === 'string' && !lines.length) lines.push(`Path: ${detail.path}`);
  if (typeof detail.query === 'string' && detail.query.trim() && !lines.length) lines.push(`Query: ${detail.query.trim()}`);
  if (lines.length) return truncateDetail([...new Set(lines)].join('\n\n'));

  return 'Completed without additional output.';
}

export function groupConversationActivity(
  messages: Message[],
  events: RunEvent[],
  compressToolCalls = false,
  approvals: ApprovalRequest[] = [],
): ConversationActivityItem[] {
  const sourceEvents = events;
  events = coalesceAcpActivity(coalesceNativeSubagentActivity(events), messages);
  const ordered = [
    ...messages.map(value => ({ type: 'message' as const, value, at: value.createdAt })),
    ...events.filter(event => !isNativeMessageTransportArtifact(event)).map(value => ({ type: 'activity' as const, value, at: value.createdAt })),
    // Resolution is the user-visible event; do not move it back to request creation.
    ...approvals.filter(value => value.status !== 'pending').map(value => ({ type: 'approval' as const, value, at: value.resolvedAt ?? value.createdAt })),
  ].sort((left, right) => left.at - right.at);

  const grouped: ConversationActivityItem[] = [];
  for (const item of ordered) {
    if (item.type === 'activity' && item.value.kind === 'reasoning') {
      const previous = grouped.at(-1);
      if (previous?.type === 'reasoning-group' && previous.values[0].taskId === item.value.taskId) {
        previous.values.push(item.value);
      } else {
        grouped.push({ type: 'reasoning-group', values: [item.value] });
      }
      continue;
    }
    if (item.type !== 'activity' || (item.value.kind !== 'tool' && item.value.kind !== 'subagent')) {
      // `at` is only a sorting aid; the richer item remains structurally
      // compatible with the public discriminated union returned from here.
      grouped.push(item);
      continue;
    }
    const previous = grouped.at(-1);
    const previousCompaction = previous?.type === 'tool-group' && isContextCompaction(previous.values.at(-1)!);
    const currentCompaction = isContextCompaction(item.value);
    const previousCompactionId = previousCompaction ? contextCompactionId(previous!.values.at(-1)!) : null;
    const currentCompactionId = contextCompactionId(item.value);
    const compactionLifecycle = previousCompaction || currentCompaction;
    const previousNativeSubagent = previous?.type === 'tool-group' ? nativeSubagentActivity(previous.values.at(-1)!) : null;
    const currentNativeSubagent = nativeSubagentActivity(item.value);
    if (previous?.type === 'tool-group' && !previousNativeSubagent && !currentNativeSubagent && (
      compactionLifecycle
        ? previousCompactionId !== null && previousCompactionId === currentCompactionId
        : toolFamily(previous.values[0]) === toolFamily(item.value)
          && (acpUpdate(previous.values[0])?.status === 'failed') === (acpUpdate(item.value)?.status === 'failed')
    )) {
      previous.values.push(item.value);
    } else {
      grouped.push({ type: 'tool-group', values: [item.value] });
    }
  }
  // Empty reasoning is a temporary waiting status, not transcript content. Any
  // later visible event replaces it; only the current trailing status survives.
  const latestReplacementAt = new Map<string, number>();
  for (const message of messages) {
    if (!message.text.trim() && !message.attachments?.length) continue;
    latestReplacementAt.set(message.taskId, Math.max(latestReplacementAt.get(message.taskId) ?? -Infinity, message.createdAt));
  }
  for (const event of sourceEvents) {
    if (isNativeMessageTransportArtifact(event) || isBlankReasoning(event)) continue;
    latestReplacementAt.set(event.taskId, Math.max(latestReplacementAt.get(event.taskId) ?? -Infinity, event.createdAt));
  }
  for (const approval of approvals) {
    if (approval.status === 'pending') continue;
    latestReplacementAt.set(approval.taskId, Math.max(latestReplacementAt.get(approval.taskId) ?? -Infinity, approval.resolvedAt ?? approval.createdAt));
  }
  const visible = grouped.filter(item => item.type !== 'reasoning-group' || item.values.some(value => !isBlankReasoning(value)) ||
    (latestReplacementAt.get(item.values[0].taskId) ?? -Infinity) < item.values.at(-1)!.createdAt);
  if (!compressToolCalls) return visible;

  // Compression is turn-scoped rather than adjacency-scoped. Transient thinking,
  // reasoning summaries and compaction rows may sit between calls without
  // fragmenting one family into repeated 2-call summaries. Move an aggregate to
  // its latest occurrence so the compact timeline still reads chronologically.
  const compacted: ConversationActivityItem[] = [];
  const families = new Map<string, Extract<ConversationActivityItem, { type: 'tool-group' }>>();
  for (const item of visible) {
    if (item.type === 'message' || item.type === 'approval') {
      families.clear();
      compacted.push(item);
      continue;
    }
    if (item.type !== 'tool-group' || item.values.every(isContextCompaction) || item.values.some(nativeSubagentActivity)) {
      compacted.push(item);
      continue;
    }
    const family = item.values.every(isShellActivity) ? 'command_execution' : toolFamily(item.values[0]);
    const key = `${item.values[0].taskId}:${family}:${item.values.some(event => acpUpdate(event)?.status === 'failed') ? 'failed' : ''}`;
    const existing = families.get(key);
    if (!existing) {
      families.set(key, item);
      compacted.push(item);
      continue;
    }
    existing.values.push(...item.values);
    const earlier = compacted.indexOf(existing);
    if (earlier >= 0) compacted.splice(earlier, 1);
    compacted.push(existing);
  }
  return compacted;
}
