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
  'getSnapshot' | 'sendMessage' | 'cancelTask' | 'resumeTask' | 'listTerminals' | 'readTerminal'>;

// Mobile currently displays conversations, not the complete diagnostic timeline.
// Keep a bounded recent activity window so long-running desktop history cannot
// prevent pairing. Clone events; the authoritative desktop history is untouched.
export function controllerSnapshot(snapshot: Snapshot): Snapshot {
  return { ...snapshot, events: [...snapshot.events]
    .sort((a,b)=>b.createdAt-a.createdAt).slice(0,100)
    .map(event=>({...event,detail:event.detail.length>4000?event.detail.slice(0,4000)+'\n[Remaining detail available on desktop]':event.detail})) };
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
        case 'getSnapshot': result = controllerSnapshot(await this.client.getSnapshot()); break;
        case 'sendMessage': {
          const sendResult = await this.client.sendMessage(request.params.taskId, request.params.text, request.params.attachmentIds);
          // Older mobile builds expect every send response to be a Snapshot.
          // Only a pairing peer that explicitly negotiated the capability sees
          // the small durable receipt; no mutation is retried to learn this.
          result = isSendReceipt(sendResult) && options.allowSendReceipt
            ? sendResult
            : controllerSnapshot(isSendReceipt(sendResult) ? await this.client.getSnapshot() : sendResult);
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
