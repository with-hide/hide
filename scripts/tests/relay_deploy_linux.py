#!/usr/bin/env python3
"""Disposable GitHub Linux runner only: real systemd, proxy and collector.

The relay is a tiny HTTP process fixture; real Stackdriver output talks to local
metadata/Logging fixtures. TLS uses a fixture CA, never public TLS or real GCP.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import ssl
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
    if url.startswith("https://relay.withhide.dev/"):
        return subprocess.check_output(["curl", "--fail", "--silent", "--max-time", "2",
            "--noproxy", "*", "--resolve", "relay.withhide.dev:443:127.0.0.1", url])
    with urllib.request.urlopen(url, timeout=2) as response:
        return response.read()


def wait_for(predicate, description, seconds=35):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            if predicate():
                return
        except (OSError, urllib.error.URLError, subprocess.CalledProcessError):
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
    token_requested = threading.Event()
    delivered = threading.Event()
    disconnected = threading.Event()

    class Sink(BaseHTTPRequestHandler):
        def reply(self, status, body):
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Metadata-Flavor", "Google")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            assert self.headers.get("Metadata-Flavor") == "Google"
            values = {
                "/computeMetadata/v1/project/project-id": b"fixture-project",
                "/computeMetadata/v1/instance/zone": b"projects/123456789/zones/asia-northeast1-a",
                "/computeMetadata/v1/instance/id": b"1234",
                "/computeMetadata/v1/instance/service-accounts/default/token":
                    b'{"access_token":"fixture-token","token_type":"Bearer","expires_in":3600}',
            }
            if self.path.endswith("/token"):
                token_requested.set()
            self.reply(200 if self.path in values else 404, values.get(self.path, b"{}"))

        def do_POST(self):
            assert self.path == "/v2/entries:write"
            assert self.headers.get("Authorization") == "Bearer fixture-token"
            body = self.rfile.read(int(self.headers["Content-Length"]))
            if disconnected.is_set():
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                return
            assert len(received) < 4096, "fixture request cap exceeded"
            received.append(json.loads(body))
            if not rejected.is_set():
                delivered.set()
            self.reply(403 if rejected.is_set() else 200,
                       b'{"error":{"code":403,"status":"PERMISSION_DENIED"}}' if rejected.is_set() else b"{}")

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Sink)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    servers, threads = [server], [thread]
    try:
        for user in ("hide-relay", "hide-relay-proxy", "hide-relay-logs"):
            subprocess.run(["useradd", "--system", "--user-group", "--no-create-home", "--shell", "/usr/sbin/nologin", user], check=True)
        certificates = Path("/etc/hide-relay-test")
        certificates.mkdir()
        certificate, key = certificates / "certificate.crt", certificates / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                        "-subj", "/CN=relay.withhide.dev", "-addext", "subjectAltName=DNS:relay.withhide.dev,IP:127.0.0.1",
                        "-keyout", str(key), "-out", str(certificate)], check=True, capture_output=True)
        subprocess.run(["chown", "root:hide-relay-proxy", str(key)], check=True)
        key.chmod(0o640)
        shutil.copyfile(certificate, "/usr/local/share/ca-certificates/hide-relay-test.crt")
        subprocess.run(["update-ca-certificates"], check=True, capture_output=True)
        logging = ThreadingHTTPServer(("127.0.0.1", 0), Sink)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(certificate, key)
        logging.socket = tls.wrap_socket(logging.socket, server_side=True)
        logging_thread = threading.Thread(target=logging.serve_forever)
        servers.append(logging)
        threads.append(logging_thread)
        logging_thread.start()
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
        self.wfile.write(b"ok\\n" if self.path == "/health" else self.headers.get("X-Real-IP", "missing").encode())
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
            # A locally trusted fixture certificate avoids an external CA.
            # HTTPS and Caddy's automatic HTTP redirect remain enabled.
            # Production parser, canary, Stackdriver and service sandboxes stay.
            caddy = staged / "Caddyfile"
            caddy.write_text(caddy.read_text().replace("relay.withhide.dev {",
                            f"relay.withhide.dev {{\n\ttls {certificate} {key}"))
            config = staged / "fluent-bit.conf"
            config.write_text(config.read_text() + f'''
    metadata_server http://127.0.0.1:{server.server_port}
    cloud_logging_base_url https://127.0.0.1:{logging.server_port}
    tls On
    tls.verify On
    tls.ca_file {certificate}
''')
            def inventory(directory):
                (directory / "inventory.json").write_text(json.dumps({p.name: DEPLOY.digest(p) for p in directory.iterdir()
                    if p.name != "inventory.json"}, sort_keys=True) + "\n")
            inventory(staged)
            DEPLOY.install(staged)
            # A supplied address is replaced by the proxy's actual socket peer.
            assert subprocess.check_output(["curl", "--fail", "--silent", "--noproxy", "*",
                "--resolve", "relay.withhide.dev:443:127.0.0.1", "-H", "X-Real-IP: 198.51.100.99",
                "https://relay.withhide.dev/ws"]) == b"127.0.0.1"
            wait_for(lambda: any(entry["jsonPayload"] == {"event": "server_failed"}
                     for batch in received for entry in batch["entries"]), "collector delivered no relay events")
            assert token_requested.is_set()
            for batch in received:
                assert batch["resource"] == {"type": "gce_instance", "labels": {
                    "project_id": "fixture-project", "instance_id": "1234", "zone": "asia-northeast1-a"}}, batch
                for entry in batch["entries"]:
                    assert entry["logName"] == "projects/fixture-project/logs/hide-relay", entry
                    assert entry["jsonPayload"] in ({"event": "server_failed"}, {"event": "collector_canary"}), entry
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
                return subprocess.check_output(["curl", "--silent", "--max-time", "2", "--noproxy", "*",
                    "--resolve", "relay.withhide.dev:443:127.0.0.1", "--output", "/dev/null",
                    "--write-out", "%{http_code}", "https://relay.withhide.dev/collector-health"]) == b"500"
            wait_for(unhealthy, "collector failure remained healthy", seconds=120)
            # No more relay events: the canary must keep a permanent 403
            # unhealthy beyond the real five-minute health error window.
            until = time.monotonic() + 310
            while time.monotonic() < until:
                assert unhealthy(), "quiet persistent delivery failure became healthy"
                threading.Event().wait(5)
            delivered.clear()
            subprocess.run(["systemctl", "restart", "hide-relay-logs"], check=True)
            wait_for(unhealthy, "restarted collector concealed ongoing delivery refusal", seconds=90)
            assert not delivered.is_set(), "refused delivery was recorded as successful"
            rejected.clear()
            subprocess.run(["systemctl", "restart", "hide-relay-logs"], check=True)
            wait_for(delivered.is_set, "delivery did not actually resume", seconds=90)
            wait_for(lambda: request("https://relay.withhide.dev/collector-health"), "collector did not recover")
            disconnected.set()
            delivered.clear()
            subprocess.run(["systemctl", "restart", "hide-relay-logs"], check=True)
            wait_for(unhealthy, "transport disconnection remained healthy", seconds=90)
            assert not delivered.is_set()
            disconnected.clear()
            subprocess.run(["systemctl", "restart", "hide-relay-logs"], check=True)
            wait_for(delivered.is_set, "delivery did not resume after transport recovery", seconds=90)
            inputs = json.loads((staged / "inputs.json").read_text())
            inputs["source"] = "2" * 40
            (staged / "inputs.json").write_text(json.dumps(inputs))
            receipt = json.loads((staged / "source.json").read_text())
            receipt["source"] = inputs["source"]
            (staged / "source.json").write_text(json.dumps(receipt))
            # Relay stays healthy and HTTP still redirects; only the HTTPS
            # reverse proxy breaks. Installation must observe that failure.
            caddy.write_text(caddy.read_text().replace("reverse_proxy 127.0.0.1:8787",
                                                       "reverse_proxy 127.0.0.1:8788"))
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
        for owned in servers:
            owned.shutdown()
            owned.server_close()
        for owned in threads:
            owned.join(timeout=5)


if __name__ == "__main__":
    main()
