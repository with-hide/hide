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
a row that names why it needs them (`install_scripts`). A download is asked
again while its failure is transient, inside a fixed budget, and the grok
installer is downloaded to a file before it runs, so a failed download ends
the step with its own error. A CLI that answers
another version than the table's fails the install, so a vendor release moves
the pin in a pull request before a lane measures it. So does a CLI whose
`--help` lists no full-access option its adapter declares
(contracts/agent-journey.json, `full_access_flag`, each option's name):
Settings › Agents adds it
to every start Hide makes, and a CLI that does not know it would refuse to
start. Codex takes its full access as `-c` configuration overrides, which it
accepts with any key, a misspelled one included, and then starts with its
defaults (read-only, asking), so its help proves nothing: the lane runs it
with the adapter's full-access words in an empty HOME instead and requires
the access they give in the prompt input it renders (`FULL_ACCESS_PROOF`).
"""

import argparse
import hashlib
import http.client
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import ssl
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request

CHECKOUT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CHECKOUT / "scripts"))
from agent_journey.clis import AGY_SYSTEMS, TABLE, matches, table as pins  # noqa: E402

WINDOWS = os.name == "nt"
# Where each pinned release comes from: the vendor's installer, or its release files.
GROK_INSTALLER = "https://x.ai/cli/install.sh"
OMP_RELEASES = "https://github.com/can1357/oh-my-pi/releases/download"
# A download that failed on a rate limit, a server fault or a connection that
# dropped is asked again up to this many times, after these waits. The budget
# bounds the whole download, its waits included: no attempt starts once it is
# spent, one request waits no longer than what it leaves, and the body is read
# in pieces checked against it. The install steps run under a ten-minute limit,
# so the budget leaves the rest of a step for the other CLIs' downloads and for
# the checks after them.
DOWNLOAD_RETRY_DELAYS = (2, 5, 15)
DOWNLOAD_BUDGET_SECONDS = 180
# How long one socket read may wait before the connection counts as stalled.
# The omp binary is over 200 MB, so a transfer slower than about 1.2 MB/s does
# not finish inside the budget; at that speed the step was close to its own
# limit anyway.
DOWNLOAD_ATTEMPT_SECONDS = 60
# The body is read in pieces of this size, so a server that keeps trickling
# bytes cannot outlast the budget.
DOWNLOAD_CHUNK_BYTES = 1 << 16
# The answers a second ask may get differently: a rate limit, a request that
# timed out, and every server fault. A 403, a 404 and a URL that answers
# nothing are the same on the second ask.
RETRY_STATUSES = frozenset({408, 425, 429}) | frozenset(range(500, 600))
# The connection failures a second ask may get past: a wait that timed out and
# a connection that was dropped, reset, refused or ended partway through. Every
# other failure of the operating system or of TLS says the same thing on the
# second ask, a certificate the name does not match and a name that does not
# resolve included.
TRANSIENT_CONNECTIONS = (TimeoutError, ConnectionResetError, ConnectionRefusedError,
                         ConnectionAbortedError, BrokenPipeError, ssl.SSLEOFError,
                         http.client.IncompleteRead, http.client.RemoteDisconnected)


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
    GROK_BIN_DIR; on Windows it runs under Git Bash in the same form. The
    installer is downloaded to a file and run from there: piping it into Bash
    hid its download's failure behind the pipeline's last command, so a curl
    that got a 500 ended the step as a missing program later."""
    folder = prefix / "grok"
    installer = fetch(GROK_INSTALLER)
    with tempfile.TemporaryDirectory() as temporary:
        script = Path(temporary) / "install.sh"
        script.write_bytes(installer)
        # Already on PATH means the installer never tries ~/.local/bin or
        # /usr/local/bin links. Set this inside Bash so Git Bash converts its
        # inherited Windows PATH before the Unix path is prepended.
        run([bash(), "-c", 'export PATH="$GROK_BIN_DIR:$PATH"; exec bash "$0" "$1"',
             script.as_posix(), row["version"]],
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
    """The bytes at `url`, asked again while the failure is transient: a rate
    limit, a server fault, or a connection that dropped or timed out. The budget
    bounds the whole download, the waits between attempts included: no attempt
    starts without time left, one request waits no longer than that leaves, and
    the body is checked against it as it arrives, so a server that keeps
    trickling bytes cannot outlast it. A refusal the next ask would answer the
    same way (403, 404, a certificate of another name, a URL nothing answers)
    stops at once with its own cause. The step ends with the download's own
    error; a file that arrives and does not match its pin is the caller's to
    refuse, since no retry can repair it."""
    end = time.monotonic() + DOWNLOAD_BUDGET_SECONDS
    attempts = len(DOWNLOAD_RETRY_DELAYS) + 1
    for attempt in range(1, attempts + 1):
        left = end - time.monotonic()
        if left <= 0:
            raise SystemExit(f"{url} could not be downloaded inside {DOWNLOAD_BUDGET_SECONDS} s")
        try:
            with urllib.request.urlopen(url, timeout=min(DOWNLOAD_ATTEMPT_SECONDS, left)) as answer:
                body = bytearray()
                for chunk in iter(lambda: answer.read(DOWNLOAD_CHUNK_BYTES), b""):
                    body += chunk
                    if time.monotonic() >= end:
                        raise SystemExit(f"{url} could not be downloaded inside "
                                         f"{DOWNLOAD_BUDGET_SECONDS} s: its body was still arriving")
                return bytes(body)
        except (OSError, http.client.HTTPException) as error:
            if not transient(error):
                raise SystemExit(f"{url} could not be downloaded: {error}")
            if attempt == attempts:
                raise SystemExit(f"{url} could not be downloaded in {attempts} attempts: {error}")
            wait = DOWNLOAD_RETRY_DELAYS[attempt - 1]
            if time.monotonic() + wait >= end:
                raise SystemExit(f"{url} could not be downloaded in {attempt} attempt"
                                 f"{'s' if attempt > 1 else ''} inside {DOWNLOAD_BUDGET_SECONDS} s: {error}")
            time.sleep(wait)


def transient(error):
    """Whether asking the same URL again may get a different answer: a rate
    limit, a server fault, or a connection that dropped, reset, refused or
    timed out. A 403, a 404, a certificate that names another host, an unknown
    URL scheme and a name that does not resolve say the same thing however often
    they are asked, so a retry only spends the step's minutes."""
    if isinstance(error, urllib.error.HTTPError):
        return error.code in RETRY_STATUSES
    if isinstance(error, urllib.error.URLError):
        return isinstance(error.reason, TRANSIENT_CONNECTIONS)
    return isinstance(error, TRANSIENT_CONNECTIONS)


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


# What a CLI renders with its full-access words given, and what that must
# say, for a CLI whose help cannot prove them. Codex 0.166.0 renders these two
# lines with `-c approval_policy=never -c sandbox_mode=danger-full-access`
# and its bypass flag alike, and drops the first for a misspelled key
# (measured 2026-11-01).
FULL_ACCESS_PROOF = {
    "codex": (["debug", "prompt-input"],
              ["Approval policy is currently never", "`sandbox_mode` is `danger-full-access`"]),
}


def full_access_words(kind):
    """The words a full-access start of `kind` adds, from the generated web
    contract (contracts/agent-adapters.json), which carries each option's
    values where the journey contract carries only its name."""
    rows = json.loads((CHECKOUT / "contracts/agent-adapters.json").read_text())
    for row in rows:
        if row.get("program") == kind:
            return [word for option in row.get("full_access") or [] for word in option]
    raise SystemExit(f"{kind}: contracts/agent-adapters.json has no row for it")


def rendered(program, words, command):
    """What `program` prints for `command` with `words` before it, in an
    empty HOME, so no configuration of the runner's decides the access."""
    with tempfile.TemporaryDirectory() as home:
        env = {**os.environ, "HOME": home, "USERPROFILE": home, "CODEX_HOME": str(Path(home) / ".codex"),
               "XDG_CONFIG_HOME": str(Path(home) / ".config")}
        Path(env["CODEX_HOME"]).mkdir()
        result = subprocess.run([str(program), *words, *command], capture_output=True, text=True,
                                timeout=120, stdin=subprocess.DEVNULL, env=env, cwd=home)
    return result.stdout + result.stderr


def unproved(kind, program):
    """The lines the CLI's full-access render lacks; none for a CLI without a proof."""
    if kind not in FULL_ACCESS_PROOF:
        return []
    command, lines = FULL_ACCESS_PROOF[kind]
    text = rendered(program, full_access_words(kind), command)
    return [line for line in lines if line not in text]


def full_access_flags():
    """Each started agent's full-access arguments by its Herdr kind, from the
    generated adapter contract."""
    rows = json.loads((CHECKOUT / "contracts/agent-journey.json").read_text())
    return {row["herdr_kind"]: row.get("full_access_flag") or [] for row in rows}


def helped(program):
    """What the program's `--help` prints."""
    result = subprocess.run([str(program), "--help"], capture_output=True, text=True, timeout=120,
                            stdin=subprocess.DEVNULL)
    return result.stdout + result.stderr


def unknown_flags(flags, help_text):
    """The declared flags the help does not list as a whole word."""
    return [flag for flag in flags if not re.search(r"(?<![\w-])" + re.escape(flag) + r"(?![\w-])", help_text)]


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
    access = full_access_flags()
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
        flags = access.get(kind, [])
        if kind in FULL_ACCESS_PROOF:
            lacking = unproved(kind, program)
            if lacking:
                mismatches.append(f"{kind} {version}: its adapter's full-access words "
                                  f"{' '.join(full_access_words(kind))} do not give full access: "
                                  f"its {' '.join(FULL_ACCESS_PROOF[kind][0])} lacks {'; '.join(lacking)}")
        else:
            missing = unknown_flags(flags, helped(program)) if flags else []
            if missing:
                mismatches.append(f"{kind} {version}: --help lists no {', '.join(missing)}, "
                                  "which its adapter declares as full access")
        lines.append(f"{kind}={program}")
    print(f"::notice title=agent CLIs::{', '.join(notes)}")
    if mismatches:
        for mismatch in mismatches:
            print(f"::error title=agent CLI::{mismatch}")
        raise SystemExit("an installed CLI does not answer as its pin and adapter say: " + "; ".join(mismatches))
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
