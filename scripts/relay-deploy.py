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
# The phone relay, its proxy (shared with the device relay) and its collector.
SERVICES = ("hide-relay.service", "hide-relay-proxy.service", "hide-relay-logs.service")
RETENTION = ("hide-relay-retention.service", "hide-relay-retention.timer")
# The device relay beside the phone relay. The meter starts and stops the relay
# itself (by its monthly limit and Caddy's certificate), so an installation
# starts only the limits, one meter pass and the meter's timer.
DEVICE_RELAY = "hide-iroh-relay.service"
DEVICE_LIMITS = "hide-iroh-relay-limits.service"
DEVICE_METER = "hide-iroh-relay-meter.service"
DEVICE_TIMER = "hide-iroh-relay-meter.timer"
DEVICE_UNITS = (DEVICE_RELAY, DEVICE_LIMITS, DEVICE_METER, DEVICE_TIMER)
UNITS = SERVICES + RETENTION + DEVICE_UNITS
# Everything an installation stops before switching releases.
STOPPED = SERVICES + DEVICE_UNITS + ("hide-relay-retention.timer",)
BINARIES = ("hide-relay", "iroh-relay")
DEVICE_HOST = "device-relay.withhide.dev"
METER_STATE = "var/lib/hide-iroh-relay-meter"
SHA = r"[0-9a-f]{40}"
DIGEST = r"[0-9a-f]{64}"
# Explicit CLI values, never environment defaults. This is the input contract.
INPUTS = {
    "source": (SHA, "exact protected-main commit"),
    "sha256": (DIGEST, "binary digest read from the verified CI artifact"),
    "iroh_relay_sha256": (DIGEST, "iroh-relay digest read from the same verified CI artifact"),
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
    "network": (r"[a-z]([-a-z0-9]{0,61}[a-z0-9])?", "VPC network of the relay VM"),
    "target_tag": (r"[a-z]([-a-z0-9]{0,61}[a-z0-9])?", "network tag the relay VM carries"),
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
    for event in ("cap_exceeded", "server_failed", "device_relay_budget_warning",
                  "device_relay_stopped", "device_relay_meter_failed"):
        write(event + ".json", {
            "displayName": "hide-relay-" + event, "enabled": True, "combiner": "OR",
            "notificationChannels": [values["channel"]],
            "conditions": [{"displayName": event, "conditionMatchedLog": {
                "filter": scope + f' AND jsonPayload.event="{event}"'}}],
            "alertStrategy": {"notificationRateLimit": {"period": "300s"}, "autoClose": "1800s"},
        })
    # Cloud-observed delivery survives collector restart and its initially
    # empty local health window. This stores counts, never payload labels.
    write("collector-delivery-metric.json", {
        "name": "hide-relay-delivery",
        "description": "Content-free relay collector canaries received by Logging",
        "filter": scope + ' AND jsonPayload.event="collector_canary"',
        "metricDescriptor": {"metricKind": "DELTA", "valueType": "INT64", "unit": "1"},
    })
    write("collector-delivery-alert.json", {
        "displayName": "hide-relay-delivery-absent", "enabled": True, "combiner": "OR",
        "notificationChannels": [values["channel"]],
        "conditions": [{"displayName": "No received collector canaries", "conditionAbsent": {
            "filter": 'resource.type="gce_instance" AND '
                      'metric.type="logging.googleapis.com/user/hide-relay-delivery" '
                      f'AND resource.labels.project_id="{values["project"]}" '
                      f'AND resource.labels.instance_id="{values["instance_id"]}"',
            "duration": "300s", "trigger": {"count": 1},
        }}],
    })
    for name, host, path in (("relay", "relay.withhide.dev", "/health"),
                             ("collector", "relay.withhide.dev", "/collector-health"),
                             ("device-relay", DEVICE_HOST, "/ping")):
        write(name + "-uptime.json", {
            "displayName": "hide-relay-" + name, "period": "60s", "timeout": "10s",
            "monitoredResource": {"type": "uptime_url", "labels": {
                "project_id": values["project"], "host": host}},
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
    # Address discovery for the device relay: iroh-relay answers QUIC on UDP
    # 7842 itself, beside Caddy's 80/443.
    write("device-relay-firewall.json", {
        "name": "hide-iroh-relay-qad", "network": "global/networks/" + values["network"],
        "direction": "INGRESS", "priority": 1000, "sourceRanges": ["0.0.0.0/0"],
        "allowed": [{"IPProtocol": "udp", "ports": ["7842"]}], "targetTags": [values["target_tag"]],
    })
    write("budget.json", {
        "displayName": "hide-relay-monthly",
        "budgetFilter": {"projects": ["projects/" + values["project_number"]], "calendarPeriod": "MONTH",
                         "creditTypesTreatment": "EXCLUDE_ALL_CREDITS"},
        "amount": {"specifiedAmount": {"currencyCode": "KRW", "units": "100000"}},
        "thresholdRules": [{"thresholdPercent": n} for n in (0.5, 0.9, 1.0)],
        "notificationsRule": {"monitoringNotificationChannels": [values["channel"]],
                              "disableDefaultIamRecipients": False},
    })


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def regular(path, limit):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= limit:
        raise ValueError("missing, linked or oversized deployment file: " + path.name)


def iroh_relay_version():
    return (TEMPLATES / "iroh-relay.version").read_text().strip()


def verify_artifact(artifact, values):
    regular(artifact / "source.json", 16384)
    for binary in BINARIES:
        regular(artifact / binary, 64 * 1024 * 1024)
    receipt = json.loads((artifact / "source.json").read_text())
    if (receipt.get("source") != values["source"]
            or receipt.get("sha256") != values["sha256"]
            or receipt.get("repository") != "with-hide/hide"
            or receipt.get("workflow") != ".github/workflows/relay-artifact.yml"
            or receipt.get("target") != "x86_64-unknown-linux-gnu"
            or receipt.get("iroh_relay") != {"version": iroh_relay_version(), "sha256": values["iroh_relay_sha256"]}
            or digest(artifact / "hide-relay") != values["sha256"]
            or digest(artifact / "iroh-relay") != values["iroh_relay_sha256"]):
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
        for binary in BINARIES:
            shutil.copyfile(artifact / binary, output / binary)
            (output / binary).chmod(0o755)
        shutil.copyfile(artifact / "source.json", output / "source.json")
        (output / "inputs.json").write_text(json.dumps(values) + "\n")
        inventory = {p.name: digest(p) for p in output.iterdir()}
        (output / "inventory.json").write_text(json.dumps(inventory, sort_keys=True) + "\n")
        output.rename(destination)


def run(*argv):
    # OS boundary only. Output can include deployment values or request objects;
    # report the executable and exit status, never replay arbitrary stderr.
    operation = Path(argv[0]).name
    if operation == "systemctl":
        operation += " " + argv[1]
    try:
        capture = argv[0] == "curl" or argv[:2] == ("systemctl", "show")
        # A curl that retries is bounded by its own retry budget.
        limit = int(argv[argv.index("--retry-max-time") + 1]) + 20 if "--retry-max-time" in argv else 90
        result = subprocess.run(argv, stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, timeout=limit, check=False)
    except subprocess.TimeoutExpired:
        raise RuntimeError(operation + " timed out") from None
    if result.returncode:
        raise RuntimeError(f"{operation} failed (exit {result.returncode})")
    return result.stdout


def stop(command, units):
    # A unit this release adds is unknown to systemd until the switch and its
    # daemon-reload, and systemctl refuses to stop a unit that is not loaded.
    loaded = [unit for unit in units
              if command("systemctl", "show", "--property=LoadState", "--value", unit) == b"loaded\n"]
    if loaded:
        command("systemctl", "stop", *loaded)


def atomic_link(path, target):
    temporary = path.with_name(path.name + ".next")
    temporary.unlink(missing_ok=True)
    temporary.symlink_to(target)
    temporary.replace(path)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def verify_inventory(release):
    """A release is exactly the files its own inventory lists, unchanged.

    This is all a previous release is checked against when it is restored: it
    was staged by an older script, from older templates and pins.
    """
    regular(release / "inventory.json", 16384)
    inventory = json.loads((release / "inventory.json").read_text())
    if {p.name for p in release.iterdir()} != set(inventory) | {"inventory.json"}:
        raise ValueError("unexpected release file set")
    for name, checksum in inventory.items():
        regular(release / name, 64 * 1024 * 1024 if name in BINARIES else 16384)
        if digest(release / name) != checksum:
            raise ValueError("staged release changed: " + name)
    return inventory


def verify_release(release):
    """A release this script staged: its inventory, its file set and its inputs."""
    inventory = verify_inventory(release)
    if set(inventory) != {p.name for p in TEMPLATES.iterdir()} | set(BINARIES) | {"source.json", "inputs.json"}:
        raise ValueError("unexpected release file set")
    values = json.loads((release / "inputs.json").read_text())
    checked_values(values)
    verify_artifact(release, values)
    return values


def activate(root, command, device_required=True):
    """Start the release current points at: the phone relay, then the device relay.

    A release staged before the device relay existed has no device units, and
    starts only the phone relay. The device relay never holds the phone relay
    back: it starts after the phone relay is healthy and enabled. When it does
    not come up, its units are stopped; an installation then fails (and is rolled
    back), while a recovery keeps the phone relay running and returns the reason.
    """
    command("systemctl", "daemon-reload")
    command("systemctl", "restart", "systemd-journald@hide-relay.service")
    for service in SERVICES:
        # Inactive units may already have been released from systemd's memory;
        # reset-failed only applies to retained failures (including start limits).
        state = command("systemctl", "show", "--property=ActiveState", "--value", service)
        if state == b"failed\n":
            command("systemctl", "reset-failed", service)
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
    if not (root / "opt/hide-relay/current" / DEVICE_RELAY).exists():
        return None
    try:
        start_device_relay(root, command)
    except RuntimeError as failure:
        stop(command, DEVICE_UNITS)
        if device_required:
            raise
        return f"device relay stopped: {failure}"
    return None


def start_device_relay(root, command):
    for service in (DEVICE_LIMITS, DEVICE_METER):
        if command("systemctl", "show", "--property=ActiveState", "--value", service) == b"failed\n":
            command("systemctl", "reset-failed", service)
    # Its limits, then one meter pass, which starts the relay once Caddy holds
    # its certificate; the timer repeats that pass every minute.
    command("systemctl", "start", DEVICE_LIMITS)
    command("systemctl", "start", DEVICE_METER)
    command("systemctl", "start", DEVICE_TIMER)
    # A relay its monthly limit stopped stays stopped; Caddy answers 503 then.
    if not (root / METER_STATE / "stopped").exists():
        response = command("curl", "--silent", "--max-time", "10", "--max-filesize", "32",
                           "--retry", "30", "--retry-all-errors", "--retry-delay", "5", "--retry-max-time", "170",
                           "--fail", "--noproxy", "*", "--resolve", DEVICE_HOST + ":443:127.0.0.1",
                           "--output", "/dev/null", "--write-out", "%{http_code}", f"https://{DEVICE_HOST}/ping")
        if response != b"200":
            raise RuntimeError("HTTPS device relay probe did not return 200")
    command("systemctl", "enable", DEVICE_TIMER)


def recover(root, command=run):
    base = root / "opt/hide-relay"
    pending = base / "pending.json"
    if not pending.exists():
        raise ValueError("no interrupted installation")
    previous = json.loads(pending.read_text())["previous"]
    if previous is not None and not re.fullmatch("releases/" + SHA, previous):
        raise ValueError("invalid recovery record")
    current = base / "current"
    if current.is_symlink():
        # A first installation can fail its syntax check before activate loads
        # the new units. Make the published unit files visible before cleanup.
        command("systemctl", "daemon-reload")
        stop(command, STOPPED)
    problem = None
    if previous:
        verify_inventory(base / previous)
        atomic_link(current, previous)
        sync_directory(base)
        problem = activate(root, command, device_required=False)
    elif current.is_symlink():
        command("systemctl", "disable", *SERVICES, "hide-relay-retention.timer", DEVICE_TIMER)
        current.unlink()
        command("systemctl", "daemon-reload")
    pending.unlink()
    sync_directory(base)
    return problem


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
            path.chmod(0o755 if path.name in BINARIES else 0o644)
            with path.open("rb") as stream:
                os.fsync(stream.fileno())
        sync_directory(temporary)
        temporary.rename(target)
        sync_directory(releases)
    # Syntax checks run before stopping a working release. Caddy adapt never
    # obtains certificates, and collector --dry-run never authenticates.
    command("/usr/bin/caddy", "adapt", "--config", str(target / "Caddyfile"), "--adapter", "caddyfile")
    command("/usr/sbin/nft", "--check", "--file", str(target / "iroh-relay.nft"))
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
        if previous is not None:
            stop(command, STOPPED)
        atomic_link(current, "releases/" + values["source"])
        sync_directory(base)
        command("/opt/fluent-bit/bin/fluent-bit", "--dry-run", "-c", str(base / "current/fluent-bit.conf"))
        activate(root, command)
    except Exception as failure:
        try:
            problem = recover(root, command)
        except Exception as rollback:
            raise RuntimeError(f"installation failed ({failure}); recovery incomplete ({rollback}), run recover") from rollback
        restored = "previous state restored" + (f", but {problem}" if problem else "")
        raise RuntimeError(f"installation failed ({failure}); {restored}") from failure
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
                problem = recover(Path("/"))
                if problem:
                    print("relay deployment: phone relay restored; " + problem, file=sys.stderr)
            else:
                install(args.release)
    print("relay deployment: " + args.action + " succeeded")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print("relay deployment failed: " + str(error), file=sys.stderr)
        sys.exit(1)
