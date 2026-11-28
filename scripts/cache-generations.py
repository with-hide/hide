#!/usr/bin/env python3
"""Delete the replaced generations of main's dependency caches.

Each Rust dependency set (`.github/actions/rust-cache`) and each hide-vt entry
(`.github/actions/vt-cache`) is saved on main under a key that ends with a
hash of its inputs, so a changed lockfile, manifest, toolchain or pin saves a
new entry beside the old one and nothing removes the old one before GitHub
evicts it. This keeps, for each set and system, the entry main saved last and
deletes the others.

The entry saved last is the current one without recomputing rust-cache's key:
main saves a set only under a key that does not exist yet, so a newer entry
means the inputs moved since the older one was saved. A lockfile change that
is reverted is the one exception, and it costs one rebuild: the deleted entry
is missed once and saved again. docs/BUILD.md, "CI's Rust caches", owns the
sets and this rule.

    cache-generations.py --repo with-hide/hide [--dry-run]
"""
import argparse
from datetime import datetime
import json
import re
import subprocess
import sys

MAIN = "refs/heads/main"

# `v0-rust-<set>-<root manifest>-<system>-<processor>-<environment>-<lockfile>`
# and `vt-<inputs|zig>-v1-<system>-<processor>-<pins>`.
FAMILIES = (
    ("rust", re.compile(r"v0-rust-(?P<set>[a-z]+)-[0-9a-f]{64}-(?P<system>[A-Za-z_]+-[a-z0-9]+)-[0-9a-f]{8}-[0-9a-f]{8}")),
    ("vt", re.compile(r"vt-(?P<set>inputs|zig)-v1-(?P<system>[A-Za-z]+-[A-Z0-9]+)-[0-9a-f]{64}")),
)
# A key under one of these that no pattern reads means the key's shape moved
# (a new rust-cache, a renamed set), and guessing its group could delete a
# current entry.
OWNED_PREFIXES = ("v0-rust-", "vt-")


class UnknownKey(Exception):
    pass


def group_of(key):
    for family, pattern in FAMILIES:
        match = pattern.fullmatch(key)
        if match:
            return family, match["set"], match["system"]
    if key.startswith(OWNED_PREFIXES):
        raise UnknownKey(f"cannot tell the set and system of cache key {key!r}; update scripts/cache-generations.py")
    return None


def created(entry):
    return datetime.fromisoformat(entry["created_at"])


def replaced(entries):
    """The entries of main a newer entry of the same set and system replaced."""
    groups = {}
    for entry in entries:
        if entry["ref"] != MAIN:
            continue
        group = group_of(entry["key"])
        if group is not None:
            groups.setdefault(group, []).append(entry)
    stale = []
    for members in groups.values():
        stale.extend(sorted(members, key=created)[:-1])
    return sorted(stale, key=lambda entry: entry["key"])


def gh(*args):
    return subprocess.run(["gh", "api", *args], check=True, capture_output=True, text=True).stdout


def main_entries(repo):
    lines = gh("--paginate", f"repos/{repo}/actions/caches?ref={MAIN}&per_page=100",
               "--jq", ".actions_caches[] | @json")
    return [json.loads(line) for line in lines.splitlines() if line]


def delete(repo, entry):
    result = subprocess.run(["gh", "api", "-X", "DELETE", f"repos/{repo}/actions/caches/{entry['id']}"],
                            capture_output=True, text=True)
    # Already gone (evicted, or deleted by an overlapping run) is the outcome asked for.
    if result.returncode != 0 and "HTTP 404" not in result.stderr:
        raise RuntimeError(f"deleting cache {entry['id']} ({entry['key']}) failed: {result.stderr.strip()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--dry-run", action="store_true", help="name the entries without deleting them")
    args = parser.parse_args()
    stale = replaced(main_entries(args.repo))
    for entry in stale:
        if not args.dry_run:
            delete(args.repo, entry)
        print(f"{'would delete' if args.dry_run else 'deleted'} id={entry['id']} bytes={entry['size_in_bytes']} "
              f"created={entry['created_at']} key={entry['key']}")
    print(f"{len(stale)} replaced entries, {sum(entry['size_in_bytes'] for entry in stale)} bytes")


if __name__ == "__main__":
    try:
        main()
    except UnknownKey as error:
        print(f"::error::{error}")
        sys.exit(1)
