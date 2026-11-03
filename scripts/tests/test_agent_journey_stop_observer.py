"""Exercise native HTTP input and lifecycle without callback filesystem writes."""
import http.client
import json
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import stop_observer


class StopObservation(unittest.TestCase):
    def make(self, root):
        observer = stop_observer.Observer(root / "home", root, ["hooks", "observer.json"])
        self.addCleanup(observer.close)
        return observer

    def post(self, observer, raw):
        config = json.loads(observer.config.read_text())
        hook = config["hooks"]["Stop"][0]["hooks"][0]
        self.assertEqual(hook["type"], "http")
        url = urlsplit(hook["url"])
        connection = http.client.HTTPConnection(url.hostname, url.port, timeout=5)
        try:
            connection.request("POST", url.path, raw)
            response = connection.getresponse()
            self.assertEqual((response.status, response.read()), (200, b""))
        finally:
            connection.close()

    def test_native_input_stays_in_memory_even_when_all_file_writes_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            parent = root / "home" / "hooks"
            parent.mkdir(parents=True, mode=0o755)
            mode = parent.stat().st_mode
            observer = self.make(root)
            self.assertEqual(parent.stat().st_mode, mode)
            self.post(observer, b"[]")
            self.assertEqual(observer.read(), [])  # outside the armed cell
            observer.arm("JOURNEY-1111AAAA", "JOURNEY-2222BBBB", 60)
            for active, answer in ((False, "JOURNEY-1111AAAA"), (True, "JOURNEY-2222BBBB")):
                self.post(observer, json.dumps(dict(sessionId="session", reason="end_turn",
                    stopHookActive=active, lastAssistantMessage="private prose " + answer)).encode())
            rows = observer.read()
            self.assertEqual([r["active"] for r in rows], [False, True])
            self.assertEqual([r["answers"] for r in rows], [["JOURNEY-1111AAAA"], ["JOURNEY-2222BBBB"]])
            self.assertNotIn("private prose", json.dumps(rows))
            original = Path.open
            def refuse_writes(path, mode="r", *args, **kwargs):
                if any(flag in mode for flag in "wax+"):
                    raise OSError("disk writes unavailable")
                return original(path, mode, *args, **kwargs)
            with patch.object(Path, "open", refuse_writes):
                self.post(observer, b"[]")
            self.assertEqual(observer.read()[-1], {"error": "observer_failed"})
            port = observer.server.server_address[1]
            observer.close()
            self.assertFalse(observer.thread.is_alive())
            self.assertFalse(observer.config.exists())
            with self.assertRaises(OSError):
                socket.create_connection(("127.0.0.1", port), timeout=1)

    def test_bad_payload_and_record_overflow_are_failure_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            observer = self.make(Path(directory))
            observer.arm("held", "letter", 60)
            for raw in (b"[]", b"null", b"{}", b"{"):
                self.post(observer, raw)
            self.assertTrue(all(row.get("error") for row in observer.read()))
            for _ in range(stop_observer.RECORD_CAP):
                self.post(observer, b"{}")
            self.assertEqual(observer.read(), [{"error": "observer_overflow"}])
            self.assertEqual(len(observer.records), stop_observer.RECORD_CAP)

    def test_truncated_request_and_stopped_server_cannot_certify_prior_records(self):
        with tempfile.TemporaryDirectory() as directory:
            observer = self.make(Path(directory))
            observer.arm("held", "letter", 60)
            # The accepted request reserves failure evidence before body parsing.
            connection = socket.create_connection(observer.server.server_address, timeout=2)
            connection.sendall(("POST " + observer.route + " HTTP/1.0\r\nContent-Length: 20\r\n\r\n{}").encode())
            connection.shutdown(socket.SHUT_WR)
            connection.recv(4096)
            connection.close()
            self.assertTrue(observer.read()[0].get("error"))
            observer.server.shutdown()
            observer.thread.join(2)
            self.assertEqual(observer.read(), [{"error": "observer_not_running"}])

    def test_owner_close_interrupts_an_accepted_incomplete_request(self):
        with tempfile.TemporaryDirectory() as directory:
            observer = self.make(Path(directory))
            observer.arm("held", "letter", 60)
            accepted = threading.Event()
            setup = stop_observer.Handler.setup
            def signal(handler):
                setup(handler)
                accepted.set()
            with patch.object(stop_observer.Handler, "setup", signal):
                connection = socket.create_connection(observer.server.server_address, timeout=2)
                connection.sendall(b"POST / HTTP/1.0\r\nUnfinished: ")
                self.assertTrue(accepted.wait(2))
                closer = threading.Thread(target=observer.close)
                closer.start()
                closer.join(1)
                try:
                    self.assertFalse(closer.is_alive(), "owner close waited for unfinished input")
                    self.assertFalse(observer.thread.is_alive())
                finally:
                    connection.close()
                    closer.join(6)

    def test_linked_config_parent_and_rebound_cleanup_never_touch_outside_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            probe, outside = root / "probe", root / "outside"
            probe.mkdir()
            outside.mkdir()
            home = probe / "home"
            home.mkdir()
            (home / "hooks").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(ValueError):
                stop_observer.Observer(home, probe, ["hooks", "observer.json"])
            self.assertEqual(list(outside.iterdir()), [])
            (home / "hooks").unlink()
            observer = stop_observer.Observer(home, probe, ["hooks", "observer.json"])
            owned = home / "owned-hooks"
            (home / "hooks").rename(owned)
            sentinel = outside / "observer.json"
            sentinel.write_text("keep")
            (home / "hooks").symlink_to(outside, target_is_directory=True)
            try:
                with self.assertRaises(ValueError):
                    observer.close()
                self.assertEqual(sentinel.read_text(), "keep")
            finally:
                (home / "hooks").unlink()
                owned.rename(home / "hooks")
                observer.close()

    def test_observer_refuses_a_home_outside_its_private_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ValueError):
                stop_observer.Observer(root / "operator", root / "probe", ["hooks", "observer.json"])


if __name__ == "__main__":
    unittest.main()
