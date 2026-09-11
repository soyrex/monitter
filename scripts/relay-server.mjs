import { WebSocketServer } from 'ws';

const port = Number(process.env.MONITTER_RELAY_PORT || 8789);
const host = process.env.MONITTER_RELAY_HOST || '127.0.0.1';
// Base64-encrypted 1 MiB controller responses require a larger wire ceiling.
const maxFrameBytes = 2 * 1024 * 1024;
const maxBufferedBytes = 2 * 1024 * 1024;
const maxConnections = 64;
const maxRooms = 32;
const joinTimeoutMs = 10_000;
const heartbeatMs = 30_000;
const rooms = new Map();
const wss = new WebSocketServer({ port, host, maxPayload: maxFrameBytes, perMessageDeflate: false });

function leave(socket) {
  const joined = socket.room;
  if (!joined) return;
  const room = rooms.get(joined);
  room?.delete(socket);
  if (room?.size) for (const peer of room) send(peer, { type: 'peer_left' });
  else rooms.delete(joined);
  socket.room = undefined;
}

function send(socket, message) {
  if (socket.readyState !== socket.OPEN) return;
  if (socket.bufferedAmount > maxBufferedBytes) return socket.close(1013, 'backpressure');
  socket.send(JSON.stringify(message));
}

wss.on('connection', socket => {
  if (wss.clients.size > maxConnections) return socket.close(1013, 'connection limit');
  socket.alive = true;
  const joinTimer = setTimeout(() => socket.close(1008, 'join timeout'), joinTimeoutMs);
  socket.on('pong', () => { socket.alive = true; });
  socket.on('message', (raw, isBinary) => {
    if (isBinary || raw.length > maxFrameBytes) return socket.close(1009, 'frame too large');
    let message; try { message = JSON.parse(raw.toString()); } catch { return socket.close(1008, 'invalid JSON'); }
    if (!socket.room) {
      if (!message || message.type !== 'join' || typeof message.room !== 'string' || !/^[A-Za-z0-9_-]{22,128}$/.test(message.room) || !['desktop', 'mobile'].includes(message.role)) return socket.close(1008, 'join required');
      let room = rooms.get(message.room);
      if (!room) { if (rooms.size >= maxRooms) return socket.close(1013, 'room limit'); rooms.set(message.room, room = new Set()); }
      if (room.size >= 2 || [...room].some(peer => peer.role === message.role)) return socket.close(1008, 'room unavailable');
      clearTimeout(joinTimer); socket.room = message.room; socket.role = message.role; room.add(socket); send(socket, { type: 'joined' });
      if (room.size === 2) for (const peer of room) send(peer, { type: 'peer' });
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
wss.on('close', () => clearInterval(heartbeat));
wss.on('listening', () => {
  const address = wss.address();
  if (!address || typeof address === 'string') throw new Error('Relay did not bind a TCP address.');
  console.log(`Monitter opaque relay listening on ws://${host}:${address.port}`);
});
