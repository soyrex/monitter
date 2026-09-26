import type { ChannelMessage, ChannelMessagesPage, Message, Snapshot, TaskMessagesPage, UiDeltaResponse } from './types';

export type UiDeltaApplication =
  | { kind: 'snapshot'; snapshot: Snapshot; revision: string }
  | { kind: 'delta'; snapshot: Snapshot; revision: string }
  | { kind: 'unchanged'; snapshot: Snapshot; revision: string }
  | { kind: 'gap' };

export function mergeMessagesById(current: readonly Message[], upserts: readonly Message[], removedIds: readonly string[] = []): Message[] {
  const removed = new Set(removedIds);
  const byId = new Map(current.filter(message => !removed.has(message.id)).map(message => [message.id, message]));
  for (const message of upserts) if (!removed.has(message.id)) byId.set(message.id, message);
  return [...byId.values()].sort((left, right) => left.createdAt - right.createdAt || left.id.localeCompare(right.id));
}

export function mergeChannelMessagesById(current: readonly ChannelMessage[], upserts: readonly ChannelMessage[], removedIds: readonly string[] = []): ChannelMessage[] {
  const removed = new Set(removedIds);
  const byId = new Map(current.filter(message => !removed.has(message.id)).map(message => [message.id, message]));
  for (const message of upserts) if (!removed.has(message.id)) byId.set(message.id, message);
  return [...byId.values()].sort((left, right) => left.createdAt - right.createdAt || left.id.localeCompare(right.id));
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

  const retainedChannels = new Set(delta.retainedChannelIds);
  const currentChannels = new Map(current.channels.map(channel => [channel.id, channel]));
  const channelChanges = new Map((delta.channelMessageChanges ?? []).map(change => [change.channelId, change]));
  const channels = delta.metadata.channels.map(channel => {
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
  const transcripts = { ...(delta.metadata.subagentTranscripts ?? {}) };
  for (const id of retainedTranscriptIds) {
    const previous = current.subagentTranscripts?.[id];
    if (previous && Object.prototype.hasOwnProperty.call(transcripts, id) === false) transcripts[id] = previous;
  }

  const snapshot: Snapshot = {
    ...delta.metadata,
    messages: mergeMessagesById(current.messages, delta.messages, delta.removedMessageIds),
    channels,
    ...(delta.metadata.subagentTranscripts !== undefined || Object.keys(transcripts).length
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
