#!/usr/bin/env python3
"""Delete main's Rust dependency caches that main's head has replaced.

GitHub never replaces a cache entry: a changed lockfile or manifest saves a new
key beside the old one, and the old one stays until the repository's 10 GB
limit evicts something, which it did to main's newest entries while copies
nobody read stayed (2026-10-28). docs/BUILD.md, "CI's Rust caches", owns the
sets and the key shape (`.github/actions/rust-cache`).

A key names the dependencies it holds: `v0-rust-<set>-<deps>-<os>-<arch>-...`,
where <deps> hashes the lockfile, the manifests and the pinned toolchain of the
commit that saved it. This is given main's head and that head's <deps>, and per
set, system and architecture it deletes every entry of another <deps>, but only
when the head's own entry is already saved there, so a set whose save has not
happened yet keeps what it has. It deletes nothing when main has moved past the
given head, since a newer head's entry would carry another <deps>. Which run
read an entry, or when, decides nothing.

Keys from before the sets (`v0-rust-<job>-...`) are not touched: they are
deleted by hand once, after each set's new entry is saved, and GitHub drops
what is left after seven days unread.
Other refs' entries are left alone: a pull request saves none, and GitHub
scopes any it holds to that pull request.

A cache that could not be listed or deleted is a warning in the run's log and
the run still passes: the next run on main tries again, and pruning is no
build's result.

    python3 scripts/prune-rust-caches.py --repo with-hide/hide --head <sha> --deps <hash> [--dry-run]
"""
import argparse
import json
import re
import subprocess
import sys

MAIN = 'refs/heads/main'
PREFIX = 'v0-rust-'
SETS = ('workspace', 'platform', 'release')
KEY = re.compile(rf'^{PREFIX}(?P<set>{"|".join(SETS)})-(?P<deps>[0-9a-f]{{64}})'
                 r'-(?P<os>Linux|Darwin|Windows_NT)-(?P<arch>x64|arm64)-[0-9a-f]+-[0-9a-f]+$')


def doomed(entries, deps):
    """The entries to delete, oldest first; `entries` are GitHub's cache objects."""
    groups = {}
    for entry in entries:
        match = KEY.match(entry['key']) if entry['ref'] == MAIN else None
        if match:
            groups.setdefault((match['set'], match['os'], match['arch']), []).append((match['deps'], entry))
    gone = []
    for group in groups.values():
        if any(saved == deps for saved, _ in group):
            gone += [entry for saved, entry in group if saved != deps]
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
    parser.add_argument('--head', required=True, help="the main commit whose run just ended")
    parser.add_argument('--deps', required=True, help="that commit's dependency hash, as the key carries it")
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if not re.fullmatch(r'[0-9a-f]{64}', args.deps):
        parser.error(f'--deps is not a 64-digit hash: {args.deps!r}')
    try:
        entries = listed(args.repo)
        # Last, right before deleting: a newer head's entry carries other deps.
        head = main_head(args.repo)
    except subprocess.CalledProcessError as error:
        warn(f'reading main\'s Rust caches or head failed: {error.stderr.strip()}')
        return 0
    if head != args.head:
        print(f'main moved from {args.head} to {head}; nothing deleted')
        return 0
    gone = []
    for entry in doomed(entries, args.deps):
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
