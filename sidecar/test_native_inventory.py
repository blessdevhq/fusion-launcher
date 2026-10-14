"""Manual native acceptance: run this file directly with CPython and libtorrent 2.0.11.

The dependency may be installed in the interpreter or provided through
FUSION_NATIVE_LIBTORRENT_PATH. The default optional wheel directory is
.tmp/acceptance/native-sidecar. Uses only a loopback seeder with generated data;
Set FUSION_NATIVE_SIDECAR_EXE to test the production PyInstaller executable.
This verifies the sidecar protocol with real libtorrent; Tauri process-tree
shutdown is tested separately by the native Windows job tests.
"""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]


def until(condition, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(.05)
    raise TimeoutError('native condition timed out')


def main():
    binding_path = Path(os.environ.get('FUSION_NATIVE_LIBTORRENT_PATH', ROOT / '.tmp' / 'acceptance' / 'native-sidecar')).resolve()
    sys.path.insert(0, str(binding_path))
    import libtorrent as lt
    import torrent_sidecar as implementation
    (ROOT / '.tmp' / 'acceptance').mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='native-inventory-', dir=ROOT / '.tmp' / 'acceptance') as temporary:
        base = Path(temporary)
        source = base / 'seed' / 'legal-generated-game'
        source.mkdir(parents=True)
        with (source / 'payload.bin').open('wb') as payload:
            block = os.urandom(1024 * 1024)
            for _ in range(256 if os.environ.get('FUSION_NATIVE_MANAGER_TEST_BINARY') else 128):
                payload.write(block)
            payload.write(block[:1234])
        (source / 'empty.bin').touch()
        (source / 'second-unaligned.bin').write_bytes(block[:1234])
        files = lt.file_storage()
        lt.add_files(files, str(source))
        torrent = lt.create_torrent(files, 256 * 1024)
        lt.set_piece_hashes(torrent, str(source.parent))
        info = lt.torrent_info(torrent.generate())
        storage = info.files()
        pad_files = sum(bool(storage.file_flags(index) & lt.file_storage.flag_pad_file)
                        for index in range(storage.num_files()))
        payload_size = sum(storage.file_size(index) for index in range(storage.num_files())
                           if not storage.file_flags(index) & lt.file_storage.flag_pad_file)
        assert pad_files > 0, 'Native fixture must exercise hybrid/v2 alignment padding'
        original = lt.parse_magnet_uri(lt.make_magnet_uri(info))
        for mapped in (str(base / 'foreign-canary.bin'), '../foreign-canary.bin'):
            unsafe = lt.add_torrent_params()
            unsafe.ti = info
            unsafe.info_hashes = info.info_hashes()
            unsafe.renamed_files = {0: mapped}
            restored = lt.read_resume_data(lt.write_resume_data_buf(unsafe))
            try:
                implementation.validate_resume(restored, original)
                raise AssertionError('Native mapped resume was accepted')
            except ValueError:
                pass
        print(json.dumps({'native_resume_mapping_rejected_before_add': True}), flush=True)
        settings = {
            'listen_interfaces': '127.0.0.1:0',
            'enable_dht': False, 'enable_lsd': False,
            'enable_upnp': False, 'enable_natpmp': False,
            'out_enc_policy': lt.enc_policy.forced,
            'in_enc_policy': lt.enc_policy.forced,
            'allowed_enc_level': lt.enc_level.both,
            'alert_mask': 0x7fffffff,
        }
        seeder = lt.session(settings)
        params = lt.add_torrent_params()
        params.ti = info
        params.save_path = str(source.parent)
        params.flags &= ~lt.torrent_flags.paused
        params.flags &= ~lt.torrent_flags.auto_managed
        handle = seeder.add_torrent(params)
        until(lambda: handle.status().is_seeding)
        handle.set_upload_limit(8 * 1024 * 1024)
        port = until(lambda: seeder.listen_port())
        magnet = lt.make_magnet_uri(info) + '&x.pe=127.0.0.1:' + str(port)
        print(json.dumps({'seeder_port': port, 'fixture_bytes': payload_size,
                          'pad_files': pad_files, 'info_hash': str(info.info_hash())}), flush=True)
        manager_test = os.environ.get('FUSION_NATIVE_MANAGER_TEST_BINARY')
        if manager_test:
            environment = os.environ.copy()
            environment['FUSION_NATIVE_MAGNET'] = magnet
            process = subprocess.Popen([manager_test,
                'torrent::runtime_tests::native_packaged_sidecar_manager_pause_resume_and_remove',
                '--ignored', '--nocapture'], env=environment)
            def manager_finished():
                for alert in seeder.pop_alerts():
                    if isinstance(alert, (lt.peer_disconnected_alert, lt.peer_error_alert, lt.peer_connect_alert)):
                        print('SEED ' + alert.message(), flush=True)
                return process.poll() is not None
            try:
                until(manager_finished, timeout=180)
                assert process.returncode == 0, 'Native TorrentManager acceptance failed'
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
            seeder.pause()
            return
        destination = base / 'destination'
        destination.mkdir()
        executable = os.environ.get('FUSION_NATIVE_SIDECAR_EXE')
        launch_code = "import sys,runpy; sys.path.insert(0,sys.argv.pop(1)); runpy.run_path(sys.argv.pop(1),run_name='__main__')"
        command = ([str(Path(executable).resolve()), magnet, str(destination)] if executable else
                   [sys.executable, '-c', launch_code, str(binding_path),
                    str(ROOT / 'sidecar' / 'torrent_sidecar.py'), magnet, str(destination)])
        def spawn():
            return subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True)
        process = spawn()
        output = queue.Queue()
        diagnostics = []
        def read_stdout(proc, messages):
            for line in proc.stdout:
                messages.put(json.loads(line))
        def read_stderr(proc, messages):
            for line in proc.stderr:
                messages.append(line)
        def start_readers():
            threading.Thread(target=read_stdout, args=(process, output), daemon=True).start()
            threading.Thread(target=read_stderr, args=(process, diagnostics), daemon=True).start()
        start_readers()
        def next_inventory():
            if process.poll() is not None:
                raise RuntimeError('sidecar exited: ' + ''.join(diagnostics))
            try:
                payload = output.get_nowait()
                return payload.get('inventory')
            except queue.Empty:
                return None
        try:
            inventory = until(next_inventory)
            before = [str(path.relative_to(destination)) for path in destination.rglob('*')]
            print(json.dumps({'version': lt.version, 'executable': executable or sys.executable,
                              'fixture_bytes': payload_size,
                              'inventory': inventory, 'before_ack_files': before}), flush=True)
            assert before == [], 'Native libtorrent created files before ownership approval'
            for relative in inventory:
                path = destination / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                with path.open('xb'):
                    pass
            identities = {relative: (destination / relative).stat().st_ino for relative in inventory}
            process.stdin.write('{"action":"inventory_approved"}\n')
            process.stdin.flush()
            paused_bytes = 0
            def bytes_received():
                nonlocal paused_bytes
                for alert in seeder.pop_alerts():
                    if isinstance(alert, (lt.peer_disconnected_alert, lt.peer_error_alert, lt.peer_connect_alert)):
                        print('SEED ' + alert.message(), flush=True)
                try:
                    state = output.get_nowait()
                    assert not (state.get('state') in ('finished', 'seeding') and state.get('total_bytes') == 0)
                    paused_bytes = state.get('downloaded_bytes', 0)
                    return paused_bytes >= 100 * 1024 * 1024
                except queue.Empty:
                    return False
            until(bytes_received)
            process.stdin.write('{"action":"pause"}\n')
            process.stdin.flush()
            code = process.wait(timeout=10)
            assert code == 0, ''.join(diagnostics)
            def final_pause_status():
                try:
                    state = output.get_nowait()
                    return state if state.get('state') == 'paused' else None
                except queue.Empty:
                    return None
            paused_bytes = until(final_pause_status, timeout=2)['downloaded_bytes']
            for relative in inventory:
                assert (destination / relative).stat().st_ino == identities[relative], relative
            resume = next(destination.glob('*.fastresume'))
            assert resume.stat().st_size > 0, ''.join(diagnostics)
            assert lt.read_resume_data(resume.read_bytes()).ti is not None, 'Fastresume must cache metadata'
            if os.environ.get('FUSION_NATIVE_STRIP_RESUME_METADATA'):
                legacy_resume = lt.bdecode(resume.read_bytes())
                for key in (b'info', b'peers', b'peers6'):
                    legacy_resume.pop(key, None)
                resume.write_bytes(lt.bencode(legacy_resume))
                assert lt.read_resume_data(resume.read_bytes()).ti is None
                print(json.dumps({'legacy_fastresume_without_metadata_or_peers': True}), flush=True)
            print(json.dumps({'paused_exit': code, 'paused_bytes': paused_bytes,
                              'fastresume_size': resume.stat().st_size, 'identities_preserved': True}), flush=True)
            output = queue.Queue()
            diagnostics = []
            resumed_at = time.monotonic()
            process = spawn()
            start_readers()
            resumed_inventory = until(next_inventory)
            assert resumed_inventory == inventory
            for relative in inventory:
                assert (destination / relative).stat().st_ino == identities[relative], relative
            process.stdin.write('{"action":"inventory_approved"}\n')
            process.stdin.flush()
            resumed_bytes = 0
            def resume_progress():
                nonlocal resumed_bytes
                try:
                    state = output.get_nowait()
                    assert not (state.get('state') in ('finished', 'seeding') and state.get('total_bytes') == 0)
                    resumed_bytes = state.get('downloaded_bytes', 0)
                    return resumed_bytes >= paused_bytes + 4 * 1024 * 1024
                except queue.Empty:
                    return False
            until(resume_progress)
            elapsed = time.monotonic() - resumed_at
            process.stdin.write('{"action":"stop"}\n')
            process.stdin.flush()
            assert process.wait(timeout=10) == 0
            until(lambda: any('loaded fastresume data' in line for line in diagnostics), timeout=2)
            for relative in inventory:
                assert (destination / relative).stat().st_ino == identities[relative], relative
            assert lt.read_resume_data(resume.read_bytes()).ti is not None, 'Resumed save must cache metadata'
            print(json.dumps({'resume_to_next_4MiB_seconds': round(elapsed, 3),
                              'resumed_bytes': resumed_bytes, 'loaded_fastresume': True,
                              'stop_exit': 0, 'identities_preserved': True}), flush=True)
        finally:
            if process.poll() is None:
                if os.name == 'nt':
                    subprocess.run(['taskkill', '/T', '/F', '/PID', str(process.pid)],
                                   capture_output=True, check=False)
                else:
                    process.kill()
                process.wait(timeout=10)
            print(''.join(diagnostics), file=sys.stderr)
        seeder.pause()
        del handle
        del seeder


if __name__ == '__main__':
    main()
