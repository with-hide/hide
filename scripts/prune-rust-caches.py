#!/usr/bin/env python3
"""Keep one Rust dependency cache per set and system on main; delete the rest.

GitHub never replaces a cache entry: a changed lockfile or manifest saves a new
key beside the old one, and the old one stays until the repository's 10 GB
limit evicts something, which it did to main's newest entries while copies
nobody read stayed (2026-10-28). docs/BUILD.md, "CI's Rust caches", owns the
sets and the key shape (`.github/actions/rust-cache`).

Of main's `v0-rust-` entries this keeps the newest per set, system and
architecture, and deletes every older one and every key that is not one of the
sets (a key from before the sets). Other refs' entries are left alone: a pull
request saves none, and GitHub scopes any it holds to that pull request. The
newest entry of a group is never deleted, and an entry saved after the listing
is not in it, so a save racing this run is kept.

A cache that could not be listed or deleted is a warning in the run's log and
the run still passes: the next run on main tries again, and pruning is no
build's result.

    python3 scripts/prune-rust-caches.py --repo with-hide/hide [--dry-run]
"""
import argparse
import json
import re
import subprocess
import sys

MAIN = 'refs/heads/main'
PREFIX = 'v0-rust-'
SETS = ('workspace', 'platform', 'release')
# v0-rust-<set>-<root manifest sha256>-<os>-<arch>-<environment>-<lockfile>
KEY = re.compile(rf'^{PREFIX}(?P<set>{"|".join(SETS)})-[0-9a-f]{{64}}'
                 r'-(?P<os>[A-Za-z_]+)-(?P<arch>[a-z0-9]+)-[0-9a-f]+-[0-9a-f]+$')


def doomed(entries):
    """The entries to delete, oldest first; `entries` are GitHub's cache objects."""
    newest = {}
    legacy = []
    for entry in entries:
        if entry['ref'] != MAIN or not entry['key'].startswith(PREFIX):
            continue
        match = KEY.match(entry['key'])
        if match is None:
            legacy.append(entry)
            continue
        group = (match['set'], match['os'], match['arch'])
        kept = newest.get(group)
        if kept is None or (entry['created_at'], entry['id']) > (kept['created_at'], kept['id']):
            newest[group] = entry
    kept = {entry['id'] for entry in newest.values()}
    stale = [entry for entry in entries
             if entry['ref'] == MAIN and KEY.match(entry['key']) and entry['id'] not in kept]
    return sorted(legacy + stale, key=lambda entry: (entry['created_at'], entry['id']))


def gh(*args):
    return subprocess.run(['gh', *args], check=True, capture_output=True, text=True).stdout


def listed(repo):
    pages = gh('api', '--paginate', '--slurp',
               f'repos/{repo}/actions/caches?ref={MAIN}&key={PREFIX}&per_page=100')
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
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    try:
        entries = listed(args.repo)
    except subprocess.CalledProcessError as error:
        warn(f'listing main\'s Rust caches failed: {error.stderr.strip()}')
        return 0
    gone = doomed(entries)
    failed = []
    for entry in gone:
        print(f'{"would delete" if args.dry_run else "delete"} {entry["key"]} '
              f'({entry["size_in_bytes"] // 2**20} MB, created {entry["created_at"]})')
        reason = None if args.dry_run else delete(args.repo, entry)
        if reason is not None:
            warn(f'deleting {entry["key"]} failed: {reason}')
            failed.append(entry)
    gone = [entry for entry in gone if entry not in failed]
    total = sum(entry['size_in_bytes'] for entry in entries)
    freed = sum(entry['size_in_bytes'] for entry in gone)
    print(f'{len(entries) - len(gone)} of {len(entries)} Rust entries on main kept, '
          f'{(total - freed) // 2**20} MB of {total // 2**20} MB')
    return 0


if __name__ == '__main__':
    sys.exit(main())
