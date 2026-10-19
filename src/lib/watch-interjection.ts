import type { Message, Snapshot } from '$lib/types';

export interface WatchAssistantBaseline {
  id: string;
  text: string;
  streamStatus: Message['streamStatus'];
}
export interface WatchInterjectionState {
  taskId: string;
  text: string;
  capturedAt: number;
  knownMessageIds: string[];
  assistantBaseline: WatchAssistantBaseline[];
  knownQueueIds: string[];
  acknowledged: boolean;
  acknowledgementId: string | null;
  acknowledgedAt: number | null;
  closed: boolean;
  response: Message | null;
}
export interface WatchInterjectionProjection {
  /** Persist this returned state so compact snapshots cannot erase a received reply. */
  state: WatchInterjectionState;
  response: Message | null;
  status: string;
  pending: boolean;
  finished: boolean;
}
type WatchExchangeSnapshot = Pick<Snapshot, 'messages' | 'queuedMessages'>;

export function captureWatchInterjection(snapshot: WatchExchangeSnapshot, taskId: string, text: string, now = Date.now()): WatchInterjectionState {
  const messages = snapshot.messages.filter(message => message.taskId === taskId);
  return {
    taskId, text, capturedAt: now,
    knownMessageIds: messages.map(message => message.id),
    assistantBaseline: messages.filter(message => message.role === 'assistant').map(({ id, text, streamStatus }) => ({ id, text, streamStatus })),
    knownQueueIds: snapshot.queuedMessages.filter(message => message.taskId === taskId).map(message => message.id),
    acknowledged: false, acknowledgementId: null, acknowledgedAt: null, closed: false, response: null,
  };
}

/** Acknowledgement and replies must belong to this task; unchanged history is never a reply. */
export function projectWatchInterjection(snapshot: WatchExchangeSnapshot, previous: WatchInterjectionState): WatchInterjectionProjection {
  const messages = snapshot.messages.filter(message => message.taskId === previous.taskId);
  const acknowledgement = messages.filter(message => message.role === 'user' && !message.senderAgentId &&
    message.text === previous.text && !previous.knownMessageIds.includes(message.id))
    .sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id))[0];
  const acknowledgementId = previous.acknowledgementId ?? acknowledgement?.id ?? null;
  const acknowledgedAt = previous.acknowledgedAt ?? acknowledgement?.createdAt ?? null;
  const acknowledged = previous.acknowledged || acknowledgementId !== null;
  // All turn boundaries use backend timestamps, never the phone's wall clock.
  const nextTurn = acknowledgementId && acknowledgedAt !== null ? messages.filter(message =>
    message.role === 'user' && !message.senderAgentId && message.id !== acknowledgementId &&
    !previous.knownMessageIds.includes(message.id) && message.createdAt >= acknowledgedAt)
    .sort((a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id))[0] : undefined;
  const closed = previous.closed || !!nextTurn;
  const candidates = acknowledged && !previous.closed ? messages.filter(message => {
    if (message.role !== 'assistant') return false;
    const baseline = previous.assistantBaseline.find(row => row.id === message.id);
    // A same-ID edit observed after another owner turn cannot be attributed to
    // this exchange. Keep only a reply already captured before that boundary.
    if (nextTurn && (baseline || message.createdAt >= nextTurn.createdAt)) return false;
    return baseline ? baseline.text !== message.text || baseline.streamStatus !== message.streamStatus
      : acknowledgedAt !== null && message.createdAt >= acknowledgedAt;
  }).sort((a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id)) : [];
  // Preserve an already selected reply when a bounded snapshot omits its older row.
  // Do not regress to a different, older reply still present in that snapshot.
  const candidate = candidates[0];
  const selected = candidate && (!previous.response || candidate.id === previous.response.id || candidate.createdAt >= previous.response.createdAt)
    ? candidate : previous.response;
  const response = selected && JSON.stringify(selected) !== JSON.stringify(previous.response) ? { ...selected } : previous.response;
  const state = acknowledged === previous.acknowledged && response === previous.response && closed === previous.closed
    ? previous : { ...previous, acknowledged, acknowledgementId, acknowledgedAt, closed, response };
  if (response) {
    const pending = !closed && response.streamStatus === 'streaming';
    return { state, response, pending, finished: !pending, status: pending ? 'Receiving reply…' : 'Reply received' };
  }
  if (closed) return { state, response: null, pending: false, finished: true, status: 'A later request started before a reply was captured' };
  if (acknowledged) return { state, response: null, pending: true, finished: false, status: 'Steered · waiting for reply…' };
  const queued = snapshot.queuedMessages.filter(message => message.taskId === previous.taskId && message.text === previous.text && !previous.knownQueueIds.includes(message.id))
    .sort((a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id))[0];
  const failed = queued?.status === 'error';
  const status = failed ? queued.error?.trim() || 'The steer could not be sent.'
    : queued?.status === 'queued' ? 'Steer unavailable · queued for the next turn'
    : queued?.status === 'sending' && queued.origin === 'steering' ? 'Steering into the current turn…'
    : 'Sending steer…';
  return { state, response: null, status, pending: !failed, finished: failed };
}
