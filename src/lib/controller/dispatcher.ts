import type { Attachment, AttachmentFileData, Snapshot } from '../types';
import type { MonitterBridge } from '../bridge';
import {
  CONTROLLER_MAX_REQUEST_BYTES,
  CONTROLLER_MAX_RESPONSE_BYTES,
  CONTROLLER_MUTATING_ACTIONS,
  CONTROLLER_PROTOCOL_VERSION,
  authenticatedControllerContext,
  parseControllerRequest,
  type AuthenticatedControllerContext,
  type ControllerClient,
  type ControllerSendReceipt,
  type ControllerRequest,
  type ControllerResult,
  type ControllerResponse,
} from './protocol';

export type MonitterControllerClient = Pick<MonitterBridge,
  'getSnapshot' | 'getLegacySnapshot' | 'sendMessage' | 'cancelTask' | 'resumeTask' | 'listTerminals' | 'readTerminal'> & Partial<Pick<MonitterBridge, 'getTaskMessages'>>;

const CONTROLLER_SNAPSHOT_BUDGET_BYTES = CONTROLLER_MAX_RESPONSE_BYTES - 1024;
const CONTROLLER_MESSAGE_TAIL_PER_TASK = 32;
const CONTROLLER_MESSAGE_TAIL_TOTAL = 600;
const CONTROLLER_CHANNEL_MESSAGE_TAIL = 24;
const CONTROLLER_CHANNEL_MESSAGE_TOTAL = 300;
const CONTROLLER_TEXT_LIMIT = 4096;
const CONTROLLER_EVENT_DETAIL_LIMIT = 4000;
const CONTROLLER_EVENT_TITLE_LIMIT = 512;
const CONTROLLER_TRUNCATION_NOTICE = '[Earlier messages are available on the desktop.]';

/**
 * Mobile v1 has no transcript paging. Project its compatibility Snapshot here,
 * rather than raising the paired transport frame ceiling. This clones the
 * desktop-owned data, keeps task/agent identities, and retains chronological
 * recent conversation tails. A system notice makes per-task history clipping
 * visible in the phone UI.
 */
export function controllerSnapshot(snapshot: Snapshot): Snapshot {
  const messages = recentTaskMessages(snapshot);
  const channels = recentChannelMessages(snapshot);
  const projected = limitStrings({
    ...snapshot,
    messages,
    channels,
    events: [...snapshot.events]
      .sort((a, b) => b.createdAt - a.createdAt || a.id.localeCompare(b.id)).slice(0, 100)
      .map(event => ({
        ...event,
        title: truncate(event.title, CONTROLLER_EVENT_TITLE_LIMIT),
        detail: truncate(event.detail, CONTROLLER_EVENT_DETAIL_LIMIT, '\n[Remaining detail available on desktop]'),
      })),
    // These desktop-only queues and approval payloads are not rendered by the
    // controller. Keeping their full records can otherwise crowd out chat.
    collaborations: [], queuedMessages: [], approvalRequests: [], approvalRules: [],
    workPlans: [], projectBoardMessages: [], subagentSessions: [], subagentTranscripts: {},
    schedules: [], scheduleRuns: [], pendingThrowawayTaskIds: [],
  }) as Snapshot;
  return fitControllerSnapshot(projected);
}

function recentTaskMessages(snapshot: Snapshot): Snapshot['messages'] {
  const grouped = new Map<string, Snapshot['messages']>();
  for (const message of snapshot.messages) {
    const messages = grouped.get(message.taskId) ?? [];
    messages.push(message); grouped.set(message.taskId, messages);
  }
  const perTask = [...grouped.entries()].map(([taskId, messages]) => {
    const ordered = [...messages].sort(compareChronological);
    return { taskId, ordered, tail: ordered.slice(-CONTROLLER_MESSAGE_TAIL_PER_TASK) };
  });
  const content = perTask.flatMap(item => item.tail).sort(compareChronological).slice(-CONTROLLER_MESSAGE_TAIL_TOTAL);
  const retainedTaskIds = new Set(content.map(message => message.taskId));
  // A task omitted by the global content window receives the same visible
  // desktop-history marker as a per-task tail. That prevents silent loss when
  // a workspace has more active conversations than the content allowance.
  const notices = perTask.flatMap(({ taskId, ordered, tail }) => ordered.length > tail.length || !retainedTaskIds.has(taskId)
    ? [{ id: historyNoticeId(taskId), taskId, role: 'system' as const, text: CONTROLLER_TRUNCATION_NOTICE, createdAt: tail[0]?.createdAt ?? 0 }]
    : []);
  return [...notices, ...content].sort(compareChronological);
}

function recentChannelMessages(snapshot: Snapshot): Snapshot['channels'] {
  let remaining = CONTROLLER_CHANNEL_MESSAGE_TOTAL;
  return snapshot.channels.map(channel => {
    const messages = [...channel.messages].sort(compareChronological).slice(-CONTROLLER_CHANNEL_MESSAGE_TAIL);
    const allowed = Math.max(0, Math.min(remaining, messages.length));
    remaining -= allowed;
    return { ...channel, messages: allowed ? messages.slice(-allowed) : [] };
  });
}

function fitControllerSnapshot(snapshot: Snapshot): Snapshot {
  // Measure encoded bytes rather than estimating characters: UTF-8 and JSON
  // escaping make estimates unsafe. Remove only the oldest retained history
  // until the conservative result budget fits the 1 MiB response envelope.
  const result = { ...snapshot, messages: [...snapshot.messages], channels: snapshot.channels.map(channel => ({ ...channel, messages: [...channel.messages] })), events: [...snapshot.events] };
  while (encodedBytes(result) > CONTROLLER_SNAPSHOT_BUDGET_BYTES) {
    const oldest = oldestHistory(result);
    if (!oldest) break;
    if (oldest.kind === 'message') {
      const [removed] = result.messages.splice(oldest.index, 1);
      if (removed && !result.messages.some(message => message.taskId === removed.taskId && !isHistoryNotice(message)) &&
          !result.messages.some(message => message.taskId === removed.taskId && isHistoryNotice(message))) {
        result.messages.push({ id: historyNoticeId(removed.taskId), taskId: removed.taskId, role: 'system', text: CONTROLLER_TRUNCATION_NOTICE, createdAt: removed.createdAt });
      }
    }
    else if (oldest.kind === 'channel') result.channels[oldest.channelIndex].messages.splice(oldest.index, 1);
    else result.events.splice(oldest.index, 1);
  }
  result.messages.sort(compareChronological);
  return result;
}

function oldestHistory(snapshot: Snapshot): { kind: 'message' | 'event'; index: number } | { kind: 'channel'; channelIndex: number; index: number } | null {
  const candidates: { createdAt: number; kind: 'message' | 'event' | 'channel'; index: number; channelIndex?: number }[] = [];
  snapshot.messages.forEach((message, index) => {
    // Retain the tiny visible notice while trimming its task's remaining tail.
    if (!isHistoryNotice(message)) candidates.push({ createdAt: message.createdAt, kind: 'message', index });
  });
  snapshot.events.forEach((event, index) => candidates.push({ createdAt: event.createdAt, kind: 'event', index }));
  snapshot.channels.forEach((channel, channelIndex) => channel.messages.forEach((message, index) => candidates.push({ createdAt: message.createdAt, kind: 'channel', index, channelIndex })));
  const candidate = candidates.sort((a, b) => a.createdAt - b.createdAt || a.kind.localeCompare(b.kind) || a.index - b.index)[0];
  if (!candidate) return null;
  return candidate.kind === 'channel'
    ? { kind: 'channel', channelIndex: candidate.channelIndex!, index: candidate.index }
    : { kind: candidate.kind, index: candidate.index };
}

function limitStrings<T>(value: T): T {
  if (typeof value === 'string') return truncate(value, CONTROLLER_TEXT_LIMIT) as T;
  if (Array.isArray(value)) return value.map(item => limitStrings(item)) as T;
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, item]) => [key, limitStrings(item)])) as T;
  return value;
}

function truncate(value: string, limit: number, suffix = '\n[Truncated for mobile]'): string {
  return value.length > limit ? value.slice(0, Math.max(0, limit - suffix.length)) + suffix : value;
}

function compareChronological<T extends { createdAt: number; id: string }>(a: T, b: T): number {
  if (a.createdAt !== b.createdAt) return a.createdAt - b.createdAt;
  if (isHistoryNotice(a) !== isHistoryNotice(b)) return isHistoryNotice(a) ? -1 : 1;
  return a.id.localeCompare(b.id);
}

function encodedBytes(value: unknown): number { return new TextEncoder().encode(JSON.stringify(value)).byteLength; }

function isHistoryNotice(value: { id: string; taskId?: string; text?: string; role?: string }): boolean {
  return value.role === 'system' && typeof value.taskId === 'string' && value.text === CONTROLLER_TRUNCATION_NOTICE && value.id === historyNoticeId(value.taskId);
}

/** A stable v5-shaped UUID derived from the task id; no random state enters a snapshot read. */
function historyNoticeId(taskId: string): string {
  const hex = [0x811c9dc5, 0x9e3779b9, 0x85ebca6b, 0xc2b2ae35]
    .map(seed => fnv1a(taskId, seed).toString(16).padStart(8, '0')).join('');
  const variant = ((Number.parseInt(hex[8], 16) & 0x3) | 0x8).toString(16);
  return `00000000-${hex.slice(0, 4)}-5${hex.slice(5, 8)}-${variant}${hex.slice(9, 12)}-${hex.slice(12, 24)}`;
}

function fnv1a(value: string, seed: number): number {
  let hash = seed >>> 0;
  for (let index = 0; index < value.length; index += 1) hash = Math.imul(hash ^ value.charCodeAt(index), 0x01000193) >>> 0;
  return hash;
}

interface Receipt { fingerprint: string; response: Promise<ControllerResponse>; }
interface PendingUpload {
  readonly subject: string; readonly taskId: string; readonly filename: string; readonly mimeType: string; readonly size: number; readonly previewDataUrl?: string;
  readonly chunks: Uint8Array[]; readonly createdAt: number; received: number;
}
const UPLOAD_TTL_MS = 2 * 60_000;
const MAX_PENDING_UPLOADS = 8;
const MAX_PENDING_UPLOAD_BYTES = 8 * 8 * 1024 * 1024;

export interface ControllerDispatcherOptions { maxMutationReceipts?: number; }
export interface ControllerDispatchOptions {
  /** Only a peer that explicitly offered this capability may receive receipts. */
  readonly allowSendReceipt?: boolean;
  /** Upload RPCs are limited to the visitor sharing bridge, never general mobile control. */
  readonly allowUploads?: boolean;
}

/**
 * The dispatcher is deliberately transport-independent. A future encrypted,
 * paired transport must authenticate its peer and pass a non-wire context for
 * every call; this foundation does not expose a listener or implement pairing.
 *
 * Mutation receipts are process/session-local. Once full they are never evicted
 * (including completed calls): a new authenticated session must be established
 * before more mutating requests can be accepted. Pending receipts are never
 * evicted, so concurrent retries receive the original result rather than a
 * second task operation. Receipts do not survive an app restart.
 */
export class ControllerDispatcher {
  private readonly receipts = new Map<string, Receipt>();
  private readonly uploads = new Map<string, PendingUpload>();
  private readonly maxMutationReceipts: number;

  constructor(private readonly client: ControllerClient, options: ControllerDispatcherOptions = {}) {
    // Eight maximum-size files need 2,048 chunk receipts plus their starts,
    // finishes and send; keep every retry idempotent for that advertised limit.
    const capacity = options.maxMutationReceipts ?? 4096;
    if (!Number.isSafeInteger(capacity) || capacity < 1 || capacity > 4096) throw new Error('maxMutationReceipts must be an integer from 1 to 4096.');
    this.maxMutationReceipts = capacity;
  }

  async dispatch(input: unknown, context: unknown, options: ControllerDispatchOptions = {}): Promise<ControllerResponse> {
    const parsed = parseControllerRequest(input);
    if ('ok' in parsed) return parsed;
    if (!authenticatedControllerContext(context)) return this.error(parsed.id, 'unauthenticated', 'An authenticated paired transport is required.');
    if (CONTROLLER_MUTATING_ACTIONS.has(parsed.request.action)) return this.dispatchMutation(parsed.id, parsed.request, context, options);
    return this.execute(parsed.id, parsed.request, options, context);
  }

  async dispatchJson(json: string, context: unknown, options: ControllerDispatchOptions = {}): Promise<string> {
    if (new TextEncoder().encode(json).byteLength > CONTROLLER_MAX_REQUEST_BYTES) {
      return JSON.stringify(this.error(null, 'invalid_request', 'Request exceeds the controller size limit.'));
    }
    let input: unknown;
    try { input = JSON.parse(json); }
    catch { return JSON.stringify(this.error(null, 'invalid_request', 'Request must be valid JSON.')); }
    const response = await this.dispatch(input, context, options);
    return JSON.stringify(response);
  }

  private async dispatchMutation(id: string, request: ControllerRequest, context: AuthenticatedControllerContext, options: ControllerDispatchOptions): Promise<ControllerResponse> {
    // Chunk data can be 32 KiB. Keep a fixed-length cryptographic digest in the
    // receipt cache so the 4,096 idempotency slots never retain whole uploads.
    const fingerprint = request.action === 'appendAttachmentUpload'
      ? `appendAttachmentUpload:${request.params.uploadId}:${await sha256(request.params.dataBase64)}`
      : JSON.stringify({ action: request.action, params: request.params });
    // Request IDs are scoped to the authenticated transport subject. A receipt
    // from one paired device must never be returned to another device.
    const receiptKey = `${context.subject}\u0000${id}`;
    const existing = this.receipts.get(receiptKey);
    if (existing) {
      if (existing.fingerprint !== fingerprint) return this.error(id, 'id_conflict', 'Request id was already used with different mutation data.');
      return existing.response;
    }
    if (this.receipts.size >= this.maxMutationReceipts) return this.error(id, 'dedup_saturated', 'Mutation receipt cache is full; establish a new authenticated session.');
    const receipt: Receipt = { fingerprint, response: this.execute(id, request, options, context) };
    this.receipts.set(receiptKey, receipt);
    return receipt.response;
  }

  private async execute(id: string, request: ControllerRequest, options: ControllerDispatchOptions, context?: AuthenticatedControllerContext): Promise<ControllerResponse> {
    try {
      let result: ControllerResult;
      switch (request.action) {
        case 'getSnapshot': result = controllerSnapshot(await (this.client.getLegacySnapshot?.() ?? this.client.getSnapshot())); break;
        case 'getTaskMessages': {
          if (!this.client.getTaskMessages) throw new Error('Task message history is unavailable on this desktop.');
          const page = await this.client.getTaskMessages(request.params.taskId, request.params.beforeId, request.params.limit ?? 40);
          result = { ...page, messages: page.messages.map(message => ({ ...message,
            text: truncate(message.text, 8_000),
            attachments: message.attachments?.map(attachment => ({ ...attachment, previewDataUrl: undefined })),
          })) };
          break;
        }
        case 'sendMessage': {
          const sendResult = await this.client.sendMessage(request.params.taskId, request.params.text, request.params.attachmentIds);
          // Older mobile builds expect every send response to be a Snapshot.
          // Only a pairing peer that explicitly negotiated the capability sees
          // the small durable receipt; no mutation is retried to learn this.
          result = isSendReceipt(sendResult) && options.allowSendReceipt
            ? sendResult
            : controllerSnapshot(isSendReceipt(sendResult) ? await (this.client.getLegacySnapshot?.() ?? this.client.getSnapshot()) : sendResult);
          break;
        }
        case 'beginAttachmentUpload': {
          if (!options.allowUploads || !context || !this.client.storeAttachment) throw new Error('Attachment uploads are unavailable for this connection.');
          this.sweepUploads();
          const activeForSubject = [...this.uploads.values()].filter(upload => upload.subject === context.subject).length;
          const reservedBytes = [...this.uploads.values()].reduce((total, upload) => total + upload.size, 0);
          if (activeForSubject >= MAX_PENDING_UPLOADS || this.uploads.size >= MAX_PENDING_UPLOADS || reservedBytes + request.params.size > MAX_PENDING_UPLOAD_BYTES) throw new Error('Too many attachment uploads are in progress.');
          const uploadId = crypto.randomUUID();
          this.uploads.set(uploadId, { subject: context.subject, ...request.params, chunks: [], createdAt: Date.now(), received: 0 });
          result = { uploadId };
          break;
        }
        case 'appendAttachmentUpload': {
          if (!options.allowUploads || !context) throw new Error('Attachment uploads require an authenticated connection.');
          const upload = this.requireUpload(request.params.uploadId, context.subject);
          const chunk = base64ToBytes(request.params.dataBase64);
          if (upload.received + chunk.byteLength > upload.size) { this.uploads.delete(request.params.uploadId); throw new Error('Attachment exceeds its declared size.'); }
          upload.chunks.push(chunk); upload.received += chunk.byteLength;
          result = { uploadId: request.params.uploadId };
          break;
        }
        case 'finishAttachmentUpload': {
          if (!options.allowUploads || !context || !this.client.storeAttachment) throw new Error('Attachment uploads are unavailable for this connection.');
          const upload = this.requireUpload(request.params.uploadId, context.subject);
          this.uploads.delete(request.params.uploadId);
          if (upload.received !== upload.size) throw new Error('Attachment upload is incomplete.');
          const file: AttachmentFileData = { filename: upload.filename, mimeType: upload.mimeType, dataBase64: bytesToBase64(upload.chunks, upload.size) };
          result = await this.client.storeAttachment(upload.taskId, file, upload.previewDataUrl);
          break;
        }
        case 'cancelTask': result = controllerSnapshot(await this.client.cancelTask(request.params.taskId)); break;
        case 'resumeTask': result = controllerSnapshot(await this.client.resumeTask(request.params.taskId)); break;
        case 'listTerminals': result = await this.client.listTerminals(); break;
        case 'readTerminal': result = await this.client.readTerminal(request.params.id, request.params.afterSeq); break;
      }
      const response: ControllerResponse = { version: CONTROLLER_PROTOCOL_VERSION, id, ok: true, result };
      if (new TextEncoder().encode(JSON.stringify(response)).byteLength > CONTROLLER_MAX_RESPONSE_BYTES) {
        return this.error(id, 'response_too_large', 'Controller response exceeds the size limit.');
      }
      return response;
    } catch (reason) {
      const detail = reason instanceof Error && reason.message ? `: ${reason.message.slice(0, 512)}` : '';
      return this.error(id, 'bridge_error', `Monitter could not complete this action${detail}`);
    }
  }

  private error(id: string | null, code: Extract<ControllerResponse, { ok: false }>['error']['code'], message: string): ControllerResponse {
    return { version: CONTROLLER_PROTOCOL_VERSION, id, ok: false, error: { code, message } };
  }

  private requireUpload(id: string, subject: string): PendingUpload {
    this.sweepUploads();
    const upload = this.uploads.get(id);
    if (!upload || upload.subject !== subject) throw new Error('Attachment upload is unavailable.');
    return upload;
  }

  private sweepUploads() {
    const oldest = Date.now() - UPLOAD_TTL_MS;
    for (const [id, upload] of this.uploads) if (upload.createdAt < oldest) this.uploads.delete(id);
  }
}

async function sha256(value: string): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value)));
  return Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value); const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function bytesToBase64(chunks: readonly Uint8Array[], size: number): string {
  const all = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { all.set(chunk, offset); offset += chunk.byteLength; }
  // 24 KiB is divisible by three, so joining these encodings preserves base64 groups.
  let output = '';
  for (let start = 0; start < all.byteLength; start += 24 * 1024) {
    let binary = ''; const end = Math.min(start + 24 * 1024, all.byteLength);
    for (let index = start; index < end; index += 1) binary += String.fromCharCode(all[index]);
    output += btoa(binary);
  }
  return output;
}

function isSendReceipt(value: Snapshot | ControllerSendReceipt): value is ControllerSendReceipt {
  return 'accepted' in value && value.accepted === true && !('tasks' in value);
}
