#!/usr/bin/env python3
"""Delete main's Rust dependency caches that a run of main's head replaced.

GitHub never replaces a cache entry: a changed lockfile or manifest saves a new
key beside the old one, and the old one stays until the repository's 10 GB
limit evicts something, which it did to main's newest entries while copies
nobody read stayed (2026-10-28). docs/BUILD.md, "CI's Rust caches", owns the
sets and the key shape (`.github/actions/rust-cache`).

This runs after a run of main's head (a push to main, or the nightly) and is
given the time that run started. Every job of that run restored or saved its
set's key for the head it built, which touches the entry: a restore moves its
`last_accessed_at` and a save sets its `created_at`. So, per set, system and
architecture:

- an entry touched since the run started is what main's head uses, or newer
  (a later merge's run), and is kept;
- the group's other entries are deleted, but only when the group holds a
  touched entry, so a set the run did not build (the release set on a push)
  and a set whose save failed keep what they have;
- a key from before the sets is deleted only when a touched entry of the same
  system replaces it.

Other refs' entries are left alone: a pull request saves none, and GitHub
scopes any it holds to that pull request.

A cache that could not be listed or deleted is a warning in the run's log and
the run still passes: the next run on main tries again, and pruning is no
build's result.

    python3 scripts/prune-rust-caches.py --repo with-hide/hide --since <run start> [--dry-run]
"""
import argparse
from datetime import datetime
import json
import re
import subprocess
import sys

MAIN = 'refs/heads/main'
PREFIX = 'v0-rust-'
SETS = ('workspace', 'platform', 'release')
SYSTEM = r'(?P<os>Linux|Darwin|Windows_NT)-(?P<arch>x64|arm64)'
# v0-rust-<set>-<root manifest sha256>-<os>-<arch>-<environment>-<lockfile>
KEY = re.compile(rf'^{PREFIX}(?P<set>{"|".join(SETS)})-[0-9a-f]{{64}}-{SYSTEM}-[0-9a-f]+-[0-9a-f]+$')
# v0-rust-<job>-<os>-<arch>-<environment>-<lockfile>, keyed by job before the sets.
LEGACY = re.compile(rf'^{PREFIX}[a-z0-9-]+?-{SYSTEM}-[0-9a-f]+-[0-9a-f]+$')


def moment(text):
    return datetime.fromisoformat(text.replace('Z', '+00:00'))


def touched(entry, since):
    return max(moment(entry['created_at']), moment(entry['last_accessed_at'])) >= since


def doomed(entries, since):
    """The entries to delete, oldest first; `entries` are GitHub's cache objects."""
    groups = {}
    legacy = []
    for entry in entries:
        if entry['ref'] != MAIN or not entry['key'].startswith(PREFIX):
            continue
        match = KEY.match(entry['key'])
        if match:
            groups.setdefault((match['set'], match['os'], match['arch']), []).append(entry)
        elif LEGACY.match(entry['key']):
            legacy.append(entry)
    gone = []
    replaced = set()
    for (_, os, arch), group in groups.items():
        kept = [entry for entry in group if touched(entry, since)]
        if kept:
            replaced.add((os, arch))
            gone += [entry for entry in group if entry not in kept]
    for entry in legacy:
        match = LEGACY.match(entry['key'])
        if (match['os'], match['arch']) in replaced:
            gone.append(entry)
    return sorted(gone, key=lambda entry: (entry['created_at'], entry['id']))


def listed(repo):
    pages = subprocess.run(['gh', 'api', '--paginate', '--slurp',
                            f'repos/{repo}/actions/caches?ref={MAIN}&key={PREFIX}&per_page=100'],
                           check=True, capture_output=True, text=True).stdout
    return [entry for page in json.loads(pages) for entry in page['actions_caches']]


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
    parser.add_argument('--since', required=True, type=moment,
                        help="when the run of main's head started, ISO 8601")
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    try:
        entries = listed(args.repo)
    except subprocess.CalledProcessError as error:
        warn(f'listing main\'s Rust caches failed: {error.stderr.strip()}')
        return 0
    gone = []
    for entry in doomed(entries, args.since):
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
