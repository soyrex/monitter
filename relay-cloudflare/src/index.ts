export interface Env {
  ROOM: DurableObjectNamespace<RelayRoom>;
  PAIRINGS: DurableObjectNamespace<PairingRegistry>;
}

type Role = 'desktop' | 'mobile';
type Attachment = {
  room: string;
  role: Role;
  joined: boolean;
  heartbeatNegotiated: boolean;
  // A departed socket may remain visible while its close handshake finishes.
  // This flag makes that later close/error a no-op for any replacement peer.
  departed: boolean;
  openedAt: number;
  pendingSinceAt: number;
  rateWindowStartedAt: number;
  messagesInWindow: number;
};

const ROOM_PATTERN = /^[A-Za-z0-9_-]{22,128}$/;
const MAX_FRAME_BYTES = 2 * 1024 * 1024;
const MAX_MESSAGES_PER_WINDOW = 120;
const RATE_WINDOW_MS = 10_000;
const JOIN_TIMEOUT_MS = 10_000;
const HEARTBEAT_CAPABILITY = 'relay-heartbeat-v1';
const HEARTBEAT_PING = '{"type":"ping"}';
const HEARTBEAT_PONG = '{"type":"pong"}';
const HEARTBEAT_LEASE_MS = 45_000;
// The DO only owns the rendezvous deadline. The encrypted protocol owns approval.
const PAIRING_TTL_MS = 5 * 60_000;
const PAIRING_REQUEST_BYTES = 4 * 1024;
const PAIRING_LIMIT = 1_000;
const PAIRING_POST_LIMIT = 5;
const PAIRING_LOOKUP_LIMIT = 10;
const PAIRING_GLOBAL_POST_LIMIT = 100;
const PAIRING_GLOBAL_LOOKUP_LIMIT = 200;
const PAIRING_RATE_KEY_LIMIT = 1_024;
const RATE_WINDOW_PAIRING_MS = 60_000;
const PAIRING_CODE = /^\d{9}$/;
const BASE64URL = /^[A-Za-z0-9_-]+$/;
const TEXT = new TextEncoder();

function validRoom(value: string | null): value is string {
  return value !== null && ROOM_PATTERN.test(value);
}

function bytesToBase64url(bytes: Uint8Array): string {
  let binary = '';
  for (const value of bytes) binary += String.fromCharCode(value);
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

function randomDigits(): string {
  const value = crypto.getRandomValues(new Uint32Array(1))[0] % 1_000_000_000;
  return String(value).padStart(9, '0');
}

async function rateKey(request: Request): Promise<string> {
  const source = request.headers.get('CF-Connecting-IP') ?? 'missing';
  const digest = await crypto.subtle.digest('SHA-256', TEXT.encode(source));
  return bytesToBase64url(new Uint8Array(digest));
}

function allowedOrigin(value: string | null): string | null {
  if (!value) return null;
  if (value === 'https://share.monitter.com') return value;
  if (value === 'https://appassets.androidplatform.net' || value === 'tauri://localhost' || value === 'http://tauri.localhost' || value === 'https://tauri.localhost') return value;
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol) || url.pathname !== '/' || url.search || url.hash) return null;
    return ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname) ? value : null;
  } catch { return null; }
}

function cors(origin: string | null): Headers {
  const headers = new Headers({ Vary: 'Origin', 'Cache-Control': 'no-store' });
  if (origin) {
    headers.set('Access-Control-Allow-Origin', origin);
    headers.set('Access-Control-Allow-Methods', 'POST, PUT, GET, DELETE, OPTIONS');
    headers.set('Access-Control-Allow-Headers', 'Content-Type, Authorization');
    headers.set('Access-Control-Max-Age', '600');
  }
  return headers;
}

async function readBoundedText(request: Request, limit: number): Promise<string | null> {
  const length = request.headers.get('Content-Length');
  if (length && (!/^\d+$/.test(length) || Number(length) > limit)) return null;
  if (!request.body) return '';
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const item = await reader.read();
      if (item.done) break;
      size += item.value.byteLength;
      if (size > limit) { await reader.cancel(); return null; }
      chunks.push(item.value);
    }
  } finally { reader.releaseLock(); }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return new TextDecoder().decode(bytes);
}

function response(status: number, body?: BodyInit | null, origin?: string | null): Response {
  return new Response(body, { status, headers: cors(origin ?? null) });
}

function validPublicKey(value: unknown): value is string {
  // A raw uncompressed P-256 point is 65 bytes and base64url-encodes to 87
  // characters. Its first byte is 0x04, whose first base64url character is B.
  return typeof value === 'string' && value.length === 87 && value.startsWith('B') && BASE64URL.test(value);
}

type Invitation = { version: 2; relayUrl: string; room: string; publicKey: string; directUrls?: string[] };

function relayUrlForRequest(request: Request): string | null {
  const requestUrl = new URL(request.url);
  const host = request.headers.get('Host') ?? requestUrl.host;
  const hostname = host.replace(/^\[/, '').replace(/\].*$/, '').split(':')[0];
  const local = ['localhost', '127.0.0.1', '::1'].includes(hostname);
  const scheme = requestUrl.protocol === 'https:' ? 'wss:' : local && requestUrl.protocol === 'http:' ? 'ws:' : '';
  return scheme ? `${scheme}//${host}/relay` : null;
}

function validDirectCandidates(value: unknown): boolean {
  if (value === undefined) return true;
  return Array.isArray(value) && value.length <= 3 && value.every(candidate => {
    if (typeof candidate !== 'string' || candidate.length > 160) return false;
    try {
      const url = new URL(candidate);
      const octets = url.hostname.split('.').map(Number);
      return url.protocol === 'ws:' && url.pathname === '/pairing/direct' && !!url.port
        && !url.username && !url.password && !url.search && !url.hash
        && octets.length === 4 && octets.every(n => Number.isInteger(n) && n >= 0 && n <= 255)
        && (octets[0] === 10 || octets[0] === 192 && octets[1] === 168 || octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31);
    } catch { return false; }
  });
}

function validInvitation(value: unknown, expectedRelayUrl: string | null): value is Invitation {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const invitation = value as Record<string, unknown>;
  const keys = Object.keys(invitation).sort();
  if (!['publicKey,relayUrl,room,version', 'directUrls,publicKey,relayUrl,room,version'].includes(keys.join(',')) || !validDirectCandidates(invitation.directUrls) || invitation.version !== 2 || !validRoom(typeof invitation.room === 'string' ? invitation.room : null) || !validPublicKey(invitation.publicKey)) return false;
  return expectedRelayUrl !== null && invitation.relayUrl === expectedRelayUrl;
}

function validLocalInvitation(value: unknown): value is Invitation {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const invitation = value as Record<string, unknown>;
  const keys = Object.keys(invitation).sort();
  if (!['publicKey,relayUrl,room,version', 'directUrls,publicKey,relayUrl,room,version'].includes(keys.join(',')) || !validDirectCandidates(invitation.directUrls) || invitation.version !== 2 || !validRoom(typeof invitation.room === 'string' ? invitation.room : null) || !validPublicKey(invitation.publicKey) || typeof invitation.relayUrl !== 'string') return false;
  try {
    const relay = new URL(invitation.relayUrl);
    return (relay.protocol === 'ws:' || relay.protocol === 'wss:') && relay.pathname === '/relay' && !relay.search && !relay.hash && ['localhost', '127.0.0.1', '[::1]'].includes(relay.hostname);
  } catch { return false; }
}

function close(socket: WebSocket, code: number, reason: string): void {
  try { socket.close(code, reason); } catch { /* The peer may already be gone. */ }
}

function attachment(socket: WebSocket): Attachment | null {
  const value = socket.deserializeAttachment();
  if (!value || typeof value !== 'object') return null;
  const state = value as Partial<Attachment>;
  if (!validRoom(typeof state.room === 'string' ? state.room : null) ||
    (state.role !== 'desktop' && state.role !== 'mobile') ||
    typeof state.joined !== 'boolean' || typeof state.openedAt !== 'number' ||
    typeof state.rateWindowStartedAt !== 'number' || typeof state.messagesInWindow !== 'number') return null;
  // Attachments from the pre-heartbeat worker remain valid legacy sockets
  // across a rolling deployment. Defaults are persisted on their next update.
  return {
    ...state,
    heartbeatNegotiated: state.heartbeatNegotiated === true,
    departed: state.departed === true,
    pendingSinceAt: typeof state.pendingSinceAt === 'number' ? state.pendingSinceAt : state.openedAt,
  } as Attachment;
}

function send(socket: WebSocket, value: object): void {
  try { socket.send(JSON.stringify(value)); } catch { close(socket, 1011, 'send failed'); }
}

export function openSockets(sockets: WebSocket[]): WebSocket[] {
  return sockets.filter(socket => socket.readyState === WebSocket.OPEN);
}

export class RelayRoom extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // Exact JSON avoids waking a hibernating Durable Object for a negotiated
    // client heartbeat. The timestamp is used below as the lease proof.
    this.ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair(HEARTBEAT_PING, HEARTBEAT_PONG));
  }

  async fetch(request: Request): Promise<Response> {
    if (request.method !== 'GET' || request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') {
      return new Response('WebSocket upgrade required.', { status: 426 });
    }
    const url = new URL(request.url);
    const room = url.searchParams.get('room');
    const role = url.searchParams.get('role');
    if (!validRoom(room) || (role !== 'desktop' && role !== 'mobile')) {
      return new Response('Invalid relay route.', { status: 400 });
    }

    await this.pruneExpiredSockets();
    const sockets = this.activeSockets();
    if (sockets.length >= 2) return new Response('Room unavailable.', { status: 409 });
    // A role is reserved as soon as its WebSocket is upgraded. This prevents a
    // concurrent duplicate from racing the later join message.
    if (sockets.some(socket => attachment(socket)?.role === role)) return new Response('Room unavailable.', { status: 409 });

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    const now = Date.now();
    server.serializeAttachment({ room, role, joined: false, heartbeatNegotiated: false, departed: false, openedAt: now, pendingSinceAt: now, rateWindowStartedAt: now, messagesInWindow: 0 } satisfies Attachment);
    this.ctx.acceptWebSocket(server);
    await this.scheduleDeadline();
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const state = attachment(socket);
    if (!state) return close(socket, 1008, 'invalid connection');
    if (state.departed) return close(socket, 1008, 'connection departed');
    if (typeof message !== 'string' || new TextEncoder().encode(message).byteLength > MAX_FRAME_BYTES) {
      return close(socket, 1009, 'frame too large');
    }
    const now = Date.now();
    if (now - state.rateWindowStartedAt >= RATE_WINDOW_MS) {
      state.rateWindowStartedAt = now;
      state.messagesInWindow = 0;
    }
    if (++state.messagesInWindow > MAX_MESSAGES_PER_WINDOW) return close(socket, 1008, 'rate limit');
    socket.serializeAttachment(state);

    let value: unknown;
    try { value = JSON.parse(message); } catch { return close(socket, 1008, 'invalid JSON'); }
    if (!value || typeof value !== 'object' || Array.isArray(value)) return close(socket, 1008, 'invalid frame');
    const frame = value as Record<string, unknown>;
    if (!state.joined) {
      if (frame.type !== 'join' || frame.room !== state.room || frame.role !== state.role) {
        return close(socket, 1008, 'join required');
      }
      const capabilities = frame.capabilities;
      if (capabilities !== undefined && (!Array.isArray(capabilities) || capabilities.length > 8 || capabilities.some(value => typeof value !== 'string' || value.length > 128))) {
        return close(socket, 1008, 'invalid join capabilities');
      }
      state.joined = true;
      state.heartbeatNegotiated = Array.isArray(capabilities) && capabilities.includes(HEARTBEAT_CAPABILITY);
      socket.serializeAttachment(state);
      send(socket, state.heartbeatNegotiated ? { type: 'joined', capabilities: [HEARTBEAT_CAPABILITY] } : { type: 'joined' });
      const peers = this.joinedSockets();
      if (peers.length === 2) {
        for (const peer of peers) send(peer, { type: 'peer' });
      }
      await this.scheduleDeadline();
      return;
    }
    // Frame contents remain opaque: only the envelope shape and size are checked.
    if (frame.type === 'hello') {
      if (typeof frame.nonce !== 'string' || frame.nonce.length > 128) return close(socket, 1008, 'invalid frame');
    } else if (frame.type === 'secure') {
      if (!Number.isSafeInteger(frame.seq) || (frame.seq as number) < 0 || typeof frame.ciphertext !== 'string' || frame.ciphertext.length > MAX_FRAME_BYTES) return close(socket, 1008, 'invalid frame');
    } else return close(socket, 1008, 'invalid frame');
    for (const peer of this.joinedSockets()) if (peer !== socket) send(peer, frame);
  }

  async webSocketClose(socket: WebSocket): Promise<void> {
    await this.depart(socket);
    await this.scheduleDeadline();
  }

  async webSocketError(socket: WebSocket, _error: unknown): Promise<void> {
    // A hibernatable socket error is terminal for this connection. `depart`
    // records it before closing so a later close cannot affect a replacement.
    await this.depart(socket);
    close(socket, 1011, 'socket error');
    await this.scheduleDeadline();
  }

  async alarm(): Promise<void> {
    await this.pruneExpiredSockets();
    await this.scheduleDeadline();
  }

  private joinedSockets(): WebSocket[] {
    return this.activeSockets().filter(socket => attachment(socket)?.joined);
  }

  private activeSockets(): WebSocket[] {
    return openSockets(this.ctx.getWebSockets()).filter(socket => !attachment(socket)?.departed);
  }

  private heartbeatDeadline(socket: WebSocket, state: Attachment): number {
    const heartbeat = this.ctx.getWebSocketAutoResponseTimestamp(socket)?.getTime() ?? 0;
    return Math.max(state.openedAt, heartbeat) + HEARTBEAT_LEASE_MS;
  }

  private deadline(socket: WebSocket, state: Attachment): number | null {
    if (!state.joined) return state.openedAt + JOIN_TIMEOUT_MS;
    if (state.heartbeatNegotiated) return this.heartbeatDeadline(socket, state);
    // A completed legacy pair retains its historic long-lived behaviour.
    if (this.joinedSockets().length === 2) return null;
    // Once a peer leaves, give the survivor a fresh pairing window rather than
    // scheduling an alarm in the past from its original connection time.
    return state.pendingSinceAt + PAIRING_TTL_MS;
  }

  private async pruneExpiredSockets(): Promise<void> {
    const now = Date.now();
    for (const socket of this.activeSockets()) {
      const state = attachment(socket);
      if (!state) { close(socket, 1008, 'invalid connection'); continue; }
      const deadline = this.deadline(socket, state);
      if (deadline !== null && deadline <= now) {
        await this.depart(socket);
        close(socket, 1008, state.joined ? (state.heartbeatNegotiated ? 'heartbeat timeout' : 'pairing timeout') : 'join timeout');
      }
    }
  }

  private async depart(socket: WebSocket): Promise<void> {
    const state = attachment(socket);
    if (!state || state.departed) return;
    state.departed = true;
    socket.serializeAttachment(state);
    if (!state.joined) return;
    const now = Date.now();
    for (const peer of this.joinedSockets()) {
      if (peer === socket) continue;
      const peerState = attachment(peer);
      if (!peerState) continue;
      peerState.pendingSinceAt = now;
      peer.serializeAttachment(peerState);
      send(peer, { type: 'peer_left' });
    }
  }

  private async scheduleDeadline(): Promise<void> {
    const deadlines = this.activeSockets().flatMap(socket => {
      const state = attachment(socket);
      const deadline = state ? this.deadline(socket, state) : null;
      return deadline === null ? [] : [deadline];
    });
    if (deadlines.length === 0) return this.ctx.storage.deleteAlarm();
    const deadline = Math.min(...deadlines);
    await this.ctx.storage.setAlarm(deadline);
  }
}

type PairingRow = { code: string; invitation: string; expires_at: number; revocation_token: string; persistent: number };

export class PairingRegistry extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS pairings (
        code TEXT PRIMARY KEY,
        invitation TEXT NOT NULL,
        expires_at INTEGER NOT NULL,
        revocation_token TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS rate_limits (
        key TEXT PRIMARY KEY,
        window_started_at INTEGER NOT NULL,
        count INTEGER NOT NULL
      );
    `);
    // Existing reservations remain single-use; adding the opt-in flag is additive.
    const columns = this.ctx.storage.sql.exec<{name: string}>('PRAGMA table_info(pairings)').toArray();
    if (!columns.some(column => column.name === 'persistent')) this.ctx.storage.sql.exec('ALTER TABLE pairings ADD COLUMN persistent INTEGER NOT NULL DEFAULT 0');
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const code = url.pathname.match(/^\/pairing\/(\d{9})$/)?.[1] ?? null;
    const key = request.headers.get('X-Monitter-Pairing-Rate-Key');
    if (!key || !BASE64URL.test(key) || key.length > 64) return response(400, 'Invalid pairing request.');
    const post = request.method === 'POST' && url.pathname === '/pairing';
    const category = post ? 'post' : 'lookup';
    const limit = post ? PAIRING_POST_LIMIT : PAIRING_LOOKUP_LIMIT;
    if (!this.consumeRate(`${category}:global`, post ? PAIRING_GLOBAL_POST_LIMIT : PAIRING_GLOBAL_LOOKUP_LIMIT) || !this.consumeRate(`${category}:${key}`, limit)) return response(429, 'Too many pairing requests.');
    if (post) return this.create(request);
    if (request.method === 'GET' && code) return this.claim(request, code);
    if (request.method === 'DELETE' && code) return this.revoke(request, code);
    if (request.method === 'PUT' && code) return this.renew(request, code);
    return response(404, 'Not found.');
  }

  private consumeRate(key: string, limit: number): boolean {
    const now = Date.now();
    this.ctx.storage.sql.exec('DELETE FROM rate_limits WHERE window_started_at < ?', now - RATE_WINDOW_PAIRING_MS * 2);
    const row = this.ctx.storage.sql.exec<{ window_started_at: number; count: number }>('SELECT window_started_at, count FROM rate_limits WHERE key = ?', key).toArray()[0];
    if (!row && this.ctx.storage.sql.exec<{ count: number }>('SELECT COUNT(*) AS count FROM rate_limits').one().count >= PAIRING_RATE_KEY_LIMIT) return false;
    if (!row || now - row.window_started_at >= RATE_WINDOW_PAIRING_MS) {
      this.ctx.storage.sql.exec('INSERT INTO rate_limits (key, window_started_at, count) VALUES (?, ?, 1) ON CONFLICT(key) DO UPDATE SET window_started_at = excluded.window_started_at, count = excluded.count', key, now);
      return true;
    }
    if (row.count >= limit) return false;
    this.ctx.storage.sql.exec('UPDATE rate_limits SET count = count + 1 WHERE key = ?', key);
    return true;
  }

  private async create(request: Request): Promise<Response> {
    const body = await readBoundedText(request, PAIRING_REQUEST_BYTES);
    if (body === null) return response(413, 'Invitation too large.');
    let parsed: unknown;
    try { parsed = JSON.parse(body); } catch { return response(400, 'Invalid invitation.'); }
    const expectedRelay = request.headers.get('X-Monitter-Expected-Relay');
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return response(400, 'Invalid invitation.');
    const fields = parsed as Record<string, unknown>;
    if (Object.keys(fields).some(key => !['invitation', 'persistent'].includes(key)) || (fields.persistent !== undefined && typeof fields.persistent !== 'boolean')) return response(400, 'Invalid invitation.');
    const persistent = fields.persistent === true;
    const candidate = (parsed as Record<string, unknown>).invitation;
    if (!(expectedRelay?.startsWith('wss:') ? validInvitation(candidate, expectedRelay) : validLocalInvitation(candidate))) return response(400, 'Invalid invitation.');

    const authorization = request.headers.get('Authorization');
    const managementToken = authorization?.startsWith('Bearer ') ? authorization.slice(7) : '';
    if (persistent && (!BASE64URL.test(managementToken) || managementToken.length !== 43)) return response(400, 'A persistent pairing management token is required.');
    const invitation = JSON.stringify((parsed as { invitation: Invitation }).invitation);
    if (persistent) {
      // The client persists its management token before POST. A lost response
      // can therefore recover the same locator instead of leaking a reservation.
      const existing = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token, persistent FROM pairings WHERE persistent = 1 AND revocation_token = ?', managementToken).toArray()[0];
      if (existing) {
        this.ctx.storage.sql.exec('UPDATE pairings SET invitation = ? WHERE code = ?', invitation, existing.code);
        return Response.json({ code: existing.code, expiresAt: null, persistent: true, revocationToken: managementToken });
      }
    }
    const now = Date.now();
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE persistent = 0 AND expires_at <= ?', now);
    const active = this.ctx.storage.sql.exec<{ count: number }>('SELECT COUNT(*) AS count FROM pairings').one().count;
    if (active >= PAIRING_LIMIT) return response(503, 'Pairing capacity reached.');
    const token = persistent ? managementToken : bytesToBase64url(crypto.getRandomValues(new Uint8Array(32)));
    const expiresAt = persistent ? 0 : now + PAIRING_TTL_MS;
    for (let attempt = 0; attempt < 8; attempt += 1) {
      const code = randomDigits();
      try {
        this.ctx.storage.sql.exec('INSERT INTO pairings (code, invitation, expires_at, revocation_token, persistent) VALUES (?, ?, ?, ?, ?)', code, invitation, expiresAt, token, persistent ? 1 : 0);
        await this.scheduleExpiryAlarm();
        return Response.json(persistent ? { code, expiresAt: null, persistent: true, revocationToken: token } : { code, expiresAt, revocationToken: token }, { status: 201 });
      } catch { /* A random code collision is retried without exposing data. */ }
    }
    return response(503, 'Pairing capacity reached.');
  }

  private async claim(request: Request, code: string): Promise<Response> {
    if ((request.headers.get('Content-Length') && request.headers.get('Content-Length') !== '0') || request.body) return response(400, 'Claim requests cannot have a body.');
    const row = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token, persistent FROM pairings WHERE code = ?', code).toArray()[0];
    if (!row) return response(404, 'Pairing not found.');
    if (!row.persistent && row.expires_at <= Date.now()) {
      this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
      return response(404, 'Pairing not found.');
    }
    // Legacy claims are consumed synchronously. Opted-in persistent locators
    // reveal only the public invitation; desktop approval still grants access.
    if (!row.persistent) this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
    return Response.json(row.persistent ? { invitation: JSON.parse(row.invitation), expiresAt: null, persistent: true } : { invitation: JSON.parse(row.invitation), expiresAt: row.expires_at });
  }

  private async revoke(request: Request, code: string): Promise<Response> {
    const authorization = request.headers.get('Authorization');
    const token = authorization?.startsWith('Bearer ') ? authorization.slice(7) : '';
    if (!BASE64URL.test(token) || token.length !== 43) return response(404, 'Pairing not found.');
    const row = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token, persistent FROM pairings WHERE code = ?', code).toArray()[0];
    if (!row || (!row.persistent && row.expires_at <= Date.now()) || row.revocation_token !== token) {
      if (row && !row.persistent && row.expires_at <= Date.now()) this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
      return response(404, 'Pairing not found.');
    }
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
    return response(204);
  }

  private async renew(request: Request, code: string): Promise<Response> {
    const authorization = request.headers.get('Authorization');
    const token = authorization?.startsWith('Bearer ') ? authorization.slice(7) : '';
    if (!BASE64URL.test(token) || token.length !== 43) return response(404, 'Pairing not found.');
    const body = await readBoundedText(request, PAIRING_REQUEST_BYTES);
    if (body === null) return response(413, 'Invitation too large.');
    let parsed: unknown;
    try { parsed = JSON.parse(body); } catch { return response(400, 'Invalid invitation.'); }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || Object.keys(parsed).join(',') !== 'invitation') return response(400, 'Invalid invitation.');
    const candidate = (parsed as Record<string, unknown>).invitation;
    const expectedRelay = request.headers.get('X-Monitter-Expected-Relay');
    if (!(expectedRelay?.startsWith('wss:') ? validInvitation(candidate, expectedRelay) : validLocalInvitation(candidate))) return response(400, 'Invalid invitation.');
    // No await between authority check and update: revoke/renew cannot interleave.
    const row = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token, persistent FROM pairings WHERE code = ?', code).toArray()[0];
    if (!row || !row.persistent || row.revocation_token !== token) return response(404, 'Pairing not found.');
    this.ctx.storage.sql.exec('UPDATE pairings SET invitation = ? WHERE code = ?', JSON.stringify(candidate), code);
    return Response.json({ code, expiresAt: null, persistent: true, revocationToken: token });
  }

  async alarm(): Promise<void> {
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE persistent = 0 AND expires_at <= ?', Date.now());
    await this.scheduleExpiryAlarm();
  }

  private async scheduleExpiryAlarm(): Promise<void> {
    const row = this.ctx.storage.sql.exec<{ expires_at: number }>('SELECT expires_at FROM pairings WHERE persistent = 0 ORDER BY expires_at LIMIT 1').toArray()[0];
    if (row) await this.ctx.storage.setAlarm(row.expires_at);
    else await this.ctx.storage.deleteAlarm();
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === '/health') return new Response('ok');
    if (url.pathname === '/pairing' || url.pathname.startsWith('/pairing/')) {
      const origin = request.headers.get('Origin');
      const acceptedOrigin = allowedOrigin(origin);
      if (origin && !acceptedOrigin) return response(403, 'Origin denied.');
      if (request.method === 'OPTIONS') return new Response(null, { status: 204, headers: cors(acceptedOrigin) });
      const headers = new Headers(request.headers);
      headers.set('X-Monitter-Pairing-Rate-Key', await rateKey(request));
      headers.set('X-Monitter-Expected-Relay', relayUrlForRequest(request) ?? '');
      headers.delete('CF-Connecting-IP');
      const upstream = await env.PAIRINGS.getByName('v2:global').fetch(new Request(request, { headers }));
      const responseHeaders = new Headers(upstream.headers);
      for (const [name, value] of cors(acceptedOrigin)) responseHeaders.set(name, value);
      return new Response(upstream.body, { status: upstream.status, headers: responseHeaders });
    }
    if (url.pathname !== '/relay') return new Response('Not found.', { status: 404 });
    if (request.method !== 'GET' || request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') {
      return new Response('WebSocket upgrade required.', { status: 426 });
    }
    const room = url.searchParams.get('room');
    const role = url.searchParams.get('role');
    if (!validRoom(room) || (role !== 'desktop' && role !== 'mobile')) return new Response('Invalid relay route.', { status: 400 });
    return env.ROOM.getByName(`v1:${room}`).fetch(request);
  },
} satisfies ExportedHandler<Env>;
import { DurableObject } from 'cloudflare:workers';
