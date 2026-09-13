export interface Env {
  ROOM: DurableObjectNamespace<RelayRoom>;
  PAIRINGS: DurableObjectNamespace<PairingRegistry>;
}

type Role = 'desktop' | 'mobile';
type Attachment = {
  room: string;
  role: Role;
  joined: boolean;
  openedAt: number;
  rateWindowStartedAt: number;
  messagesInWindow: number;
};

const ROOM_PATTERN = /^[A-Za-z0-9_-]{22,128}$/;
const MAX_FRAME_BYTES = 2 * 1024 * 1024;
const MAX_MESSAGES_PER_WINDOW = 120;
const RATE_WINDOW_MS = 10_000;
const JOIN_TIMEOUT_MS = 10_000;
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
    headers.set('Access-Control-Allow-Methods', 'POST, GET, DELETE, OPTIONS');
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

type Invitation = { version: 2; relayUrl: string; room: string; publicKey: string };

function relayUrlForRequest(request: Request): string | null {
  const requestUrl = new URL(request.url);
  const host = request.headers.get('Host') ?? requestUrl.host;
  const hostname = host.replace(/^\[/, '').replace(/\].*$/, '').split(':')[0];
  const local = ['localhost', '127.0.0.1', '::1'].includes(hostname);
  const scheme = requestUrl.protocol === 'https:' ? 'wss:' : local && requestUrl.protocol === 'http:' ? 'ws:' : '';
  return scheme ? `${scheme}//${host}/relay` : null;
}

function validInvitation(value: unknown, expectedRelayUrl: string | null): value is Invitation {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const invitation = value as Record<string, unknown>;
  const keys = Object.keys(invitation).sort();
  if (keys.join(',') !== 'publicKey,relayUrl,room,version' || invitation.version !== 2 || !validRoom(typeof invitation.room === 'string' ? invitation.room : null) || !validPublicKey(invitation.publicKey)) return false;
  return expectedRelayUrl !== null && invitation.relayUrl === expectedRelayUrl;
}

function validLocalInvitation(value: unknown): value is Invitation {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const invitation = value as Record<string, unknown>;
  const keys = Object.keys(invitation).sort();
  if (keys.join(',') !== 'publicKey,relayUrl,room,version' || invitation.version !== 2 || !validRoom(typeof invitation.room === 'string' ? invitation.room : null) || !validPublicKey(invitation.publicKey) || typeof invitation.relayUrl !== 'string') return false;
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
  return state as Attachment;
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

    const sockets = openSockets(this.ctx.getWebSockets());
    if (sockets.length >= 2) return new Response('Room unavailable.', { status: 409 });
    // A role is reserved as soon as its WebSocket is upgraded. This prevents a
    // concurrent duplicate from racing the later join message.
    if (sockets.some(socket => attachment(socket)?.role === role)) return new Response('Room unavailable.', { status: 409 });

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    const now = Date.now();
    server.serializeAttachment({ room, role, joined: false, openedAt: now, rateWindowStartedAt: now, messagesInWindow: 0 } satisfies Attachment);
    this.ctx.acceptWebSocket(server);
    await this.scheduleDeadline();
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const state = attachment(socket);
    if (!state) return close(socket, 1008, 'invalid connection');
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
      state.joined = true;
      socket.serializeAttachment(state);
      send(socket, { type: 'joined' });
      const peers = this.joinedSockets();
      if (peers.length === 2) {
        for (const peer of peers) send(peer, { type: 'peer' });
        await this.ctx.storage.deleteAlarm();
      }
      else await this.scheduleDeadline();
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
    const departed = attachment(socket);
    if (!departed) return;
    for (const peer of this.joinedSockets()) if (peer !== socket) send(peer, { type: 'peer_left' });
    await this.scheduleDeadline();
  }

  async webSocketError(socket: WebSocket, _error: unknown): Promise<void> {
    // A hibernatable socket error is terminal for this connection. Notify the
    // peer without recording the error or any protocol payload.
    for (const peer of this.joinedSockets()) if (peer !== socket) send(peer, { type: 'peer_left' });
    close(socket, 1011, 'socket error');
    await this.scheduleDeadline();
  }

  async alarm(): Promise<void> {
    const now = Date.now();
    for (const socket of openSockets(this.ctx.getWebSockets())) {
      const state = attachment(socket);
      if (!state) { close(socket, 1008, 'invalid connection'); continue; }
      const deadline = state.joined ? state.openedAt + PAIRING_TTL_MS : state.openedAt + JOIN_TIMEOUT_MS;
      if (deadline <= now) close(socket, 1008, state.joined ? 'pairing timeout' : 'join timeout');
    }
    await this.scheduleDeadline();
  }

  private joinedSockets(): WebSocket[] {
    return openSockets(this.ctx.getWebSockets()).filter(socket => attachment(socket)?.joined);
  }

  private async scheduleDeadline(): Promise<void> {
    const states = openSockets(this.ctx.getWebSockets()).map(socket => attachment(socket)).filter((state): state is Attachment => state !== null);
    if (states.length === 2 && states.every(state => state.joined)) return this.ctx.storage.deleteAlarm();
    if (states.length === 0) return this.ctx.storage.deleteAlarm();
    const deadline = Math.min(...states.map(state => state.openedAt + (state.joined ? PAIRING_TTL_MS : JOIN_TIMEOUT_MS)));
    await this.ctx.storage.setAlarm(deadline);
  }
}

type PairingRow = { code: string; invitation: string; expires_at: number; revocation_token: string };

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
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || Object.keys(parsed as object).length !== 1) return response(400, 'Invalid invitation.');
    const candidate = (parsed as Record<string, unknown>).invitation;
    if (!(expectedRelay?.startsWith('wss:') ? validInvitation(candidate, expectedRelay) : validLocalInvitation(candidate))) return response(400, 'Invalid invitation.');

    const now = Date.now();
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE expires_at <= ?', now);
    const active = this.ctx.storage.sql.exec<{ count: number }>('SELECT COUNT(*) AS count FROM pairings').one().count;
    if (active >= PAIRING_LIMIT) return response(503, 'Pairing capacity reached.');
    const invitation = JSON.stringify((parsed as { invitation: Invitation }).invitation);
    const token = bytesToBase64url(crypto.getRandomValues(new Uint8Array(32)));
    const expiresAt = now + PAIRING_TTL_MS;
    for (let attempt = 0; attempt < 8; attempt += 1) {
      const code = randomDigits();
      try {
        this.ctx.storage.sql.exec('INSERT INTO pairings (code, invitation, expires_at, revocation_token) VALUES (?, ?, ?, ?)', code, invitation, expiresAt, token);
        await this.scheduleExpiryAlarm();
        return Response.json({ code, expiresAt, revocationToken: token }, { status: 201 });
      } catch { /* A random code collision is retried without exposing data. */ }
    }
    return response(503, 'Pairing capacity reached.');
  }

  private async claim(request: Request, code: string): Promise<Response> {
    if ((request.headers.get('Content-Length') && request.headers.get('Content-Length') !== '0') || request.body) return response(400, 'Claim requests cannot have a body.');
    const row = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token FROM pairings WHERE code = ?', code).toArray()[0];
    if (!row) return response(404, 'Pairing not found.');
    if (row.expires_at <= Date.now()) {
      this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
      return response(404, 'Pairing not found.');
    }
    // Both the read and deletion run synchronously in the DO turn: concurrent
    // claims cannot observe the same invitation.
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
    return Response.json({ invitation: JSON.parse(row.invitation), expiresAt: row.expires_at });
  }

  private async revoke(request: Request, code: string): Promise<Response> {
    const authorization = request.headers.get('Authorization');
    const token = authorization?.startsWith('Bearer ') ? authorization.slice(7) : '';
    if (!BASE64URL.test(token) || token.length !== 43) return response(404, 'Pairing not found.');
    const row = this.ctx.storage.sql.exec<PairingRow>('SELECT code, invitation, expires_at, revocation_token FROM pairings WHERE code = ?', code).toArray()[0];
    if (!row || row.expires_at <= Date.now() || row.revocation_token !== token) {
      if (row?.expires_at <= Date.now()) this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
      return response(404, 'Pairing not found.');
    }
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE code = ?', code);
    return response(204);
  }

  async alarm(): Promise<void> {
    this.ctx.storage.sql.exec('DELETE FROM pairings WHERE expires_at <= ?', Date.now());
    await this.scheduleExpiryAlarm();
  }

  private async scheduleExpiryAlarm(): Promise<void> {
    const row = this.ctx.storage.sql.exec<{ expires_at: number }>('SELECT expires_at FROM pairings ORDER BY expires_at LIMIT 1').toArray()[0];
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
