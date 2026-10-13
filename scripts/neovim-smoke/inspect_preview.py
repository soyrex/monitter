"""Read Neovim status from an explicitly selected GPUI preview profile."""
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import sys
import uuid

def call(profile, command, args):
    profile = str(Path(profile).resolve())
    key = hashlib.sha256(profile.encode()).hexdigest()[:16]
    path = str(Path('/tmp').resolve() / f'monitter-gpui-{os.geteuid()}-{key}' / 'backend.sock')
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(10)
        stream.connect(path)
        def send(value):
            data = json.dumps(value).encode()
            stream.sendall(struct.pack('>I', len(data)) + data)
        def exact(length):
            data = b''
            while len(data) < length:
                chunk = stream.recv(length - len(data))
                if not chunk:
                    raise RuntimeError('Private IPC closed')
                data += chunk
            return data
        def receive():
            length = struct.unpack('>I', exact(4))[0]
            if length > 32 * 1024 * 1024:
                raise RuntimeError('Private IPC frame exceeds limit')
            return json.loads(exact(length))
        send({'version': 1, 'profile': profile})
        receive()
        send({'id': str(uuid.uuid4()), 'method': 'invoke', 'args': {'command': command, 'args': args}})
        response = receive()
        if response.get('error'):
            raise RuntimeError(response['error'])
        return response['result']

if __name__ == '__main__':
    profile = sys.argv[1]
    for session in call(profile, 'list_neovim', {}):
        state = call(profile, 'read_neovim', {'id': session['id'], 'after': 0})
        summary = {key: state.get(key) for key in ('id', 'seq', 'layout', 'currentWindow', 'mode', 'exited', 'error')}
        summary['windowCount'] = len(state.get('windows', []))
        print(json.dumps(summary))
