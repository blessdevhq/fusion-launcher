"""Loopback TLS fixture: authored payload, no public network or production trust."""
import http.server
import pathlib
import ssl
import sys

root = pathlib.Path(sys.argv[1]).resolve()
class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def _send_bytes(self, code, body):
        self.send_response(code)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def do_GET(self):
        if self.path in ('/index.json', '/game.json'):
            self._send_bytes(200, (root / self.path[1:]).read_bytes())
            return
        mode = self.path.split('/')[-1]
        if mode == 'redirect':
            self.send_response(302)
            self.send_header('Location', 'http://acquisition.test:9/payload')
            self.end_headers()
            return
        rel = self.path.split('?', 1)[0].split('#', 1)[0].lstrip('/')
        if mode not in ('oversized', 'chunked-limit', 'truncated', 'eof') and rel and '..' not in rel.split('/') and '\\' not in rel:
            candidate = (root / rel).resolve()
            if candidate.is_relative_to(root) and candidate.is_file():
                self._send_bytes(200, candidate.read_bytes())
                return
        payload_path = root / 'payload.zip'
        if not payload_path.is_file():
            self._send_bytes(404, b'')
            return
        payload = payload_path.read_bytes()
        self.send_response(200)
        if mode == 'oversized':
            self.send_header('Content-Length', str(1 << 40))
        elif mode == 'chunked-limit':
            self.send_header('Transfer-Encoding', 'chunked')
        elif mode != 'eof':
            self.send_header('Content-Length', str(len(payload)))
        self.send_header('Connection', 'close')
        self.end_headers()
        if mode == 'chunked-limit':
            for _ in range(32):
                self.wfile.write(b'1000\r\n' + b'x' * 4096 + b'\r\n')
            self.wfile.write(b'0\r\n\r\n')
        elif mode in ('truncated', 'eof'):
            self.wfile.write(payload[:len(payload) // 2])
        else:
            self.wfile.write(payload)
        self.wfile.flush()
    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(sys.argv[2], sys.argv[3])
server.socket = context.wrap_socket(server.socket, server_side=True)
print(server.server_port, flush=True)
server.serve_forever()
