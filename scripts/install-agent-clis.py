#!/usr/bin/env python3
"""Installs agent CLIs on a CI runner at the versions contracts/agent-clis.json pins.

Usage: install-agent-clis.py <kind>... | --all [--prefix DIR]

CI (CI=true), or the guarded local fake journey's private cache installer.
The latter requires --private-fake and a HOME inside the pin entry; its caller
owns the sandbox, lease and byte cap. Each CLI goes under DIR (default
$RUNNER_TEMP/agent-clis), and the script prints one `<kind>=<program>` line
per CLI; with GITHUB_PATH set it puts each program's folder on the PATH of
the later steps, and with GITHUB_OUTPUT set it writes the same lines as the
step's outputs. npm installs run no package's install scripts but those of
a row that names why it needs them (`install_scripts`). A CLI that answers
another version than the table's fails the install, so a vendor release moves
the pin in a pull request before a lane measures it.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import urllib.request

CHECKOUT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CHECKOUT / "scripts"))
from agent_journey.clis import AGY_SYSTEMS, TABLE, matches, table as pins  # noqa: E402

WINDOWS = os.name == "nt"
# Where each pinned release comes from: the vendor's installer, or its release files.
GROK_INSTALLER = "https://x.ai/cli/install.sh"
OMP_RELEASES = "https://github.com/can1357/oh-my-pi/releases/download"


def table():
    rows = pins()
    for kind, row in rows.items():
        if not row.get("version") or row.get("installer") not in INSTALLERS:
            raise SystemExit(f"{TABLE}: {kind} needs a version and one of the installers {sorted(INSTALLERS)}")
        if row["installer"] == "agy":
            files = row.get("files") or {}
            missing = [system for system in AGY_SYSTEMS
                       if not str((files.get(system) or {}).get("url", "")).startswith("https://")
                       or not re.fullmatch(r"[0-9a-f]{128}", str((files.get(system) or {}).get("sha512", "")))]
            if missing:
                raise SystemExit(f"{TABLE}: {kind} needs a url and sha512 for {', '.join(missing)}")
    return rows


def bash():
    """The bash on PATH: on Windows, Git Bash. A bare `bash` would be looked
    up in System32 first, where the WSL launcher is."""
    program = shutil.which("bash")
    if not program:
        raise SystemExit("bash is not on PATH")
    return program


def run(argv, env=None):
    """A command whose output goes to stderr, so stdout carries only the programs."""
    result = subprocess.run(argv, env={**os.environ, **(env or {})}, stdout=sys.stderr, timeout=600)
    if result.returncode:
        raise SystemExit(f"{argv[0]} exited {result.returncode}")


def npm(kinds, rows, prefix):
    """One npm install of every npm CLI into its own prefix, as the trust
    contract installs them, with no install script but those the table names."""
    folder = prefix / "npm"
    program = shutil.which("npm")
    if not program:
        raise SystemExit("npm is not on PATH: set up Node before this step")
    run([program, "install", "--prefix", str(folder), "--no-audit", "--no-fund", "--ignore-scripts",
         *(f"{rows[kind]['package']}@{rows[kind]['version']}" for kind in kinds)])
    scripted = [rows[kind]["package"] for kind in kinds if rows[kind].get("install_scripts")]
    if scripted:
        run([program, "rebuild", "--prefix", str(folder), *scripted])
    # npm's shim on Windows is a `.cmd` beside the script it runs elsewhere;
    # a package that names its native program is reached by that path instead.
    return {kind: folder / rows[kind].get("program", f"node_modules/.bin/{kind}{'.cmd' if WINDOWS else ''}")
            for kind in kinds}


def grok(row, prefix):
    """Grok's installer takes the version as its argument and the folder as
    GROK_BIN_DIR; on Windows it runs under Git Bash in the same form."""
    folder = prefix / "grok"
    # Already on PATH means the installer never tries ~/.local/bin or
    # /usr/local/bin links. Set this inside Bash so Git Bash converts its
    # inherited Windows PATH before the Unix path is prepended.
    run([bash(), "-c", f'export PATH="$GROK_BIN_DIR:$PATH"; curl -fsSL {GROK_INSTALLER} | bash -s "$0"', row["version"]],
        {"GROK_BIN_DIR": folder.as_posix() if WINDOWS else str(folder)})
    return folder / ("grok.exe" if WINDOWS else "grok")


def omp(row, prefix):
    """omp's release on GitHub: the binary for this system, checked against
    the release's SHA256SUMS.txt. The installer at omp.sh resolves a release
    through a build server that answered `no_build` for every version on
    2026-10-18, while each release carries its binaries."""
    asset = f"omp-{target()}" + (".exe" if WINDOWS else "")
    base = f"{OMP_RELEASES}/v{row['version']}"
    sums = fetch(f"{base}/SHA256SUMS.txt").decode()
    expected = next((line.split()[0] for line in sums.splitlines() if line.split()[1:] == [asset]), None)
    if not expected:
        raise SystemExit(f"omp: v{row['version']} lists no {asset} in SHA256SUMS.txt")
    binary = fetch(f"{base}/{asset}")
    if hashlib.sha256(binary).hexdigest() != expected:
        raise SystemExit(f"omp: {asset} of v{row['version']} does not match its SHA256SUMS.txt")
    folder = prefix / "omp"
    folder.mkdir(parents=True, exist_ok=True)
    program = folder / ("omp.exe" if WINDOWS else "omp")
    program.write_bytes(binary)
    program.chmod(0o755)
    return program


def target():
    """This runner as `<system>-<arch>`, the form omp's assets and agy's row name."""
    machine = platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x64"
    system = "windows" if WINDOWS else "darwin" if sys.platform == "darwin" else "linux"
    return f"{system}-{arch}"


def fetch(url):
    with urllib.request.urlopen(url, timeout=300) as answer:
        return answer.read()


def agy(row, prefix):
    """agy's file for this system at the url the row records, checked against
    the row's sha512: the vendor's installer serves only its latest release,
    while each release's file stays at its own address. The archive's
    `antigravity` is the program; the installer's last step, `agy install`,
    only edits the shell's PATH and aliases, which the runner does not need."""
    system = target()
    if system not in row["files"]:
        raise SystemExit(f"agy: {TABLE.name} names no file for {system}")
    url, sha512 = row["files"][system]["url"], row["files"][system]["sha512"]
    payload = fetch(url)
    if hashlib.sha512(payload).hexdigest() != sha512:
        raise SystemExit(f"agy: {url} does not match the sha512 {TABLE.name} records")
    if url.endswith(".tar.gz"):
        with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
            member = archive.extractfile("antigravity")
            payload = member.read() if member else b""
        if not payload:
            raise SystemExit(f"agy: {url} holds no antigravity program")
    folder = prefix / "agy"
    folder.mkdir(parents=True, exist_ok=True)
    program = folder / ("agy.exe" if WINDOWS else "agy")
    program.write_bytes(payload)
    program.chmod(0o755)
    return program


INSTALLERS = {"npm": None, "grok": grok, "omp": omp, "agy": agy}


def answered(program):
    """What the program says its version is."""
    result = subprocess.run([str(program), "--version"], capture_output=True, text=True, timeout=120,
                            stdin=subprocess.DEVNULL)
    return (result.stdout + result.stderr).strip().splitlines()[0][:120] if result.stdout + result.stderr else ""


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("kinds", nargs="*")
    parser.add_argument("--all", action="store_true", help="every CLI the table pins")
    parser.add_argument("--prefix", type=Path)
    parser.add_argument("--private-fake", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if bool(args.kinds) == args.all:
        parser.error("name the CLIs or pass --all, not both")
    if args.private_fake:
        if not args.prefix or Path.home().resolve() != (args.prefix / "home").resolve():
            parser.error("private fake installation needs a pin-local HOME")
    elif os.environ.get("CI") != "true":
        print("install-agent-clis.py installs CLIs on a CI runner only", file=sys.stderr)
        return 2
    rows = table()
    unknown = [kind for kind in args.kinds if kind not in rows]
    if unknown:
        raise SystemExit(f"not in {TABLE.name}: {', '.join(unknown)}")
    prefix = (args.prefix or Path(os.environ["RUNNER_TEMP"]) / "agent-clis").resolve()
    prefix.mkdir(parents=True, exist_ok=True)
    kinds = list(rows) if args.all else list(dict.fromkeys(args.kinds))
    programs = npm([kind for kind in kinds if rows[kind]["installer"] == "npm"], rows, prefix) \
        if any(rows[kind]["installer"] == "npm" for kind in kinds) else {}
    for kind in kinds:
        if rows[kind]["installer"] != "npm":
            programs[kind] = INSTALLERS[rows[kind]["installer"]](rows[kind], prefix)
    lines, notes, mismatches = [], [], []
    for kind in kinds:
        program = programs[kind]
        if not program.is_file():
            raise SystemExit(f"{kind}: the installer left no program at {program}")
        # The local journey runs --version only after applying the adapter's
        # update policy in its private HOME and entering its stricter sandbox.
        # Starting here would bypass config-based policies during installation.
        if args.private_fake:
            notes.append(f"{kind}: installed pin; journey must check version")
            lines.append(f"{kind}={program}")
            continue
        version = answered(program)
        notes.append(f"{kind} {version}")
        if not matches(rows[kind]["version"], version):
            mismatches.append(f"{kind} answered {version!r}, {TABLE.name} pins {rows[kind]['version']}")
        lines.append(f"{kind}={program}")
    print(f"::notice title=agent CLIs::{', '.join(notes)}")
    if mismatches:
        for mismatch in mismatches:
            print(f"::error title=agent CLI version::{mismatch}")
        raise SystemExit("an installed CLI is not the pinned version: " + "; ".join(mismatches))
    for name, value in (("GITHUB_PATH", "\n".join(dict.fromkeys(str(programs[kind].parent) for kind in kinds))),
                        ("GITHUB_OUTPUT", "\n".join(lines))):
        if os.environ.get(name):
            with open(os.environ[name], "a", encoding="utf-8") as stream:
                stream.write(value + "\n")
    if args.private_fake:
        (prefix / "installed.json").write_text(json.dumps({kind: str(path) for kind, path in programs.items()}))
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
