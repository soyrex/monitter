import type { ApprovalRequest, Message, RunEvent } from '$lib/types';

/** Durable, user-authored Stop record emitted by the native cancellation path. */
export const CANCELLATION_EVENT_TITLE = 'You cancelled this run.';

/** Only this concise status record belongs in the ordinary chat transcript. */
export function isCancellationEvent(event: RunEvent): boolean {
  return event.kind === 'status' && event.title === CANCELLATION_EVENT_TITLE;
}

/** The transcript companion to the diagnostic cancellation event. */
export function isCancellationMessage(message: Pick<Message, 'role' | 'text' | 'senderAgentId' | 'attachments'>): boolean {
  return message.role === 'system' && message.text === CANCELLATION_EVENT_TITLE &&
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
  const normalize = (value: string) => {
    const title = value.trim().toLowerCase();
    const aliases: Record<string, string> = {
      'web search': 'web_search', 'run command': 'command_execution',
      'command execution': 'command_execution', 'file change': 'file_change',
    };
    return aliases[title] || title;
  };
  const title = normalize(event.title);
  const specificTitle = title && !['tool activity', 'tool result', 'tool', 'tool_result'].includes(title);
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

export function groupConversationActivity(
  messages: Message[],
  events: RunEvent[],
  compressToolCalls = false,
  approvals: ApprovalRequest[] = [],
): ConversationActivityItem[] {
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
    if (item.type !== 'activity' || item.value.kind !== 'tool') {
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
    if (previous?.type === 'tool-group' && (
      compactionLifecycle
        ? previousCompactionId !== null && previousCompactionId === currentCompactionId
        : toolFamily(previous.values[0]) === toolFamily(item.value)
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
  for (const event of events) {
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
    if (item.type !== 'tool-group' || item.values.every(isContextCompaction)) {
      compacted.push(item);
      continue;
    }
    const family = item.values.every(isShellActivity) ? 'command_execution' : toolFamily(item.values[0]);
    const key = `${item.values[0].taskId}:${family}`;
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
