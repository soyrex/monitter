from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
import base64
class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        valid = self.headers.get('Authorization') == 'Basic ' + base64.b64encode(b'monitter-test:basic-auth-test-only').decode()
        protected = self.path.startswith('/private')
        code = 401 if protected and not valid else 200
        print(self.path.split('?')[0], code, flush=True)
        self.send_response(code)
        if code == 401: self.send_header('WWW-Authenticate', 'Basic realm="Monitter local test"')
        self.send_header('Content-Type','text/html; charset=utf-8')
        self.end_headers()
        title = 'Basic Auth verified' if protected and valid else 'Authentication required' if code == 401 else 'Public test page'
        self.wfile.write(('<html><title>'+title+'</title><body style="font:24px system-ui;padding:40px"><h1>'+title+'</h1><a href="/private">Protected page</a> | <a href="/public">Public page</a></body></html>').encode())
server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
print('PORT',server.server_port,flush=True)
server.serve_forever()
