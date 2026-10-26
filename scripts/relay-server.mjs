import { WebSocketServer } from 'ws';
import { createServer } from 'node:http';
import { randomBytes, randomInt, timingSafeEqual } from 'node:crypto';

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
const pairings = new Map(), pairingRates = new Map();
function pairingOrigin(value) {
  if (!value) return null;
  if (['https://share.monitter.com','https://appassets.androidplatform.net','tauri://localhost','http://tauri.localhost','https://tauri.localhost'].includes(value)) return value;
  try { const url=new URL(value); return ['http:','https:'].includes(url.protocol)&&['localhost','127.0.0.1','[::1]'].includes(url.hostname)&&url.pathname==='/'&&!url.search&&!url.hash?value:null; } catch { return null; }
}
function pairingRate(key,limit,now) {
  for (const [key,row] of pairingRates) if (now-row.started>=120000) pairingRates.delete(key);
  const row=pairingRates.get(key);
  if (!row || now-row.started>=60000) {if (!row&&pairingRates.size>=1024)return false;pairingRates.set(key,{started:now,count:1});return true;}
  if (row.count>=limit)return false;row.count++;return true;
}
function validPairingInvitation(value) {
  if (!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).sort().join(',')!=='publicKey,relayUrl,room,version'||value.version!==2||!/^[A-Za-z0-9_-]{22,128}$/.test(value.room)||!/^B[A-Za-z0-9_-]{86}$/.test(value.publicKey))return false;
  try {const url=new URL(value.relayUrl);return ['ws:','wss:'].includes(url.protocol)&&['localhost','127.0.0.1','[::1]'].includes(url.hostname)&&url.pathname==='/relay'&&!url.search&&!url.hash;}catch{return false;}
}
function pairingTokenMatches(expected,token) {return typeof token==='string'&&/^[A-Za-z0-9_-]{43}$/.test(token)&&timingSafeEqual(Buffer.from(expected),Buffer.from(token));}
const server=createServer(async(req,res)=>{
  const url=new URL(req.url||'/', 'http://localhost');
  const origin=pairingOrigin(req.headers.origin);
  const reply=(status,body)=>{res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store','Vary':'Origin',...(origin?{'Access-Control-Allow-Origin':origin,'Access-Control-Allow-Methods':'POST, PUT, GET, DELETE, OPTIONS','Access-Control-Allow-Headers':'Content-Type, Authorization','Access-Control-Max-Age':'600'}:{})});res.end(body===undefined?'':JSON.stringify(body));};
  if (url.pathname==='/health')return reply(200,{ok:true});
  if (!(url.pathname==='/pairing'||url.pathname.startsWith('/pairing/')))return reply(404,{error:'Not found.'});
  if (req.headers.origin&&!origin)return reply(403,{error:'Origin denied.'});
  if (req.method==='OPTIONS')return reply(204);
  const now=Date.now();for(const [code,row]of pairings)if(!row.persistent&&row.expiresAt<=now)pairings.delete(code);
  const post=req.method==='POST'&&url.pathname==='/pairing',category=post?'post':'lookup';
  if(!pairingRate(`${category}:global`,post?100:200,now)||!pairingRate(`${category}:${req.socket.remoteAddress}`,post?5:10,now))return reply(429,{error:'Too many pairing requests.'});
  const code=/^\/pairing\/(\d{9})$/.exec(url.pathname)?.[1];
  if(req.method==='GET'&&code){
    if(req.headers['transfer-encoding']||(req.headers['content-length']&&req.headers['content-length']!=='0'))return reply(400,{error:'Claim requests cannot have a body.'});
    const row=pairings.get(code);if(!row)return reply(404,{error:'Pairing not found.'});
    if(!row.persistent)pairings.delete(code);
    return reply(200,{invitation:row.invitation,expiresAt:row.expiresAt,...(row.persistent?{persistent:true}:{})});
  }
  if(req.method==='DELETE'&&code){const row=pairings.get(code),token=req.headers.authorization?.startsWith('Bearer ')?req.headers.authorization.slice(7):'';if(!row||!pairingTokenMatches(row.revocationToken,token))return reply(404,{error:'Pairing not found.'});pairings.delete(code);return reply(204);}
  if(!post&&!(req.method==='PUT'&&code))return reply(404,{error:'Not found.'});
  let size=0,chunks=[];
  try{for await(const chunk of req){size+=chunk.length;if(size>4096)return reply(413,{error:'Invitation too large.'});chunks.push(chunk);}}catch{return reply(400,{error:'Invalid invitation.'});}
  let body;try{body=JSON.parse(Buffer.concat(chunks).toString());}catch{return reply(400,{error:'Invalid invitation.'});}
  if(!body||typeof body!=='object'||Array.isArray(body)||Object.keys(body).some(key=>!(post?['invitation','persistent']:['invitation']).includes(key))||(body.persistent!==undefined&&typeof body.persistent!=='boolean')||!validPairingInvitation(body.invitation))return reply(400,{error:'Invalid invitation.'});
  if(!post){const row=pairings.get(code),token=req.headers.authorization?.startsWith('Bearer ')?req.headers.authorization.slice(7):'';if(!row||!row.persistent||!pairingTokenMatches(row.revocationToken,token))return reply(404,{error:'Pairing not found.'});row.invitation=body.invitation;return reply(200,{code,expiresAt:null,persistent:true,revocationToken:row.revocationToken});}
  const persistent=body.persistent===true,managementToken=req.headers.authorization?.startsWith('Bearer ')?req.headers.authorization.slice(7):'';
  if(persistent&&!/^[A-Za-z0-9_-]{43}$/.test(managementToken))return reply(400,{error:'A persistent pairing management token is required.'});
  if(persistent){for(const [existingCode,row]of pairings){if(row.persistent&&pairingTokenMatches(row.revocationToken,managementToken)){row.invitation=body.invitation;return reply(200,{code:existingCode,expiresAt:null,persistent:true,revocationToken:managementToken});}}}
  if(pairings.size>=1000)return reply(503,{error:'Pairing capacity reached.'});
  let nextCode;for(let i=0;i<8;i++){const candidate=String(randomInt(1000000000)).padStart(9,'0');if(!pairings.has(candidate)){nextCode=candidate;break;}}
  if(!nextCode)return reply(503,{error:'Pairing capacity reached.'});
  const row={invitation:body.invitation,persistent,expiresAt:persistent?null:now+300000,revocationToken:persistent?managementToken:randomBytes(32).toString('base64url')};pairings.set(nextCode,row);
  return reply(201,{code:nextCode,expiresAt:row.expiresAt,revocationToken:row.revocationToken,...(persistent?{persistent:true}:{})});
});
const wss = new WebSocketServer({ server, maxPayload: maxFrameBytes, perMessageDeflate: false });
server.listen(port,host);

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
