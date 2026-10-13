"""Stdin lifecycle regressions; libtorrent/native network are not required."""

import importlib.util
import io
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import Mock, patch


with patch.dict(sys.modules, {"libtorrent": Mock()}):
    spec = importlib.util.spec_from_file_location(
        "torrent_sidecar", Path(__file__).with_name("torrent_sidecar.py")
    )
    sidecar = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sidecar)


class StdinLifecycleTests(unittest.TestCase):
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
