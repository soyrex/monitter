import type { Attachment, AttachmentFileData, Snapshot, TerminalRead, TerminalSession } from '../types';

/**
 * Wire format for the future paired-controller relay.  This module deliberately
 * contains no transport, pairing, encryption, or authentication implementation.
 */
export const CONTROLLER_PROTOCOL_VERSION = 1 as const;
export const CONTROLLER_MAX_REQUEST_BYTES = 64 * 1024;
export const CONTROLLER_MAX_RESPONSE_BYTES = 1024 * 1024;
export const CONTROLLER_MAX_TEXT_LENGTH = 32 * 1024;
export const CONTROLLER_MAX_SUBJECT_LENGTH = 256;
/** Raw bytes in one upload chunk. Keeps its base64 JSON request well below 64 KiB. */
export const CONTROLLER_UPLOAD_CHUNK_BYTES = 32 * 1024;
export const CONTROLLER_MAX_UPLOAD_BYTES = 8 * 1024 * 1024;
export const CONTROLLER_MAX_UPLOADS_PER_SESSION = 8;
export const CONTROLLER_MAX_UPLOAD_PREVIEW_CHARS = 48_000;
/** A durable desktop acknowledgement, deliberately smaller than a Snapshot. */
export interface ControllerSendReceipt { readonly accepted: true; }

export type ControllerAction =
  | 'getSnapshot'
  | 'sendMessage'
  | 'beginAttachmentUpload'
  | 'appendAttachmentUpload'
  | 'finishAttachmentUpload'
  | 'cancelTask'
  | 'resumeTask'
  | 'listTerminals'
  | 'readTerminal';

export interface GetSnapshotRequest { readonly action: 'getSnapshot'; readonly params: Record<string, never>; }
export interface SendMessageRequest { readonly action: 'sendMessage'; readonly params: { taskId: string; text: string; attachmentIds?: string[] }; }
export interface BeginAttachmentUploadRequest { readonly action: 'beginAttachmentUpload'; readonly params: { taskId: string; filename: string; mimeType: string; size: number; previewDataUrl?: string }; }
export interface AppendAttachmentUploadRequest { readonly action: 'appendAttachmentUpload'; readonly params: { uploadId: string; dataBase64: string }; }
export interface FinishAttachmentUploadRequest { readonly action: 'finishAttachmentUpload'; readonly params: { uploadId: string }; }
export interface CancelTaskRequest { readonly action: 'cancelTask'; readonly params: { taskId: string }; }
export interface ResumeTaskRequest { readonly action: 'resumeTask'; readonly params: { taskId: string }; }
export interface ListTerminalsRequest { readonly action: 'listTerminals'; readonly params: Record<string, never>; }
export interface ReadTerminalRequest { readonly action: 'readTerminal'; readonly params: { id: string; afterSeq: number }; }

export type ControllerRequest =
  | GetSnapshotRequest
  | SendMessageRequest
  | BeginAttachmentUploadRequest
  | AppendAttachmentUploadRequest
  | FinishAttachmentUploadRequest
  | CancelTaskRequest
  | ResumeTaskRequest
  | ListTerminalsRequest
  | ReadTerminalRequest;

export type ControllerResult = Snapshot | ControllerSendReceipt | TerminalSession[] | TerminalRead | Attachment | { uploadId: string };

export type ControllerErrorCode =
  | 'unauthenticated'
  | 'invalid_request'
  | 'unsupported_action'
  | 'id_conflict'
  | 'dedup_saturated'
  | 'busy'
  | 'bridge_error'
  | 'response_too_large';

export interface ControllerError {
  code: ControllerErrorCode;
  message: string;
}

export type ControllerResponse =
  | { version: typeof CONTROLLER_PROTOCOL_VERSION; id: string; ok: true; result: ControllerResult }
  | { version: typeof CONTROLLER_PROTOCOL_VERSION; id: string | null; ok: false; error: ControllerError };

/**
 * This is an in-process assertion supplied by a future authenticated paired
 * transport. It is intentionally not a request field and must never be parsed
 * from controller JSON.
 */
export interface AuthenticatedControllerContext {
  authenticated: true;
  subject: string;
}

export interface ControllerClient {
  getSnapshot(): Promise<Snapshot>;
  /**
   * New desktop bridges may return a durable acceptance receipt. Callers that
   * did not negotiate receipt support must receive a Snapshot instead.
   */
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot | ControllerSendReceipt>;
  /** Optional so older desktop bridges retain their original controller surface. */
  storeAttachment?(taskId: string, file: AttachmentFileData, previewDataUrl?: string): Promise<Attachment>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resumeTask(taskId: string): Promise<Snapshot>;
  listTerminals(): Promise<TerminalSession[]>;
  readTerminal(id: string, afterSeq: number): Promise<TerminalRead>;
}

export const CONTROLLER_MUTATING_ACTIONS: ReadonlySet<ControllerAction> = new Set([
  'sendMessage', 'beginAttachmentUpload', 'appendAttachmentUpload', 'finishAttachmentUpload', 'cancelTask', 'resumeTask',
]);

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const ACTIONS: ReadonlySet<string> = new Set<ControllerAction>([
  'getSnapshot', 'sendMessage', 'beginAttachmentUpload', 'appendAttachmentUpload', 'finishAttachmentUpload', 'cancelTask', 'resumeTask', 'listTerminals', 'readTerminal',
]);

function record(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function exactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && actual.every(key => keys.includes(key));
}

function requestId(value: unknown): value is string {
  return typeof value === 'string' && UUID.test(value);
}

function idParameter(value: unknown): value is string {
  return requestId(value);
}

function validUploadChunk(value: string): boolean {
  // Canonical padded/unpadded base64 only. Decoding is done by the dispatcher,
  // after the raw-byte bound has been established here.
  if (value.length === 0 || value.length > Math.ceil(CONTROLLER_UPLOAD_CHUNK_BYTES / 3) * 4) return false;
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(value) || value.length % 4 === 1) return false;
  try {
    const decoded = atob(value);
    return decoded.length > 0 && decoded.length <= CONTROLLER_UPLOAD_CHUNK_BYTES && btoa(decoded).replace(/=+$/, '') === value.replace(/=+$/, '');
  } catch { return false; }
}

function validUploadPreview(value: unknown): value is string {
  return typeof value === 'string' && value.length <= CONTROLLER_MAX_UPLOAD_PREVIEW_CHARS && /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/]+={0,2}$/i.test(value);
}

function invalid(id: string | null, message: string): ControllerResponse {
  return { version: CONTROLLER_PROTOCOL_VERSION, id, ok: false, error: { code: 'invalid_request', message } };
}

/** Parse and strictly validate untrusted controller JSON. */
export function parseControllerRequest(value: unknown): { id: string; request: ControllerRequest } | ControllerResponse {
  if (!record(value)) return invalid(null, 'Request must be a JSON object.');
  const id = requestId(value.id) ? value.id : null;
  if (!exactKeys(value, ['version', 'id', 'action', 'params'])) return invalid(id, 'Request fields are invalid.');
  if (value.version !== CONTROLLER_PROTOCOL_VERSION) return invalid(id, 'Unsupported protocol version.');
  if (!requestId(value.id)) return invalid(null, 'Request id must be a UUID.');
  if (typeof value.action !== 'string' || !ACTIONS.has(value.action)) {
    return { version: CONTROLLER_PROTOCOL_VERSION, id: value.id, ok: false, error: { code: 'unsupported_action', message: 'Controller action is not allowed.' } };
  }
  if (!record(value.params)) return invalid(value.id, 'Request params must be an object.');

  switch (value.action) {
    case 'getSnapshot':
    case 'listTerminals':
      return exactKeys(value.params, [])
        ? { id: value.id, request: { action: value.action, params: {} } }
        : invalid(value.id, 'This action does not accept params.');
    case 'sendMessage': {
      const attachmentIds = value.params.attachmentIds;
      if (!(exactKeys(value.params, ['taskId', 'text']) || exactKeys(value.params, ['taskId', 'text', 'attachmentIds'])) || !idParameter(value.params.taskId) ||
          typeof value.params.text !== 'string' || value.params.text.length > CONTROLLER_MAX_TEXT_LENGTH) {
        return invalid(value.id, `sendMessage requires a task UUID and text of at most ${CONTROLLER_MAX_TEXT_LENGTH} characters.`);
      }
      if (attachmentIds !== undefined && (!Array.isArray(attachmentIds) || attachmentIds.length > CONTROLLER_MAX_UPLOADS_PER_SESSION || attachmentIds.some(item => !idParameter(item)))) return invalid(value.id, 'sendMessage attachment IDs are invalid.');
      if (!value.params.text.length && (!attachmentIds || attachmentIds.length === 0)) return invalid(value.id, 'sendMessage requires text or an attachment.');
      return { id: value.id, request: { action: 'sendMessage', params: { taskId: value.params.taskId, text: value.params.text, ...(attachmentIds ? { attachmentIds: [...attachmentIds] } : {}) } } };
    }
    case 'beginAttachmentUpload':
      { const size = value.params.size;
      if (!(exactKeys(value.params, ['taskId', 'filename', 'mimeType', 'size']) || exactKeys(value.params, ['taskId', 'filename', 'mimeType', 'size', 'previewDataUrl'])) || !idParameter(value.params.taskId) ||
        typeof value.params.filename !== 'string' || !value.params.filename.trim() || value.params.filename.length > 255 || /[\\/\0]/.test(value.params.filename) ||
        typeof value.params.mimeType !== 'string' || value.params.mimeType.length > 255 ||
        typeof size !== 'number' || !Number.isSafeInteger(size) || size < 1 || size > CONTROLLER_MAX_UPLOAD_BYTES ||
        (value.params.previewDataUrl !== undefined && !validUploadPreview(value.params.previewDataUrl))) return invalid(value.id, `Attachment upload requires a task UUID, safe file metadata, and size of 1-${CONTROLLER_MAX_UPLOAD_BYTES} bytes.`);
      return { id: value.id, request: { action: 'beginAttachmentUpload', params: { taskId: value.params.taskId, filename: value.params.filename, mimeType: value.params.mimeType, size: size as number, ...(value.params.previewDataUrl ? { previewDataUrl: value.params.previewDataUrl } : {}) } } };
      }
    case 'appendAttachmentUpload':
      if (!exactKeys(value.params, ['uploadId', 'dataBase64']) || !idParameter(value.params.uploadId) || typeof value.params.dataBase64 !== 'string' || !validUploadChunk(value.params.dataBase64)) return invalid(value.id, 'Attachment chunk is invalid or too large.');
      return { id: value.id, request: { action: 'appendAttachmentUpload', params: { uploadId: value.params.uploadId, dataBase64: value.params.dataBase64 } } };
    case 'finishAttachmentUpload':
      return exactKeys(value.params, ['uploadId']) && idParameter(value.params.uploadId)
        ? { id: value.id, request: { action: 'finishAttachmentUpload', params: { uploadId: value.params.uploadId } } }
        : invalid(value.id, 'Attachment upload ID is invalid.');
    case 'cancelTask':
    case 'resumeTask':
      return exactKeys(value.params, ['taskId']) && idParameter(value.params.taskId)
        ? { id: value.id, request: { action: value.action, params: { taskId: value.params.taskId } } }
        : invalid(value.id, `${value.action} requires a task UUID.`);
    case 'readTerminal':
      return exactKeys(value.params, ['id', 'afterSeq']) && idParameter(value.params.id) &&
        typeof value.params.afterSeq === 'number' && Number.isSafeInteger(value.params.afterSeq) && value.params.afterSeq >= 0
        ? { id: value.id, request: { action: 'readTerminal', params: { id: value.params.id, afterSeq: value.params.afterSeq } } }
        : invalid(value.id, 'readTerminal requires a terminal UUID and non-negative sequence number.');
    default:
      return invalid(value.id, 'Controller action is not allowed.');
  }
}

export function authenticatedControllerContext(value: unknown): value is AuthenticatedControllerContext {
  return record(value) && value.authenticated === true && typeof value.subject === 'string' &&
    value.subject.length > 0 && value.subject.length <= CONTROLLER_MAX_SUBJECT_LENGTH;
}
