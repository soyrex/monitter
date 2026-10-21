import type { ChannelMessage, ChannelMessagesPage, Message, Snapshot, TaskMessagesPage, UiDeltaResponse } from './types';

export type UiDeltaApplication =
  | { kind: 'snapshot'; snapshot: Snapshot; revision: string }
  | { kind: 'delta'; snapshot: Snapshot; revision: string }
  | { kind: 'unchanged'; snapshot: Snapshot; revision: string }
  | { kind: 'gap' };

export function mergeMessagesById(current: readonly Message[], upserts: readonly Message[], removedIds: readonly string[] = []): Message[] {
  // Metadata-only deltas are common (status, activity, settings) and carry no
  // transcript edits. Keep the immutable page array shared instead of building
  // a map and sorting every loaded message on each such refresh.
  if (upserts.length === 0 && removedIds.length === 0) return current as Message[];
  const removed = new Set(removedIds);
  const byId = new Map(current.filter(message => !removed.has(message.id)).map(message => [message.id, message]));
  for (const message of upserts) if (!removed.has(message.id)) byId.set(message.id, message);
  return [...byId.values()].sort((left, right) => left.createdAt - right.createdAt || left.id.localeCompare(right.id));
}

export function mergeChannelMessagesById(current: readonly ChannelMessage[], upserts: readonly ChannelMessage[], removedIds: readonly string[] = []): ChannelMessage[] {
  // Channel metadata can advance independently from its transcript. Preserve
  // the existing array when a delta has no channel-message changes.
  if (upserts.length === 0 && removedIds.length === 0) return current as ChannelMessage[];
  const removed = new Set(removedIds);
  const byId = new Map(current.filter(message => !removed.has(message.id)).map(message => [message.id, message]));
  for (const message of upserts) if (!removed.has(message.id)) byId.set(message.id, message);
  return [...byId.values()].sort((left, right) => left.createdAt - right.createdAt || left.id.localeCompare(right.id));
}

/** Patch only known metadata fields; malformed patches trigger a revision reset. */
function patchedMetadata(current: Snapshot, patch: NonNullable<NonNullable<UiDeltaResponse['delta']>['metadataPatch']>): Snapshot | null {
  const next = { ...current, messages: [] } as Snapshot;
  const target = next as unknown as Record<string, unknown>;
  const fields = new Set(['hosts', 'agents', 'tasks', 'mailBatches', 'workPlans', 'buildRequests', 'buildRuns', 'events', 'channels', 'projectBoardMessages', 'projects', 'settings', 'collaborations', 'subagentSessions', 'subagentTranscripts', 'queuedMessages', 'approvalRequests', 'approvalRules', 'schedules', 'chatArchiveRules', 'scheduleRuns', 'pendingThrowawayTaskIds']);
  try {
    for (const [wireKey, change] of Object.entries(patch.fields)) {
      const key = wireKey.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
      if (!fields.has(key)) return null;
      if (change.kind === 'replace') {
        const previous = target[key];
        if (change.value === null || previous !== undefined && (Array.isArray(previous) !== Array.isArray(change.value) || typeof previous !== typeof change.value)) return null;
        target[key] = change.value;
      } else if (change.kind === 'rows') {
        const rows = target[key];
        if (!Array.isArray(rows)) return null;
        const entries = new Map<string, unknown>();
        for (const row of rows) {
          if (!row || typeof row.id !== 'string' || entries.has(row.id)) return null;
          entries.set(row.id, row);
        }
        for (const id of change.removed) entries.delete(id);
        for (const [id, row] of Object.entries(change.upserts)) {
          if (!row || typeof row !== 'object' || !('id' in row) || row.id !== id) return null;
          entries.set(id, row);
        }
        const order = change.order ?? [...entries.keys()];
        if (order.length !== entries.size || new Set(order).size !== order.length || order.some(id => !entries.has(id))) return null;
        target[key] = order.map(id => entries.get(id));
      } else if (change.kind === 'map') {
        const previous = target[key];
        if (previous !== undefined && (!previous || typeof previous !== 'object' || Array.isArray(previous))) return null;
        const entries = new Map(Object.entries(previous ?? {}));
        for (const id of change.removed) entries.delete(id);
        for (const [id, value] of Object.entries(change.upserts)) entries.set(id, value);
        target[key] = Object.fromEntries(entries);
      } else return null;
    }
    // Channel bodies are merged independently below.
    if (!Array.isArray(next.channels) || next.channels.some(channel => !channel || typeof channel.id !== 'string')) return null;
    next.channels = next.channels.map(channel => ({ ...channel, messages: [] }));
    return next;
  } catch { return null; }
}

/** Applies only a delta whose base revision is exactly the cache revision. */
export function applyUiDelta(current: Snapshot | null, currentRevision: string | undefined, response: UiDeltaResponse): UiDeltaApplication {
  if (response.snapshot) return { kind: 'snapshot', snapshot: response.snapshot, revision: response.revision };
  if (!response.delta) {
    return current && currentRevision === response.revision
      ? { kind: 'unchanged', snapshot: current, revision: response.revision }
      : { kind: 'gap' };
  }
  const delta = response.delta;
  if (!current || !currentRevision || delta.fromRevision !== currentRevision) return { kind: 'gap' };

  if (delta.metadata != null && delta.metadataPatch != null) return { kind: 'gap' };
  const metadata = delta.metadata ?? (delta.metadataPatch ? patchedMetadata(current, delta.metadataPatch) : null);
  if (!metadata) return { kind: 'gap' };
  const retainedChannels = new Set(delta.retainedChannelIds);
  const currentChannels = new Map(current.channels.map(channel => [channel.id, channel]));
  const channelChanges = new Map((delta.channelMessageChanges ?? []).map(change => [change.channelId, change]));
  const channels = metadata.channels.map(channel => {
    const previous = currentChannels.get(channel.id);
    const change = channelChanges.get(channel.id);
    if (change) {
      return {
        ...channel,
        messages: change.reset
          ? mergeChannelMessagesById([], change.messages)
          : mergeChannelMessagesById(previous?.messages ?? [], change.messages, change.removedMessageIds),
      };
    }
    return retainedChannels.has(channel.id) && previous
      ? { ...channel, messages: previous.messages }
      : channel;
  });

  const retainedTranscriptIds = new Set(delta.retainedSubagentTranscriptIds);
  const transcripts = { ...(metadata.subagentTranscripts ?? {}) };
  for (const id of retainedTranscriptIds) {
    const previous = current.subagentTranscripts?.[id];
    if (previous && Object.prototype.hasOwnProperty.call(transcripts, id) === false) transcripts[id] = previous;
  }

  const snapshot: Snapshot = {
    ...metadata,
    messages: mergeMessagesById(current.messages, delta.messages, delta.removedMessageIds),
    channels,
    ...(metadata.subagentTranscripts !== undefined || Object.keys(transcripts).length
      ? { subagentTranscripts: transcripts }
      : { subagentTranscripts: undefined }),
  };
  return { kind: 'delta', snapshot, revision: response.revision };
}

export function mergeTaskMessagesPage(snapshot: Snapshot, page: TaskMessagesPage): Snapshot {
  if (page.messages.some(message => message.taskId !== page.messages[0]?.taskId)) {
    throw new Error('Task message page contains messages from different tasks.');
  }
  return { ...snapshot, messages: mergeMessagesById(snapshot.messages, page.messages) };
}

export function mergeChannelMessagesPage(snapshot: Snapshot, page: ChannelMessagesPage): Snapshot {
  const channel = snapshot.channels.find(item => item.id === page.channelId);
  if (!channel) throw new Error('Channel history page did not match a channel in the current snapshot.');
  return {
    ...snapshot,
    channels: snapshot.channels.map(item => item.id === page.channelId
      ? { ...item, messages: mergeChannelMessagesById(item.messages, page.messages) }
      : item),
  };
}

export function mergeOlderTranscriptMessages(current: readonly Message[], older: readonly Message[]): Message[] {
  return mergeMessagesById(current, older);
}

export interface TranscriptViewportAnchor { key: string; offset: number; }
/** Capture a stable item key and its offset from the scroll viewport before history is prepended. */
export function captureTranscriptAnchor(viewport: HTMLElement | null | undefined): TranscriptViewportAnchor | null {
  if (!viewport) return null;
  const viewportTop = viewport.getBoundingClientRect().top;
  for (const row of viewport.querySelectorAll<HTMLElement>('[data-item-key]')) {
    const rect = row.getBoundingClientRect();
    if (rect.bottom > viewportTop + 10) {
      const key = row.dataset.itemKey;
      if (key) return { key, offset: rect.top - viewportTop };
    }
  }
  return null;
}
