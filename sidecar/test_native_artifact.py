"""Native single-file artifact acceptance using generated data and loopback peers.

Run with the isolated sidecar build venv. FUSION_NATIVE_SIDECAR_EXE selects the
rebuilt PyInstaller binary; FUSION_NATIVE_ADAPTER_TEST_BINARY additionally runs
the ignored Rust adapter/Windows Job publication test against this live seeder.
No commercial payload, public tracker, or external WebSeed is involved.
"""

import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

import libtorrent as lt


ROOT = Path(__file__).resolve().parents[1]


def until(predicate, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.05)
    raise TimeoutError('native artifact condition timed out')


def main():
    (ROOT / '.tmp' / 'acceptance').mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='native-artifact-', dir=ROOT / '.tmp' / 'acceptance') as temporary:
        base = Path(temporary)
        source = base / 'seed'
        source.mkdir()
        data = os.urandom(16 * 1024 * 1024 + 1234)
        (source / 'payload.bin').write_bytes(data)
        expected = hashlib.sha256(data).hexdigest()
        storage = lt.file_storage()
        storage.add_file('payload.bin', len(data))
        torrent = lt.create_torrent(storage, 256 * 1024, lt.create_torrent.v1_only)
        lt.set_piece_hashes(torrent, str(source))
        info = lt.torrent_info(torrent.generate())
        seeder = lt.session({
            'listen_interfaces': '127.0.0.1:0',
            'enable_dht': False, 'enable_lsd': False, 'enable_upnp': False, 'enable_natpmp': False,
            'out_enc_policy': lt.enc_policy.forced, 'in_enc_policy': lt.enc_policy.forced,
            'allowed_enc_level': lt.enc_level.both,
        })
        params = lt.add_torrent_params()
        params.ti = info
        params.save_path = str(source)
        params.flags &= ~lt.torrent_flags.paused
        params.flags &= ~lt.torrent_flags.auto_managed
        handle = seeder.add_torrent(params)
        until(lambda: handle.status().is_seeding)
        port = until(seeder.listen_port)
        magnet = lt.make_magnet_uri(info) + '&x.pe=127.0.0.1:' + str(port)
        executable = os.environ.get('FUSION_NATIVE_SIDECAR_EXE')
        if executable:
            executable = str(Path(executable).resolve())
            probe = subprocess.run([executable, '--probe'], check=True, capture_output=True, text=True, timeout=5)
            document = json.loads(probe.stdout)
            assert document['artifact_v1'] and document['explicit_webseeds']
            print(json.dumps({'packaged_probe': document}), flush=True)
        adapter_binary = os.environ.get('FUSION_NATIVE_ADAPTER_TEST_BINARY')
        if adapter_binary:
            environment = os.environ.copy()
            environment.update(FUSION_NATIVE_MAGNET=magnet, FUSION_NATIVE_SHA256=expected)
            subprocess.run([str(Path(adapter_binary).resolve()),
                            'fusion_source::adapters::torrent::tests::native_artifact_adapter_stops_and_publishes',
                            '--ignored', '--nocapture'], check=True, env=environment, timeout=180)

        approved_requests = []
        unapproved_requests = []

        class RelayFixture(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def do_GET(self):
                if self.path != '/private-token/payload.bin':
                    unapproved_requests.append(self.path)
                    self.send_error(403)
                    return
                value = self.headers.get('Range')
                start, end = 0, len(data) - 1
                if value:
                    parts = value.removeprefix('bytes=').split('-')
                    start, end = int(parts[0]), int(parts[1]) if parts[1] else len(data) - 1
                assert 0 <= start <= end < len(data)
                approved_requests.append((start, end))
                self.send_response(206 if value else 200)
                if value:
                    self.send_header('Content-Range', f'bytes {start}-{end}/{len(data)}')
                self.send_header('Content-Length', str(end - start + 1))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(data[start:end + 1])

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(('127.0.0.1', 0), RelayFixture)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        origin = f'http://127.0.0.1:{server.server_port}'
        destination = base / 'destination'
        destination.mkdir()
        # A native magnet WebSeed must be replaced by explicit relay admission.
        requested = magnet + '&ws=' + origin + '/unapproved'
        command = ([executable] if executable else [sys.executable, str(ROOT / 'sidecar' / 'torrent_sidecar.py')])
        command += ['--artifact', '--web-seed', origin + '/private-token/payload.bin', requested, str(destination)]
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        output = queue.Queue()
        diagnostics = []

        def stdout_reader():
            for line in process.stdout:
                output.put(json.loads(line))

        def stderr_reader():
            for line in process.stderr:
                diagnostics.append(line)
                if os.environ.get('FUSION_NATIVE_VERBOSE'):
                    print(line, file=sys.stderr, end='', flush=True)

        threading.Thread(target=stdout_reader, daemon=True).start()
        threading.Thread(target=stderr_reader, daemon=True).start()

        def next_payload(predicate):
            if process.poll() is not None:
                raise RuntimeError('sidecar exited: ' + ''.join(diagnostics))
            while True:
                try:
                    payload = output.get_nowait()
                except queue.Empty:
                    return None
                if predicate(payload):
                    return payload

        try:
            metadata = until(lambda: next_payload(lambda payload: payload.get('artifact_layout')))
            print(json.dumps({'native_metadata': metadata['artifact_layout']}), flush=True)
            assert metadata['artifact_layout'] == {'info_hash': str(info.info_hash()),
                                                  'files': [{'path': 'payload.bin', 'size': len(data)}]}
            assert list(destination.iterdir()) == [], 'payload writes before approval'
            assert approved_requests == [] and unapproved_requests == [], 'WebSeeds fetched before approval'
            payload_path = destination / 'payload.bin'
            with payload_path.open('xb'):
                pass
            original_identity = payload_path.stat().st_ino
            # Metadata has arrived. Disable all peer piece delivery so successful
            # completion must use the explicitly admitted BEP 19 loopback URL.
            seeder.pause()
            process.stdin.write('{"action":"inventory_approved"}\n')
            process.stdin.flush()
            finished = until(lambda: next_payload(lambda payload: payload['state'] in ('finished', 'seeding')
                                                   and payload['downloaded_bytes'] == len(data)))
            process.stdin.write('{"action":"stop"}\n')
            process.stdin.flush()
            process.wait(timeout=10)
            assert process.returncode == 0, ''.join(diagnostics)
            assert hashlib.sha256(payload_path.read_bytes()).hexdigest() == expected
            assert payload_path.stat().st_ino == original_identity
            assert [path.name for path in destination.iterdir()] == ['payload.bin'], 'artifact mode wrote resume state'
            assert approved_requests and not unapproved_requests
            print(json.dumps({'native_artifact_webseed': True, 'libtorrent': lt.version,
                              'range_requests': len(approved_requests), 'unapproved_requests': len(unapproved_requests),
                              'bytes': finished['downloaded_bytes'], 'sha256': expected}), flush=True)
        except Exception:
            print(''.join(diagnostics), file=sys.stderr, flush=True)
            raise
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == '__main__':
    main()
