"""Makes the state folder a released Hide writes, for the upgrade test.

    python3 scripts/release_state/generate.py v1.2.1

It downloads the release's Linux device kit (`hide-device-linux-<arch>-<ver>`,
the same `hided`, `hide`, `hide-agent-hooks` and `hide-holder` the release's
packages carry, checked against the release's `device-bundles.json`), builds
one image from it, and runs two containers on a private network: the core,
whose account has an ssh `Host mini`, and the device that alias reaches,
running `sshd`. Each has a fixed made-up machine id. `scenario.py` then
drives the old core through its /ws and its own CLI (see its steps), the core
is stopped, and its state folder is copied out with only what a start reads.

The kit is the one for this machine's Docker architecture, so nothing runs
under emulation. Nothing touches this machine's `~/.hide`, `~/.claude`, the
installed app, or any real device: both accounts live only in the containers.

The folder lands in `hide-core/tests/fixtures/release-state/<tag>/` (or
`--out`), with `<tag>.json`, what each step did, beside it; the
containers, network, image and work folder are removed on every exit.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
REPO = "with-hide/hide"
CORE_ID = "c0de0000000000000000000000000001"
DEVICE_ID = "de510000000000000000000000000002"
# What a start reads. Everything else the old core wrote (logs, holders,
# sockets, scrollback, pane credentials, the daemon seat, attachments,
# backups, pairing secrets) is left behind.
KEEP_FILES = {
    "core-state.json", "workspace-views.json", "labels.json", "delivery-ledger.json", "missions.json",
    "local-issues.json", "composer-marks.json", "hook-facts.json", "node.json",
    "host-id", "github-snapshot.json", "global-instruction.md",
    "links.sqlite3", "usage.sqlite3", "suggestions.sqlite3", "session-search.sqlite3", "project-memory.sqlite3",
    "factory.sqlite3", "factory.v1.sqlite3",
}
KEEP_FOLDERS = {"engine", "mission-events"}
# What a start reads from the device's account outside its state folder: the kit's
# record of what it installed and the agent configuration it put Hide's hooks
# into. Kept per machine in `hide-kit/tests/fixtures/release-home/<tag>/<machine>/`,
# whose upgrade test reads them with the kit's own loader.
HOME_FILES = (".hide/kit/installed.json", ".claude/settings.json", ".claude.json", ".hide/agent-hooks/claude-trust.json")


FOLD = """
import pathlib, sqlite3
for path in sorted(pathlib.Path.home().joinpath('.hide/state').glob('*.sqlite3')):
    connection = sqlite3.connect(path)
    connection.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    connection.execute('PRAGMA journal_mode=DELETE')
    connection.execute('VACUUM')
    connection.close()
"""


def run(*args, check=True, capture=False, **kwargs):
    return subprocess.run(list(args), check=check, text=True,
                          stdout=subprocess.PIPE if capture else None, **kwargs)


def docker(*args, check=True, capture=False):
    return run("docker", *args, check=check, capture=capture)


def download(tag, work):
    """The release's device kit for this Docker's architecture, checked."""
    version = tag.removeprefix("v")
    arch = docker("info", "--format", "{{.Architecture}}", capture=True).stdout.strip()
    arch = {"arm64": "aarch64", "amd64": "x86_64"}.get(arch, arch)
    platform = f"linux-{arch}"
    asset = f"hide-device-{platform}-{version}.tar.gz"
    # The public download address, not the API: a release asset fetched
    # through the API counts against the account's shared rate limit, which
    # agents beside this run exhaust. The digests below are what is trusted.
    for name in (asset, "device-bundles.json"):
        urllib.request.urlretrieve(f"https://github.com/{REPO}/releases/download/{tag}/{name}", work / name)
    manifest = json.loads((work / "device-bundles.json").read_text())
    bundle = manifest["platforms"][platform]
    digest = hashlib.sha256((work / asset).read_bytes()).hexdigest()
    if manifest["version"] != version or bundle["asset"] != asset or digest != bundle["sha256"]:
        raise SystemExit(f"{asset} does not match the release's device-bundles.json")
    (work / "hide").mkdir()
    run("tar", "-xzf", str(work / asset), "-C", str(work / "hide"))
    for name, program in bundle["files"].items():
        if hashlib.sha256((work / "hide" / name).read_bytes()).hexdigest() != program["sha256"]:
            raise SystemExit(f"{name} does not match the release's device-bundles.json")
        # The 1.2.3 archive keeps `hided` without its execute bit; the core
        # sets it when it installs a kit, and so does this.
        (work / "hide" / name).chmod(0o755)
    return manifest


# The operator's own Claude Code settings, there before Hide's kit adds its
# hooks: a permission and a Stop hook of another tool, which every later
# build's install and removal must leave as they are.
OPERATOR_SETTINGS = json.dumps({
    "permissions": {"allow": ["Bash(ls:*)"]},
    "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo operator-stop"}]}]}})


def setup(tag, work, names):
    image, network, core, device = names
    for name in ("Dockerfile", "agent.py", "agent.c"):
        shutil.copy(HERE / name, work / name)
    (work / "settings.json").write_text(OPERATOR_SETTINGS)
    docker("build", "-q", "-t", image, str(work))
    docker("network", "create", network)
    key = work / "id_ed25519"
    run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "release-state", "-f", str(key))
    device_script = (
        f"echo {DEVICE_ID} > /etc/machine-id && useradd -m -s /bin/bash remote && mkdir -p /home/remote/.ssh /run/sshd"
        " && cp /key.pub /home/remote/.ssh/authorized_keys && mkdir -p /home/remote/.claude"
        " && cp /settings.json /home/remote/.claude/settings.json"
        " && chown -R remote:remote /home/remote/.ssh /home/remote/.claude"
        " && chmod 700 /home/remote/.ssh && chmod 600 /home/remote/.ssh/authorized_keys && exec /usr/sbin/sshd -D -e")
    docker("run", "-d", "--name", device, "--hostname", "device", "--network", network, "--network-alias", "device",
           "-v", f"{key}.pub:/key.pub:ro", "-v", f"{work / 'settings.json'}:/settings.json:ro", image, "sh", "-c", device_script)
    core_script = (
        f"echo {CORE_ID} > /etc/machine-id && useradd -m -s /bin/bash me && mkdir -p /home/me/.ssh"
        " && cp /key /home/me/.ssh/id_ed25519"
        f" && printf 'Host mini\\n  HostName device\\n  User remote\\n  StrictHostKeyChecking accept-new\\n' > /home/me/.ssh/config"
        # Claude Code is set up for both accounts, and the core's Hide AI
        # runs on it (`agent.py` answers), as an operator's first run picks.
        " && mkdir -p /home/me/.claude /home/me/.local/state/hide && cp /settings.json /home/me/.claude/settings.json"
        " && printf '{\"provider\":\"claude\",\"models\":{\"claude\":\"sonnet\"}}' > /home/me/.local/state/hide/ai.json"
        " && chown -R me:me /home/me/.ssh /home/me/.claude /home/me/.local && chmod 700 /home/me/.ssh && chmod 600 /home/me/.ssh/*"
        " && exec sleep infinity")
    docker("run", "-d", "--name", core, "--hostname", "core", "--network", network,
           "-v", f"{key}:/key:ro", "-v", f"{work / 'settings.json'}:/settings.json:ro",
           "-v", f"{ROOT / 'scripts'}:/scripts:ro", image, "sh", "-c", core_script)
    as_me = ["exec", "-u", "me", "-e", "HOME=/home/me", "-e", "USER=me", "-e", "SHELL=/bin/bash", "-w", "/home/me", core]
    # sshd and the account come up a moment after the container.
    docker(*as_me, "sh", "-c", "for i in $(seq 30); do ssh -o BatchMode=yes mini true 2>/dev/null && exit 0; sleep 1; done; exit 1")
    return as_me


def generate(tag, out, home_out):
    suffix = secrets.token_hex(3)
    version = tag.removeprefix("v")
    names = (f"hide-release-state:{version}-{suffix}", f"hide-release-state-{suffix}",
             f"hide-release-state-core-{suffix}", f"hide-release-state-device-{suffix}")
    image, network, core, device = names
    runs = ROOT / "agents/runs"
    runs.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix=f"release-state-{version}-", dir=runs))
    # The old core's log is a run artifact, kept under agents/runs/ on every exit.
    log = runs / f"release-state-{version}.log"
    try:
        manifest = download(tag, work)
        as_me = setup(tag, work, names)
        docker(*as_me[:-1], "-d", core, "sh", "-c", "/opt/hide/hided > /home/me/hided.log 2>&1")
        docker(*as_me, "sh", "-c", "for i in $(seq 30); do /opt/hide/hide status >/dev/null 2>&1 && exit 0; sleep 1; done; exit 1")
        scenario = docker(*as_me, "python3", "-I", "/scripts/release_state/scenario.py", "/home/me/scenario.json",
                          check=False)
        docker("cp", f"{core}:/home/me/scenario.json", str(work / "scenario.json"))
        docker(*as_me, "sh", "-c", "/opt/hide/hide stop; for i in $(seq 30); do pgrep -u me -x hided >/dev/null || exit 0; sleep 1; done; exit 1")
        # Fold every SQLite log into its store and drop its free pages, so
        # the folder holds no -wal and no empty space. Python's SQLite, since
        # the shell's defensive mode refuses Project Memory's FTS triggers.
        docker(*as_me, "python3", "-I", "-c", FOLD)
        docker("cp", f"{core}:/home/me/.hide/state", str(work / "state"))
        # The device's account only: the core runs outside a desktop
        # package, whose kit installs nothing (`kit.unavailable`).
        for machine, (container, home) in {"device": (device, "/home/remote")}.items():
            for name in HOME_FILES:
                target = work / "homes" / machine / name
                target.parent.mkdir(parents=True, exist_ok=True)
                docker("cp", f"{container}:{home}/{name}", str(target), check=False, capture=True)
        report = json.loads((work / "scenario.json").read_text())
        for step in report["steps"]:
            print(f"{ {True: 'ok  ', None: 'n/a ', False: 'FAIL'}[step['ok']] } {step['step']}: {json.dumps(step['detail'])[:300 if step['ok'] else 3000]}")
        if scenario.returncode != 0:
            raise SystemExit(f"the scenario stopped before its device and agents were up; the core's log is {log}")
        keep(work / "state", out)
        keep_homes(work / "homes", home_out)
        (out.parent / f"{out.name}.json").write_text(json.dumps({
            "tag": tag, "commit": manifest["commit"], "core_node": CORE_ID, "device": {"alias": "mini", "machine_id": DEVICE_ID},
            "steps": [{"step": step["step"], "ok": step["ok"],
                       **({} if step["ok"] else {"why": str(step["detail"])[:300]})}
                      for step in report["steps"]]}, indent=1) + "\n")
        print(f"wrote {out.relative_to(ROOT) if out.is_relative_to(ROOT) else out}; the old core's log is {log}")
    finally:
        docker("cp", f"{core}:/home/me/hided.log", str(log), check=False, capture=True)
        for container in (core, device):
            docker("rm", "-f", container, check=False, capture=True)
        docker("network", "rm", network, check=False, capture=True)
        docker("rmi", "-f", image, check=False, capture=True)
        shutil.rmtree(work, ignore_errors=True)


def keep(state, out):
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    for entry in sorted(state.iterdir()):
        if entry.is_file() and entry.name in KEEP_FILES:
            shutil.copy(entry, out / entry.name)
        elif entry.is_dir() and entry.name in KEEP_FOLDERS:
            shutil.copytree(entry, out / entry.name, ignore=shutil.ignore_patterns("*.sock", "*.lock", "*.log"))


def keep_homes(homes, out):
    """Each account's files a start reads outside the state folder, under
    `<machine>/` at their place in the home."""
    if out.exists():
        shutil.rmtree(out)
    for path in sorted(homes.rglob("*")):
        if path.is_file():
            target = out / path.relative_to(homes)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(path, target)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("tag", help="a published release tag, such as v1.2.1")
    parser.add_argument("--out", type=Path, help="default: hide-core/tests/fixtures/release-state/<tag>")
    parser.add_argument("--home-out", type=Path, help="default: hide-kit/tests/fixtures/release-home/<tag>")
    args = parser.parse_args()
    out = (args.out or ROOT / "hide-core/tests/fixtures/release-state" / args.tag).resolve()
    home_out = (args.home_out or ROOT / "hide-kit/tests/fixtures/release-home" / args.tag).resolve()
    generate(args.tag, out, home_out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
