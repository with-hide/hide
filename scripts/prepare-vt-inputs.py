#!/usr/bin/env python3
"""Prepares the inputs hide-vt builds from, so its build needs no network.

Usage: prepare-vt-inputs.py [--quiet] [--manifest <build-inputs.json>]

`hide-vt/build-inputs.json` pins every input by digest: the Zig 0.16.0
compiler for this machine, the Herdr v0.9.5 source archive whose
`vendor/libghostty-vt` is the libghostty-vt source dist with Herdr's patches
applied, and the Zig packages that source's build needs. This script downloads
each one once, refuses any whose digest differs, checks that each of Herdr's
patches is applied to the extracted tree, has Zig fetch the packages (Zig
checks each against its own content hash) and moves the finished folder into
place in one rename. `hide-vt/build.rs` then reads only that folder, and runs
Zig with fetching turned off.

The folder is a download cache shared by every checkout on the machine, like
the Rust toolchain, never a build directory: Zig compiles into the worktree's
own `target/`. It lives under `$HIDE_BUILD_CACHE` (default `~/.cache/hide-build`,
or `%LOCALAPPDATA%\\hide-build` on Windows) at `vt/<manifest digest>-<host>`,
and the three most recently prepared entries are kept. docs/BUILD.md, "The
terminal parser and Zig", owns the reasons.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MANIFEST = REPO / "hide-vt" / "build-inputs.json"
KEEP_ENTRIES = 3
# A temporary folder this old belongs to a run that died before its rename.
STALE_TMP_SECONDS = 24 * 60 * 60
DOWNLOAD_ATTEMPTS = 3
# Up to 28 s for a scanner to let go of a file just written (move_into_place).
RENAME_ATTEMPTS = 8
DOWNLOAD_TIMEOUT_SECONDS = 120


class Refused(Exception):
    """An input that does not match its pin, said in one line."""


def manifest_digest(raw: bytes) -> str:
    # A Windows checkout may carry CRLF; the pin is the text, not its endings.
    return hashlib.sha256(raw.replace(b"\r\n", b"\n")).hexdigest()[:16]


def zig_host() -> str:
    machine = platform.machine().lower()
    arch = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64", "amd64": "x86_64"}.get(machine)
    system = {"darwin": "macos", "linux": "linux", "win32": "windows"}.get(sys.platform)
    if arch is None or system is None:
        raise Refused(f"no Zig archive is pinned for {sys.platform} on {machine}")
    return f"{arch}-{system}"


def cache_root() -> Path:
    explicit = os.environ.get("HIDE_BUILD_CACHE")
    if explicit:
        return Path(explicit)
    if sys.platform == "win32":
        return Path(os.environ["LOCALAPPDATA"]) / "hide-build"
    return Path.home() / ".cache" / "hide-build"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def download(url: str, dest: Path, sha256: str) -> None:
    last_error: Exception | None = None
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "hide-prepare-vt-inputs"})
            with urllib.request.urlopen(request, timeout=DOWNLOAD_TIMEOUT_SECONDS) as response, dest.open("wb") as out:
                shutil.copyfileobj(response, out, 1 << 20)
            break
        except OSError as error:
            last_error = error
            if attempt < DOWNLOAD_ATTEMPTS:
                time.sleep(2 * attempt)
    else:
        raise Refused(f"could not download {url}: {last_error}")
    actual = sha256_file(dest)
    if actual != sha256:
        raise Refused(f"{url} has sha256 {actual}, but hide-vt/build-inputs.json pins {sha256}")


def safe_members(archive: tarfile.TarFile, prefix: str):
    for member in archive.getmembers():
        name = member.name
        if not name.startswith(prefix):
            continue
        if member.issym() or member.islnk() or ".." in Path(name).parts:
            raise Refused(f"refusing archive entry {name}: links and parent paths are not expected")
        yield member


def extract_tar_tree(archive_path: Path, prefix: str, dest: Path) -> None:
    """Extracts the entries under `prefix` into `dest`, without the prefix."""
    with tarfile.open(archive_path) as archive:
        found = False
        for member in safe_members(archive, prefix):
            relative = member.name[len(prefix):].lstrip("/")
            if not relative:
                continue
            found = True
            target = dest / relative
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source, target.open("wb") as out:
                    shutil.copyfileobj(source, out, 1 << 20)
                if member.mode & 0o111:
                    target.chmod(0o755)
    if not found:
        raise Refused(f"{archive_path.name} holds nothing under {prefix}")


def extract_zig(archive_path: Path, dest: Path) -> None:
    if archive_path.name.endswith(".zip"):
        with zipfile.ZipFile(archive_path) as archive:
            names = archive.namelist()
            root = names[0].split("/", 1)[0] + "/"
            for name in names:
                if not name.startswith(root) or ".." in Path(name).parts:
                    raise Refused(f"refusing archive entry {name} in {archive_path.name}")
                relative = name[len(root):]
                if not relative or name.endswith("/"):
                    continue
                target = dest / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(name) as source, target.open("wb") as out:
                    shutil.copyfileobj(source, out, 1 << 20)
        return
    with tarfile.open(archive_path) as archive:
        root = archive.getmembers()[0].name.split("/", 1)[0] + "/"
    extract_tar_tree(archive_path, root, dest)


def zig_binary(folder: Path) -> Path:
    return folder / ("zig.exe" if sys.platform == "win32" else "zig")


def check_patches_applied(source: Path, herdr: Path, patches: dict[str, str]) -> None:
    # Herdr's tree is the libghostty-vt dist with its patches already applied,
    # so each patch must apply in reverse; one that does not means the tree is
    # not the one its patch list describes. The patches name files as
    # `a/vendor/libghostty-vt/...`, three components above the tree.
    environment = dict(os.environ, GIT_CEILING_DIRECTORIES=str(source.parent))
    for relative in patches:
        patch = herdr / relative
        result = subprocess.run(
            ["git", "apply", "--check", "--reverse", "-p3", str(patch)],
            cwd=source, env=environment, capture_output=True, text=True,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip().splitlines()
            raise Refused(f"{Path(relative).name} is not applied to the extracted libghostty-vt: {detail[0] if detail else 'git apply failed'}")


def fetch_packages(zig: Path, packages: list[dict[str, str]], work: Path, dest: Path) -> None:
    """Has Zig fetch exactly the pinned packages, each checked by its hash."""
    project = work / "hide_vt_packages"
    project.mkdir(parents=True)
    environment = dict(os.environ, ZIG_GLOBAL_CACHE_DIR=str(work / "global"), ZIG_LOCAL_CACHE_DIR=str(work / "local"))
    # `zig init` writes a manifest whose fingerprint matches the project name.
    subprocess.run([str(zig), "init", "-m"], cwd=project, env=environment, check=True, capture_output=True)
    zon = project / "build.zig.zon"
    text = zon.read_text().rstrip()
    if not text.endswith("}"):
        raise Refused("zig init wrote a manifest this script cannot extend")
    lines = "".join(
        f'        .p{index} = .{{ .url = "{package["url"]}", .hash = "{package["hash"]}" }},\n'
        for index, package in enumerate(packages)
    )
    zon.write_text(text[:-1] + "    .dependencies = .{\n" + lines + "    },\n}\n")
    result = subprocess.run([str(zig), "build", "--fetch"], cwd=project, env=environment, capture_output=True, text=True)
    if result.returncode != 0:
        raise Refused(f"zig could not fetch the pinned packages: {result.stderr.strip()}")
    fetched = project / "zig-pkg"
    names = sorted(path.name for path in fetched.iterdir()) if fetched.is_dir() else []
    expected = sorted(package["hash"] for package in packages)
    if names != expected:
        raise Refused(f"zig fetched {names}, but hide-vt/build-inputs.json pins {expected}")
    fetched.rename(dest)


def move_into_place(staged: Path, final: Path) -> None:
    for attempt in range(1, RENAME_ATTEMPTS + 1):
        try:
            staged.rename(final)
            return
        except OSError as error:
            if (final / "ready.json").is_file():
                # Another checkout finished the same entry first; its copy is the same pin.
                return
            # Windows refuses to rename a folder while another process holds a
            # file in it open, as a malware scanner does with the Zig binary
            # just written; that hold ends on its own, so wait for it, boundedly.
            if not isinstance(error, PermissionError) or attempt == RENAME_ATTEMPTS:
                raise Refused(f"could not move the prepared inputs into {final}: {error}") from error
            time.sleep(attempt)


def prune(vt_root: Path, keep: Path) -> None:
    now = time.time()
    ready = []
    for entry in vt_root.iterdir():
        if entry.name.startswith(".tmp-"):
            if now - entry.stat().st_mtime > STALE_TMP_SECONDS:
                shutil.rmtree(entry, ignore_errors=True)
            continue
        stamp = entry / "ready.json"
        if stamp.is_file():
            ready.append((stamp.stat().st_mtime, entry))
    ready.sort(reverse=True)
    for _, entry in ready[KEEP_ENTRIES:]:
        if entry != keep:
            shutil.rmtree(entry, ignore_errors=True)


def prepare(quiet: bool, manifest_path: Path = MANIFEST) -> Path:
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw)
    digest = manifest_digest(raw)
    host = zig_host()
    vt_root = cache_root() / "vt"
    final = vt_root / f"{digest}-{host}"
    stamp = final / "ready.json"
    if stamp.is_file() and json.loads(stamp.read_text()).get("digest") == digest:
        os.utime(stamp)
        prune(vt_root, final)
        return final

    vt_root.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix=f".tmp-{digest}-{host}-", dir=vt_root))
    try:
        downloads = work / "downloads"
        downloads.mkdir()
        staged = work / "inputs"
        staged.mkdir()

        zig_pin = manifest["zig"]["archives"].get(host)
        if zig_pin is None:
            raise Refused(f"hide-vt/build-inputs.json pins no Zig archive for {host}")
        zig_archive = downloads / zig_pin["url"].rsplit("/", 1)[1]
        if not quiet:
            print(f"downloading {zig_pin['url']}", file=sys.stderr)
        download(zig_pin["url"], zig_archive, zig_pin["sha256"])
        extract_zig(zig_archive, staged / "zig")
        zig = zig_binary(staged / "zig")
        version = subprocess.run([str(zig), "version"], capture_output=True, text=True, check=True).stdout.strip()
        if version != manifest["zig"]["version"]:
            raise Refused(f"the pinned Zig archive reports {version}, not {manifest['zig']['version']}")

        source = manifest["source"]
        source_archive = downloads / "source.tar.gz"
        if not quiet:
            print(f"downloading {source['url']}", file=sys.stderr)
        download(source["url"], source_archive, source["sha256"])
        root = source["root"] + "/"
        extract_tar_tree(source_archive, root + source["tree"] + "/", staged / "source")
        herdr = staged / "herdr"
        with tarfile.open(source_archive) as archive:
            for relative, sha256 in {**source["files"], **source["patches"]}.items():
                member = archive.getmember(root + relative)
                target = herdr / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as data, target.open("wb") as out:
                    shutil.copyfileobj(data, out)
                actual = sha256_file(target)
                if actual != sha256:
                    raise Refused(f"{relative} in the Herdr archive has sha256 {actual}, but hide-vt/build-inputs.json pins {sha256}")
        check_patches_applied(staged / "source", herdr, source["patches"])

        if not quiet:
            print(f"fetching {len(manifest['packages'])} Zig packages", file=sys.stderr)
        fetch_packages(zig, manifest["packages"], work / "fetch", staged / "packages")

        (staged / "ready.json").write_text(json.dumps({"digest": digest, "host": host, "zig": version}) + "\n")
        if final.exists() and not (final / "ready.json").is_file():
            # What a run that died during its rename left behind.
            shutil.rmtree(final)
        move_into_place(staged, final)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    prune(vt_root, final)
    return final


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--quiet", action="store_true", help="print nothing but the folder")
    parser.add_argument("--manifest", type=Path, default=MANIFEST, help="the pins to prepare (tests use a copy)")
    arguments = parser.parse_args()
    try:
        folder = prepare(arguments.quiet, arguments.manifest)
    except Refused as refusal:
        print(f"prepare-vt-inputs: {refusal}", file=sys.stderr)
        return 1
    print(folder)
    return 0


if __name__ == "__main__":
    sys.exit(main())
