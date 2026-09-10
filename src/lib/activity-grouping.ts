import type { Message, RunEvent } from '$lib/types';

export type ConversationActivityItem =
  | { type: 'message'; value: Message }
  | { type: 'activity'; value: RunEvent }
  | { type: 'tool-group'; values: RunEvent[] };

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

/**
 * Groups only neighboring tool events of the same reported tool or connector family. Since
 * grouping happens after messages and reasoning are merged into the timeline,
 * it can never span a response or user-turn boundary.
 */
/** Connector methods (gmail.search_emails / gmail.read_email) share a family. */
export function toolFamily(event: RunEvent) {
  const identity = toolIdentity(event);
  const mcp = identity.match(/^(?:mcp__)?([^_]+(?:_[^_]+)*)__([^_].*)$/);
  if (mcp) return mcp[1];
  const dotted = identity.match(/^([a-z][a-z0-9_-]*)[.\/]([a-z][a-z0-9_-]*)$/);
  return dotted ? dotted[1] : identity;
}

export function groupConversationActivity(
  messages: Message[],
  events: RunEvent[],
): ConversationActivityItem[] {
  const ordered = [
    ...messages.map(value => ({ type: 'message' as const, value })),
    ...events.map(value => ({ type: 'activity' as const, value })),
  ].sort((left, right) => left.value.createdAt - right.value.createdAt);

  const grouped: ConversationActivityItem[] = [];
  for (const item of ordered) {
    if (item.type !== 'activity' || item.value.kind !== 'tool') {
      grouped.push(item);
      continue;
    }
    const previous = grouped.at(-1);
    if (previous?.type === 'tool-group' && toolFamily(previous.values[0]) === toolFamily(item.value)) {
      previous.values.push(item.value);
    } else {
      grouped.push({ type: 'tool-group', values: [item.value] });
    }
  }
  return grouped;
}
