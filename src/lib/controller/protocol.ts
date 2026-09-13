import type { Snapshot, TerminalRead, TerminalSession } from '../types';

/**
 * Wire format for the future paired-controller relay.  This module deliberately
 * contains no transport, pairing, encryption, or authentication implementation.
 */
export const CONTROLLER_PROTOCOL_VERSION = 1 as const;
export const CONTROLLER_MAX_REQUEST_BYTES = 64 * 1024;
export const CONTROLLER_MAX_RESPONSE_BYTES = 1024 * 1024;
export const CONTROLLER_MAX_TEXT_LENGTH = 32 * 1024;
export const CONTROLLER_MAX_SUBJECT_LENGTH = 256;
/** A durable desktop acknowledgement, deliberately smaller than a Snapshot. */
export interface ControllerSendReceipt { readonly accepted: true; }

export type ControllerAction =
  | 'getSnapshot'
  | 'sendMessage'
  | 'cancelTask'
  | 'resumeTask'
  | 'listTerminals'
  | 'readTerminal';

export interface GetSnapshotRequest { readonly action: 'getSnapshot'; readonly params: Record<string, never>; }
export interface SendMessageRequest { readonly action: 'sendMessage'; readonly params: { taskId: string; text: string }; }
export interface CancelTaskRequest { readonly action: 'cancelTask'; readonly params: { taskId: string }; }
export interface ResumeTaskRequest { readonly action: 'resumeTask'; readonly params: { taskId: string }; }
export interface ListTerminalsRequest { readonly action: 'listTerminals'; readonly params: Record<string, never>; }
export interface ReadTerminalRequest { readonly action: 'readTerminal'; readonly params: { id: string; afterSeq: number }; }

export type ControllerRequest =
  | GetSnapshotRequest
  | SendMessageRequest
  | CancelTaskRequest
  | ResumeTaskRequest
  | ListTerminalsRequest
  | ReadTerminalRequest;

export type ControllerResult = Snapshot | ControllerSendReceipt | TerminalSession[] | TerminalRead;

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
  sendMessage(taskId: string, text: string): Promise<Snapshot | ControllerSendReceipt>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resumeTask(taskId: string): Promise<Snapshot>;
  listTerminals(): Promise<TerminalSession[]>;
  readTerminal(id: string, afterSeq: number): Promise<TerminalRead>;
}

export const CONTROLLER_MUTATING_ACTIONS: ReadonlySet<ControllerAction> = new Set([
  'sendMessage', 'cancelTask', 'resumeTask',
]);

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const ACTIONS: ReadonlySet<string> = new Set<ControllerAction>([
  'getSnapshot', 'sendMessage', 'cancelTask', 'resumeTask', 'listTerminals', 'readTerminal',
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
    case 'sendMessage':
      if (!exactKeys(value.params, ['taskId', 'text']) || !idParameter(value.params.taskId) ||
          typeof value.params.text !== 'string' || value.params.text.length === 0 || value.params.text.length > CONTROLLER_MAX_TEXT_LENGTH) {
        return invalid(value.id, `sendMessage requires a task UUID and text of 1-${CONTROLLER_MAX_TEXT_LENGTH} characters.`);
      }
      return { id: value.id, request: { action: 'sendMessage', params: { taskId: value.params.taskId, text: value.params.text } } };
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
