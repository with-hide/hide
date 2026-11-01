#!/usr/bin/env python3
"""Produces the pinned Windows ConPTY runtime a pane holder runs on.

Usage: fetch-conpty.py [--out <folder>] [--contract <file>]

The runtime is Microsoft's `Microsoft.Windows.Console.ConPTY` package from
nuget.org, at the version and digest `contracts/conpty.json` pins. The
package is refused unless its size and sha256 are the pinned ones, each file
is taken from its place in the package and refused unless its sha256 is the
pinned one, and the runtime folder (`conpty.dll`, `x64/OpenConsole.exe`,
`arm64/OpenConsole.exe` and the file list `conpty-files.json`) is made in a
staging folder and renamed into place whole, in a cache keyed by the
package's digest, so a second build on the same machine downloads nothing
and a cached folder that no longer matches the pin is made again. Prints the
folder's path on its last line.

With `--out`, the runtime is also copied to `<folder>` itself and that path
is printed: a node finds its holder's runtime in `holder-conpty` beside its
own program (`hide_kit::holders::CONPTY_SOURCE`), so the app package and a
lane's binaries are given `--out <their folder>/holder-conpty`. An existing
`<folder>` that is not exactly the pinned runtime is refused rather than
replaced.

Any failure exits non-zero and names what differed, so a build that needs the
runtime fails rather than carrying one it did not check.
"""

import argparse
import hashlib
import json
import os
import shutil
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

CHECKOUT = Path(__file__).resolve().parent.parent
FILE_LIST = "conpty-files.json"
# The package is fetched in one request; the pin's size bounds what is read.
READ_LIMIT_SLACK = 1


def digest(path):
    hasher = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 16), b""):
            hasher.update(block)
    return hasher.hexdigest()


def load_contract(path):
    try:
        contract = json.loads(Path(path).read_text())
    except (OSError, ValueError) as error:
        raise SystemExit(f"the ConPTY pin {path} cannot be read: {error}")
    for key in ("package", "version", "source_url", "sha256", "size", "files"):
        if key not in contract:
            raise SystemExit(f"the ConPTY pin {path} has no {key!r}")
    if not contract["files"]:
        raise SystemExit(f"the ConPTY pin {path} names no file")
    for name, entry in contract["files"].items():
        if name == FILE_LIST or not entry.get("from") or not entry.get("sha256"):
            raise SystemExit(f"the ConPTY pin {path} names {name!r} without its place or digest")
        if Path(name).is_absolute() or ".." in Path(name).parts:
            raise SystemExit(f"the ConPTY pin {path} names {name!r} outside the runtime folder")
    return contract


def cache_root():
    override = os.environ.get("HIDE_CONPTY_CACHE")
    if override:
        return Path(override)
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
    elif sys.platform == "darwin":
        base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / "Library" / "Caches")
    else:
        base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "hide" / "conpty-runtime"


def file_list(contract):
    return {
        "package": contract["package"],
        "version": contract["version"],
        "files": {name: entry["sha256"] for name, entry in sorted(contract["files"].items())},
    }


def runtime_matches(folder, contract):
    """Whether `folder` holds exactly the pinned runtime, every file by digest."""
    if not folder.is_dir():
        return False
    expected = set(contract["files"]) | {FILE_LIST}
    found = {
        path.relative_to(folder).as_posix()
        for path in folder.rglob("*")
        if path.is_file() or path.is_symlink()
    }
    if found != expected:
        return False
    for name, entry in contract["files"].items():
        path = folder / name
        if path.is_symlink() or digest(path) != entry["sha256"]:
            return False
    try:
        listed = json.loads((folder / FILE_LIST).read_text())
    except (OSError, ValueError):
        return False
    return listed == file_list(contract)


def download(contract, into):
    """The package at `into`, checked against the pin's size and digest."""
    limit = int(contract["size"]) + READ_LIMIT_SLACK
    try:
        with urllib.request.urlopen(contract["source_url"], timeout=60) as answer:
            body = answer.read(limit)
    except OSError as error:
        raise SystemExit(f"the ConPTY package could not be downloaded from {contract['source_url']}: {error}")
    if len(body) != int(contract["size"]):
        raise SystemExit(
            f"the ConPTY package is {len(body)} bytes{' or more' if len(body) == limit else ''}, "
            f"not the pinned {contract['size']}"
        )
    actual = hashlib.sha256(body).hexdigest()
    if actual != contract["sha256"]:
        raise SystemExit(f"the ConPTY package has sha256 {actual}, not the pinned {contract['sha256']}")
    into.write_bytes(body)


def extract(contract, package, folder):
    with zipfile.ZipFile(package) as archive:
        names = set(archive.namelist())
        for name, entry in contract["files"].items():
            if entry["from"] not in names:
                raise SystemExit(f"the ConPTY package has no {entry['from']} for {name}")
            target = folder / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry["from"]) as source, open(target, "wb") as sink:
                shutil.copyfileobj(source, sink)
            actual = digest(target)
            if actual != entry["sha256"]:
                raise SystemExit(f"{name} in the ConPTY package has sha256 {actual}, not the pinned {entry['sha256']}")
    (folder / FILE_LIST).write_text(json.dumps(file_list(contract), indent=2) + "\n")


def cached_runtime(contract):
    root = cache_root() / contract["sha256"]
    folder = root / "conpty"
    if runtime_matches(folder, contract):
        return folder
    root.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix="staging-", dir=root))
    try:
        package = staging / "package.nupkg"
        download(contract, package)
        made = staging / "conpty"
        made.mkdir()
        extract(contract, package, made)
        if not runtime_matches(made, contract):
            raise SystemExit("the ConPTY runtime made from the package does not match the pin")
        if folder.exists():
            # A cached folder that no longer matches the pin: set aside whole,
            # then removed, so the name is complete or absent.
            stale = staging / "stale"
            folder.rename(stale)
        made.rename(folder)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return folder


def copy_to(runtime, target, contract):
    target_parent = target.parent
    if target.exists() or target.is_symlink():
        if runtime_matches(target, contract):
            return target
        raise SystemExit(f"{target} exists and is not the pinned ConPTY runtime; remove it and run this again")
    target_parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".conpty-", dir=target_parent))
    try:
        shutil.copytree(runtime, staging / "runtime")
        (staging / "runtime").rename(target)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    if not runtime_matches(target, contract):
        raise SystemExit(f"the ConPTY runtime copied to {target} does not match the pin")
    return target


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", type=Path)
    parser.add_argument("--contract", type=Path, default=CHECKOUT / "contracts" / "conpty.json")
    args = parser.parse_args(argv)
    contract = load_contract(args.contract)
    runtime = cached_runtime(contract)
    if args.out is not None:
        runtime = copy_to(runtime, args.out.absolute(), contract)
    print(runtime)


if __name__ == "__main__":
    main()
