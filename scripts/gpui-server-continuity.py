#!/usr/bin/env python3
"""Bounded experimental-only IPC/PTY continuity proof; never calls a provider."""
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import sys
import uuid

ROOT = Path(__file__).resolve().parent.parent
PROFILE = (Path.home() / 'Library/Application Support/com.monitter.gpui.experimental').resolve()
KEY = hashlib.sha256(os.fsencode(PROFILE)).hexdigest()[:16]
SOCKET = Path('/private/tmp') / f'monitter-gpui-{os.geteuid()}-{KEY}' / 'backend.sock'
PROOF = ROOT / 'artifacts/gpui/server-continuity.json'

def send(sock, value):
    data = json.dumps(value, separators=(',', ':')).encode()
    assert 0 < len(data) <= 32 * 1024 * 1024
    sock.sendall(struct.pack('>I', len(data)) + data)

def receive(sock):
    def exact(size):
        data = bytearray()
        while len(data) < size:
            chunk = sock.recv(size - len(data))
            if not chunk:
                raise RuntimeError('IPC connection closed')
            data.extend(chunk)
        return bytes(data)
    size, = struct.unpack('>I', exact(4))
    assert 0 < size <= 32 * 1024 * 1024
    return json.loads(exact(size))

def call(method, args):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(20)
        sock.connect(str(SOCKET))
        send(sock, {'version': 1, 'profile': str(PROFILE)})
        welcome = receive(sock)
        request_id = str(uuid.uuid4())
        send(sock, {'id': request_id, 'method': method, 'args': args})
        response = receive(sock)
        assert response['id'] == request_id
        if response['error'] is not None:
            raise RuntimeError(response['error'])
        return welcome, response['result']

def invoke(command, args):
    return call('invoke', {'command': command, 'args': args})

def observe(proof):
    welcome, read = invoke('read_terminal', {'id': proof['terminalId'], 'afterSeq': 0})
    assert welcome['instanceId'] == proof['serverInstanceId']
    assert welcome['pid'] == proof['serverPid']
    text = b''.join(bytes(chunk['data']) for chunk in read['chunks']).decode(errors='replace')
    import re
    shell = re.findall(r'GPUI_CONTINUITY_PID=(\d+)', text)
    ticks = re.findall(r'GPUI_CONTINUITY_TICK=(\d+)', text)
    sample = {'serverPid': welcome['pid'], 'serverInstanceId': welcome['instanceId'],
              'terminalId': proof['terminalId'], 'nextSeq': read['nextSeq'],
              'status': read['status'], 'shellPid': int(shell[-1]) if shell else None,
              'tick': int(ticks[-1]) if ticks else 0}
    if proof.get('shellPid') is not None and sample['shellPid'] is not None:
        assert sample['shellPid'] == proof['shellPid']
    return sample

mode = sys.argv[1]
if mode == 'prepare':
    if PROOF.exists() and not json.loads(PROOF.read_text()).get('closed', False):
        raise RuntimeError('A prior proof terminal requires explicit cleanup first')
    _, changes = call('read_ui_changes', {'revision': None})
    hosts = changes['snapshot']['hosts']
    local = next(host for host in hosts if host['kind'] == 'local')
    command = "printf 'GPUI_CONTINUITY_PID=%s\\n' $$; i=0; while [ $i -lt 180 ]; do i=$((i+1)); printf 'GPUI_CONTINUITY_TICK=%s\\n' $i; sleep 1; done"
    welcome, terminal = invoke('open_terminal', {'target': {'hostId': local['id'],
        'cwd': str(ROOT), 'command': command}, 'cols': 80, 'rows': 24})
    proof = {'serverPid': welcome['pid'], 'serverInstanceId': welcome['instanceId'],
             'terminalId': terminal['id'], 'samples': []}
elif mode in ('observe', 'close'):
    proof = json.loads(PROOF.read_text())
else:
    raise RuntimeError('Use prepare, observe [label], or close')
if mode == 'close':
    welcome, _ = invoke('close_terminal', {'id': proof['terminalId']})
    assert welcome['instanceId'] == proof['serverInstanceId']
    proof['closed'] = True
else:
    sample = observe(proof)
    sample['label'] = sys.argv[2] if len(sys.argv) > 2 else mode
    proof['samples'].append(sample)
    if sample['shellPid'] is not None:
        proof['shellPid'] = sample['shellPid']
PROOF.write_text(json.dumps(proof, indent=2))
print(json.dumps({'proof': str(PROOF), 'closed': proof.get('closed', False),
                  'sample': proof['samples'][-1] if proof['samples'] else None}))
