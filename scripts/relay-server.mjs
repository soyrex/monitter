import { WebSocketServer } from 'ws';

const port = Number(process.env.MONITTER_RELAY_PORT || 8789);
const host = process.env.MONITTER_RELAY_HOST || '127.0.0.1';
// Base64-encrypted 1 MiB controller responses require a larger wire ceiling.
const maxFrameBytes = 2 * 1024 * 1024;
const maxMessagesPerWindow = Number(process.env.MONITTER_RELAY_RATE_LIMIT || 0);
const rateWindowMs = 10_000;
const maxBufferedBytes = 2 * 1024 * 1024;
const maxConnections = 64;
const maxRooms = 32;
const joinTimeoutMs = 10_000;
const heartbeatMs = 30_000;
const relayHeartbeatCapability = 'relay-heartbeat-v1';
const relayHeartbeatLeaseMs = 45_000;
const leaseCheckMs = 5_000;
const rooms = new Map();
const wss = new WebSocketServer({ port, host, maxPayload: maxFrameBytes, perMessageDeflate: false });

function leave(socket) {
  if (socket.departed) return;
  socket.departed = true;
  const joined = socket.room;
  if (!joined) return;
  const room = rooms.get(joined);
  room?.delete(socket);
  if (room?.size) for (const peer of room) send(peer, { type: 'peer_left' });
  else rooms.delete(joined);
  socket.room = undefined;
}

function expiredLease(socket, now = Date.now()) {
  return Boolean(socket.heartbeatNegotiated && socket.room && now - socket.lastHeartbeatAt >= relayHeartbeatLeaseMs);
}

function pruneExpiredRoom(room, now = Date.now()) {
  for (const peer of room) {
    if (!expiredLease(peer, now)) continue;
    leave(peer);
    peer.close(1008, 'heartbeat timeout');
  }
}

function send(socket, message) {
  if (socket.readyState !== socket.OPEN) return;
  if (socket.bufferedAmount > maxBufferedBytes) return socket.close(1013, 'backpressure');
  socket.send(JSON.stringify(message));
}

wss.on('connection', socket => {
  if (wss.clients.size > maxConnections) return socket.close(1013, 'connection limit');
  socket.alive = true;
  socket.departed = false;
  socket.rateWindowStartedAt = Date.now();
  socket.messagesInWindow = 0;
  const joinTimer = setTimeout(() => socket.close(1008, 'join timeout'), joinTimeoutMs);
  socket.on('pong', () => { socket.alive = true; });
  socket.on('message', (raw, isBinary) => {
    if (socket.departed) return;
    if (isBinary || raw.length > maxFrameBytes) return socket.close(1009, 'frame too large');
    if (maxMessagesPerWindow > 0) {
      const now = Date.now();
      if (now - socket.rateWindowStartedAt >= rateWindowMs) { socket.rateWindowStartedAt = now; socket.messagesInWindow = 0; }
      if (++socket.messagesInWindow > maxMessagesPerWindow) return socket.close(1008, 'rate limit');
    }
    let message; try { message = JSON.parse(raw.toString()); } catch { return socket.close(1008, 'invalid JSON'); }
    if (!socket.room) {
      if (!message || message.type !== 'join' || typeof message.room !== 'string' || !/^[A-Za-z0-9_-]{22,128}$/.test(message.room) || !['desktop', 'mobile'].includes(message.role) || (message.capabilities !== undefined && (!Array.isArray(message.capabilities) || message.capabilities.length > 8 || message.capabilities.some(capability => typeof capability !== 'string' || capability.length > 128)))) return socket.close(1008, 'join required');
      let room = rooms.get(message.room);
      if (room) {
        pruneExpiredRoom(room);
        room = rooms.get(message.room);
      }
      if (!room) { if (rooms.size >= maxRooms) return socket.close(1013, 'room limit'); rooms.set(message.room, room = new Set()); }
      if (room.size >= 2 || [...room].some(peer => peer.role === message.role)) return socket.close(1008, 'room unavailable');
      clearTimeout(joinTimer);
      socket.room = message.room;
      socket.role = message.role;
      socket.heartbeatNegotiated = Array.isArray(message.capabilities) && message.capabilities.includes(relayHeartbeatCapability);
      socket.lastHeartbeatAt = Date.now();
      room.add(socket);
      send(socket, socket.heartbeatNegotiated ? { type: 'joined', capabilities: [relayHeartbeatCapability] } : { type: 'joined' });
      if (room.size === 2) for (const peer of room) send(peer, { type: 'peer' });
      return;
    }
    if (socket.heartbeatNegotiated && message?.type === 'ping' && Object.keys(message).length === 1) {
      socket.lastHeartbeatAt = Date.now();
      send(socket, { type: 'pong' });
      return;
    }
    // The relay only routes bounded opaque protocol frames; it never decrypts or logs them.
    if (!message || !['hello', 'secure'].includes(message.type)) return socket.close(1008, 'invalid frame');
    for (const peer of rooms.get(socket.room) || []) if (peer !== socket) send(peer, message);
  });
  socket.on('close', () => { clearTimeout(joinTimer); leave(socket); });
  socket.on('error', () => { clearTimeout(joinTimer); leave(socket); });
});

const heartbeat = setInterval(() => {
  for (const socket of wss.clients) {
    if (!socket.alive) { socket.terminate(); continue; }
    socket.alive = false; socket.ping();
  }
}, heartbeatMs);
const leaseCleanup = setInterval(() => {
  const now = Date.now();
  for (const socket of wss.clients) {
    if (!expiredLease(socket, now)) continue;
    leave(socket); socket.close(1008, 'heartbeat timeout');
  }
}, leaseCheckMs);
wss.on('close', () => { clearInterval(heartbeat); clearInterval(leaseCleanup); });
wss.on('listening', () => {
  const address = wss.address();
  if (!address || typeof address === 'string') throw new Error('Relay did not bind a TCP address.');
  console.log(`Monitter opaque relay listening on ws://${host}:${address.port}`);
});
