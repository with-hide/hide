#!/usr/bin/env python3
"""Git merge driver for contracts/hide-cli.json, plus its one-time registration.

The file is generated (`hide contract --json | python3 -m json.tool`, see
hided/src/cli_contract.rs) and is one 10,000-line document, so two branches that
each add a command conflict on it even though they touched different commands.

As a merge driver (git calls it with `%O %A %B %P`: the common ancestor, the
current branch's version, the other branch's version and the path) it merges the
three versions as data:

- objects key by key, recursively; a key one side added or removed is taken;
- `commands`, a list of objects, entry by entry on their `command` key, and the
  order of the commands by git's own text merge over the command names, so two
  branches that add a command at the same place in the list still conflict;
- `digest` is not merged but computed again exactly as `digest_of` does in Rust:
  sha256 of the body without `digest`, keys sorted, no whitespace; a number
  whose text Rust may write differently (a float with an exponent) is refused
  and the file left as a text merge, since the digest could not be promised.

When both sides changed the same entry differently, the driver changes nothing
that decides it: it leaves git's ordinary text merge of the three files, with
its conflict markers, and exits 1. Neither side is ever picked silently.
The `the_committed_contract_is_the_exported_one` test in hided stays the proof
that the merged file is the contract the code exports; the driver never builds
`hide`.

    python3 scripts/merge-hide-cli-contract.py --register

writes `merge.hide-cli-contract.driver` into the repository's git config, which
every worktree of the clone reads. A checkout that has not run it merges with
git's ordinary text merge, because `.gitattributes` only names the driver and
git ignores a driver nobody configured.
"""

from __future__ import annotations

import hashlib
import json
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

DRIVER = "hide-cli-contract"
SCRIPT = "scripts/merge-hide-cli-contract.py"
LIST_KEYS = {("commands",): "command"}

class _Absent:
    """A key or entry a side does not have."""


_ABSENT = _Absent()


class Conflict(Exception):
    """Both sides changed the same entry differently."""

    def __init__(self, where: str):
        super().__init__(where)
        self.where = where


class Unsupported(Exception):
    """A value whose digest this script cannot promise to equal Rust's."""

    def __init__(self, value):
        super().__init__(f"{value!r} is a number serde_json may write differently than Python")


def check_numbers(value):
    """Refuse the numbers whose text can differ between Rust and Python.

    serde_json writes a float with the shortest digits that read back and an
    exponent without padding (`1e-8`), where Python writes `1e-08`, and the two
    switch to an exponent at different sizes, so a float is written the same
    only when Python's text has no exponent. An integer past 64 bits is read by
    serde_json as a float. The contract holds integers and `0.0` today.
    """
    if isinstance(value, dict):
        for item in value.values():
            check_numbers(item)
    elif isinstance(value, list):
        for item in value:
            check_numbers(item)
    elif isinstance(value, float):
        if "e" in repr(value).lower() or value != value or value in (float("inf"), float("-inf")):
            raise Unsupported(value)
    elif isinstance(value, int) and not isinstance(value, bool):
        if not -(2**63) <= value < 2**64:
            raise Unsupported(value)


def canonical(value):
    """The document with every object's keys in order, as `canonical` in Rust."""
    if isinstance(value, dict):
        return {key: canonical(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [canonical(item) for item in value]
    return value


def digest_of(body: dict) -> str:
    """`sha256:<hex>` of the body, as `digest_of` in hided/src/cli_contract.rs."""
    check_numbers(body)
    text = json.dumps(canonical(body), separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def render(document: dict) -> str:
    """The committed format: `python3 -m json.tool` over the exported contract."""
    return json.dumps(canonical(document), indent=4, ensure_ascii=True) + "\n"


def with_digest(body: dict) -> dict:
    body = {key: value for key, value in body.items() if key != "digest"}
    return {**body, "digest": digest_of(body)}


def merge_value(base, ours, theirs, path: tuple):
    if ours == theirs:
        return ours
    if base == ours:
        return theirs
    if base == theirs:
        return ours
    if (
        isinstance(ours, dict)
        and isinstance(theirs, dict)
        and (isinstance(base, dict) or base is _ABSENT)
    ):
        return merge_object(base, ours, theirs, path)
    key = LIST_KEYS.get(path)
    if key and all(isinstance(side, list) for side in (base, ours, theirs)):
        return merge_keyed_list(base, ours, theirs, path, key)
    raise Conflict("/".join(path) or "the document")


def merge_object(base, ours: dict, theirs: dict, path: tuple) -> dict:
    base = {} if base is _ABSENT else base
    merged = {}
    for name in sorted(set(base) | set(ours) | set(theirs)):
        value = merge_value(
            base.get(name, _ABSENT),
            ours.get(name, _ABSENT),
            theirs.get(name, _ABSENT),
            path + (name,),
        )
        if value is not _ABSENT:
            merged[name] = value
    return merged


def keyed(items: list, key: str, path: tuple) -> dict:
    entries = {}
    for item in items:
        if not isinstance(item, dict) or key not in item or item[key] in entries:
            raise Conflict("/".join(path) + f" (an entry without a unique {key})")
        entries[item[key]] = item
    return entries


def merge_keyed_list(base: list, ours: list, theirs: list, path: tuple, key: str) -> list:
    entries = [keyed(side, key, path) for side in (base, ours, theirs)]
    names = merge_order(*[list(side) for side in entries], path)
    if len(set(names)) != len(names):
        # Moves of one entry to two places: `git merge-file` keeps both.
        raise Conflict("/".join(path) + " (the order of the entries)")
    merged = []
    for name in names + sorted(set().union(*entries) - set(names)):
        value = merge_value(
            entries[0].get(name, _ABSENT),
            entries[1].get(name, _ABSENT),
            entries[2].get(name, _ABSENT),
            path + (name,),
        )
        if value is _ABSENT:
            continue
        if name not in names:
            # The order dropped an entry the entries keep: a change that was
            # not a removal on both sides.
            raise Conflict("/".join(path + (name,)) + " (the order of the entries)")
        merged.append(value)
    return merged


def merge_order(base: list, ours: list, theirs: list, path: tuple) -> list:
    """The three orders merged by `git merge-file`, a conflict where it conflicts."""
    with tempfile.TemporaryDirectory() as folder:
        files = []
        for label, names in (("ours", ours), ("base", base), ("theirs", theirs)):
            file = Path(folder) / label
            file.write_text("".join(f"{name}\n" for name in names), encoding="utf-8")
            files.append(str(file))
        done = subprocess.run(
            ["git", "merge-file", "-p", *files], capture_output=True, check=False
        )
    if done.returncode != 0:
        raise Conflict("/".join(path) + " (the order of the entries)")
    return done.stdout.decode("utf-8").splitlines()


def merge_documents(base: dict, ours: dict, theirs: dict) -> dict:
    """The merged document with its digest, or `Conflict`."""
    sides = [{k: v for k, v in side.items() if k != "digest"} for side in (base, ours, theirs)]
    return with_digest(merge_object(*sides, ()))


def text_merge(base: Path, ours: Path, theirs: Path) -> int:
    """Git's ordinary merge of the three files, written over `ours`; its conflict count."""
    done = subprocess.run(
        ["git", "merge-file", "-L", "ours", "-L", "base", "-L", "theirs", str(ours), str(base), str(theirs)],
        check=False,
    )
    return done.returncode


def load(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise ValueError(f"{path} is not a JSON object")
    return document


def drive(base: Path, ours: Path, theirs: Path, name: str) -> int:
    try:
        merged = merge_documents(load(base), load(ours), load(theirs))
    except Conflict as conflict:
        print(f"{name}: both sides changed {conflict.where}; merge it by hand", file=sys.stderr)
        text_merge(base, ours, theirs)
        return 1
    except Unsupported as unsupported:
        print(f"{name}: {unsupported}; merged as text, regenerate the digest", file=sys.stderr)
        text_merge(base, ours, theirs)
        return 1
    except ValueError as invalid:
        # A side that is not JSON (a half-resolved merge) is git's to merge as text.
        print(f"{name}: {invalid}; merged as text", file=sys.stderr)
        return 1 if text_merge(base, ours, theirs) else 0
    ours.write_text(render(merged), encoding="utf-8", newline="\n")
    print(
        f"{name}: merged command by command and its digest computed again; the committed-contract test in hided proves the result",
        file=sys.stderr,
    )
    return 0


def register() -> int:
    """Put the driver in the repository's own config, which every worktree reads."""
    interpreter = shlex.quote(Path(sys.executable).as_posix())
    command = f"{interpreter} {SCRIPT} %O %A %B %P"
    for setting, value in (
        (f"merge.{DRIVER}.name", "contracts/hide-cli.json by command"),
        (f"merge.{DRIVER}.driver", command),
    ):
        subprocess.run(["git", "config", "--local", setting, value], check=True)
    print(f"registered merge.{DRIVER}.driver: {command}")
    return 0


def main(argv: list[str]) -> int:
    if argv == ["--register"]:
        return register()
    if len(argv) != 4:
        print("usage: merge-hide-cli-contract.py --register | <base> <ours> <theirs> <path>", file=sys.stderr)
        return 2
    base, ours, theirs, name = argv
    return drive(Path(base), Path(ours), Path(theirs), name)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
