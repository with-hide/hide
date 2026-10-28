#!/usr/bin/env python3
"""Delete main's Rust dependency caches that main's head has replaced.

GitHub never replaces a cache entry: a changed lockfile or manifest saves a new
key beside the old one, and the old one stays until the repository's 10 GB
limit evicts something, which it did to main's newest entries while copies
nobody read stayed (2026-10-28). docs/BUILD.md, "CI's Rust caches", owns the
sets and the key shape (`.github/actions/rust-cache`).

A key names the dependencies it holds:
`v0-rust-<set>-<manifest>-<os>-<arch>-<environment>-<lockfile>`, where
<manifest> hashes the root `Cargo.toml` and <lockfile> is rust-cache's hash of
the lockfile, the members' manifests and `rust-toolchain.toml`, recomputed
here (`lockfile_hash`). This is given main's head, run from its checkout, and
per set, system and architecture it deletes every entry of another manifest or
lockfile hash, but only when the head's own entry is already saved there, so a
set whose save has not happened yet keeps what it has, and so does every set
should rust-cache ever hash differently from `lockfile_hash`. It deletes nothing
when main has moved past the given head, since a newer head's entry carries
other hashes. Which run read an entry, or when, decides nothing.

Keys from before the sets (`v0-rust-<job>-...`) are not touched: they are
deleted by hand once, after each set's new entry is saved, and GitHub drops
what is left after seven days unread. Other refs' entries are left alone: a
pull request saves none, and GitHub scopes any it holds to that pull request.

A cache that could not be listed or deleted is a warning in the run's log and
the run still passes: the next run on main tries again, and pruning is no
build's result.

    python3 scripts/prune-rust-caches.py --repo with-hide/hide --head <sha> --manifest <hash> [--dry-run]
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

MAIN = 'refs/heads/main'
PREFIX = 'v0-rust-'
SETS = ('workspace', 'platform', 'release')
KEY = re.compile(rf'^{PREFIX}(?P<set>{"|".join(SETS)})-(?P<manifest>[0-9a-f]{{64}})'
                 r'-(?P<os>Linux|Darwin|Windows_NT)-(?P<arch>x64|arm64)-[0-9a-f]+-(?P<lockfile>[0-9a-f]{8})$')


def javascript_json(value):
    """`JSON.stringify(value)`, which rust-cache hashes."""
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


def lockfile_hash(root):
    """rust-cache v2's lockfile hash of the workspace at `root` (`src/config.ts`).

    Each member's manifest with its own and its path dependencies' versions and
    paths blanked, then the lockfile's registry and git packages, then
    `rust-toolchain.toml`, through one SHA-1, of which it keeps 8 digits. It
    reproduces the hashes CI's logs show for main's commits of 2026-10-24 to 28.
    """
    members = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['members']
    hasher = hashlib.sha1()
    if any(character in member for member in members for character in '*?['):
        raise ValueError(f'a workspace member is a glob, which cargo metadata would expand: {members}')
    # Sorted as manifest paths: `hide-node-link/` comes before `hide-node/`.
    for manifest_path in sorted(f'{member}/Cargo.toml' for member in members):
        manifest = tomllib.loads((root / manifest_path).read_text())
        if 'version' in manifest.get('package', {}):
            manifest['package']['version'] = '0.0.0'
        for section in ('dependencies', 'build-dependencies', 'dev-dependencies'):
            for dependency in manifest.get(section, {}).values():
                if isinstance(dependency, dict) and 'path' in dependency:
                    dependency['version'] = '0.0.0'
                    dependency['path'] = ''
        hasher.update(javascript_json(manifest))
    lock = tomllib.loads((root / 'Cargo.lock').read_text())
    hasher.update(javascript_json([package for package in lock['package']
                                   if 'source' in package or 'checksum' in package]))
    hasher.update((root / 'rust-toolchain.toml').read_bytes())
    return hasher.hexdigest()[:8]


def doomed(entries, manifest, lockfile):
    """The entries to delete, oldest first; `entries` are GitHub's cache objects."""
    groups = {}
    for entry in entries:
        match = KEY.match(entry['key']) if entry['ref'] == MAIN else None
        if match:
            head = (match['manifest'], match['lockfile']) == (manifest, lockfile)
            groups.setdefault((match['set'], match['os'], match['arch']), []).append((head, entry))
    gone = []
    for group in groups.values():
        if any(head for head, _ in group):
            gone += [entry for head, entry in group if not head]
    return sorted(gone, key=lambda entry: (entry['created_at'], entry['id']))


def gh(*args):
    return subprocess.run(['gh', *args], check=True, capture_output=True, text=True).stdout


def listed(repo):
    pages = gh('api', '--paginate', '--slurp', f'repos/{repo}/actions/caches?ref={MAIN}&key={PREFIX}&per_page=100')
    return [entry for page in json.loads(pages) for entry in page['actions_caches']]


def main_head(repo):
    return gh('api', f'repos/{repo}/commits/main', '--jq', '.sha').strip()


def delete(repo, entry):
    """Why deleting one entry failed, or None; one already gone counts as deleted."""
    result = subprocess.run(['gh', 'api', '-X', 'DELETE', f'repos/{repo}/actions/caches/{entry["id"]}'],
                            capture_output=True, text=True)
    if result.returncode != 0 and 'HTTP 404' not in result.stderr:
        return result.stderr.strip()
    return None


def warn(message):
    print(f'::warning title=rust cache prune::{message}')


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n', 1)[0])
    parser.add_argument('--repo', required=True)
    parser.add_argument('--head', required=True, help='the main commit whose run just ended, checked out here')
    parser.add_argument('--manifest', required=True, help="the root Cargo.toml's hashFiles, as the key carries it")
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if not re.fullmatch(r'[0-9a-f]{64}', args.manifest):
        parser.error(f'--manifest is not a 64-digit hash: {args.manifest!r}')
    lockfile = lockfile_hash(Path.cwd())
    print(f'main {args.head}: manifest {args.manifest[:8]}, lockfile {lockfile}')
    try:
        entries = listed(args.repo)
        # Last, right before deleting: a newer head's entry carries other hashes.
        head = main_head(args.repo)
    except subprocess.CalledProcessError as error:
        warn(f'reading main\'s Rust caches or head failed: {error.stderr.strip()}')
        return 0
    if head != args.head:
        print(f'main moved from {args.head} to {head}; nothing deleted')
        return 0
    gone = []
    for entry in doomed(entries, args.manifest, lockfile):
        print(f'{"would delete" if args.dry_run else "delete"} {entry["key"]} '
              f'({entry["size_in_bytes"] // 2**20} MB, created {entry["created_at"]})')
        reason = None if args.dry_run else delete(args.repo, entry)
        if reason is None:
            gone.append(entry)
        else:
            warn(f'deleting {entry["key"]} failed: {reason}')
    total = sum(entry['size_in_bytes'] for entry in entries)
    freed = sum(entry['size_in_bytes'] for entry in gone)
    print(f'{len(entries) - len(gone)} of {len(entries)} Rust entries on main kept, '
          f'{(total - freed) // 2**20} MB of {total // 2**20} MB')
    return 0


if __name__ == '__main__':
    sys.exit(main())
