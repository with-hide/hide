#!/usr/bin/env python3
"""Meter the device relay's monthly traffic and own whether it runs.

Run every minute by hide-iroh-relay-meter.timer, as root: it reads the
nftables counters iroh-relay.nft keeps, adds what the relay sent to this UTC
month, warns once at 80%, and stops the relay for the rest of the month at the
cap. A tick that cannot count or save what the relay sent stops the relay and
its limits until one can: a relay the meter does not count is not under its
cap. The phone relay is metered by no part of this. `resume` lets the operator
start it again before the month ends. Every line it prints is a categorical
event with counts; no address, endpoint id or key ever reaches it.
"""

import contextlib
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

CAP_BYTES = 300 * 10**9
WARN_BYTES = CAP_BYTES * 8 // 10
UNIT = "hide-iroh-relay.service"
# Stopping it removes the table; the relay's next start loads it again (Requires=).
LIMITS = "hide-iroh-relay-limits.service"
TABLE = ("inet", "hide_iroh_relay")
STATE = Path("/var/lib/hide-iroh-relay-meter")
CERTIFICATE = Path("/var/lib/hide-relay-proxy/caddy/certificates/acme-v02.api.letsencrypt.org-directory"
                   "/device-relay.withhide.dev/device-relay.withhide.dev.crt")


def run(*argv):
    result = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=20, check=False)
    if result.returncode:
        raise RuntimeError(f"{argv[0]} {argv[1]} failed (exit {result.returncode})")
    return result.stdout


def read_counters(command=run):
    """The table's handle and its two counters, or None while no table is loaded.

    The handle changes when the limits unit loads the table again, which also
    sets its counters back to zero. Only the table list and the two counters
    are read, nothing that holds an address.
    """
    tables = json.loads(command("/usr/sbin/nft", "-j", "list", "tables"))["nftables"]
    handle = next((item["table"]["handle"] for item in tables
                   if item.get("table", {}).get("family") == TABLE[0] and item["table"].get("name") == TABLE[1]), None)
    if handle is None:
        return None

    def counter(name):
        listing = json.loads(command("/usr/sbin/nft", "-j", "list", "counter", *TABLE, name))["nftables"]
        return next(item["counter"] for item in listing if "counter" in item)

    return {"table": handle, "sent": counter("sent")["bytes"], "connections": counter("connections")["packets"]}


def fresh(month, hour):
    return {"month": month, "sent": 0, "warned": False, "stopped": False, "resumed": False,
            "hour": hour, "hour_sent": 0, "hour_connections": 0,
            "table": None, "counter_sent": 0, "counter_connections": 0, "certificate": None}


def advance(state, counters, now):
    """Fold one reading into the state; return it with the events to print.

    Traffic since the previous reading counts toward the period that reading
    was in. A table loaded again since then is counted from zero; what it
    counted between the last reading and its removal (at most one interval) is
    not seen.
    """
    month, hour = now.strftime("%Y-%m"), now.strftime("%Y-%m-%dT%H")
    state = dict(state) if state else fresh(month, hour)
    events = []
    if counters is not None:
        same = counters["table"] == state["table"]
        sent = counters["sent"] - state["counter_sent"] if same else counters["sent"]
        connections = counters["connections"] - state["counter_connections"] if same else counters["connections"]
        if sent < 0 or connections < 0:
            raise ValueError("a counter moved backwards inside one table")
        state["sent"] += sent
        state["hour_sent"] += sent
        state["hour_connections"] += connections
        state.update(table=counters["table"], counter_sent=counters["sent"],
                     counter_connections=counters["connections"])
    else:
        state.update(table=None, counter_sent=0, counter_connections=0)
    if state["hour"] != hour:
        events.append({"event": "device_relay_hour", "hour": state["hour"],
                       "connections": state["hour_connections"], "sent_bytes": state["hour_sent"]})
        state.update(hour=hour, hour_sent=0, hour_connections=0)
    if state["month"] != month:
        state.update(month=month, sent=0, warned=False, stopped=False, resumed=False)
    if state["sent"] >= WARN_BYTES and not state["warned"]:
        state["warned"] = True
        events.append({"event": "device_relay_budget_warning"})
    if state["sent"] >= CAP_BYTES and not state["stopped"] and not state["resumed"]:
        state["stopped"] = True
        events.append({"event": "device_relay_stopped"})
    return state, events


def paused(state):
    return state["stopped"] and not state["resumed"]


def active_state(command):
    return command("systemctl", "show", "--property=ActiveState", "--value", UNIT).strip()


def is_running(active):
    return active in (b"active", b"activating", b"reloading")


def reconcile(state, directory, certificate, command):
    """Make the relay and Caddy's marker match the state; safe to repeat.

    The relay copies Caddy's certificate when it starts, so the meter keeps the
    fingerprint of the file it started the relay with and restarts the relay
    only when Caddy's file differs. It cannot look at the relay's own copy:
    systemd hides every unit's credentials from this one.
    """
    marker = directory / "stopped"
    if paused(state):
        marker.touch()
    else:
        marker.unlink(missing_ok=True)
    active = active_state(command)
    running = is_running(active)
    if paused(state) or not certificate.exists():
        if running:
            command("systemctl", "stop", UNIT)
        return
    fingerprint = hashlib.sha256(certificate.read_bytes()).hexdigest()
    if not running:
        if active == b"failed":
            command("systemctl", "reset-failed", UNIT)
        command("systemctl", "start", UNIT)
    elif state.get("certificate") != fingerprint:
        # Caddy renewed the certificate; the relay reads it only at start.
        command("systemctl", "restart", UNIT)
    else:
        return
    state["certificate"] = fingerprint
    save(directory, state)


def load(directory):
    path = directory / "state.json"
    return json.loads(path.read_text()) if path.exists() else None


def save(directory, state):
    temporary = directory / "state.json.next"
    with temporary.open("w") as stream:
        json.dump(state, stream)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(directory / "state.json")


@contextlib.contextmanager
def locked(directory):
    """One meter at a time: the timer's tick and the operator's resume."""
    directory.mkdir(exist_ok=True)
    with open(directory / "lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def emit(events):
    for event in events:
        print(json.dumps(event, separators=(",", ":")), flush=True)


def tick(directory=STATE, now=None, command=run, certificate=CERTIFICATE):
    """One reading, and the relay made to match it.

    Fails closed: when the counters, the state or its save fail, or the relay
    runs with no table to count it (another tool flushed the ruleset), the
    relay and its limits are stopped before the failure is reported, so no
    relay forwards while what it sends goes uncounted. The next tick that
    counts and saves starts them again, and the table with them.
    """
    try:
        with locked(directory):
            counters = read_counters(command)
            state, events = advance(load(directory), counters, now or datetime.datetime.now(datetime.timezone.utc))
            # An alert printed twice after a crash is better than one never printed.
            emit(events)
            save(directory, state)
            if counters is None and is_running(active_state(command)):
                raise RuntimeError("the relay runs with no counters table loaded")
            reconcile(state, directory, certificate, command)
    except Exception as error:
        try:
            command("systemctl", "stop", UNIT, LIMITS)
        except Exception as stopping:
            raise RuntimeError(f"{type(error).__name__}: {error}; stopping the relay failed: {stopping}") from error
        raise


def resume(directory=STATE, now=None, command=run, certificate=CERTIFICATE):
    """The operator starts a stopped relay again until the month ends."""
    with locked(directory):
        state, events = advance(load(directory), read_counters(command),
                                now or datetime.datetime.now(datetime.timezone.utc))
        if not state["stopped"]:
            raise ValueError("the device relay is not stopped by its monthly limit")
        state["resumed"] = True
        events.append({"event": "device_relay_resumed"})
        emit(events)
        save(directory, state)
        reconcile(state, directory, certificate, command)


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("tick", "resume"):
        raise ValueError("usage: iroh-relay-meter.py tick|resume")
    (tick if sys.argv[1] == "tick" else resume)()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # The alert pipeline forwards this event; the detail stays on the machine.
        emit([{"event": "device_relay_meter_failed"}])
        print("meter failed: " + type(error).__name__ + ": " + str(error), file=sys.stderr)
        sys.exit(1)
