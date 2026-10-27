"""The agent floor measurement (`.github/workflows/agent-floor.yml`): the
table that installs one agent's CLI at an older version, and the record of
which versions passed on which system.

A table is contracts/agent-clis.json's shape holding the measured agent's
row alone, so the run installs and measures that CLI and nothing else. An npm
row follows the package as it was at that version: a CLI that was published
under another name before (`RENAMED`) is installed from that name, and a
native program the pin reaches by its path is reached so only where that
version's package names one; otherwise the version's own launcher is run.
"""

import json
import re
import shutil
import subprocess

from .contract import misses

VERSION = re.compile(r"^\d+\.\d+\.\d+$")
SYSTEMS = ("macos", "linux", "windows")
# The name each npm package was published under before its current one, read
# from npm (`npm view <name> versions`) on 2026-10-27: OpenCode 1.x is
# `opencode-ai` and 2.x `@opencode/cli`; Pi up to 0.73.1 is
# `@mariozechner/pi-coding-agent` and from 0.74.0 `@earendil-works/...`.
RENAMED = {
    "@opencode/cli": ("opencode-ai",),
    "@earendil-works/pi-coding-agent": ("@mariozechner/pi-coding-agent",),
}
# An installer that takes no version cannot install an older one.
UNVERSIONED = {"agy": "its installer serves only the vendor's current release (scripts/install-agy.sh)"}


class FloorError(Exception):
    pass


def npm_manifest(package, version):
    """npm's manifest of `package@version`, or None when npm has no such version."""
    program = shutil.which("npm")
    if not program:
        raise FloorError("npm is not on PATH")
    found = subprocess.run([program, "view", f"{package}@{version}", "--json"], capture_output=True, text=True,
                           timeout=120)
    if found.returncode:
        if "E404" in found.stdout + found.stderr:
            return None
        raise FloorError(f"npm view {package}@{version} exited {found.returncode}: {found.stderr.strip()[:200]}")
    manifest = json.loads(found.stdout or "null")
    # A range that names several versions answers a list; an exact one, one object.
    return manifest if isinstance(manifest, dict) else None


def npm_row(kind, pinned, version, manifest=npm_manifest):
    for package in (pinned["package"], *RENAMED.get(pinned["package"], ())):
        found = manifest(package, version)
        if found is None:
            continue
        row = {"version": version, "installer": "npm", "package": package}
        bins = found.get("bin") or {}
        bins = {package.rsplit("/", 1)[-1]: bins} if isinstance(bins, str) else bins
        path = bins.get(kind)
        if not path:
            raise FloorError(f"{kind}: {package}@{version} names no `{kind}` program (bin {sorted(bins)})")
        # The pin reaches a native program its postinstall puts in place by
        # path; a version whose package names a script instead runs through
        # npm's launcher, as an operator's install does.
        if pinned.get("program") and path.endswith(".exe"):
            row["program"] = f"node_modules/{package}/{path.removeprefix('./')}"
        if pinned.get("install_scripts") and (found.get("scripts") or {}).get("postinstall"):
            row["install_scripts"] = pinned["install_scripts"]
        return row
    raise FloorError(f"{kind}: npm has no {version} of {' or '.join((pinned['package'], *RENAMED.get(pinned['package'], ())))}")


def table(pinned, kind, version, manifest=npm_manifest):
    """The CLI table that installs `kind` at `version` and no other CLI."""
    if not VERSION.match(version):
        raise FloorError(f"{version!r} is not an x.y.z version")
    rows = pinned["clis"]
    if kind not in rows:
        raise FloorError(f"no CLI row for {kind}")
    row = rows[kind]
    if row["installer"] in UNVERSIONED:
        raise FloorError(f"{kind} cannot be measured at an older version: {UNVERSIONED[row['installer']]}")
    if row["installer"] == "npm":
        measured = npm_row(kind, row, version, manifest)
    else:
        measured = {key: value for key, value in row.items() if key != "pinned"} | {"version": version}
    return {**pinned, "clis": {kind: measured}}


def ordered(versions):
    return sorted(versions, key=lambda version: tuple(int(part) for part in version.split(".")))


def record(row, reports):
    """Per version, each system's misses (empty is a pass, None no report)
    from `reports`, keyed by (system, version)."""
    versions = ordered({version for _, version in reports})
    return {version: {system: (misses(row, reports[system, version]) if reports.get((system, version)) is not None
                               else None)
                      for system in SYSTEMS}
            for version in versions}


def boundary(result):
    """(oldest version that passed every system, newest version that failed
    on any), each None when the run has none; a system with no report is
    neither."""
    passed = [version for version, cells in result.items() if all(cells[system] == [] for system in SYSTEMS)]
    failed = [version for version, cells in result.items() if any(cells[system] for system in SYSTEMS)]
    return (passed[0] if passed else None), (failed[-1] if failed else None)
