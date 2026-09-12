import type { Snapshot } from '../types';
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

export interface ControllerDispatcherOptions { maxMutationReceipts?: number; }
export interface ControllerDispatchOptions {
  /** Only a peer that explicitly offered this capability may receive receipts. */
  readonly allowSendReceipt?: boolean;
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
  private readonly maxMutationReceipts: number;

  constructor(private readonly client: ControllerClient, options: ControllerDispatcherOptions = {}) {
    const capacity = options.maxMutationReceipts ?? 256;
    if (!Number.isSafeInteger(capacity) || capacity < 1 || capacity > 4096) throw new Error('maxMutationReceipts must be an integer from 1 to 4096.');
    this.maxMutationReceipts = capacity;
  }

  async dispatch(input: unknown, context: unknown, options: ControllerDispatchOptions = {}): Promise<ControllerResponse> {
    const parsed = parseControllerRequest(input);
    if ('ok' in parsed) return parsed;
    if (!authenticatedControllerContext(context)) return this.error(parsed.id, 'unauthenticated', 'An authenticated paired transport is required.');
    if (CONTROLLER_MUTATING_ACTIONS.has(parsed.request.action)) return this.dispatchMutation(parsed.id, parsed.request, context, options);
    return this.execute(parsed.id, parsed.request, options);
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

  private dispatchMutation(id: string, request: ControllerRequest, context: AuthenticatedControllerContext, options: ControllerDispatchOptions): Promise<ControllerResponse> {
    const fingerprint = JSON.stringify({ action: request.action, params: request.params });
    // Request IDs are scoped to the authenticated transport subject. A receipt
    // from one paired device must never be returned to another device.
    const receiptKey = `${context.subject}\u0000${id}`;
    const existing = this.receipts.get(receiptKey);
    if (existing) {
      if (existing.fingerprint !== fingerprint) return Promise.resolve(this.error(id, 'id_conflict', 'Request id was already used with different mutation data.'));
      return existing.response;
    }
    if (this.receipts.size >= this.maxMutationReceipts) return Promise.resolve(this.error(id, 'dedup_saturated', 'Mutation receipt cache is full; establish a new authenticated session.'));
    const receipt: Receipt = { fingerprint, response: this.execute(id, request, options) };
    this.receipts.set(receiptKey, receipt);
    return receipt.response;
  }

  private async execute(id: string, request: ControllerRequest, options: ControllerDispatchOptions): Promise<ControllerResponse> {
    try {
      let result: ControllerResult;
      switch (request.action) {
        case 'getSnapshot': result = controllerSnapshot(await this.client.getSnapshot()); break;
        case 'sendMessage': {
          const sendResult = await this.client.sendMessage(request.params.taskId, request.params.text);
          // Older mobile builds expect every send response to be a Snapshot.
          // Only a pairing peer that explicitly negotiated the capability sees
          // the small durable receipt; no mutation is retried to learn this.
          result = isSendReceipt(sendResult) && options.allowSendReceipt
            ? sendResult
            : controllerSnapshot(isSendReceipt(sendResult) ? await this.client.getSnapshot() : sendResult);
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
}

function isSendReceipt(value: Snapshot | ControllerSendReceipt): value is ControllerSendReceipt {
  return 'accepted' in value && value.accepted === true && !('tasks' in value);
}
