#!/usr/bin/env python3
"""Five keys-fixture printers: first flush and live identities at phase boundaries.

The pane fixture owns the printers; Owned below owns only command delivery.
Receipts prove first flush, not continued output or terminal rendering.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shlex
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from holder_owned import Owned, abort_on_signals, runs, started  # noqa: E402

START_SECONDS = 30
COUNT = 5


def atomic(path, value):
    temporary = path.with_name(path.name + f".{os.getpid()}.tmp")
    with temporary.open("x") as stream:
        json.dump(value, stream)
        stream.write("\n")
    try:
        os.link(temporary, path)  # Publish a complete file, refusing an earlier receipt.
    finally:
        temporary.unlink()


def read(path):
    with path.open() as stream:
        return json.load(stream)


def positive(value):
    return type(value) is int and value > 0


def live(rows):
    if len({row["pid"] for row in rows}) != len(rows):
        raise ValueError("duplicate printer pid")
    for row in rows:
        if not positive(row["pid"]) or not positive(row["birth"]):
            raise ValueError("invalid printer identity")
        if not runs(row["pid"], row["birth"]):
            raise RuntimeError(f"printer ended or changed identity: {row['pane']}")


def receipts(folder, request):
    rows = []
    for index, pane in enumerate(request["panes"]):
        path = folder / f"{index}.json"
        if not path.exists():
            continue
        row = read(path)
        if any(row.get(key) != value for key, value in
               (("phase", str(folder)), ("nonce", request["nonce"]), ("pane", pane))):
            raise ValueError("stale or mismatched first-flush receipt")
        rows.append(row)
    live(rows)
    return rows


def remaining(deadline):
    left = deadline - time.monotonic()
    if left <= 0:
        raise TimeoutError("five-printer startup exceeded its common deadline")
    return left


def start(folder, seconds, panes, sender):
    if len(panes) != COUNT or len(set(panes)) != COUNT or not all(panes):
        raise ValueError("exactly five distinct panes required")
    if not math.isfinite(seconds) or seconds <= 0 or not sender:
        raise ValueError("positive printer duration and delivery command required")
    folder.mkdir(mode=0o700)  # A previous phase, including a failed one, is never reused.
    request = {"phase": str(folder), "nonce": uuid.uuid4().hex, "panes": panes}
    atomic(folder / "request.json", request)
    owned = Owned()
    report = {**request, "complete": False, "printers": []}
    began = time.monotonic()
    deadline = began + START_SECONDS
    try:
        for index, pane in enumerate(panes):
            command = shlex.join(["/usr/bin/python3", str(Path(__file__).resolve()),
                                  "print", str(folder), request["nonce"], str(index), str(seconds)])
            command = "printf '\\033c'; stty -echo -icanon; " + command
            remaining(deadline)
            child = owned.spawn([*sender, pane, "--focus", "--interrupt", command])
            code = child.wait(timeout=remaining(deadline))
            if code:
                raise RuntimeError(f"printer command delivery failed: {pane}: {code}")
            report["printers"] = receipts(folder, request)
            remaining(deadline)
        while True:
            remaining(deadline)
            report["printers"] = receipts(folder, request)
            remaining(deadline)  # Includes the final OS identity reads in the bound.
            if len(report["printers"]) == COUNT:
                break
            time.sleep(min(0.025, remaining(deadline)))
        report["complete"] = True
    except BaseException as error:
        report["error"] = str(error)
        raise
    finally:
        report["startup_seconds"] = time.monotonic() - began
        cleanup_began = time.monotonic()
        left = owned.close()
        report["delivery_cleanup_seconds"] = time.monotonic() - cleanup_began
        report["delivery_processes_left"] = left
        if left:
            report["complete"] = False
        atomic(folder / "start.json", report)
        if left:
            raise RuntimeError(f"printer delivery cleanup failed: {left}")


def printer(folder, nonce, index, seconds):
    request = read(folder / "request.json")
    if request["nonce"] != nonce or not 0 <= index < COUNT:
        raise ValueError("printer request mismatch")
    receipt = folder / f"{index}.json"
    if receipt.exists():
        raise ValueError("printer receipt already exists")
    pid = os.getpid()
    birth = started(pid)
    if not positive(birth):
        raise RuntimeError("printer birth unavailable")
    end = time.monotonic() + seconds
    print("drive 00000", flush=True)
    atomic(receipt, {"phase": str(folder), "nonce": nonce, "pane": request["panes"][index],
                     "pid": pid, "birth": birth})
    index = 1
    while time.monotonic() < end:
        time.sleep(0.008)
        print(f"drive {index:05d}", flush=True)
        index += 1


def check(folder, boundary, exclude=None):
    request = read(folder / "request.json")
    report = read(folder / "start.json")
    rows = report["printers"]
    if (report["complete"] is not True or len(rows) != COUNT
            or [row["pane"] for row in rows] != request["panes"]
            or len(set(request["panes"])) != COUNT
            or any(row["nonce"] != request["nonce"] or row["phase"] != str(folder) for row in rows)
            or len({row["pid"] for row in rows}) != COUNT):
        raise ValueError("incomplete or mismatched printer start")
    if exclude is not None and exclude not in request["panes"]:
        raise ValueError("echo pane is not in this phase")
    selected = [row for row in rows if row["pane"] != exclude]
    result = {"boundary": boundary, "printers": selected, "cat_pane": exclude, "complete": False}
    try:
        live(selected)
        result["complete"] = True
    finally:
        atomic(folder / f"{boundary}.json", result)


def observer(path, pid):
    birth = started(pid)
    if not positive(birth):
        raise RuntimeError("observer is not running")
    deadline = time.monotonic() + START_SECONDS
    while True:
        remaining(deadline)
        if not runs(pid, birth):
            raise RuntimeError("observer ended before navigation")
        if path.exists():
            if read(path) != {"pid": pid, "boundary": "Network.enable completed"}:
                raise ValueError("wrong observer readiness")
            return
        time.sleep(min(0.025, remaining(deadline)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    launch = commands.add_parser("start")
    launch.add_argument("folder", type=Path)
    launch.add_argument("seconds", type=float)
    launch.add_argument("panes", nargs=COUNT)
    launch.add_argument("sender", nargs=argparse.REMAINDER)
    output = commands.add_parser("print")
    output.add_argument("folder", type=Path)
    output.add_argument("nonce")
    output.add_argument("index", type=int)
    output.add_argument("seconds", type=float)
    boundary = commands.add_parser("check")
    boundary.add_argument("folder", type=Path)
    boundary.add_argument("boundary", choices=("resources-before", "resources-after", "echo-before", "echo-after"))
    boundary.add_argument("--exclude")
    ready = commands.add_parser("observer")
    ready.add_argument("path", type=Path)
    ready.add_argument("pid", type=int)
    args = parser.parse_args()
    if args.mode == "print":
        printer(args.folder.resolve(), args.nonce, args.index, args.seconds)
    else:
        abort_on_signals()
        if args.mode == "start":
            start(args.folder.resolve(), args.seconds, args.panes, args.sender)
        elif args.mode == "check":
            check(args.folder.resolve(), args.boundary, args.exclude)
        else:
            observer(args.path, args.pid)


if __name__ == "__main__":
    main()
