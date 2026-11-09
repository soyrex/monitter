#!/usr/bin/env python3
"""Loopback compatibility adapter for Monitter's local SystemOne client."""
import argparse
import hmac
import json
import os
import re
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# Loaded only by main so tests can exercise the adapter without an installation.
PYTHON = None
SERVE = None
MODEL = Path('/unconfigured-laya-model')


def load_runtime():
    path = Path(__file__).resolve().with_name('runtime.json')
    try:
        data = json.loads(path.read_text())
        paths = [Path(data[key]) for key in ('python', 'serveDir', 'modelDir')]
        if not all(path.is_absolute() for path in paths):
            raise ValueError('paths must be absolute')
        python, serve, model = paths
        if not python.is_file() or not os.access(python, os.X_OK):
            raise ValueError('python is not executable')
        if not (serve / 'mlx_omarchy_laya/server.py').is_file():
            raise ValueError('serveDir does not contain mlx_omarchy_laya/server.py')
        if not (model / 'manifest.json').is_file() or not (model / 'model.safetensors').is_file():
            raise ValueError('modelDir is not a converted Laya model')
        return python, serve, model
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise RuntimeError(f'Invalid or missing {path}; run the offline installer: {exc}') from exc

MAX_BODY = 1024 * 1024
MAX_RESPONSE = 4 * 1024 * 1024
TIMEOUT = 120
WORKER_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
QUEUE_WAIT = 5


def prepare_request(data):
    """Match native JSON rendering while retaining original score legends."""
    legends = {}
    for qid, question in data.get('questions', {}).items():
        if not isinstance(question, dict):
            continue
        criteria = question.get('criteria')
        kind = question.get('type')
        def render(value):
            return value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
        if kind == 'choice' and isinstance(criteria, dict):
            question['criteria'] = {key: (value if value is None or value == '' else render(value))
                                    for key, value in criteria.items()}
        elif kind == 'score' and isinstance(criteria, list):
            legends[qid] = {str(i): value for i, value in enumerate(criteria)}
            question['criteria'] = [render(value) for value in criteria]
        elif kind == 'noul' and isinstance(criteria, dict):
            question['criteria'] = {key: (value if value is None or value == '' else render(value))
                                    for key, value in criteria.items()}
    return legends


def normalize(payload, legends=None):
    if not isinstance(payload, dict) or not isinstance(payload.get('answers'), dict):
        raise ValueError('Invalid worker response')
    payload['model'] = 'laya'
    for qid, answer in payload['answers'].items():
        if not isinstance(answer, dict):
            raise ValueError('Invalid worker answer')
        if answer.get('type') == 'score' and legends and qid in legends:
            answer['legend'] = legends[qid]
        if 'rl_agent' in answer:
            answer['action'] = answer.pop('rl_agent')
        if answer.get('type') == 'noul' and 'confidence' not in answer:
            p = answer.get('noul')
            if isinstance(p, bool) or not isinstance(p, (int, float)) or not 0 <= p <= 1:
                raise ValueError('Invalid noul probability')
            answer['confidence'] = round(max(p, 1 - p), 4)
    return payload


def worker_request(port, method, path, body=None, timeout=TIMEOUT):
    request = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=body, method=method,
                                     headers={'Content-Type': 'application/json'})
    with WORKER_OPENER.open(request, timeout=timeout) as response:
        data = response.read(MAX_RESPONSE + 1)
        if len(data) > MAX_RESPONSE:
            raise ValueError('Worker response too large')
        return json.loads(data)


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 8
    def __init__(self, address, worker_port, key):
        super().__init__(address, Handler)
        self.worker_port, self.key = worker_port, key
        self.slots = threading.BoundedSemaphore(8)
        self.inference = threading.Lock()
        self.worker = None
    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise
    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(10)
    def log_message(self, format, *args):
        pass
    def reply(self, status, body):
        encoded = json.dumps(body, ensure_ascii=False, allow_nan=False).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
    def authorized(self):
        key = self.server.key
        return not key or hmac.compare_digest(self.headers.get('Authorization', ''), 'Bearer ' + key)
    def do_GET(self):
        if self.path == '/health':
            try:
                if self.server.worker is not None and (self.server.worker.poll() is not None or not owns_listener(self.server.worker, self.server.worker_port)):
                    raise RuntimeError('Worker exited or lost its listener')
                health = worker_request(self.server.worker_port, 'GET', '/health', timeout=3)
                validate_health(health)
                self.reply(200, {'status':'ok', 'model':'laya', 'variant':'english', 'backend':'MLX GPU FP32'})
            except Exception:
                self.reply(503, {'error':'MLX worker unavailable'})
        elif self.path == '/v1/models':
            if not self.authorized():
                return self.reply(401, {'error':'Unauthorized'})
            self.reply(200, {'object':'list', 'data':[{'id':'laya', 'object':'model'}]})
        else:
            self.reply(404, {'error':'Not found'})
    def do_POST(self):
        if self.path != '/v1/systemone':
            return self.reply(404, {'error':'Not found'})
        if not self.authorized():
            return self.reply(401, {'error':'Unauthorized'})
        try:
            if self.headers.get('Transfer-Encoding'):
                return self.reply(400, {'error':'Transfer encoding unsupported'})
            length = int(self.headers.get('Content-Length', ''))
            if length < 0 or length > MAX_BODY:
                return self.reply(413, {'error':'Request too large'})
            body = self.rfile.read(length)
            if len(body) != length:
                raise ValueError('Incomplete request')
            data = json.loads(body)
            if not isinstance(data, dict) or data.get('model', 'laya') != 'laya':
                raise ValueError('Unknown model or invalid request')
            data.pop('model', None)
            if 'state' not in data or not isinstance(data.get('questions'), dict):
                raise ValueError('state and questions are required')
            if len(data['questions']) > 8:
                raise ValueError('Too many questions')
        except (ValueError, UnicodeError, TimeoutError):
            return self.reply(400, {'error':'Invalid request'})
        legends = prepare_request(data)
        if not self.server.inference.acquire(timeout=QUEUE_WAIT):
            return self.reply(503, {'error':'MLX worker busy'})
        try:
            if self.server.worker is not None and (self.server.worker.poll() is not None or not owns_listener(self.server.worker, self.server.worker_port)):
                raise RuntimeError('Worker exited or lost its listener')
            response = worker_request(self.server.worker_port, 'POST', '/v1/decisions', json.dumps(data, ensure_ascii=False).encode())
            self.reply(200, normalize(response, legends))
        except urllib.error.HTTPError as e:
            message = None
            if 400 <= e.code < 500:
                try:
                    raw = e.read(4097)
                    if len(raw) <= 4096:
                        candidate = json.loads(raw).get('error')
                        if isinstance(candidate, str) and re.fullmatch(
                            r'Question exceeds (?:option token limit \(48\)|option token budget \(\d+ tokens per option\)|heading token budget \(\d+ tokens\)|question head token budget \(\d+ tokens\)|state context limit \(\d+ tokens\)|final sequence limit \(\d+ tokens\))', candidate
                        ):
                            message = candidate
                except (ValueError, AttributeError, OSError):
                    pass
            if message:
                self.reply(422, {'error':message})
            else:
                self.reply(400 if 400 <= e.code < 500 else 502, {'error':'MLX worker rejected request'})
        except Exception:
            self.reply(502, {'error':'MLX worker unavailable or returned invalid output'})
        finally:
            self.server.inference.release()


def owns_listener(child, port):
    """Verify the owned child, not a competing process, holds the worker socket."""
    try:
        sockets = set()
        for fd in Path(f'/proc/{child.pid}/fd').iterdir():
            try: sockets.add(os.readlink(fd))
            except OSError: continue
        with open('/proc/net/tcp') as table:
            for line in table.readlines()[1:]:
                fields = line.split()
                if (fields[1] == f'0100007F:{port:04X}' and fields[3] == '0A'
                        and f'socket:[{fields[9]}]' in sockets):
                    return True
    except (OSError, IndexError):
        pass
    return False


def validate_health(health):
    if (health.get('status') != 'ok' or health.get('dtype') != 'float32'
            or health.get('backend') != 'mlx-omarchy' or 'gpu' not in str(health.get('device','')).lower()
            or Path(health.get('checkpoint', '')).resolve() != MODEL.resolve()):
        raise ValueError('Worker is not the expected FP32 GPU model')


def main():
    global PYTHON, SERVE, MODEL
    parser = argparse.ArgumentParser()
    parser.add_argument('--host', default='127.0.0.1', choices=['127.0.0.1'])
    parser.add_argument('--port', type=int, default=8080)
    parser.add_argument('--worker-port', type=int, default=8082)
    parser.add_argument('--vulkan', action='store_true', help='Monitter compatibility: use the MLX GPU')
    parser.add_argument('--server', action='store_true', help='Monitter compatibility')
    parser.add_argument('--model', help='Converted Laya model directory')
    args = parser.parse_args()
    try:
        PYTHON, SERVE, MODEL = load_runtime()
    except RuntimeError as exc:
        parser.error(str(exc))
    if args.model is not None and Path(args.model).resolve() != MODEL.resolve():
        parser.error('Only the verified converted Laya model is supported')
    if not 1 <= args.port <= 65535 or not 1 <= args.worker_port <= 65535 or args.port == args.worker_port:
        parser.error('Ports must be distinct and valid')
    server = Server((args.host, args.port), args.worker_port, os.environ.get('LAYA_API_KEY', ''))
    # Refuse an occupied worker port rather than connecting to or stopping its owner.
    with socket.socket() as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try: probe.bind(('127.0.0.1', args.worker_port))
        except OSError: raise SystemExit('Worker port is already occupied')
    env = os.environ.copy()
    env.update({'PYTHONPATH':str(SERVE), 'HF_HUB_OFFLINE':'1', 'TRANSFORMERS_OFFLINE':'1', 'MLX_OMARCHY_OFFLINE':'1'})
    env.pop('LAYA_API_KEY', None)
    serving = False
    stopped = threading.Event()
    def stop(_signal, _frame):
        stopped.set()
        if serving:
            threading.Thread(target=server.shutdown, daemon=True).start()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    child = subprocess.Popen([str(PYTHON), str(Path(__file__).resolve().with_name('worker.py')), '--model', str(MODEL), '--host', '127.0.0.1', '--port', str(args.worker_port), '--dtype', 'float32', '--max-questions', '8'], env=env)
    server.worker = child
    try:
        deadline = time.monotonic() + 120
        while True:
            if stopped.is_set(): raise RuntimeError('Interrupted during MLX startup')
            if child.poll() is not None: raise RuntimeError('MLX worker exited during startup')
            try:
                validate_health(worker_request(args.worker_port, 'GET', '/health', timeout=2))
                if child.poll() is not None or not owns_listener(child, args.worker_port):
                    raise RuntimeError('Expected worker does not own its listener')
                break
            except (OSError, ValueError, urllib.error.URLError):
                if time.monotonic() >= deadline: raise RuntimeError('MLX worker startup timed out')
                time.sleep(.2)
        serving = True
        if stopped.is_set():
            threading.Thread(target=server.shutdown, daemon=True).start()
        server.serve_forever(poll_interval=.2)
    finally:
        server.server_close()
        if child.poll() is None:
            child.terminate()
            try: child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill(); child.wait()

if __name__ == '__main__':
    main()
