#!/usr/bin/env python3
"""Deterministic SSH shim for Codex app-server tests.

It deliberately executes only the final remote command locally. The real
transport's SSH argument construction, quoting, and supervision remain under
test; this shim supplies a process boundary without changing PATH or SSH
configuration. Reverse forwards use real loopback sockets. Only deterministic
test commands are logged so tests can assert that bearer tokens stay off argv.
"""
import os
import json
from pathlib import Path
import select
import socket
import threading
import sys


def main() -> int:
    args = sys.argv[1:]
    root = Path(__file__).parent
    with (root / "ssh-argv.jsonl").open("a") as log:
        log.write(json.dumps(args) + "\n")
    if any("hostkey-failure" in arg for arg in args):
        print("Host key verification failed.", file=sys.stderr)
        return 255
    # Reverse-forward helpers intentionally have no remote command.
    if "-N" in args:
        remote_bind, remote_port, destination, port = args[args.index("-R") + 1].split(":")
        assert remote_bind == destination == "127.0.0.1" and remote_port == "0"
        listener = socket.socket()
        listener.bind((remote_bind, 0))
        listener.listen()
        print(f"Allocated port {listener.getsockname()[1]} for remote forward", file=sys.stderr, flush=True)
        def forward(incoming):
            with incoming, socket.create_connection((destination, int(port)), timeout=3) as outgoing:
                while True:
                    ready, _, _ = select.select([incoming, outgoing], [], [], 3)
                    if not ready:
                        return
                    for source in ready:
                        data = source.recv(65536)
                        if not data:
                            return
                        (outgoing if source is incoming else incoming).sendall(data)
        while True:
            incoming, _ = listener.accept()
            threading.Thread(target=forward, args=(incoming,), daemon=True).start()
    command = args[-1] if args else ""
    if not command:
        return 2
    os.execl("/bin/sh", "sh", "-c", command)


if __name__ == "__main__":
    raise SystemExit(main())
