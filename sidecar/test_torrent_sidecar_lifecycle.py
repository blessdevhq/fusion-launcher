"""Stdin lifecycle regressions; libtorrent/native network are not required."""

import importlib.util
import io
from pathlib import Path
import sys
import threading
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch


with patch.dict(sys.modules, {"libtorrent": Mock()}):
    spec = importlib.util.spec_from_file_location(
        "torrent_sidecar", Path(__file__).with_name("torrent_sidecar.py")
    )
    sidecar = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sidecar)


def hashes(v1="requested", v2=None):
    return SimpleNamespace(has_v1=lambda: v1 is not None, has_v2=lambda: v2 is not None,
                           v1=v1, v2=v2)


class StdinLifecycleTests(unittest.TestCase):
    def test_resume_rejects_mapped_paths_before_add(self):
        for mapped in ("C:/foreign/payload.bin", "../foreign.bin", "renamed.bin"):
            with self.subTest(mapped=mapped):
                params = SimpleNamespace(renamed_files={0: mapped}, info_hashes=hashes(), ti=None)
                with self.assertRaises(ValueError):
                    sidecar.validate_resume(params, SimpleNamespace(info_hashes=hashes()))

    def test_resume_rejects_mismatched_state_or_cached_metadata_hashes(self):
        for saved, metadata in ((hashes("other"), None), (hashes(), Mock(info_hashes=lambda: hashes("other")))):
            with self.subTest(saved=saved, metadata=metadata):
                with self.assertRaises(ValueError):
                    sidecar.validate_resume(SimpleNamespace(renamed_files={}, info_hashes=saved, ti=metadata),
                                            SimpleNamespace(info_hashes=hashes()))

    def test_v1_magnet_accepts_matching_hybrid_resume_metadata(self):
        hybrid = hashes("requested", "v2-cached")
        sidecar.validate_resume(SimpleNamespace(renamed_files={}, info_hashes=hybrid,
                                               ti=Mock(info_hashes=lambda: hybrid)),
                                SimpleNamespace(info_hashes=hashes()))

    def test_invalid_resume_falls_back_to_original_magnet_before_native_add(self):
        invalid_resumes = [SimpleNamespace(renamed_files={0: "C:/foreign.bin"}),
                           SimpleNamespace(renamed_files={}, info_hashes=hashes("other"), ti=None)]
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent.parent / "src-tauri" / "target") as directory:
            path = Path(directory) / "resume.fastresume"
            path.write_bytes(b"invalid cached resume")
            for invalid in invalid_resumes:
                magnet = SimpleNamespace(info_hashes=hashes(), flags=0, file_priorities=[])
                session = Mock()
                with patch.object(sidecar.lt, "parse_magnet_uri", return_value=magnet), \
                     patch.object(sidecar.lt, "read_resume_data", return_value=invalid), \
                     patch.object(sidecar.lt, "torrent_flags", Mock(upload_mode=1, default_dont_download=2,
                                                                   auto_managed=4, paused=8, seed_mode=16)):
                    sidecar.add_magnet_with_resume(session, "magnet:?xt=test", directory, str(path))
                session.add_torrent.assert_called_once_with(magnet)

    def test_payload_priorities_leave_hybrid_padding_disabled(self):
        handle = Mock()
        storage = handle.torrent_file.return_value.files.return_value
        storage.num_files.return_value = 4
        storage.file_flags.side_effect = [0, 1, 0, 1]
        with patch.object(sidecar.lt, "file_storage", Mock(flag_pad_file=1)):
            self.assertEqual(sidecar.payload_priorities(handle), [4, 0, 4, 0])

    def test_resume_keeps_explicit_magnet_discovery_when_saved_peers_are_empty(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent.parent / "src-tauri" / "target") as directory:
            path = Path(directory) / "resume.fastresume"
            path.write_bytes(b"saved without metadata or peers")
            saved = SimpleNamespace(peers=[], trackers=["https://old"], tracker_tiers=[3],
                                    url_seeds=[], flags=0, file_priorities=[4], renamed_files={},
                                    info_hashes=hashes(), ti=None)
            magnet = SimpleNamespace(peers=[("127.0.0.1", 54321)],
                                     trackers=["https://old", "https://new"], tracker_tiers=[0, 2],
                                     url_seeds=["https://seed"], info_hashes=hashes())
            session = Mock()
            with patch.object(sidecar.lt, "parse_magnet_uri", return_value=magnet), \
                 patch.object(sidecar.lt, "read_resume_data", return_value=saved), \
                 patch.object(sidecar.lt, "torrent_flags", Mock(upload_mode=1, default_dont_download=2,
                                                               auto_managed=4, paused=8, seed_mode=16)):
                sidecar.add_magnet_with_resume(session, "magnet:?xt=test", directory, str(path))
            self.assertEqual(saved.peers, [("127.0.0.1", 54321)])
            self.assertEqual(saved.trackers, ["https://old", "https://new"])
            self.assertEqual(saved.tracker_tiers, [3, 2])
            self.assertEqual(saved.url_seeds, ["https://seed"])
            self.assertEqual(saved.file_priorities, [0])
            session.add_torrent.assert_called_once_with(saved)

    def test_fastresume_save_requests_metadata_for_periodic_and_final_saves(self):
        handle = Mock()
        handle.is_valid.return_value = True
        handle.status.return_value.has_metadata = True
        with patch.object(sidecar, "get_resume_flag", side_effect=lambda name: {
                "only_if_modified": 4, "flush_disk_cache": 1, "save_info_dict": 2}[name]):
            sidecar.request_save_resume(handle, only_if_modified=True)
            sidecar.request_save_resume(handle, only_if_modified=False)
        self.assertEqual([call.args[0] for call in handle.save_resume_data.call_args_list], [6, 3])

    def test_approval_before_metadata_cannot_enable_payload(self):
        approved = threading.Event()
        with patch.object(sys, "stdin", io.StringIO('{"action":"inventory_approved"}\n')):
            sidecar.listen_stdin(Mock(), [False], threading.Event(), threading.Event(), approved)
        self.assertFalse(approved.is_set())

    def test_approval_after_metadata_enables_payload(self):
        ready = threading.Event()
        ready.set()
        approved = threading.Event()
        with patch.object(sys, "stdin", io.StringIO('{"action":"inventory_approved"}\n')):
            sidecar.listen_stdin(Mock(), [False], threading.Event(), ready, approved)
        self.assertTrue(approved.is_set())

    def test_payload_gate_is_set_before_add_and_cannot_auto_clear(self):
        flags = Mock(upload_mode=4, auto_managed=8, paused=16, default_dont_download=64, seed_mode=1)
        params = Mock(flags=1 | 8 | 16 | 32, file_priorities=[4, 4])
        with patch.object(sidecar.lt, "torrent_flags", flags):
            sidecar.hold_payload(params)
        self.assertEqual(params.flags, 4 | 32 | 64)
        self.assertEqual(params.file_priorities, [0, 0])

    def test_fastresume_save_keeps_reserved_identity_and_requires_existing_file(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent.parent / "src-tauri" / "target") as directory:
            path = Path(directory) / "resume.fastresume"
            with self.assertRaises(FileNotFoundError):
                sidecar.write_resume(str(path), b"resume")
            path.write_bytes(b"old resume with trailing bytes")
            identity = path.stat().st_ino
            sidecar.write_resume(str(path), b"new resume")
            self.assertEqual(path.read_bytes(), b"new resume")
            self.assertEqual(path.stat().st_ino, identity)
            self.assertFalse(path.with_suffix(".fastresume.tmp").exists())

    def test_inventory_is_metadata_declared_and_includes_resume(self):
        handle = Mock()
        handle.status.return_value.has_metadata = True
        files = handle.torrent_file.return_value.files.return_value
        files.num_files.return_value = 2
        files.file_flags.side_effect = [0, 1]
        files.file_path.return_value = "game/payload.bin"
        flags = Mock(flag_pad_file=1, flag_symlink=2)
        with patch.object(sidecar.lt, "file_storage", flags):
            inventory = sidecar.metadata_inventory(handle, "C:/root/.hash.fastresume")
        self.assertEqual(inventory, ["game/payload.bin", ".hash.fastresume"])

    def test_metadata_symlink_is_rejected_before_approval(self):
        handle = Mock()
        handle.status.return_value.has_metadata = True
        files = handle.torrent_file.return_value.files.return_value
        files.num_files.return_value = 1
        files.file_flags.return_value = 2
        with patch.object(sidecar.lt, "file_storage", Mock(flag_pad_file=1, flag_symlink=2)):
            with self.assertRaises(ValueError):
                sidecar.metadata_inventory(handle, None)

    def test_parent_eof_requests_normal_shutdown(self):
        stop_event = threading.Event()
        with patch.object(sys, "stdin", io.StringIO("")):
            sidecar.listen_stdin(Mock(), [False], stop_event)
        self.assertTrue(stop_event.is_set())

    def test_pause_preserves_normal_shutdown_path(self):
        handle = Mock()
        stop_event = threading.Event()
        paused = [False]
        with patch.object(sys, "stdin", io.StringIO('{"action":"pause"}\n')):
            sidecar.listen_stdin(handle, paused, stop_event)
        handle.pause.assert_called_once()
        self.assertTrue(paused[0])
        self.assertTrue(stop_event.is_set())

    def test_stop_quiesces_torrent_before_fastresume_shutdown(self):
        handle = Mock()
        stopped = threading.Event()
        with patch.object(sys, "stdin", io.StringIO('{"action":"stop"}\n')):
            sidecar.listen_stdin(handle, [False], stopped)
        handle.pause.assert_called_once()
        self.assertTrue(stopped.is_set())

    def test_malformed_command_then_eof_also_stops(self):
        stop_event = threading.Event()
        with patch.object(sys, "stdin", io.StringIO("invalid\n")):
            sidecar.listen_stdin(Mock(), [False], stop_event)
        self.assertTrue(stop_event.is_set())

    def test_broken_stdin_still_requests_shutdown(self):
        stop_event = threading.Event()
        broken_stdin = Mock()
        broken_stdin.__iter__ = Mock(side_effect=OSError("pipe failed"))
        with patch.object(sys, "stdin", broken_stdin):
            with self.assertRaises(OSError):
                sidecar.listen_stdin(Mock(), [False], stop_event)
        self.assertTrue(stop_event.is_set())


if __name__ == "__main__":
    unittest.main()
