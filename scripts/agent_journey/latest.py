"""Each agent CLI's latest release, read from where its installer reads it,
so the daily agent canary measures what an operator installing today gets.

Every source is the one the vendor's own installer asks: npm's `latest`
dist-tag for an npm row, Grok's `stable` channel pointer, omp's latest
GitHub release, and agy's manifest of each system CI installs it on, whose
file and sha512 the row records, as the pin does (contracts/README.md). A
source that does not answer a version fails the whole table rather than
leaving a row at its pin, which would read as measured.
"""

import json
import re
import shutil
import subprocess
import urllib.request

from . import clis

GROK_CHANNEL = "https://x.ai/cli/stable"
OMP_LATEST = "https://api.github.com/repos/can1357/oh-my-pi/releases/latest"
# install.sh and install.ps1 ask the manifest of the machine's platform, by
# the names `clis.AGY_SYSTEMS` gives the systems CI installs agy on.
AGY_MANIFESTS = {system: f"https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{name}.json"
                 for system, name in zip(clis.AGY_SYSTEMS, ("darwin_arm64", "linux_amd64", "windows_amd64"))}
HERDR_LATEST = "https://api.github.com/repos/herdrdev/herdr/releases/latest"
VERSION = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$")


class SourceError(Exception):
    pass


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "hide-agent-canary"})
    with urllib.request.urlopen(request, timeout=60) as answer:
        return answer.read().decode()


def checked(kind, version):
    version = (version or "").strip().removeprefix("v")
    if not VERSION.match(version):
        raise SourceError(f"{kind}: its release source answered {version!r}, not a version")
    return version


def npm_latest(package):
    program = shutil.which("npm")
    if not program:
        raise SourceError("npm is not on PATH")
    found = subprocess.run([program, "view", f"{package}@latest", "version"], capture_output=True, text=True,
                           timeout=120)
    if found.returncode:
        raise SourceError(f"npm view {package} exited {found.returncode}: {found.stderr.strip()[:200]}")
    return found.stdout


def latest(kind, row, read=fetch, npm=npm_latest):
    """The fields of `row` that name the latest release of the CLI it installs."""
    installer = row["installer"]
    if installer == "npm":
        return {"version": checked(kind, npm(row["package"]))}
    if installer == "grok":
        pointer = read(GROK_CHANNEL).strip()
        return {"version": checked(kind, pointer.splitlines()[0] if pointer else "")}
    if installer == "omp":
        return {"version": checked(kind, json.loads(read(OMP_LATEST)).get("tag_name"))}
    if installer == "agy":
        return agy_latest(kind, read)
    raise SourceError(f"{kind}: no latest source for the installer {installer!r}")


def agy_latest(kind, read):
    """agy's version and, for each system, the file and sha512 its manifest names."""
    manifests = {system: json.loads(read(url)) for system, url in AGY_MANIFESTS.items()}
    versions = {checked(kind, manifest.get("version")) for manifest in manifests.values()}
    if len(versions) != 1:
        raise SourceError(f"{kind}: its manifests name more than one release: {sorted(versions)}")
    files = {}
    for system, manifest in manifests.items():
        if not manifest.get("url") or not manifest.get("sha512"):
            raise SourceError(f"{kind}: the {system} manifest names no url and sha512")
        files[system] = {"url": manifest["url"], "sha512": manifest["sha512"].lower()}
    return {"version": versions.pop(), "files": files}


def latest_table(rows, read=fetch, npm=npm_latest):
    """`rows` with each release replaced by its latest."""
    return {kind: {**row, **latest(kind, row, read, npm)} for kind, row in rows.items()}


def herdr_latest(read=fetch):
    return checked("herdr", json.loads(read(HERDR_LATEST)).get("tag_name"))
