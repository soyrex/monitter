import { ControllerDispatcher } from './dispatcher';
import { CONTROLLER_PROTOCOL_VERSION, parseControllerRequest, type ControllerRequest, type ControllerResponse, type ControllerResult, type ControllerSendReceipt } from './protocol';
import { SecureChannel, base64urlToBytes, bytesToBase64url, deriveDirectionalKeys, deriveEcdhSecret, exportPairingPublicKey, generatePairingKeyPair, pairingCommitment, randomBytes, verificationCode, type SecureEnvelope } from './secure-session';
import type { Snapshot, TerminalRead, TerminalSession } from '../types';
import type { ControllerClient } from './protocol';

export type RemoteConnectionStatus = 'connecting' | 'waiting_for_peer' | 'pending' | 'awaiting_approval' | 'connected' | 'rejected' | 'closed' | 'error';
export interface RemoteConnectionState { readonly status: RemoteConnectionStatus; readonly error?: string; readonly verificationCode?: string; }
export interface CollaboratorIdentity { readonly name: string; readonly role: 'visitor'; }
export interface ControllerInvitationV1 { readonly version: 1; readonly relayUrl: string; readonly room: string; readonly secret: string; }
export interface ControllerInvitationV2 { readonly version: 2; readonly relayUrl: string; readonly room: string; readonly publicKey: string; }
export type ControllerInvitation = ControllerInvitationV1 | ControllerInvitationV2;
export interface DesktopBridge extends ControllerClient {}
export interface PersistentDesktopIdentity { readonly keyPair: CryptoKeyPair; readonly room: string; }
export interface PersistentMobileIdentity { readonly keyPair: CryptoKeyPair; }
export interface PersistentDesktopSessionOptions {
  readonly identity: PersistentDesktopIdentity;
  /** Checked only after the fresh ECDH handshake and encrypted pair request. */
  readonly isTrustedController?: (publicKey: string) => boolean | Promise<boolean>;
  /** Called for authenticated approved-controller activity; callers persist policy state. */
  readonly onControllerAccess?: (publicKey: string) => void | Promise<void>;
}
export interface PersistentMobileSessionOptions { readonly identity: PersistentMobileIdentity; }
export interface DesktopSession {
  readonly invitation: string;
  getStatus(): RemoteConnectionStatus;
  getVerificationCode(): string | null;
  getPeer(): CollaboratorIdentity | null;
  /** Pinned v2 controller identity after a verified ECDH handshake. */
  getPeerControllerPublicKey(): string | null;
  subscribe(listener: (state: RemoteConnectionState) => void): () => void;
  onPendingPeer(listener: () => void): () => void;
  approve(): Promise<void>;
  reject(): Promise<void>;
  close(): void;
}
export interface MobileSession {
  getStatus(): RemoteConnectionStatus;
  getVerificationCode(): string | null;
  /** True only when this connected desktop advertised durable device trust. */
  supportsRememberedDevices(): boolean;
  subscribe(listener: (state: RemoteConnectionState) => void): () => void;
  getSnapshot(): Promise<Snapshot>;
  sendMessage(taskId: string, text: string): Promise<Snapshot | ControllerSendReceipt>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resumeTask(taskId: string): Promise<Snapshot>;
  listTerminals(): Promise<TerminalSession[]>;
  readTerminal(id: string, afterSeq: number): Promise<TerminalRead>;
  close(): void;
}
/** Collaboration visitors never negotiate controller send receipts. */
export interface CollaboratorMobileSession extends Omit<MobileSession, 'sendMessage'> {
  sendMessage(taskId: string, text: string): Promise<Snapshot>;
}

type Role = 'desktop' | 'mobile';
type RelayMessage = { type: 'joined' } | { type: 'peer' } | { type: 'peer_left' } | { type: 'hello'; nonce: string } | SecureEnvelope;
type SocketFactory = (url: string) => WebSocket;
interface PairingKeys { readonly privateKey?: CryptoKey; readonly publicKey?: Uint8Array; readonly expectedPeerPublicKey?: Uint8Array; }
// Base64 and AEAD authentication expand a permitted 1 MiB controller response.
const MAX_RELAY_FRAME_BYTES = 2 * 1024 * 1024;
const MAX_INVITATION_BYTES = 1024;
const PAIRING_TIMEOUT_MS = 5 * 60_000;
const MAX_CONCURRENT_DESKTOP_RPCS = 32;
const SEND_RECEIPT_CAPABILITY = 'send-receipt-v1';
const REMEMBERED_DEVICE_CAPABILITY = 'remembered-device-v1';

function uuid(): string { return crypto.randomUUID(); }
function validRelayUrl(value: string): boolean {
  try { const url = new URL(value); return url.protocol === 'wss:' || (url.protocol === 'ws:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)); } catch { return false; }
}
function parseInvitation(value: string): ControllerInvitation {
  if (new TextEncoder().encode(value).byteLength > MAX_INVITATION_BYTES) throw new Error('Invalid controller invitation.');
  let invitation: unknown; try { invitation = JSON.parse(value); } catch { throw new Error('Invalid controller invitation.'); }
  if (!invitation || typeof invitation !== 'object') throw new Error('Invalid controller invitation.');
  const v = invitation as Record<string, unknown>;
  if (typeof v.relayUrl !== 'string' || !validRelayUrl(v.relayUrl) || typeof v.room !== 'string' || !/^[A-Za-z0-9_-]{22,128}$/.test(v.room)) throw new Error('Invalid controller invitation.');
  if (v.version === 1 && typeof v.secret === 'string' && base64urlToBytes(v.secret).byteLength === 32) return { version: 1, relayUrl: v.relayUrl, room: v.room, secret: v.secret };
  if (v.version === 2 && typeof v.publicKey === 'string' && base64urlToBytes(v.publicKey).byteLength === 65) return { version: 2, relayUrl: v.relayUrl, room: v.room, publicKey: v.publicKey };
  throw new Error('Invalid controller invitation.');
}
function asObject(value: unknown): Record<string, unknown> | null { return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null; }
function collaborator(value: unknown): CollaboratorIdentity | null {
  const candidate = asObject(value);
  if (!candidate || candidate.role !== 'visitor' || typeof candidate.name !== 'string') return null;
  const name = candidate.name.trim();
  return /^[\p{L}\p{N}][\p{L}\p{N} ._'’-]{1,47}$/u.test(name) ? { name, role: 'visitor' } : null;
}

class PairingConnection {
  protected socket: WebSocket | null = null;
  protected channel: SecureChannel | null = null;
  protected readonly nonce = randomBytes(16);
  protected peerNonce: Uint8Array | null = null;
  protected peerPublicKey: Uint8Array | null = null;
  protected status: RemoteConnectionStatus = 'connecting';
  private readonly listeners = new Set<(state: RemoteConnectionState) => void>();
  private receiveQueue: Promise<void> = Promise.resolve();
  private sendQueue: Promise<void> = Promise.resolve();
  private readonly pairingTimer: ReturnType<typeof setTimeout>;
  private closed = false;
  private verification: string | null = null;
  private commitmentSent = false;
  private revealSent = false;
  private peerCommitment: Uint8Array | null = null;
  constructor(protected readonly invitationData: ControllerInvitation, protected readonly role: Role, private readonly pairingKeys: PairingKeys = {}, private readonly socketFactory: SocketFactory = url => new WebSocket(url)) {
    this.pairingTimer = setTimeout(() => { if (!this.closed) this.fail(); }, PAIRING_TIMEOUT_MS);
    this.connect();
  }
  getStatus() { return this.status; }
  getVerificationCode() { return this.verification; }
  subscribe(listener: (state: RemoteConnectionState) => void) { this.listeners.add(listener); listener({ status: this.status, ...(this.verification ? { verificationCode: this.verification } : {}) }); return () => this.listeners.delete(listener); }
  protected setStatus(status: RemoteConnectionStatus, error?: string) { if (this.status === status && !error) return; this.status = status; for (const listener of this.listeners) listener({ status, ...(error ? { error } : {}), ...(this.verification ? { verificationCode: this.verification } : {}) }); }
  protected connect() {
    try {
      const endpoint = new URL(this.invitationData.relayUrl);
      // Public routing metadata only; the invitation secret never leaves the endpoints.
      endpoint.searchParams.set('room', this.invitationData.room);
      endpoint.searchParams.set('role', this.role);
      this.socket = this.socketFactory(endpoint.href);
    }
    catch { this.setStatus('error'); return; }
    this.socket.onopen = () => this.sendRaw({ type: 'join', room: this.invitationData.room, role: this.role });
    this.socket.onmessage = event => { this.receiveQueue = this.receiveQueue.then(() => this.receive(event.data)).catch(() => this.fail()); };
    this.socket.onerror = () => this.setStatus('error');
    this.socket.onclose = () => { this.closed = true; clearTimeout(this.pairingTimer); if (this.status !== 'rejected') this.setStatus('closed'); this.channel = null; this.peerNonce = null; this.onDisconnected(); };
  }
  protected sendRaw(value: unknown) { if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(value)); }
  protected sendSecure(value: unknown): Promise<void> {
    const send = this.sendQueue.then(async () => { if (!this.channel || this.socket?.readyState !== WebSocket.OPEN) throw new Error('Secure pairing has not completed.'); this.sendRaw(await this.channel.seal(value)); });
    this.sendQueue = send.catch(() => undefined);
    return send;
  }
  private async receive(raw: unknown) {
    // A received action may already be executing; let that finish, but never
    // dispatch a frame that was queued before a later socket close.
    if (this.closed) return;
    if (typeof raw !== 'string' || raw.length > MAX_RELAY_FRAME_BYTES) return this.fail();
    let message: unknown; try { message = JSON.parse(raw); } catch { return this.fail(); }
    const data = asObject(message); if (!data || typeof data.type !== 'string') return this.fail();
    if (data.type === 'joined') { this.setStatus('waiting_for_peer'); return; }
    if (data.type === 'peer') {
      if (this.invitationData.version === 1) this.sendRaw({ type: 'hello', nonce: bytesToBase64url(this.nonce) });
      else await this.sendV2Commit();
      return;
    }
    if (data.type === 'peer_left') { this.close(); return; }
    if (data.type === 'hello') { await this.receiveHello(data); return; }
    if (data.type === 'secure') {
      if (!this.channel || typeof data.seq !== 'number' || typeof data.ciphertext !== 'string') return this.fail();
      try { await this.onSecure(await this.channel.open(data as unknown as SecureEnvelope)); } catch { this.fail(); }
      return;
    }
    this.fail();
  }
  private async sendV2Commit() {
    if (this.commitmentSent || !this.pairingKeys.publicKey) return this.fail();
    this.commitmentSent = true;
    this.sendRaw({ type: 'hello', phase: 'commit', nonce: bytesToBase64url(await pairingCommitment(this.role, this.pairingKeys.publicKey, this.nonce)) });
  }
  private async sendV2Reveal() {
    if (!this.commitmentSent || this.revealSent || !this.pairingKeys.publicKey) return this.fail();
    this.revealSent = true;
    this.sendRaw({ type: 'hello', phase: 'reveal', nonce: bytesToBase64url(this.nonce), publicKey: bytesToBase64url(this.pairingKeys.publicKey) });
  }
  private async receiveHello(data: Record<string, unknown>) {
    try {
      if (this.invitationData.version === 1) {
        if (typeof data.nonce !== 'string' || data.nonce.length > 64 || this.channel || this.peerNonce) throw new Error();
        this.peerNonce = base64urlToBytes(data.nonce); if (this.peerNonce.byteLength !== 16) throw new Error();
        const desktopNonce = this.role === 'desktop' ? this.nonce : this.peerNonce;
        const mobileNonce = this.role === 'mobile' ? this.nonce : this.peerNonce;
        this.channel = new SecureChannel(await deriveDirectionalKeys(base64urlToBytes(this.invitationData.secret), desktopNonce, mobileNonce, this.role));
        await this.onChannelReady(); return;
      }
      if (data.phase === 'commit') {
        if (this.peerCommitment || this.channel || typeof data.nonce !== 'string' || data.nonce.length > 64) throw new Error();
        this.peerCommitment = base64urlToBytes(data.nonce); if (this.peerCommitment.byteLength !== 32) throw new Error();
        await this.sendV2Reveal(); return;
      }
      if (data.phase !== 'reveal' || !this.peerCommitment || this.channel || this.peerNonce || typeof data.nonce !== 'string' || data.nonce.length > 64 || typeof data.publicKey !== 'string' || data.publicKey.length > 128 || !this.pairingKeys.privateKey || !this.pairingKeys.publicKey) throw new Error();
      const peerNonce = base64urlToBytes(data.nonce), peerPublic = base64urlToBytes(data.publicKey);
      if (peerNonce.byteLength !== 16 || peerPublic.byteLength !== 65) throw new Error();
      const peerRole: Role = this.role === 'desktop' ? 'mobile' : 'desktop';
      if (!sameBytes(await pairingCommitment(peerRole, peerPublic, peerNonce), this.peerCommitment)) throw new Error('Pairing reveal does not match commitment.');
      if (this.pairingKeys.expectedPeerPublicKey && !sameBytes(peerPublic, this.pairingKeys.expectedPeerPublicKey)) throw new Error('Pinned desktop key mismatch.');
      this.peerNonce = peerNonce;
      this.peerPublicKey = peerPublic;
      const desktopNonce = this.role === 'desktop' ? this.nonce : peerNonce;
      const mobileNonce = this.role === 'mobile' ? this.nonce : peerNonce;
      const sharedSecret = await deriveEcdhSecret(this.pairingKeys.privateKey, peerPublic);
      const desktopPublic = this.role === 'desktop' ? this.pairingKeys.publicKey : peerPublic;
      const mobilePublic = this.role === 'mobile' ? this.pairingKeys.publicKey : peerPublic;
      this.verification = await verificationCode(desktopPublic, mobilePublic, desktopNonce, mobileNonce, sharedSecret);
      this.channel = new SecureChannel(await deriveDirectionalKeys(sharedSecret, desktopNonce, mobileNonce, this.role, 2));
      await this.onChannelReady();
    } catch { this.fail(); }
  }
  protected fail() { this.closed = true; clearTimeout(this.pairingTimer); this.setStatus('error'); this.socket?.close(); }
  protected async onChannelReady(): Promise<void> {}
  protected async onSecure(_message: unknown): Promise<void> {}
  protected onDisconnected(): void {}
  protected completePairing() { clearTimeout(this.pairingTimer); }
  close() { this.closed = true; clearTimeout(this.pairingTimer); this.setStatus('closed'); this.socket?.close(); }
}
function sameBytes(left: Uint8Array, right: Uint8Array): boolean { if (left.byteLength !== right.byteLength) return false; let mismatch = 0; for (let i = 0; i < left.byteLength; i += 1) mismatch |= left[i] ^ right[i]; return mismatch === 0; }

export async function createDesktopSession(relayUrl: string, bridge: DesktopBridge, options?: PersistentDesktopSessionOptions): Promise<DesktopSession> {
  if (!validRelayUrl(relayUrl)) throw new Error('Relay must use wss://, or ws:// only on loopback.');
  if (options && (!/^[A-Za-z0-9_-]{22,128}$/.test(options.identity.room) || !validPersistentKeyPair(options.identity.keyPair))) throw new Error('Invalid persistent controller identity.');
  const keyPair = options?.identity.keyPair ?? await generatePairingKeyPair();
  const publicKey = await exportPairingPublicKey(keyPair.publicKey);
  const invitationData: ControllerInvitationV2 = { version: 2, relayUrl, room: options?.identity.room ?? bytesToBase64url(randomBytes(18)), publicKey: bytesToBase64url(publicKey) };
  const session = new class extends PairingConnection {
    private approved = false;
    private authorizationEpoch = 0;
    close() { this.approved = false; this.authorizationEpoch++; super.close(); }
    private peerSupportsSendReceipt = false;
    private pendingRpcs = 0;
    private peer: CollaboratorIdentity | null = null;
    private readonly pendingListeners = new Set<() => void>();
    private readonly dispatcher = new ControllerDispatcher(bridge);
    readonly invitation = JSON.stringify(invitationData);
    getPeer() { return this.peer; }
    getPeerControllerPublicKey() { return this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null; }
    onPendingPeer(listener: () => void) { this.pendingListeners.add(listener); return () => this.pendingListeners.delete(listener); }
    async approve() {
      if (!this.channel || this.status !== 'pending') throw new Error('No pending paired device to approve.');
      try {
        const capabilities = [
          ...(this.peerSupportsSendReceipt ? [SEND_RECEIPT_CAPABILITY] : []),
          ...(options?.isTrustedController ? [REMEMBERED_DEVICE_CAPABILITY] : []),
        ];
        await this.sendSecure({ type: 'approved', ...(capabilities.length ? { capabilities } : {}) });
        this.approved = true;
        if (!this.peer && this.peerPublicKey) await options?.onControllerAccess?.(bytesToBase64url(this.peerPublicKey));
        this.completePairing(); this.setStatus('connected');
      }
      catch (reason) { this.approved = false; this.close(); throw reason; }
    }
    async reject() {
      this.approved = false; this.authorizationEpoch++;
      if (!this.channel) throw new Error('No pending paired device to reject.');
      await this.sendSecure({ type: 'rejected' });
      // Let the peer receive and acknowledge the terminal state by closing its
      // socket. Closing here races the relay's peer_left frame ahead of the
      // encrypted rejection on some WebSocket implementations.
      this.setStatus('rejected');
    }
    protected async onChannelReady() { /* A valid encrypted pair-request proves invitation-secret possession. */ }
    protected async onSecure(message: unknown) {
      const value = asObject(message); if (!value) throw new Error('Invalid paired message.');
      if (value.type === 'pair-request' && !this.approved) {
        if (Object.hasOwn(value, 'operator')) {
          this.peer = collaborator(value.operator);
          if (!this.peer) throw new Error('Invalid collaborator identity.');
        }
        this.peerSupportsSendReceipt = hasSendReceiptCapability(value.capabilities);
        this.setStatus('pending'); for (const listener of this.pendingListeners) listener();
        // Only controller sessions (never visitor/operator sessions) may resume
        // a remembered approval, and only after this connection's ECDH proof and
        // encrypted request. Recheck after an async trust lookup for revocation.
        if (!this.peer && options?.isTrustedController && this.peerPublicKey) {
          let known = false; try { known = await options.isTrustedController(bytesToBase64url(this.peerPublicKey)); } catch { /* Leave manual approval available. */ }
          if (known && this.status === 'pending' && !this.approved && !this.peer) await this.approve();
        }
        return;
      }
      if (value.type !== 'rpc' || !this.approved || this.status !== 'connected' || typeof value.json !== 'string') throw new Error('Unapproved controller request.');
      // Keep authenticated opening and sealing ordered, but do not let a slow
      // bridge read (notably a Snapshot) prevent a later cancel, send, or close
      // frame from being decrypted and acted upon.
      this.startRpc(value.json);
    }
    private startRpc(json: string) {
      if (this.pendingRpcs >= MAX_CONCURRENT_DESKTOP_RPCS) {
        void this.sendSecure({ type: 'response', json: JSON.stringify({ version: CONTROLLER_PROTOCOL_VERSION, id: requestIdFromJson(json), ok: false, error: { code: 'busy', message: 'Too many pending controller requests.' } }) })
          .catch(() => { if (this.status === 'connected') this.close(); });
        return;
      }
      this.pendingRpcs += 1;
      const authorizationEpoch = this.authorizationEpoch;
      const stillApproved = () => this.approved && this.status === 'connected' && authorizationEpoch === this.authorizationEpoch;
      void (async () => {
        const controllerKey = !this.peer && this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null;
        // Trust is live policy, not a property cached at socket approval. An
        // expired or revoked controller must not dispatch an RPC and then use
        // that same RPC to renew its inactivity window.
        if (controllerKey && options?.isTrustedController && !(await options.isTrustedController(controllerKey))) {
          this.approved = false;
          this.close();
          return;
        }
        let validRequest = false;
        try {
          const input: unknown = JSON.parse(json);
          validRequest = !('ok' in parseControllerRequest(input));
        } catch { /* The dispatcher returns the protocol error below. */ }
        // Renew when authenticated access arrives, even if the bridge operation is slow.
        if (controllerKey && validRequest) await options?.onControllerAccess?.(controllerKey);
        if (controllerKey && options?.isTrustedController && !(await options.isTrustedController(controllerKey))) { this.close(); return; }
        if (!stillApproved()) return;
        const response = await this.dispatcher.dispatchJson(json, { authenticated: true, subject: `paired-room:${invitationData.room}` }, { allowSendReceipt: this.peerSupportsSendReceipt });
        if (stillApproved()) await this.sendSecure({ type: 'response', json: response });
      })()
        // A close/revocation must never revive the socket. A failed response
        // write while the session is still live is a transport failure.
        .catch(() => { if (this.status === 'connected') this.close(); })
        .finally(() => { this.pendingRpcs -= 1; });
    }
  }(invitationData, 'desktop', { privateKey: keyPair.privateKey, publicKey });
  return session;
}

export function createMobileSession(invitation: string): Promise<MobileSession>;
export function createMobileSession(invitation: string, operator: CollaboratorIdentity): Promise<CollaboratorMobileSession>;
export function createMobileSession(invitation: string, operator: undefined, options: PersistentMobileSessionOptions): Promise<MobileSession>;
export async function createMobileSession(invitation: string, operator?: CollaboratorIdentity, options?: PersistentMobileSessionOptions): Promise<MobileSession | CollaboratorMobileSession> {
  if (operator && !collaborator(operator)) throw new Error('Enter a collaborator name of 2-48 characters.');
  const invitationData = parseInvitation(invitation);
  if (options && !validPersistentKeyPair(options.identity.keyPair)) throw new Error('Persistent controller sessions require a valid identity.');
  const keyPair = invitationData.version === 2 ? (options?.identity.keyPair ?? await generatePairingKeyPair()) : null;
  const publicKey = keyPair ? await exportPairingPublicKey(keyPair.publicKey) : undefined;
  const expectedPeerPublicKey = invitationData.version === 2 ? base64urlToBytes(invitationData.publicKey) : undefined;
  const session = new class extends PairingConnection {
    private readonly pending = new Map<string, { resolve: (value: ControllerResult) => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout> }>();
    private rememberedDevices = false;
    supportsRememberedDevices() { return this.rememberedDevices; }
    protected async onChannelReady() { this.setStatus('awaiting_approval'); await this.sendSecure(operator ? { type: 'pair-request', operator } : { type: 'pair-request', capabilities: [SEND_RECEIPT_CAPABILITY] }); }
    protected async onSecure(message: unknown) {
      const value = asObject(message); if (!value || typeof value.type !== 'string') throw new Error('Invalid paired response.');
      if (value.type === 'pair-request') return;
      if (value.type === 'approved') { this.rememberedDevices = invitationData.version === 2 && hasCapability(value.capabilities, REMEMBERED_DEVICE_CAPABILITY); this.completePairing(); this.setStatus('connected'); return; }
      if (value.type === 'rejected') { this.setStatus('rejected'); this.socket?.close(); return; }
      if (value.type !== 'response' || typeof value.json !== 'string') throw new Error('Unexpected paired response.');
      const response = JSON.parse(value.json) as ControllerResponse;
      const id = response && typeof response === 'object' && typeof response.id === 'string' ? response.id : null;
      const callback = id ? this.pending.get(id) : undefined;
      if (!callback) return;
      this.pending.delete(response.id as string); clearTimeout(callback.timer);
      if (response.ok) callback.resolve(response.result); else callback.reject(new Error(response.error.message));
    }
    private call(action: ControllerRequest['action'], params: Record<string, unknown>) {
      if (this.status !== 'connected') return Promise.reject(new Error('Desktop approval and a live encrypted connection are required.'));
      if (this.pending.size >= 32) return Promise.reject(new Error('Too many pending controller requests.'));
      const id = uuid(); const json = JSON.stringify({ version: CONTROLLER_PROTOCOL_VERSION, id, action, params });
      return new Promise<ControllerResult>((resolve, reject) => {
        const timer = setTimeout(() => { this.pending.delete(id); reject(new Error('Controller request timed out.')); }, 30_000);
        this.pending.set(id, { resolve, reject, timer });
        this.sendSecure({ type: 'rpc', json }).catch(reason => { const request = this.pending.get(id); if (request) { clearTimeout(request.timer); this.pending.delete(id); } reject(reason instanceof Error ? reason : new Error('Connection failed.')); });
      });
    }
    async getSnapshot() { return this.call('getSnapshot', {}) as Promise<Snapshot>; }
    async sendMessage(taskId: string, text: string) { return this.call('sendMessage', { taskId, text }) as Promise<Snapshot | ControllerSendReceipt>; }
    async cancelTask(taskId: string) { return this.call('cancelTask', { taskId }) as Promise<Snapshot>; }
    async resumeTask(taskId: string) { return this.call('resumeTask', { taskId }) as Promise<Snapshot>; }
    async listTerminals() { return this.call('listTerminals', {}) as Promise<TerminalSession[]>; }
    async readTerminal(id: string, afterSeq: number) { return this.call('readTerminal', { id, afterSeq }) as Promise<TerminalRead>; }
    protected onDisconnected() { for (const callback of this.pending.values()) { clearTimeout(callback.timer); callback.reject(new Error('Connection closed.')); } this.pending.clear(); }
    close() { this.onDisconnected(); super.close(); }
  }(invitationData, 'mobile', keyPair ? { privateKey: keyPair.privateKey, publicKey, expectedPeerPublicKey } : {});
  return session as MobileSession | CollaboratorMobileSession;
}

function hasSendReceiptCapability(value: unknown): boolean {
  return hasCapability(value, SEND_RECEIPT_CAPABILITY);
}

function hasCapability(value: unknown, capability: string): boolean {
  return Array.isArray(value) && value.length <= 8 && value.every(item => typeof item === 'string' && item.length <= 64) && value.includes(capability);
}

function requestIdFromJson(json: string): string | null {
  try {
    const request = JSON.parse(json);
    return request && typeof request === 'object' && typeof request.id === 'string' ? request.id : null;
  } catch { return null; }
}

function validPersistentKeyPair(keyPair: CryptoKeyPair): boolean {
  return keyPair.privateKey.type === 'private' && keyPair.publicKey.type === 'public' &&
    keyPair.privateKey.algorithm.name === 'ECDH' && keyPair.publicKey.algorithm.name === 'ECDH';
}

/**
 * Sessions are intentionally memory-only. Disconnects close the current session:
 * mutations are never automatically resent. Re-pairing is the supported first-pass
 * recovery path until durable receipt and device identity storage exist.
 */
