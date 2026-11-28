"""What `scripts/merge-hide-cli-contract.py` does with `contracts/hide-cli.json`.

docs/BUILD.md, "Merging contracts/hide-cli.json", is the requirement: two
branches that changed different commands merge with no hand edit and the digest
recomputed as Rust computes it; two that changed the same entry differently get
git's ordinary conflict and neither side is picked; a clone that never ran
`--register` merges the file as text.

The replay reads real merges of main by commit id, so it runs where the history
is: a shallow checkout lacks the commits and skips it.
"""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "merge-hide-cli-contract.py"
SPEC = importlib.util.spec_from_file_location("merge_hide_cli_contract", SCRIPT)
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)

CONTRACT = "contracts/hide-cli.json"


def command(name, **fields):
    return {"arguments": [], "command": name, "options": [], "refusals": ["store_not_found"], **fields}


def document(*commands, **top):
    body = {
        "access": {"modes": ["own", "all"]},
        "answers": {"agent_list": {"type": "object"}},
        "away": [],
        "commands": list(commands),
        "envelopes": {},
        "format": 1,
        "value_types": {"text": {}},
        **top,
    }
    return driver.with_digest(body)


BASE = document(command("ps"), command("agent list"), command("agent show"))


def edit(source, name, **fields):
    changed = copy.deepcopy(source)
    for entry in changed["commands"]:
        if entry["command"] == name:
            entry.update(fields)
    return driver.with_digest(changed)


def add(source, entry, after):
    changed = copy.deepcopy(source)
    names = [item["command"] for item in changed["commands"]]
    changed["commands"].insert(names.index(after) + 1, entry)
    return driver.with_digest(changed)


def merged(base, ours, theirs):
    return driver.merge_documents(base, ours, theirs)


class TheDigestIsRustsDigest(unittest.TestCase):
    """`digest_of` in hided/src/cli_contract.rs, held to the committed file."""

    def test_the_committed_file_carries_the_digest_and_format_the_driver_writes(self):
        text = (ROOT / CONTRACT).read_text(encoding="utf-8")
        committed = json.loads(text)
        self.assertEqual(driver.with_digest(committed)["digest"], committed["digest"])
        self.assertEqual(driver.render(committed), text)

    def test_keys_are_sorted_and_the_body_is_compact_without_the_digest(self):
        body = {"b": [{"z": 1, "a": "\u00e9\u001f"}], "a": None}
        expected = hashlib.sha256('{"a":null,"b":[{"a":"\u00e9\\u001f","z":1}]}'.encode()).hexdigest()
        self.assertEqual(driver.digest_of(body), "sha256:" + expected)

    def test_the_text_written_keeps_non_ascii_escaped_as_json_tool_does(self):
        self.assertIn("\\u00e9", driver.render({"a": "é"}))


class DifferentEntriesMerge(unittest.TestCase):
    def test_a_command_each_side_added_elsewhere_merges_and_the_digest_follows(self):
        ours = add(BASE, command("agent end"), after="agent show")
        theirs = add(BASE, command("mission list"), after="ps")
        result = merged(BASE, ours, theirs)
        self.assertEqual([c["command"] for c in result["commands"]], ["ps", "mission list", "agent list", "agent show", "agent end"])
        self.assertEqual(result["digest"], driver.with_digest(result)["digest"])
        self.assertNotEqual(result["digest"], ours["digest"])

    def test_two_commands_each_side_changed_merge(self):
        ours = edit(BASE, "ps", refusals=["a"])
        theirs = edit(BASE, "agent show", refusals=["b"])
        result = merged(BASE, ours, theirs)
        self.assertEqual(result["commands"][0]["refusals"], ["a"])
        self.assertEqual(result["commands"][2]["refusals"], ["b"])

    def test_different_fields_of_one_command_merge(self):
        ours = edit(BASE, "ps", refusals=["a"])
        theirs = edit(BASE, "ps", hidden=True)
        entry = merged(BASE, ours, theirs)["commands"][0]
        self.assertEqual((entry["refusals"], entry["hidden"]), (["a"], True))

    def test_a_removed_command_and_a_changed_other_one_merge(self):
        ours = copy.deepcopy(BASE)
        ours["commands"].pop(1)
        ours = driver.with_digest(ours)
        theirs = edit(BASE, "ps", refusals=["b"])
        result = merged(BASE, ours, theirs)
        self.assertEqual([c["command"] for c in result["commands"]], ["ps", "agent show"])

    def test_top_level_sections_merge_by_key(self):
        ours = driver.with_digest({**BASE, "answers": {"agent_list": {"type": "object"}, "mission": {}}})
        theirs = driver.with_digest({**BASE, "value_types": {"text": {}, "key": {}}})
        result = merged(BASE, ours, theirs)
        self.assertEqual((sorted(result["answers"]), sorted(result["value_types"])), (["agent_list", "mission"], ["key", "text"]))

    def test_the_same_change_on_both_sides_is_taken_once(self):
        ours = add(BASE, command("agent end"), after="ps")
        self.assertEqual(merged(BASE, ours, ours), ours)

    def test_one_side_unchanged_gives_the_other(self):
        theirs = add(BASE, command("agent end"), after="ps")
        self.assertEqual(merged(BASE, BASE, theirs), theirs)
        self.assertEqual(merged(BASE, theirs, BASE), theirs)


class TheSameEntryDifferentlyConflicts(unittest.TestCase):
    def conflicts(self, ours, theirs):
        with self.assertRaises(driver.Conflict) as raised:
            merged(BASE, ours, theirs)
        return raised.exception.where

    def test_both_sides_change_one_field_of_one_command_differently(self):
        where = self.conflicts(edit(BASE, "ps", refusals=["a"]), edit(BASE, "ps", refusals=["b"]))
        self.assertEqual(where, "commands/ps/refusals")

    def test_one_side_changes_a_command_the_other_removed(self):
        removed = copy.deepcopy(BASE)
        removed["commands"].pop(0)
        self.assertEqual(self.conflicts(edit(BASE, "ps", refusals=["a"]), driver.with_digest(removed)), "commands/ps")

    def test_both_sides_add_the_same_command_differently(self):
        ours = add(BASE, command("agent end", refusals=["a"]), after="ps")
        theirs = add(BASE, command("agent end", refusals=["b"]), after="ps")
        self.assertEqual(self.conflicts(ours, theirs), "commands/agent end/refusals")

    def test_two_commands_added_at_the_same_place_have_no_order_to_pick(self):
        ours = add(BASE, command("agent end"), after="ps")
        theirs = add(BASE, command("mission list"), after="ps")
        self.assertEqual(self.conflicts(ours, theirs), "commands (the order of the entries)")

    def test_a_scalar_both_sides_changed(self):
        self.assertEqual(self.conflicts(driver.with_digest({**BASE, "format": 2}), driver.with_digest({**BASE, "format": 3})), "format")


class AGitMerge(unittest.TestCase):
    """The real `git merge` of two branches, with and without the driver registered."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name) / "clone"
        self.repo.mkdir()
        isolated = mock.patch.dict(os.environ, {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
        isolated.start()
        self.addCleanup(isolated.stop)
        self.git("init", "--quiet", "--initial-branch=main")
        for key, value in (("user.name", "fixture"), ("user.email", "fixture@example.com")):
            self.git("config", key, value)
        (self.repo / "scripts").mkdir()
        shutil.copy(SCRIPT, self.repo / "scripts" / SCRIPT.name)
        shutil.copy(ROOT / ".gitattributes", self.repo / ".gitattributes")
        self.write(BASE)
        self.git("add", "-A")
        self.git("commit", "--quiet", "-m", "start")
        self.base = self.read()

    def git(self, *args, cwd=None, check=True):
        return subprocess.run(
            ["git", *args], cwd=cwd or self.repo, check=check, capture_output=True, text=True
        )

    def write(self, contract):
        path = self.repo / CONTRACT
        path.parent.mkdir(exist_ok=True)
        path.write_text(driver.render(contract), encoding="utf-8", newline="\n")

    def read(self):
        return json.loads((self.repo / CONTRACT).read_text(encoding="utf-8"))

    def branch(self, name, contract):
        self.git("checkout", "--quiet", "-b", name, "main")
        self.write(contract)
        self.git("commit", "--quiet", "-am", name)

    def two_branches(self, ours, theirs):
        self.branch("ours", ours)
        self.branch("theirs", theirs)
        self.git("checkout", "--quiet", "ours")

    def different_commands(self):
        self.two_branches(add(BASE, command("agent end"), after="agent show"), add(BASE, command("mission list"), after="ps"))

    def test_registered_the_branches_merge_with_no_hand_edit(self):
        self.different_commands()
        subprocess.run([sys.executable, "scripts/merge-hide-cli-contract.py", "--register"], cwd=self.repo, check=True, capture_output=True)
        done = self.git("merge", "--no-edit", "theirs", check=False)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        result = self.read()
        self.assertEqual([c["command"] for c in result["commands"]], ["ps", "mission list", "agent list", "agent show", "agent end"])
        self.assertEqual(result["digest"], driver.with_digest(result)["digest"])
        self.assertEqual(self.git("status", "--short").stdout, "")

    def test_registered_in_the_clone_the_driver_serves_every_worktree(self):
        self.different_commands()
        subprocess.run([sys.executable, "scripts/merge-hide-cli-contract.py", "--register"], cwd=self.repo, check=True, capture_output=True)
        other = self.repo.parent / "second"
        self.git("worktree", "add", "--quiet", "-b", "second-ours", str(other), "ours")
        done = self.git("merge", "--no-edit", "theirs", cwd=other, check=False)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(len(json.loads((other / CONTRACT).read_text())["commands"]), 5)

    def test_registered_the_same_command_changed_differently_is_a_conflict_with_markers(self):
        self.two_branches(edit(BASE, "ps", refusals=["a"]), edit(BASE, "ps", refusals=["b"]))
        subprocess.run([sys.executable, "scripts/merge-hide-cli-contract.py", "--register"], cwd=self.repo, check=True, capture_output=True)
        done = self.git("merge", "--no-edit", "theirs", check=False)
        self.assertNotEqual(done.returncode, 0)
        self.assertEqual(self.git("diff", "--name-only", "--diff-filter=U").stdout.split(), [CONTRACT])
        text = (self.repo / CONTRACT).read_text(encoding="utf-8")
        self.assertIn("<<<<<<< ours", text)
        self.assertIn("=======", text)

    def test_registered_a_side_that_is_not_json_merges_as_text(self):
        for name, text in (("ours", "not json\n"), ("theirs", "still not json\n")):
            self.git("checkout", "--quiet", "-b", name, "main")
            (self.repo / CONTRACT).write_text(text)
            self.git("commit", "--quiet", "-am", name)
        self.git("checkout", "--quiet", "ours")
        subprocess.run([sys.executable, "scripts/merge-hide-cli-contract.py", "--register"], cwd=self.repo, check=True, capture_output=True)
        done = self.git("merge", "--no-edit", "theirs", check=False)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("<<<<<<<", (self.repo / CONTRACT).read_text())

    def test_not_registered_the_file_merges_as_text_as_before(self):
        self.different_commands()
        done = self.git("merge", "--no-edit", "theirs", check=False)
        self.assertNotEqual(done.returncode, 0, "the text merge of two commands added at different ends is a conflict on the digest")
        self.assertIn("<<<<<<<", (self.repo / CONTRACT).read_text(encoding="utf-8"))

    def test_registering_twice_leaves_one_driver(self):
        for _ in range(2):
            subprocess.run([sys.executable, "scripts/merge-hide-cli-contract.py", "--register"], cwd=self.repo, check=True, capture_output=True)
        listed = self.git("config", "--local", "--get-all", "merge.hide-cli-contract.driver").stdout.splitlines()
        self.assertEqual(len(listed), 1)
        self.assertTrue(listed[0].endswith("scripts/merge-hide-cli-contract.py %O %A %B %P"))

    def test_the_attribute_names_the_driver_registration_sets(self):
        self.assertIn("contracts/hide-cli.json text eol=lf merge=hide-cli-contract", (ROOT / ".gitattributes").read_text())


# Real merges of main into a pull request's branch that conflicted on the
# contract between 11-10 and 11-28: the merge commit, by the first twelve
# characters of its id. Each is replayed with `git merge-tree` on its two parents
# so the driver gets the three versions git handed a driver (stages 1, 2, 3).
#
# `SAME`: no hand edit, and the driver's file is the merge commit's file, digest
# included. `STALE_DIGEST`: the merge commit committed a digest that is not its
# own body's (a later commit fixed it); the driver's body is the commit's and
# its digest is the right one. `CODE_MERGED`: the author rebuilt `hide` after
# merging the Rust and the file holds what neither side's file did (a command's
# `hidden` flag, a refusal the merged code now returns), which only the code can
# say; the driver finishes with the data merge and hided's committed-contract
# test is what asks for the regeneration. `CONFLICT`: both sides changed the
# same entry, and the author merged it by hand.
SAME = """
012454514b0e 1597b6d793d2 1b26e7625537 1c3807a56184 1caea4df18ee 20224a9a2a19 2326258d5dd7 278f15356da3
42c5cf492980 42cafa6f4b42 4ab007dc17a2 4e3d7c650315 53ae25e0a854 5425e508ca10 685acb41b354 69a45e438018
716f899f8ddc 741985a02956 7bed8f4552e7 7c9be76975df 7cb750ae6406 85069f0346d9 8dd1d85d5df3 8f5ef745871e
9f461000952b a6d323e42feb abdbb11ee162 b80d4d133ea0 c152ac218fe1 c52c2e4a4766 c55891bac2af defcb3e19771
df3a468e698d e67231cf0b9d ee4059a73285 ee716b7a8dc2 f155b2166549 f182c5465996 fd9b45a84632
""".split()
STALE_DIGEST = ["449db18002e9"]
CODE_MERGED = """
4f2adc6331e1 7ee433bbdfa3 91ef0448736b 9aa28801be59 a82552bdfee0 c67fe6f0d257 d3e83d815b96 e5b9dc485f0e
e9582eb42d83 f9b0f806c020
""".split()
CONFLICT = {
    "0de8fc530f86": "commands/device switch/refusals",
    "44d7557061f4": "answers/inbox/definitions/Letter/required",
    "dadde56b9a2a": "commands/device accept-ssh/options",
    "fdd39e89209e": "commands/mission node/refusals",
}


def blob(spec):
    done = subprocess.run(["git", "cat-file", "blob", spec], cwd=ROOT, capture_output=True, text=True)
    return done.stdout if done.returncode == 0 else None


class ReplayOfRealMerges(unittest.TestCase):
    def versions(self, short):
        """The base, ours and theirs git handed a driver in that merge, and the committed result."""
        full = subprocess.run(["git", "rev-parse", "--verify", "--quiet", short + "^{commit}"], cwd=ROOT, capture_output=True, text=True)
        if full.returncode != 0:
            self.skipTest(f"commit {short} is not in this checkout (a shallow clone)")
        first, second = (
            subprocess.run(["git", "rev-parse", f"{short}^{n}"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
            for n in (1, 2)
        )
        tree = subprocess.run(["git", "merge-tree", "--write-tree", first, second], cwd=ROOT, capture_output=True, text=True).stdout
        stages = {}
        for line in tree.splitlines():
            if line.endswith("\t" + CONTRACT):
                _, oid, stage = line.split("\t")[0].split()
                stages[stage] = oid
        self.assertEqual(sorted(stages), ["1", "2", "3"], f"{short} did not conflict on the contract")
        base, ours, theirs = (json.loads(blob(stages[stage])) for stage in "123")
        return base, ours, theirs, blob(f"{short}:{CONTRACT}")

    def test_merges_of_different_commands_finish_and_equal_the_merge_commit(self):
        for short in SAME:
            with self.subTest(merge=short):
                base, ours, theirs, committed = self.versions(short)
                self.assertEqual(driver.render(merged(base, ours, theirs)), committed)

    def test_a_merge_commit_with_a_stale_digest_gets_the_right_one(self):
        for short in STALE_DIGEST:
            with self.subTest(merge=short):
                base, ours, theirs, committed = self.versions(short)
                result = merged(base, ours, theirs)
                committed = json.loads(committed)
                self.assertEqual({k: v for k, v in result.items() if k != "digest"}, {k: v for k, v in committed.items() if k != "digest"})
                self.assertNotEqual(committed["digest"], driver.with_digest(committed)["digest"])
                self.assertEqual(result["digest"], driver.with_digest(result)["digest"])

    def test_merges_the_code_decided_finish_with_a_digest_of_their_own_body(self):
        for short in CODE_MERGED:
            with self.subTest(merge=short):
                result = merged(*self.versions(short)[:3])
                self.assertEqual(result["digest"], driver.with_digest(result)["digest"])

    def test_merges_that_changed_one_entry_on_both_sides_stay_conflicts(self):
        for short, where in CONFLICT.items():
            with self.subTest(merge=short):
                with self.assertRaises(driver.Conflict) as raised:
                    merged(*self.versions(short)[:3])
                self.assertEqual(raised.exception.where, where)


if __name__ == "__main__":
    unittest.main()
