#!/usr/bin/env python3
"""Disposable GitHub Linux runner only: real systemd, proxy and collector.

The relay is a tiny HTTP process fixture, and the cloud output is a loopback HTTP
sink. This tests service installation, collection and recovery, never TLS/GCP.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("relay_deploy", ROOT / "scripts/relay-deploy.py")
DEPLOY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DEPLOY)


def request(url):
    with urllib.request.urlopen(url, timeout=2) as response:
        return response.read()


def wait_for(predicate, description, seconds=35):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            if predicate():
                return
        except (OSError, urllib.error.URLError):
            pass
        threading.Event().wait(0.2)
    raise AssertionError(description)


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.geteuid() != 0:
        raise RuntimeError("requires root on a disposable GitHub runner")
    if Path("/opt/hide-relay").exists():
        raise RuntimeError("refusing an existing deployment")
    received = []
    rejected = threading.Event()

    class Sink(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers["Content-Length"]))
            received.append(json.loads(body))
            self.send_response(503 if rejected.is_set() else 200)
            self.end_headers()

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Sink)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    try:
        for user in ("hide-relay", "hide-relay-proxy", "hide-relay-logs"):
            subprocess.run(["useradd", "--system", "--user-group", "--no-create-home", "--shell", "/usr/sbin/nologin", user], check=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "agents/runs") as temporary:
            folder = Path(temporary)
            artifact = folder / "artifact"
            artifact.mkdir()
            binary = artifact / "hide-relay"
            binary.write_text('''#!/usr/bin/python3
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        print(json.dumps({"event":"server_failed", "credential":"fixture-secret", "ip":"192.0.2.10"}), file=sys.stderr, flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.headers.get("X-Real-IP", "ok").encode())
    def log_message(self, *_): pass
HTTPServer(("127.0.0.1", 8787), Handler).serve_forever()
''')
            source = "1" * 40
            checksum = hashlib.sha256(binary.read_bytes()).hexdigest()
            (artifact / "source.json").write_text(json.dumps({
                "source": source, "sha256": checksum, "repository": "with-hide/hide",
                "workflow": ".github/workflows/relay-artifact.yml", "target": "x86_64-unknown-linux-gnu",
            }))
            staged = folder / "staged"
            DEPLOY.stage(artifact, staged, dict(source=source, sha256=checksum, project="fixture-project",
                        machine_id=Path("/etc/machine-id").read_text().strip()))
            # Only the public TLS origin and external cloud output change.
            # The production parser, field allowlist and service sandboxes stay.
            caddy = staged / "Caddyfile"
            caddy.write_text(caddy.read_text().replace("relay.withhide.dev {", "http://127.0.0.1 {"))
            config = staged / "fluent-bit.conf"
            config.write_text(config.read_text().split("[OUTPUT]")[0] + f'''[OUTPUT]
    Name http
    Match hide-relay
    Host 127.0.0.1
    Port {server.server_port}
    URI /events
    Format json
    Json_Date_Key false
    Retry_Limit 3
''')
            def inventory(directory):
                (directory / "inventory.json").write_text(json.dumps({p.name: DEPLOY.digest(p) for p in directory.iterdir()
                    if p.name != "inventory.json"}, sort_keys=True) + "\n")
            inventory(staged)
            DEPLOY.install(staged)
            # A supplied address is replaced by the proxy's actual socket peer.
            forged = urllib.request.Request("http://127.0.0.1/health", headers={"X-Real-IP": "198.51.100.99"})
            with urllib.request.urlopen(forged) as response:
                assert response.read() == b"127.0.0.1"
            wait_for(lambda: bool(received), "collector delivered no events")
            assert all(event == {"event": "server_failed"} for batch in received for event in batch), received
            # Real process failure restarts the systemd-owned relay.
            old_pid = subprocess.check_output(["systemctl", "show", "hide-relay", "-p", "MainPID", "--value"]).strip()
            subprocess.run(["systemctl", "kill", "--signal=SIGKILL", "hide-relay"], check=True)
            wait_for(lambda: request("http://127.0.0.1:8787/health"), "relay did not restart")
            assert subprocess.check_output(["systemctl", "show", "hide-relay", "-p", "MainPID", "--value"]).strip() != old_pid
            # Delivery refusal must be externally visible, not merely logged.
            rejected.set()
            for _ in range(3):
                request("http://127.0.0.1:8787/health")
            def unhealthy():
                try:
                    request("http://127.0.0.1/collector-health")
                except urllib.error.HTTPError as error:
                    return error.code == 500
                return False
            wait_for(unhealthy, "collector failure remained healthy", seconds=120)
            rejected.clear()
            # Restore a clean collector before testing candidate rollback.
            subprocess.run(["systemctl", "restart", "hide-relay-logs"], check=True)
            wait_for(lambda: request("http://127.0.0.1/collector-health"), "collector did not recover")
            inputs = json.loads((staged / "inputs.json").read_text())
            inputs["source"] = "2" * 40
            (staged / "inputs.json").write_text(json.dumps(inputs))
            receipt = json.loads((staged / "source.json").read_text())
            receipt["source"] = inputs["source"]
            (staged / "source.json").write_text(json.dumps(receipt))
            unit = staged / "hide-relay.service"
            unit.write_text(unit.read_text().replace("ExecStart=/opt/hide-relay/current/hide-relay --listen 127.0.0.1:8787 --trusted-proxy 127.0.0.1", "ExecStart=/bin/false"))
            inventory(staged)
            try:
                DEPLOY.install(staged)
            except RuntimeError as error:
                assert "previous state restored" in str(error), str(error)
            else:
                raise AssertionError("broken candidate accepted")
            assert Path("/opt/hide-relay/current").resolve().name == source
            assert request("http://127.0.0.1:8787/health")
            print("PASS: actual systemd restart/rollback, proxy overwrite, collector field boundary and failure health")
    finally:
        subprocess.run(["systemctl", "stop", *DEPLOY.SERVICES, "hide-relay-retention.timer"], check=False)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
