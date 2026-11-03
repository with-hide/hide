#!/usr/bin/env python3
"""Stage or install the single-VM relay. No cloud API or credential access."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile


TEMPLATES = Path(__file__).resolve().parents[1] / "deploy" / "relay"
UNITS = ("hide-relay.service", "hide-relay-proxy.service", "hide-relay-logs.service",
         "hide-relay-retention.service", "hide-relay-retention.timer")
SERVICES = UNITS[:3]
SHA = r"[0-9a-f]{40}"
DIGEST = r"[0-9a-f]{64}"
# Explicit CLI values, never environment defaults. This is the input contract.
INPUTS = {
    "source": (SHA, "exact protected-main commit"),
    "sha256": (DIGEST, "binary digest read from the verified CI artifact"),
    "project": (r"[a-z][a-z0-9-]{4,28}[a-z0-9]", "destination GCP project ID"),
    "machine_id": (r"[0-9a-f]{32}", "target VM /etc/machine-id"),
}
CLOUD_INPUTS = {
    "project": INPUTS["project"],
    "project_number": (r"[1-9][0-9]{5,20}", "budget project number"),
    "instance_id": (r"[1-9][0-9]{0,20}", "numeric relay VM instance ID"),
    "channel": (r"projects/[a-z][a-z0-9-]{4,28}[a-z0-9]/notificationChannels/[0-9]+",
                "verified email notification channel resource name"),
    "currency": (r"KRW", "actual billing currency; other currencies require an operator decision"),
}


def checked_values(values):
    errors = [key for key, (pattern, _) in INPUTS.items()
              if not re.fullmatch(pattern, values.get(key, ""))]
    if errors:
        raise ValueError("invalid required inputs: " + ", ".join(errors))


def cloud_config(values, destination):
    """Render reviewed API bodies only; never call a cloud API."""
    errors = [key for key, (pattern, _) in CLOUD_INPUTS.items()
              if not re.fullmatch(pattern, values.get(key, ""))]
    if errors:
        raise ValueError("invalid required cloud inputs: " + ", ".join(errors))
    if not values["channel"].startswith("projects/" + values["project"] + "/"):
        raise ValueError("channel must belong to project")
    if destination.exists():
        raise ValueError("cloud output destination must be absent")
    destination.mkdir(parents=True)
    scope = (f'resource.type="gce_instance" AND resource.labels.project_id="{values["project"]}" '
             f'AND resource.labels.instance_id="{values["instance_id"]}" AND log_id("hide-relay")')
    (destination / "relay-filter.txt").write_text(scope + "\n")
    def write(name, content):
        (destination / name).write_text(json.dumps(content, indent=2) + "\n")
    for event in ("cap_exceeded", "server_failed"):
        write(event + ".json", {
            "displayName": "hide-relay-" + event, "enabled": True, "combiner": "OR",
            "notificationChannels": [values["channel"]],
            "conditions": [{"displayName": event, "conditionMatchedLog": {
                "filter": scope + f' AND jsonPayload.event="{event}"'}}],
            "alertStrategy": {"notificationRateLimit": {"period": "300s"}, "autoClose": "1800s"},
        })
    for name, path in (("relay", "/health"), ("collector", "/collector-health")):
        write(name + "-uptime.json", {
            "displayName": "hide-relay-" + name, "period": "60s", "timeout": "10s",
            "monitoredResource": {"type": "uptime_url", "labels": {
                "project_id": values["project"], "host": "relay.withhide.dev"}},
            "httpCheck": {"requestMethod": "GET", "path": path, "port": 443,
                          "useSsl": True, "validateSsl": True},
        })
        write(name + "-uptime-alert.json", {
            "displayName": "hide-relay-" + name + "-unavailable", "enabled": True, "combiner": "OR",
            "notificationChannels": [values["channel"]],
            "conditions": [{"displayName": "HTTPS unavailable", "conditionThreshold": {
                "filter": 'resource.type="uptime_url" AND metric.type="monitoring.googleapis.com/uptime_check/check_passed" '
                          'AND metric.labels.check_id="REPLACE_WITH_CREATED_CHECK_ID"',
                "comparison": "COMPARISON_GT", "thresholdValue": 1, "duration": "120s",
                "evaluationMissingData": "EVALUATION_MISSING_DATA_ACTIVE",
                "aggregations": [{"alignmentPeriod": "60s", "perSeriesAligner": "ALIGN_NEXT_OLDER",
                                  "crossSeriesReducer": "REDUCE_COUNT_FALSE", "groupByFields": ["resource.label.host"]}],
                "trigger": {"count": 1},
            }}], "alertStrategy": {"autoClose": "1800s"},
        })
    write("budget.json", {
        "displayName": "hide-relay-monthly",
        "budgetFilter": {"projects": ["projects/" + values["project_number"]], "calendarPeriod": "MONTH",
                         "creditTypesTreatment": "EXCLUDE_ALL_CREDITS"},
        "amount": {"specifiedAmount": {"currencyCode": "KRW", "units": "100000"}},
        "thresholdRules": [{"thresholdPercent": n} for n in (0.5, 0.9, 1.0)],
        "allUpdatesRule": {"monitoringNotificationChannels": [values["channel"]],
                           "disableDefaultIamRecipients": False},
    })


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def regular(path, limit):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= limit:
        raise ValueError("missing, linked or oversized deployment file: " + path.name)


def verify_artifact(artifact, values):
    regular(artifact / "source.json", 16384)
    regular(artifact / "hide-relay", 64 * 1024 * 1024)
    receipt = json.loads((artifact / "source.json").read_text())
    if (receipt.get("source") != values["source"]
            or receipt.get("sha256") != values["sha256"]
            or receipt.get("repository") != "with-hide/hide"
            or receipt.get("workflow") != ".github/workflows/relay-artifact.yml"
            or receipt.get("target") != "x86_64-unknown-linux-gnu"
            or digest(artifact / "hide-relay") != values["sha256"]):
        raise ValueError("artifact provenance or digest mismatch")


def stage(artifact, destination, values):
    """Validate before creating output; never execute the downloaded binary."""
    checked_values(values)
    verify_artifact(artifact, values)
    if destination.exists():
        raise ValueError("stage destination must be absent")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as scratch:
        output = Path(scratch) / "release"
        output.mkdir()
        for template in TEMPLATES.iterdir():
            text = template.read_text().replace("@PROJECT_ID@", values["project"])
            text = text.replace("@MACHINE_ID@", values["machine_id"])
            (output / template.name).write_text(text)
        shutil.copyfile(artifact / "hide-relay", output / "hide-relay")
        (output / "hide-relay").chmod(0o755)
        shutil.copyfile(artifact / "source.json", output / "source.json")
        (output / "inputs.json").write_text(json.dumps(values) + "\n")
        inventory = {p.name: digest(p) for p in output.iterdir()}
        (output / "inventory.json").write_text(json.dumps(inventory, sort_keys=True) + "\n")
        output.rename(destination)


def run(*argv):
    # OS boundary only. Output can include deployment values or request objects;
    # report the executable and exit status, never replay arbitrary stderr.
    result = subprocess.run(argv, stdout=subprocess.PIPE if argv[0] == "curl" else subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL,
                            timeout=90, check=False)
    if result.returncode:
        raise RuntimeError(f"{Path(argv[0]).name} failed (exit {result.returncode})")
    return result.stdout


def atomic_link(path, target):
    temporary = path.with_name(path.name + ".next")
    temporary.unlink(missing_ok=True)
    temporary.symlink_to(target)
    temporary.replace(path)


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def verify_release(release):
    regular(release / "inventory.json", 16384)
    inventory = json.loads((release / "inventory.json").read_text())
    expected = {p.name for p in TEMPLATES.iterdir()} | {"hide-relay", "source.json", "inputs.json"}
    if set(inventory) != expected or {p.name for p in release.iterdir()} != expected | {"inventory.json"}:
        raise ValueError("unexpected release file set")
    for name, checksum in inventory.items():
        regular(release / name, 64 * 1024 * 1024 if name == "hide-relay" else 16384)
        if digest(release / name) != checksum:
            raise ValueError("staged release changed: " + name)
    values = json.loads((release / "inputs.json").read_text())
    checked_values(values)
    verify_artifact(release, values)
    return values


def activate(root, command):
    command("systemctl", "daemon-reload")
    command("systemctl", "restart", "systemd-journald@hide-relay.service")
    command("systemctl", "reset-failed", *SERVICES)
    command("systemctl", "start", *SERVICES)
    for endpoint in ("http://127.0.0.1:8787/health", "http://127.0.0.1:2020/api/v1/health"):
        command("curl", "--fail", "--silent", "--retry", "10", "--retry-connrefused",
                "--retry-delay", "1", "--max-time", "2", "--max-filesize", "32",
                "--retry-max-time", "30", endpoint)
    response = command("curl", "--fail", "--silent", "--max-time", "10", "--max-filesize", "32",
                       "--retry", "5", "--retry-all-errors", "--retry-delay", "2", "--retry-max-time", "60",
                       "--noproxy", "*", "--resolve", "relay.withhide.dev:443:127.0.0.1",
                       "--write-out", "\n%{http_code}", "https://relay.withhide.dev/health")
    if response != b"ok\n\n200":
        raise RuntimeError("HTTPS relay health did not return the expected 200 response")
    for service in SERVICES:
        command("systemctl", "is-active", "--quiet", service)
    command("systemctl", "enable", *SERVICES, "hide-relay-retention.timer")
    command("systemctl", "start", "hide-relay-retention.timer", "hide-relay-retention.service")


def recover(root, command=run):
    base = root / "opt/hide-relay"
    pending = base / "pending.json"
    if not pending.exists():
        raise ValueError("no interrupted installation")
    previous = json.loads(pending.read_text())["previous"]
    if previous is not None and not re.fullmatch("releases/" + SHA, previous):
        raise ValueError("invalid recovery record")
    command("systemctl", "stop", *SERVICES, "hide-relay-retention.timer")
    if previous:
        verify_release(base / previous)
        atomic_link(base / "current", previous)
        sync_directory(base)
        activate(root, command)
    else:
        command("systemctl", "disable", *SERVICES, "hide-relay-retention.timer")
        (base / "current").unlink(missing_ok=True)
    pending.unlink()
    sync_directory(base)


def install(release, root=Path("/"), command=run):
    """Root and command are OS test boundaries; the CLI always installs at /."""
    values = verify_release(release)
    if (root / "etc/machine-id").read_text().strip() != values["machine_id"]:
        raise ValueError("machine_id does not identify this VM")
    base = root / "opt/hide-relay"
    base.mkdir(parents=True, exist_ok=True)
    if (base / "pending.json").exists():
        raise ValueError("interrupted install: run recover before installing")
    current = base / "current"
    if current.exists() and not current.is_symlink():
        raise ValueError("current is not a managed symlink")
    previous = os.readlink(current) if current.is_symlink() else None
    if previous is not None and not re.fullmatch("releases/" + SHA, previous):
        raise ValueError("current is not a managed release")
    links = {root / "etc/systemd/system" / unit: "/opt/hide-relay/current/" + unit for unit in UNITS}
    links[root / "etc/systemd/journald@hide-relay.conf"] = "/opt/hide-relay/current/journald.conf"
    for path, target in links.items():
        if (path.exists() or path.is_symlink()) and (not path.is_symlink() or os.readlink(path) != target):
            raise ValueError("refusing unmanaged configuration: " + path.name)
    releases = base / "releases"
    releases.mkdir(exist_ok=True)
    target = releases / values["source"]
    if target.exists():
        verify_release(target)
        if (target / "inventory.json").read_bytes() != (release / "inventory.json").read_bytes():
            raise ValueError("source already staged with different inputs or bytes")
    else:
        if len(list(releases.iterdir())) >= 3:
            raise ValueError("release cap reached: remove an inactive release after review")
        temporary = releases / ".staging"
        if temporary.exists():
            raise ValueError("incomplete staging directory: inspect before removing")
        shutil.copytree(release, temporary)
        for path in temporary.iterdir():
            path.chmod(0o755 if path.name == "hide-relay" else 0o644)
            with path.open("rb") as stream:
                os.fsync(stream.fileno())
        sync_directory(temporary)
        temporary.rename(target)
        sync_directory(releases)
    # Syntax checks run before stopping a working release. Caddy adapt never
    # obtains certificates, and collector --dry-run never authenticates.
    command("/usr/bin/caddy", "adapt", "--config", str(target / "Caddyfile"), "--adapter", "caddyfile")
    for path, link in links.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.is_symlink():
            path.symlink_to(link)
    pending = base / "pending.json"
    # An interrupted write may leave only this disposable temporary file.
    # Publish a complete recovery intent before stopping any running service.
    temporary_pending = pending.with_suffix(".next")
    with temporary_pending.open("w") as stream:
        json.dump({"previous": previous}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    temporary_pending.replace(pending)
    sync_directory(base)
    try:
        command("systemctl", "stop", *SERVICES, "hide-relay-retention.timer")
        atomic_link(current, "releases/" + values["source"])
        sync_directory(base)
        command("/opt/fluent-bit/bin/fluent-bit", "--dry-run", "-c", str(base / "current/fluent-bit.conf"))
        activate(root, command)
    except Exception as failure:
        try:
            recover(root, command)
        except Exception as rollback:
            raise RuntimeError("installation failed; recovery incomplete, run recover") from rollback
        raise RuntimeError("installation failed; previous state restored") from failure
    pending.unlink()
    sync_directory(base)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    staged = actions.add_parser("stage")
    staged.add_argument("--artifact", type=Path, required=True)
    staged.add_argument("--out", type=Path, required=True)
    for key, (_, note) in INPUTS.items():
        staged.add_argument("--" + key.replace("_", "-"), required=True, help=note)
    apply = actions.add_parser("install")
    apply.add_argument("release", type=Path)
    actions.add_parser("recover")
    cloud = actions.add_parser("cloud-config")
    cloud.add_argument("--out", type=Path, required=True)
    for key, (_, note) in CLOUD_INPUTS.items():
        cloud.add_argument("--" + key.replace("_", "-"), required=True, help=note)
    args = parser.parse_args()
    if args.action == "stage":
        stage(args.artifact, args.out, {key: getattr(args, key) for key in INPUTS})
    elif args.action == "cloud-config":
        cloud_config({key: getattr(args, key) for key in CLOUD_INPUTS}, args.out)
    else:
        if platform.system() != "Linux" or platform.machine() != "x86_64" or os.geteuid() != 0:
            raise ValueError("installation requires root on the dedicated Linux x86_64 VM")
        import fcntl
        with open("/run/lock/hide-relay-install.lock", "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if args.action == "recover":
                recover(Path("/"))
            else:
                install(args.release)
    print("relay deployment: " + args.action + " succeeded")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print("relay deployment failed: " + str(error), file=sys.stderr)
        sys.exit(1)
