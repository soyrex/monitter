import type { RunEvent } from './types';

export type ActivityNoticeState = 'running' | 'completed' | 'failed' | 'unknown';
type RecordValue = Record<string, unknown>;
const record = (value: unknown): RecordValue | null => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : null;

/** Inspect execution envelope fields only; arguments may contain unrelated statuses. */
export function activityNotice(event: RunEvent): { state: ActivityNoticeState; intent: string | null } {
  let detail: RecordValue = {};
  try { detail = record(JSON.parse(event.detail)) ?? {}; } catch { /* Literal output carries no execution evidence. */ }
  const item = record(detail.item) ?? record(detail.update) ?? detail;
  const status = String(item.status ?? detail.status ?? '').toLowerCase().replace(/[_-]/g, '');
  const mcp = item.type === 'mcpToolCall';
  const failedResult = mcp && ((item.error !== undefined && item.error !== null) || record(item.result)?.isError === true);
  const state: ActivityNoticeState = failedResult || ['failed', 'error', 'cancelled', 'canceled', 'interrupted'].includes(status) ? 'failed'
    : ['inprogress', 'running', 'pending', 'queued', 'started'].includes(status) ? 'running'
    : ['completed', 'succeeded', 'success'].includes(status) ? 'completed' : 'unknown';
  const friendly = (value: unknown) => typeof value === 'string' ? value.replace(/[_-]/g, ' ').trim().replace(/\s+/g, ' ').replace(/^./, letter => letter.toUpperCase()) : '';
  const tool = mcp ? friendly(item.tool) : '';
  return { state, intent: tool ? [friendly(item.server), tool].filter(Boolean).join(' · ') : null };
}

export function activityNoticeState(events: RunEvent[], turnRunning?: boolean): ActivityNoticeState {
  if (turnRunning !== undefined) return turnRunning ? 'running' : 'completed';
  const states = events.filter(event => event.kind === 'tool' || event.kind === 'subagent').map(event => activityNotice(event).state);
  if (states.includes('running')) return 'running';
  if (states.includes('failed')) return 'failed';
  return states.length && states.every(state => state === 'completed') ? 'completed' : 'unknown';
}
