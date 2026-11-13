"""Loopback TLS fixture: authored payload, no public network or production trust."""
import http.server
import pathlib
import ssl
import sys

root = pathlib.Path(sys.argv[1])
class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def do_GET(self):
        if self.path in ('/index.json', '/game.json'):
            body = (root / self.path[1:]).read_bytes()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        payload = (root / 'payload.zip').read_bytes()
        mode = self.path.split('/')[-1]
        if mode == 'redirect':
            self.send_response(302)
            self.send_header('Location', 'http://acquisition.test:9/payload')
            self.end_headers()
            return
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
