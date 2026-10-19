import type { Collaboration, RunEvent } from './types';
import type { ConversationActivityItem } from './activity-grouping';

/** User requests and durable turn endings delimit activity ownership; assistant output does not. */
export function liveTranscriptActivity(events: RunEvent[], items: ConversationActivityItem[], running: boolean, boundaries: RunEvent[] = [], transientIds: string[] = []): boolean {
  if (!running || !events.length) return false;
  const transient = new Set(transientIds);
  const requests = items.flatMap(item => item.type === 'message' && item.value.role === 'user' && !transient.has(item.value.id) && !item.value.senderAgentId && !item.value.collaborationId ? [item.value.createdAt] : []);
  const lifecycle = [...boundaries, ...items.flatMap(item => item.type === 'activity' ? [item.value] : [])].filter(event => event.taskId === events[0].taskId);
  const endings = lifecycle.flatMap(event => ['turn.completed', 'turn.failed', 'You cancelled this run.'].includes(event.title) ? [event.createdAt] : []);
  const starts = lifecycle.flatMap(event => event.title === 'turn.started' ? [event.createdAt] : []);
  const started = Math.max(-Infinity, ...starts);
  // A recorded provider turn start is stronger than a later steering message.
  const request = starts.length ? started : Math.max(-Infinity, ...requests);
  const ending = Math.max(-Infinity, ...endings);
  return events.some(event => event.createdAt >= request && event.createdAt > ending);
}

export function exchangeDirection(record: Collaboration | undefined, taskId: string): 'incoming' | 'outgoing' | 'unknown' {
  return record?.fromTaskId === taskId ? 'outgoing' : record?.toTaskId === taskId ? 'incoming' : 'unknown';
}

export function exchangeGroupState(events: RunEvent[], record?: Collaboration): string {
  const statuses = events.map(event => event.title);
  if (statuses.some(status => /error|failed|interrupted/i.test(status)) || record?.error || ['error', 'failed', 'interrupted'].includes(record?.status ?? '')) return 'failed';
  const latest = record?.status ?? statuses.at(-1) ?? 'Recorded message';
  if (/queued|running|started/i.test(latest)) return 'running';
  if (/completed|succeeded/i.test(latest)) return 'completed';
  return latest;
}

/** Adjacent lifecycle records share one disclosure without crossing messages or tool turns. */
export function transcriptExchangeGroups(items: ConversationActivityItem[]): Map<string, RunEvent[]> {
  const groups = new Map<string, RunEvent[]>();
  let previous: RunEvent[] | null = null;
  for (const item of items) {
    if (item.type !== 'activity' || item.value.kind !== 'collaboration') { previous = null; continue; }
    if (previous && previous[0].detail === item.value.detail && previous[0].taskId === item.value.taskId) previous.push(item.value);
    else { previous = [item.value]; groups.set(item.value.id, previous); }
  }
  return groups;
}
