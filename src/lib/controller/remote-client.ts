import { ControllerDispatcher } from './dispatcher';
import { OwnerRelayDisconnectedError } from '../owner-relay-error.js';
import { CONTROLLER_MAX_UPLOAD_BYTES, CONTROLLER_MAX_UPLOAD_PREVIEW_CHARS, CONTROLLER_PROTOCOL_VERSION, CONTROLLER_UPLOAD_CHUNK_BYTES, parseControllerRequest, type ControllerRequest, type ControllerResponse, type ControllerResult, type ControllerSendReceipt } from './protocol';
import { SecureChannel, base64urlToBytes, bytesToBase64url, deriveDirectionalKeys, deriveEcdhSecret, exportPairingPublicKey, generatePairingKeyPair, pairingCommitment, randomBytes, verificationCode, type SecureEnvelope } from './secure-session';
import type { Attachment, AttachmentFileData, Snapshot, TerminalRead, TerminalSession } from '../types';
import type { ControllerClient } from './protocol';

export type RemoteConnectionStatus = 'connecting' | 'waiting_for_peer' | 'pending' | 'awaiting_approval' | 'connected' | 'rejected' | 'closed' | 'error';
export interface RemoteConnectionState { readonly status: RemoteConnectionStatus; readonly error?: string; readonly verificationCode?: string; }
export type OwnerWorkspaceState = 'none' | 'requested' | 'approved' | 'rejected';
export interface OwnerInvokeContext { readonly publicKey: string; }
export type OwnerInvoke = (command: string, args: unknown, context: OwnerInvokeContext) => unknown | Promise<unknown>;
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
  /** Separate remembered full-workspace scope; controller trust never implies this. */
  readonly isTrustedOwnerWorkspace?: (publicKey: string) => boolean | Promise<boolean>;
  readonly onOwnerWorkspaceAccess?: (publicKey: string) => void | Promise<void>;
  /** Transport hook only. The desktop owner chooses the explicit command allowlist. */
  readonly onOwnerInvoke?: OwnerInvoke;
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
  getOwnerWorkspaceState(): OwnerWorkspaceState;
  hasOwnerWorkspaceAccess(): boolean;
  onOwnerWorkspaceRequest(listener: () => void): () => void;
  approveOwnerWorkspace(): Promise<void>;
  rejectOwnerWorkspace(): Promise<void>;
  revokeOwnerWorkspace(): Promise<void>;
  approve(): Promise<void>;
  reject(): Promise<void>;
  close(): void;
}
export interface MobileSession {
  getStatus(): RemoteConnectionStatus;
  getVerificationCode(): string | null;
  /** True only when this connected desktop advertised durable device trust. */
  supportsRememberedDevices(): boolean;
  /** The peer advertised the separate full owner workspace extension. */
  supportsOwnerWorkspace(): boolean;
  subscribe(listener: (state: RemoteConnectionState) => void): () => void;
  getSnapshot(): Promise<Snapshot>;
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot | ControllerSendReceipt>;
  cancelTask(taskId: string): Promise<Snapshot>;
  resumeTask(taskId: string): Promise<Snapshot>;
  listTerminals(): Promise<TerminalSession[]>;
  readTerminal(id: string, afterSeq: number): Promise<TerminalRead>;
  getOwnerWorkspaceState(): OwnerWorkspaceState;
  hasOwnerWorkspaceAccess(): boolean;
  requestOwnerWorkspace(): Promise<void>;
  invokeOwner(command: string, args: unknown): Promise<unknown>;
  close(): void;
}
/** Collaboration visitors never negotiate controller send receipts. */
export interface CollaboratorMobileSession extends Omit<MobileSession, 'sendMessage'> {
  sendMessage(taskId: string, text: string, attachmentIds?: string[]): Promise<Snapshot>;
  /** Bounded encrypted upload, stored only against this visitor's shared task. */
  storeAttachment(taskId: string, file: AttachmentFileData & { previewDataUrl?: string | null }): Promise<Attachment>;
}

type Role = 'desktop' | 'mobile';
type RelayMessage = { type: 'joined' } | { type: 'peer' } | { type: 'peer_left' } | { type: 'hello'; nonce: string } | SecureEnvelope;
type SocketFactory = (url: string) => WebSocket;
interface PairingKeys { readonly privateKey?: CryptoKey; readonly publicKey?: Uint8Array; readonly expectedPeerPublicKey?: Uint8Array; }
// Base64 and AEAD authentication expand a permitted 1 MiB controller response.
const MAX_RELAY_FRAME_BYTES = 2 * 1024 * 1024;
const MAX_INVITATION_BYTES = 1024;
const PAIRING_TIMEOUT_MS = 5 * 60_000;
const RELAY_CONNECT_TIMEOUT_MS = 15_000;
const RELAY_HEARTBEAT_CAPABILITY = 'relay-heartbeat-v1';
const RELAY_HEARTBEAT_INTERVAL_MS = 15_000;
const RELAY_HEARTBEAT_TIMEOUT_MS = 10_000;
const RELAY_PONG = '{"type":"pong"}';
const MAX_CONCURRENT_DESKTOP_RPCS = 32;
const SEND_RECEIPT_CAPABILITY = 'send-receipt-v1';
const REMEMBERED_DEVICE_CAPABILITY = 'remembered-device-v1';
const OWNER_WORKSPACE_CAPABILITY = 'owner-workspace-v1';
const OWNER_LARGE_CHUNKS_CAPABILITY = 'owner-chunks-512k-v1';
/** Owner RPC is deliberately separate from controller v1's 1 MiB protocol. */
const MAX_OWNER_TRANSFER_BYTES = 32 * 1024 * 1024;
// A 512 KiB payload double-base64s to below the relay's 2 MiB frame ceiling.
// Keep the legacy size for peers that have not advertised large chunks.
const LEGACY_OWNER_CHUNK_BYTES = 32 * 1024;
const OWNER_CHUNK_BYTES = 512 * 1024;
const OWNER_TRANSFER_TIMEOUT_MS = 60_000;
// Whisper may take up to five minutes on the desktop, especially on first use.
// Keep the longer wait scoped to voice; transport/chunk limits stay unchanged.
const OWNER_VOICE_RESPONSE_TIMEOUT_MS = 6 * 60_000;
// The desktop accepts at most eight active owner commands. Leave room for
// commands completing on the desktop while their responses reach the phone.
const MAX_CONCURRENT_MOBILE_OWNER_RPCS = 3;
const MAX_QUEUED_MOBILE_OWNER_RPCS = 64;
const MAX_QUEUED_MOBILE_OWNER_BYTES = 32 * 1024 * 1024;
// The public relay permits 120 frames per 10 seconds. Leave headroom for
// handshake and control frames even when multiple owner RPCs overlap.
const MAX_SECURE_FRAMES_PER_WINDOW = 100;
const SECURE_FRAME_WINDOW_MS = 10_000;

interface OwnerTransfer { readonly id: string; readonly bytes: number; readonly chunks: Uint8Array[]; received: number; nextIndex: number; readonly timer: ReturnType<typeof setTimeout>; }

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
  private readonly sentSecureAt: number[] = [];
  private readonly pairingTimer: ReturnType<typeof setTimeout>;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatDeadline: ReturnType<typeof setTimeout> | null = null;
  private heartbeatEnabled = false;
  private joined = false;
  private lastError: string | undefined;
  private closed = false;
  private verification: string | null = null;
  private commitmentSent = false;
  private revealSent = false;
  private peerCommitment: Uint8Array | null = null;
  constructor(protected readonly invitationData: ControllerInvitation, protected readonly role: Role, private readonly pairingKeys: PairingKeys = {}, private readonly socketFactory: SocketFactory = url => new WebSocket(url)) {
    this.pairingTimer = setTimeout(() => { if (!this.closed) this.fail('Pairing timed out. Reconnect and try pairing again.'); }, PAIRING_TIMEOUT_MS);
    this.connect();
  }
  getStatus() { return this.status; }
  getVerificationCode() { return this.verification; }
  subscribe(listener: (state: RemoteConnectionState) => void) { this.listeners.add(listener); listener({ status: this.status, ...(this.lastError ? { error: this.lastError } : {}), ...(this.verification ? { verificationCode: this.verification } : {}) }); return () => this.listeners.delete(listener); }
  protected setStatus(status: RemoteConnectionStatus, error?: string) { if (this.closed && !['closed', 'error', 'rejected'].includes(status)) return; if (this.status === status && !error) return; this.status = status; this.lastError = error; for (const listener of this.listeners) listener({ status, ...(error ? { error } : {}), ...(this.verification ? { verificationCode: this.verification } : {}) }); }
  private clearConnectTimer() { if (this.connectTimer) clearTimeout(this.connectTimer); this.connectTimer = null; }
  private clearHeartbeat() {
    if (this.heartbeatTimer) clearTimeout(this.heartbeatTimer);
    if (this.heartbeatDeadline) clearTimeout(this.heartbeatDeadline);
    this.heartbeatTimer = null; this.heartbeatDeadline = null; this.heartbeatEnabled = false;
  }
  private pingRelay() {
    if (this.closed || !this.heartbeatEnabled) return;
    this.heartbeatTimer = null;
    this.heartbeatDeadline = setTimeout(() => {
      this.heartbeatDeadline = null;
      this.fail('The relay connection stopped responding.');
    }, RELAY_HEARTBEAT_TIMEOUT_MS);
    try { this.sendRaw({ type: 'ping' }); }
    catch { this.fail('The relay connection was interrupted.'); }
  }
  private receivePong() {
    if (this.closed || !this.heartbeatEnabled || !this.heartbeatDeadline) return;
    clearTimeout(this.heartbeatDeadline); this.heartbeatDeadline = null;
    this.heartbeatTimer = setTimeout(() => this.pingRelay(), RELAY_HEARTBEAT_INTERVAL_MS);
  }
  protected disconnect() {
    this.closed = true; this.clearConnectTimer(); this.clearHeartbeat(); clearTimeout(this.pairingTimer);
    this.channel = null; this.peerNonce = null;
    // Reject pending operations immediately, even when a blackholed socket
    // cannot complete its close handshake. Reconnection never replays them.
    this.onDisconnected();
  }
  protected connect() {
    try {
      const endpoint = new URL(this.invitationData.relayUrl);
      // Public routing metadata only; the invitation secret never leaves the endpoints.
      endpoint.searchParams.set('room', this.invitationData.room);
      endpoint.searchParams.set('role', this.role);
      this.socket = this.socketFactory(endpoint.href);
    }
    catch { this.closed = true; clearTimeout(this.pairingTimer); this.setStatus('error', 'Could not open a secure connection to the pairing relay. Check your network connection.'); return; }
    // Bound both the socket handshake and relay join. Browsers can otherwise
    // sit in CONNECTING for minutes when the resolved server is unreachable.
    this.connectTimer = setTimeout(() => {
      if (!this.closed) this.fail(`Could not connect to the pairing relay at ${new URL(this.invitationData.relayUrl).hostname}. Check your network connection and try again.`);
    }, RELAY_CONNECT_TIMEOUT_MS);
    this.socket.onopen = () => { if (!this.closed) this.sendRaw({ type: 'join', room: this.invitationData.room, role: this.role, capabilities: [RELAY_HEARTBEAT_CAPABILITY] }); };
    this.socket.onmessage = event => {
      // A slow authenticated bridge operation must not hide transport liveness.
      // This is a relay-only acknowledgement and grants no peer authority.
      if (event.data === RELAY_PONG && this.heartbeatEnabled) { this.receivePong(); return; }
      this.receiveQueue = this.receiveQueue.then(() => this.receive(event.data)).catch(() => this.fail());
    };
    this.socket.onerror = () => { if (!this.closed) this.fail('The secure connection to the pairing relay failed. Check your network connection and try again.'); };
    this.socket.onclose = () => { if (this.closed) return; this.disconnect(); if (this.status !== 'rejected' && this.status !== 'error') this.setStatus('closed'); };
  }
  protected sendRaw(value: unknown) { if (!this.closed && this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(value)); }
  private async waitForRelayBudget() {
    while (true) {
      const now = Date.now();
      while (this.sentSecureAt.length && this.sentSecureAt[0] <= now - SECURE_FRAME_WINDOW_MS) this.sentSecureAt.shift();
      if (this.sentSecureAt.length < MAX_SECURE_FRAMES_PER_WINDOW) {
        this.sentSecureAt.push(now);
        return;
      }
      await new Promise(resolve => setTimeout(resolve, Math.max(1, this.sentSecureAt[0] + SECURE_FRAME_WINDOW_MS - now + 1)));
    }
  }
  protected sendSecure(value: unknown): Promise<void> {
    const send = this.sendQueue.then(async () => {
      if (this.closed || !this.channel || this.socket?.readyState !== WebSocket.OPEN) throw new Error('Secure pairing has not completed.');
      await this.waitForRelayBudget();
      if (this.closed || !this.channel || this.socket?.readyState !== WebSocket.OPEN) throw new Error('Secure pairing has not completed.');
      const frame = await this.channel.seal(value);
      if (this.closed || this.socket?.readyState !== WebSocket.OPEN) throw new Error('Connection closed.');
      this.sendRaw(frame);
    });
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
    if (data.type === 'joined') {
      if (this.joined) return this.fail();
      this.joined = true;
      this.clearConnectTimer(); this.setStatus('waiting_for_peer');
      // Do not send new frame types to legacy relays during a rolling upgrade.
      this.heartbeatEnabled = hasCapability(data.capabilities, RELAY_HEARTBEAT_CAPABILITY);
      if (this.heartbeatEnabled) this.pingRelay();
      return;
    }
    if (data.type === 'peer') {
      if (this.invitationData.version === 1) this.sendRaw({ type: 'hello', nonce: bytesToBase64url(this.nonce) });
      else await this.sendV2Commit();
      return;
    }
    if (data.type === 'peer_left') { this.close(); return; }
    if (data.type === 'hello') { await this.receiveHello(data); return; }
    if (data.type === 'secure') {
      if (!this.channel || typeof data.seq !== 'number' || typeof data.ciphertext !== 'string') return this.fail();
      try {
        const opened = await this.channel.open(data as unknown as SecureEnvelope);
        if (!this.closed) await this.onSecure(opened);
      } catch { this.fail(); }
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
  protected fail(error?: string) { if (this.closed) return; this.disconnect(); this.setStatus('error', error); this.socket?.close(); }
  protected async onChannelReady(): Promise<void> {}
  protected async onSecure(_message: unknown): Promise<void> {}
  protected onDisconnected(): void {}
  protected completePairing() { clearTimeout(this.pairingTimer); }
  close() { if (this.closed) return; this.disconnect(); this.setStatus('closed'); this.socket?.close(); }
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
    close() { this.approved = false; this.authorizationEpoch++; this.ownerWorkspaceState = 'none'; this.clearIncomingOwner(); super.close(); }
    private peerSupportsSendReceipt = false;
    private peerSupportsLargeOwnerChunks = false;
    private ownerWorkspaceState: OwnerWorkspaceState = 'none';
    private readonly ownerWorkspaceListeners = new Set<() => void>();
    private readonly incomingOwner = new Map<string, OwnerTransfer>();
    private pendingRpcs = 0;
    private pendingOwnerRpcs = 0;
    private peer: CollaboratorIdentity | null = null;
    private readonly pendingListeners = new Set<() => void>();
    private readonly dispatcher = new ControllerDispatcher(bridge);
    readonly invitation = JSON.stringify(invitationData);
    getPeer() { return this.peer; }
    getPeerControllerPublicKey() { return this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null; }
    onPendingPeer(listener: () => void) { this.pendingListeners.add(listener); return () => this.pendingListeners.delete(listener); }
    getOwnerWorkspaceState() { return this.ownerWorkspaceState; }
    hasOwnerWorkspaceAccess() { return this.ownerWorkspaceState === 'approved'; }
    onOwnerWorkspaceRequest(listener: () => void) { this.ownerWorkspaceListeners.add(listener); return () => this.ownerWorkspaceListeners.delete(listener); }
    private setOwnerWorkspaceState(state: OwnerWorkspaceState) { this.ownerWorkspaceState = state; }
    async approveOwnerWorkspace() {
      const publicKey = !this.peer && this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null;
      if (!publicKey || this.status !== 'connected' || this.ownerWorkspaceState !== 'requested') throw new Error('No pending owner workspace request to approve.');
      await this.sendSecure({ type: 'owner-workspace-approved' });
      this.setOwnerWorkspaceState('approved');
      await options?.onOwnerWorkspaceAccess?.(publicKey);
      // Approval may race a revocation/write. Never retain a scope that no
      // longer passes the separate owner policy.
      if (options?.isTrustedOwnerWorkspace && !(await options.isTrustedOwnerWorkspace(publicKey))) await this.revokeOwnerWorkspace();
    }
    async rejectOwnerWorkspace() {
      if (this.ownerWorkspaceState !== 'requested') throw new Error('No pending owner workspace request to reject.');
      this.setOwnerWorkspaceState('rejected'); await this.sendSecure({ type: 'owner-workspace-rejected' });
    }
    async revokeOwnerWorkspace() {
      if (this.ownerWorkspaceState === 'none') return;
      this.setOwnerWorkspaceState('none');
      this.clearIncomingOwner();
      if (this.channel && this.status === 'connected') await this.sendSecure({ type: 'owner-workspace-revoked' });
    }
    async approve() {
      if (!this.channel || this.status !== 'pending') throw new Error('No pending paired device to approve.');
      try {
        const capabilities = [
          ...(this.peerSupportsSendReceipt ? [SEND_RECEIPT_CAPABILITY] : []),
          ...(options?.isTrustedController ? [REMEMBERED_DEVICE_CAPABILITY] : []),
          ...(!this.peer && options?.isTrustedOwnerWorkspace && options.onOwnerInvoke ? [OWNER_WORKSPACE_CAPABILITY] : []),
          ...(!this.peer && options?.isTrustedOwnerWorkspace && options.onOwnerInvoke ? [OWNER_LARGE_CHUNKS_CAPABILITY] : []),
        ];
        await this.sendSecure({ type: 'approved', ...(capabilities.length ? { capabilities } : {}) });
        this.approved = true;
        if (!this.peer && this.peerPublicKey) await options?.onControllerAccess?.(bytesToBase64url(this.peerPublicKey));
        this.completePairing(); this.setStatus('connected');
      }
      catch (reason) { this.approved = false; this.close(); throw reason; }
    }
    async reject() {
      this.approved = false; this.authorizationEpoch++; this.ownerWorkspaceState = 'none'; this.clearIncomingOwner();
      if (!this.channel) throw new Error('No pending paired device to reject.');
      await this.sendSecure({ type: 'rejected' });
      // Let the peer receive and acknowledge the terminal state by closing its
      // socket. Closing here races the relay's peer_left frame ahead of the
      // encrypted rejection on some WebSocket implementations.
      this.setStatus('rejected');
    }
    protected async onChannelReady() { /* A valid encrypted pair-request proves invitation-secret possession. */ }
    protected onDisconnected() { this.ownerWorkspaceState = 'none'; this.clearIncomingOwner(); }
    protected async onSecure(message: unknown) {
      const value = asObject(message); if (!value) throw new Error('Invalid paired message.');
      if (value.type === 'pair-request' && !this.approved) {
        if (Object.hasOwn(value, 'operator')) {
          this.peer = collaborator(value.operator);
          if (!this.peer) throw new Error('Invalid collaborator identity.');
        }
        this.peerSupportsSendReceipt = hasSendReceiptCapability(value.capabilities);
        this.peerSupportsLargeOwnerChunks = !this.peer && hasCapability(value.capabilities, OWNER_LARGE_CHUNKS_CAPABILITY);
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
      if (value.type === 'owner-workspace-request') {
        if (this.peer || !options?.isTrustedOwnerWorkspace || !options.onOwnerInvoke || !this.approved || this.status !== 'connected' || this.ownerWorkspaceState === 'approved') throw new Error('Owner workspace request is unavailable.');
        this.setOwnerWorkspaceState('requested');
        for (const listener of this.ownerWorkspaceListeners) listener();
        const publicKey = this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null;
        // Only an explicit, separately stored owner scope can resume. A normal
        // remembered controller always remains limited until the desktop grants it.
        if (publicKey && options?.isTrustedOwnerWorkspace && await options.isTrustedOwnerWorkspace(publicKey)
          && this.ownerWorkspaceState === 'requested' && this.status === 'connected') await this.approveOwnerWorkspace();
        return;
      }
      if (value.type === 'owner-rpc-start') { this.beginOwnerRpc(value); return; }
      if (value.type === 'owner-rpc-chunk') { this.appendOwnerRpc(value); return; }
      if (value.type === 'owner-rpc-end') { this.finishOwnerRpc(value); return; }
      if (value.type !== 'rpc' || !this.approved || this.status !== 'connected' || typeof value.json !== 'string') throw new Error('Unapproved controller request.');
      // Keep authenticated opening and sealing ordered, but do not let a slow
      // bridge read (notably a Snapshot) prevent a later cancel, send, or close
      // frame from being decrypted and acted upon.
      this.startRpc(value.json);
    }
    private ownerAvailable() { return this.approved && this.status === 'connected' && this.ownerWorkspaceState === 'approved'; }
    private beginOwnerRpc(value: Record<string, unknown>) {
      const id = ownerTransferId(value.id), bytes = ownerTransferBytes(value.bytes);
      if (!id || bytes === null || !this.ownerAvailable() || this.incomingOwner.has(id) || this.incomingOwner.size >= 8 || ownerReservedBytes(this.incomingOwner) + bytes > MAX_OWNER_TRANSFER_BYTES) throw new Error('Owner workspace request is unavailable.');
      const timer = setTimeout(() => this.failIncomingOwner(id, 'Owner request transfer timed out.'), OWNER_TRANSFER_TIMEOUT_MS);
      this.incomingOwner.set(id, { id, bytes, chunks: [], received: 0, nextIndex: 0, timer });
    }
    private appendOwnerRpc(value: Record<string, unknown>) {
      const transfer = typeof value.id === 'string' ? this.incomingOwner.get(value.id) : undefined;
      if (!transfer || !this.ownerAvailable()) throw new Error('Owner workspace request is unavailable.');
      const chunkBytes = this.peerSupportsLargeOwnerChunks ? OWNER_CHUNK_BYTES : LEGACY_OWNER_CHUNK_BYTES;
      const chunk = decodeOwnerChunk(value, transfer.nextIndex, chunkBytes);
      if (!chunk || transfer.nextIndex >= Math.ceil(MAX_OWNER_TRANSFER_BYTES / chunkBytes) || transfer.received + chunk.byteLength > transfer.bytes) return this.failIncomingOwner(transfer.id, 'Owner request chunks are invalid or out of order.');
      transfer.chunks.push(chunk); transfer.received += chunk.byteLength; transfer.nextIndex += 1;
    }
    private finishOwnerRpc(value: Record<string, unknown>) {
      const transfer = typeof value.id === 'string' ? this.incomingOwner.get(value.id) : undefined;
      if (!transfer || transfer.received !== transfer.bytes) { if (typeof value.id === 'string') this.failIncomingOwner(value.id, 'Owner request transfer is incomplete.'); return; }
      this.incomingOwner.delete(transfer.id); clearTimeout(transfer.timer);
      this.startOwnerRpc(transfer.id, new TextDecoder().decode(joinOwnerChunks(transfer)));
    }
    private failIncomingOwner(id: string, error: string) {
      const transfer = this.incomingOwner.get(id); if (transfer) { clearTimeout(transfer.timer); this.incomingOwner.delete(id); }
      if (this.ownerAvailable()) void this.sendOwnerError(id, error).catch(() => void this.revokeOwnerWorkspace());
    }
    private clearIncomingOwner() { for (const transfer of this.incomingOwner.values()) clearTimeout(transfer.timer); this.incomingOwner.clear(); }
    private async sendOwnerError(id: string, error: string) { await this.sendSecure({ type: 'owner-response-error', id, error: error.slice(0, 512) }); }
    private async sendOwnerResponse(id: string, response: string, stillApproved: () => boolean) {
      const bytes = new TextEncoder().encode(response);
      if (bytes.byteLength > MAX_OWNER_TRANSFER_BYTES) return this.sendOwnerError(id, 'Owner response exceeds the 32 MiB relay transfer limit.');
      if (!stillApproved()) return;
      const chunkBytes = this.peerSupportsLargeOwnerChunks ? OWNER_CHUNK_BYTES : LEGACY_OWNER_CHUNK_BYTES;
      await this.sendSecure({ type: 'owner-response-start', id, bytes: bytes.byteLength });
      for (let index = 0, offset = 0; offset < bytes.byteLength; index += 1, offset += chunkBytes) {
        if (!stillApproved()) return;
        await this.sendSecure({ type: 'owner-response-chunk', id, index, data: encodeBase64(bytes.subarray(offset, Math.min(offset + chunkBytes, bytes.byteLength))) });
      }
      if (stillApproved()) await this.sendSecure({ type: 'owner-response-end', id });
    }
    private startOwnerRpc(expectedId: string, json: string) {
      if (!this.ownerAvailable()) return;
      if (this.pendingOwnerRpcs >= 8) { void this.sendOwnerError(expectedId, 'Too many owner workspace requests are pending.'); return; }
      this.pendingOwnerRpcs += 1;
      const authorizationEpoch = this.authorizationEpoch;
      void (async () => {
        const publicKey = this.peerPublicKey ? bytesToBase64url(this.peerPublicKey) : null;
        if (!publicKey || !options?.onOwnerInvoke || options.isTrustedOwnerWorkspace && !(await options.isTrustedOwnerWorkspace(publicKey))) { await this.revokeOwnerWorkspace(); return; }
        let request: { id: string; command: string; args: unknown };
        try {
          const parsed = JSON.parse(json) as Record<string, unknown>;
          if (!parsed || parsed.id !== expectedId || typeof parsed.command !== 'string' || !/^[a-z][a-z0-9_.-]{0,127}$/i.test(parsed.command) || !Object.hasOwn(parsed, 'args')) throw new Error();
          request = { id: parsed.id, command: parsed.command, args: parsed.args };
        } catch { throw new Error('Invalid owner workspace request.'); }
        await options.onOwnerWorkspaceAccess?.(publicKey);
        if (authorizationEpoch !== this.authorizationEpoch || this.ownerWorkspaceState !== 'approved' || options.isTrustedOwnerWorkspace && !(await options.isTrustedOwnerWorkspace(publicKey))) { await this.revokeOwnerWorkspace(); return; }
        let response: string;
        try { response = JSON.stringify({ id: request.id, ok: true, result: await options.onOwnerInvoke(request.command, request.args, { publicKey }) }); }
        catch (reason) { response = JSON.stringify({ id: request.id, ok: false, error: reason instanceof Error ? reason.message.slice(0, 512) : 'Owner command failed.' }); }
        const stillApproved = () => authorizationEpoch === this.authorizationEpoch && this.ownerWorkspaceState === 'approved' && this.status === 'connected';
        if (stillApproved() && options.isTrustedOwnerWorkspace && !(await options.isTrustedOwnerWorkspace(publicKey))) { await this.revokeOwnerWorkspace(); return; }
        if (stillApproved()) await this.sendOwnerResponse(request.id, response, stillApproved);
      })().catch(() => { if (this.status === 'connected') void this.revokeOwnerWorkspace(); })
        .finally(() => { this.pendingOwnerRpcs -= 1; });
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
        const response = await this.dispatcher.dispatchJson(json, { authenticated: true, subject: `paired-room:${invitationData.room}` }, { allowSendReceipt: this.peerSupportsSendReceipt, allowUploads: !!this.peer });
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
    private readonly pendingOwner = new Map<string, { resolve: (value: unknown) => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout> }>();
    private readonly queuedOwner: Array<{ id: string; command: string; bytes: Uint8Array; resolve: (value: unknown) => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout> }> = [];
    private queuedOwnerBytes = 0;
    private activeOwnerRpcs = 0;
    private readonly incomingOwnerResponses = new Map<string, OwnerTransfer>();
    private rememberedDevices = false;
    private ownerWorkspaceSupported = false;
    private ownerLargeChunksSupported = false;
    private ownerWorkspaceState: OwnerWorkspaceState = 'none';
    private ownerRequest: { resolve: () => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout> } | null = null;
    supportsRememberedDevices() { return this.rememberedDevices; }
    supportsOwnerWorkspace() { return this.ownerWorkspaceSupported; }
    getOwnerWorkspaceState() { return this.ownerWorkspaceState; }
    hasOwnerWorkspaceAccess() { return this.ownerWorkspaceState === 'approved'; }
    protected async onChannelReady() { this.setStatus('awaiting_approval'); await this.sendSecure(operator ? { type: 'pair-request', operator } : { type: 'pair-request', capabilities: [SEND_RECEIPT_CAPABILITY, OWNER_LARGE_CHUNKS_CAPABILITY] }); }
    protected async onSecure(message: unknown) {
      const value = asObject(message); if (!value || typeof value.type !== 'string') throw new Error('Invalid paired response.');
      if (value.type === 'pair-request') return;
      if (value.type === 'approved') { this.rememberedDevices = invitationData.version === 2 && hasCapability(value.capabilities, REMEMBERED_DEVICE_CAPABILITY); this.ownerWorkspaceSupported = invitationData.version === 2 && hasCapability(value.capabilities, OWNER_WORKSPACE_CAPABILITY); this.ownerLargeChunksSupported = this.ownerWorkspaceSupported && hasCapability(value.capabilities, OWNER_LARGE_CHUNKS_CAPABILITY); this.completePairing(); this.setStatus('connected'); return; }
      if (value.type === 'rejected') { this.disconnect(); this.setStatus('rejected'); this.socket?.close(); return; }
      if (value.type === 'owner-workspace-approved') { this.ownerWorkspaceState = 'approved'; this.settleOwnerRequest(); return; }
      if (value.type === 'owner-workspace-rejected') { this.ownerWorkspaceState = 'rejected'; this.settleOwnerRequest(new Error('Desktop declined owner workspace access.')); return; }
      if (value.type === 'owner-workspace-revoked') { this.ownerWorkspaceState = 'none'; this.rejectOwnerRequests(new Error('Owner workspace access was revoked.')); return; }
      if (value.type === 'owner-response-error') { this.failOwnerResponse(value.id, typeof value.error === 'string' ? value.error : 'Owner transfer failed.'); return; }
      if (value.type === 'owner-response-start') { this.beginOwnerResponse(value); return; }
      if (value.type === 'owner-response-chunk') { this.appendOwnerResponse(value); return; }
      if (value.type === 'owner-response-end') { this.finishOwnerResponse(value); return; }
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
    async requestOwnerWorkspace(): Promise<void> {
      if (this.status !== 'connected') throw new Error('Desktop approval and a live encrypted connection are required.');
      if (!this.ownerWorkspaceSupported) throw new Error('Update Monitter on the desktop to use the owner workspace through the relay.');
      if (this.ownerWorkspaceState === 'approved') return Promise.resolve();
      if (this.ownerRequest) return new Promise((resolve, reject) => { const previous = this.ownerRequest!; const originalResolve = previous.resolve, originalReject = previous.reject; previous.resolve = () => { originalResolve(); resolve(); }; previous.reject = reason => { originalReject(reason); reject(reason); }; });
      this.ownerWorkspaceState = 'requested';
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => this.settleOwnerRequest(new Error('Owner workspace approval timed out.')), 5 * 60_000);
        this.ownerRequest = { resolve, reject, timer };
        this.sendSecure({ type: 'owner-workspace-request' }).catch(reason => { this.ownerWorkspaceState = 'none'; this.settleOwnerRequest(reason instanceof Error ? reason : new Error('Connection failed.')); });
      });
    }
    invokeOwner(command: string, args: unknown): Promise<unknown> {
      if (this.ownerWorkspaceState !== 'approved') return Promise.reject(new Error('Explicit desktop owner workspace approval is required.'));
      if (typeof command !== 'string' || !/^[a-z][a-z0-9_.-]{0,127}$/i.test(command)) return Promise.reject(new Error('Owner command is invalid.'));
      const id = uuid(); let json: string; try { json = JSON.stringify({ id, command, args }); } catch { return Promise.reject(new Error('Owner request must be JSON serializable.')); }
      const bytes = new TextEncoder().encode(json);
      if (bytes.byteLength > MAX_OWNER_TRANSFER_BYTES) return Promise.reject(new Error('Owner request exceeds the 32 MiB relay transfer limit.'));
      return new Promise<unknown>((resolve, reject) => {
        if (this.queuedOwner.length >= MAX_QUEUED_MOBILE_OWNER_RPCS || this.queuedOwnerBytes + bytes.byteLength > MAX_QUEUED_MOBILE_OWNER_BYTES) {
          reject(new Error('Owner workspace is busy; try again shortly.'));
          return;
        }
        const timer = setTimeout(() => {
          const index = this.queuedOwner.findIndex(request => request.id === id);
          if (index < 0) return;
          this.queuedOwner.splice(index, 1);
          this.queuedOwnerBytes -= bytes.byteLength;
          reject(new Error('Owner workspace is busy; try again shortly.'));
        }, OWNER_TRANSFER_TIMEOUT_MS);
        this.queuedOwner.push({ id, command, bytes, resolve, reject, timer });
        this.queuedOwnerBytes += bytes.byteLength;
        this.drainOwnerQueue();
      });
    }
    private drainOwnerQueue() {
      while (this.ownerWorkspaceState === 'approved' && this.status === 'connected'
        && this.activeOwnerRpcs < MAX_CONCURRENT_MOBILE_OWNER_RPCS && this.queuedOwner.length) {
        const request = this.queuedOwner.shift()!;
        clearTimeout(request.timer);
        this.queuedOwnerBytes -= request.bytes.byteLength;
        this.activeOwnerRpcs += 1;
        const complete = () => { this.activeOwnerRpcs -= 1; this.drainOwnerQueue(); };
        const timer = setTimeout(() => {
          const pending = this.pendingOwner.get(request.id);
          if (!pending) return;
          this.pendingOwner.delete(request.id);
          pending.reject(new Error('Owner command timed out.'));
        }, request.command === 'transcribe_voice_message' ? OWNER_VOICE_RESPONSE_TIMEOUT_MS : OWNER_TRANSFER_TIMEOUT_MS);
        this.pendingOwner.set(request.id, {
          resolve: value => { complete(); request.resolve(value); },
          reject: reason => { complete(); request.reject(reason); },
          timer,
        });
        this.sendOwnerRequest(request.id, request.bytes).catch(reason => {
          const pending = this.pendingOwner.get(request.id);
          if (!pending) return;
          clearTimeout(pending.timer);
          this.pendingOwner.delete(request.id);
          pending.reject(reason instanceof Error ? reason : new Error('Connection failed.'));
        });
      }
    }
    private async sendOwnerRequest(id: string, bytes: Uint8Array) {
      const chunkBytes = this.ownerLargeChunksSupported ? OWNER_CHUNK_BYTES : LEGACY_OWNER_CHUNK_BYTES;
      await this.sendSecure({ type: 'owner-rpc-start', id, bytes: bytes.byteLength });
      for (let index = 0, offset = 0; offset < bytes.byteLength; index += 1, offset += chunkBytes) await this.sendSecure({ type: 'owner-rpc-chunk', id, index, data: encodeBase64(bytes.subarray(offset, Math.min(offset + chunkBytes, bytes.byteLength))) });
      await this.sendSecure({ type: 'owner-rpc-end', id });
    }
    private beginOwnerResponse(value: Record<string, unknown>) {
      const id = ownerTransferId(value.id), bytes = ownerTransferBytes(value.bytes);
      if (!id || bytes === null || !this.pendingOwner.has(id) || this.incomingOwnerResponses.has(id) || ownerReservedBytes(this.incomingOwnerResponses) + bytes > MAX_OWNER_TRANSFER_BYTES) throw new Error('Owner response is invalid.');
      const timer = setTimeout(() => this.failOwnerResponse(id, 'Owner response transfer timed out.'), OWNER_TRANSFER_TIMEOUT_MS);
      this.incomingOwnerResponses.set(id, { id, bytes, chunks: [], received: 0, nextIndex: 0, timer });
    }
    private appendOwnerResponse(value: Record<string, unknown>) {
      const transfer = typeof value.id === 'string' ? this.incomingOwnerResponses.get(value.id) : undefined;
      if (!transfer) throw new Error('Owner response is unavailable.');
      const chunkBytes = this.ownerLargeChunksSupported ? OWNER_CHUNK_BYTES : LEGACY_OWNER_CHUNK_BYTES;
      const chunk = decodeOwnerChunk(value, transfer.nextIndex, chunkBytes);
      if (!chunk || transfer.nextIndex >= Math.ceil(MAX_OWNER_TRANSFER_BYTES / chunkBytes) || transfer.received + chunk.byteLength > transfer.bytes) return this.failOwnerResponse(transfer.id, 'Owner response chunks are invalid or out of order.');
      transfer.chunks.push(chunk); transfer.received += chunk.byteLength; transfer.nextIndex += 1;
    }
    private finishOwnerResponse(value: Record<string, unknown>) {
      const transfer = typeof value.id === 'string' ? this.incomingOwnerResponses.get(value.id) : undefined;
      if (!transfer || transfer.received !== transfer.bytes) { if (typeof value.id === 'string') this.failOwnerResponse(value.id, 'Owner response transfer is incomplete.'); return; }
      this.incomingOwnerResponses.delete(transfer.id); clearTimeout(transfer.timer);
      let response: Record<string, unknown>; try { response = JSON.parse(new TextDecoder().decode(joinOwnerChunks(transfer))) as Record<string, unknown>; } catch { this.failOwnerResponse(transfer.id, 'Owner response could not be decoded.'); return; }
      const callback = response.id === transfer.id ? this.pendingOwner.get(transfer.id) : undefined;
      if (!callback) return;
      this.pendingOwner.delete(transfer.id); clearTimeout(callback.timer);
      response.ok === true ? callback.resolve(response.result) : callback.reject(new Error(typeof response.error === 'string' ? response.error : 'Owner command failed.'));
    }
    private failOwnerResponse(id: unknown, error: string) {
      if (typeof id !== 'string') return;
      const transfer = this.incomingOwnerResponses.get(id); if (transfer) { clearTimeout(transfer.timer); this.incomingOwnerResponses.delete(id); }
      const callback = this.pendingOwner.get(id); if (callback) { clearTimeout(callback.timer); this.pendingOwner.delete(id); callback.reject(new Error(error)); }
    }
    async getSnapshot() { return this.call('getSnapshot', {}) as Promise<Snapshot>; }
    async sendMessage(taskId: string, text: string, attachmentIds?: string[]) { return this.call('sendMessage', { taskId, text, ...(attachmentIds?.length ? { attachmentIds } : {}) }) as Promise<Snapshot | ControllerSendReceipt>; }
    async storeAttachment(taskId: string, file: AttachmentFileData & { previewDataUrl?: string | null }): Promise<Attachment> {
      if (typeof file?.filename !== 'string' || typeof file.mimeType !== 'string' || typeof file.dataBase64 !== 'string') throw new Error('Attachment data is invalid.');
      const bytes = decodeBase64(file.dataBase64);
      if (bytes.byteLength === 0 || bytes.byteLength > CONTROLLER_MAX_UPLOAD_BYTES) throw new Error(`Attachments must be between 1 byte and ${CONTROLLER_MAX_UPLOAD_BYTES} bytes.`);
      if (file.previewDataUrl != null && !validPreview(file.previewDataUrl)) throw new Error('Attachment preview must be a small PNG, JPEG, or WebP image.');
      const started = await this.call('beginAttachmentUpload', { taskId, filename: file.filename, mimeType: file.mimeType, size: bytes.byteLength, ...(file.previewDataUrl ? { previewDataUrl: file.previewDataUrl } : {}) });
      if (!isUploadReceipt(started)) throw new Error('Attachment upload could not start.');
      for (let offset = 0; offset < bytes.byteLength; offset += CONTROLLER_UPLOAD_CHUNK_BYTES) {
        const chunk = encodeBase64(bytes.subarray(offset, Math.min(offset + CONTROLLER_UPLOAD_CHUNK_BYTES, bytes.byteLength)));
        await this.call('appendAttachmentUpload', { uploadId: started.uploadId, dataBase64: chunk });
      }
      return this.call('finishAttachmentUpload', { uploadId: started.uploadId }) as Promise<Attachment>;
    }
    async cancelTask(taskId: string) { return this.call('cancelTask', { taskId }) as Promise<Snapshot>; }
    async resumeTask(taskId: string) { return this.call('resumeTask', { taskId }) as Promise<Snapshot>; }
    async listTerminals() { return this.call('listTerminals', {}) as Promise<TerminalSession[]>; }
    async readTerminal(id: string, afterSeq: number) { return this.call('readTerminal', { id, afterSeq }) as Promise<TerminalRead>; }
    private settleOwnerRequest(reason?: Error) { const request = this.ownerRequest; if (!request) return; this.ownerRequest = null; clearTimeout(request.timer); reason ? request.reject(reason) : request.resolve(); }
    private rejectOwnerRequests(reason: Error) {
      for (const transfer of this.incomingOwnerResponses.values()) clearTimeout(transfer.timer);
      this.incomingOwnerResponses.clear();
      for (const callback of this.pendingOwner.values()) { clearTimeout(callback.timer); callback.reject(reason); }
      this.pendingOwner.clear();
      for (const request of this.queuedOwner.splice(0)) { clearTimeout(request.timer); request.reject(reason); }
      this.queuedOwnerBytes = 0;
    }
    protected onDisconnected() {
      // Owner-workspace callers already special-case this exact error type to
      // stay quiet through transient drops (see isOwnerRelayDisconnectedError
      // call sites); a generic Error here defeats that classification.
      this.ownerWorkspaceState = 'none'; this.settleOwnerRequest(new OwnerRelayDisconnectedError());
      for (const callback of this.pending.values()) { clearTimeout(callback.timer); callback.reject(new Error('Connection closed.')); }
      this.pending.clear(); this.rejectOwnerRequests(new OwnerRelayDisconnectedError());
    }
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

function isUploadReceipt(value: ControllerResult): value is { uploadId: string } {
  return !!value && typeof value === 'object' && 'uploadId' in value && typeof value.uploadId === 'string';
}

function decodeBase64(value: string): Uint8Array {
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(value) || value.length % 4 === 1) throw new Error('Attachment data is not valid Base64.');
  let binary: string; try { binary = atob(value); } catch { throw new Error('Attachment data is not valid Base64.'); }
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let index = 0; index < bytes.byteLength; index += 1) binary += String.fromCharCode(bytes[index]);
  return btoa(binary);
}

function ownerTransferId(value: unknown): string | null {
  return typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value) ? value : null;
}
function ownerTransferBytes(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= MAX_OWNER_TRANSFER_BYTES ? value : null;
}
/** Strictly ordered chunk parser; malformed, duplicate, and skipped chunks fail closed. */
export function decodeOwnerChunk(value: Record<string, unknown>, expectedIndex: number, maxBytes = LEGACY_OWNER_CHUNK_BYTES): Uint8Array | null {
  if (value.index !== expectedIndex || typeof value.data !== 'string' || value.data.length > Math.ceil(maxBytes / 3) * 4) return null;
  try {
    const bytes = decodeBase64(value.data);
    return bytes.byteLength <= maxBytes && encodeBase64(bytes).replace(/=+$/, '') === value.data.replace(/=+$/, '') ? bytes : null;
  } catch { return null; }
}
function joinOwnerChunks(transfer: OwnerTransfer): Uint8Array {
  const bytes = new Uint8Array(transfer.bytes); let offset = 0;
  for (const chunk of transfer.chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return bytes;
}
function ownerReservedBytes(transfers: ReadonlyMap<string, OwnerTransfer>): number {
  let total = 0; for (const transfer of transfers.values()) total += transfer.bytes; return total;
}

function validPreview(value: string): boolean {
  return value.length <= CONTROLLER_MAX_UPLOAD_PREVIEW_CHARS && /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/]+={0,2}$/i.test(value);
}

/**
 * Sessions are intentionally memory-only. Disconnects close the current session:
 * mutations are never automatically resent. Re-pairing is the supported first-pass
 * recovery path until durable receipt and device identity storage exist.
 */
