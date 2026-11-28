"""Which of main's dependency cache entries a newer generation replaced.

docs/BUILD.md, "CI's Rust caches", owns the rule: per set and system, the
entry main saved last stays and the older ones go; nothing else is touched.
"""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("cache_generations", Path(__file__).parents[1] / "cache-generations.py")
generations = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generations)

MANIFEST = "bd406d4b33fd415c47b9f4dddf6d88e0c42474d9e0f645d1422ef178e6bc462c"
PINS = "c16cfc48054be4bcacf0ac7672d329f58bfa570bc482383d066ec8d4e2b9810c"
NEXT_PINS = "d89ab084e06908e12012451eab3ae1eba93d78ebaf86982fc1eaf57e24f5b0a9"


def entry(key, created, ref="refs/heads/main"):
    return {"id": hash(key + created + ref), "key": key, "ref": ref, "created_at": created, "size_in_bytes": 1}


class Replaced(unittest.TestCase):
    def test_keeps_the_newest_entry_of_each_set_and_system_and_deletes_the_rest(self):
        old_linux = entry(f"v0-rust-workspace-{MANIFEST}-Linux-x64-770ca156-10e4678e", "2026-11-24T15:15:49.207811Z")
        new_linux = entry(f"v0-rust-workspace-{MANIFEST}-Linux-x64-770ca156-e574a365", "2026-11-27T09:01:02.000001Z")
        # Three generations on Windows, listed out of order; a new root
        # manifest or toolchain also starts a new generation of the same set.
        windows = [
            entry(f"v0-rust-release-{MANIFEST}-Windows_NT-x64-15d0a0c1-10e4678e", "2026-11-24T18:19:10.106343Z"),
            entry(f"v0-rust-release-{NEXT_PINS}-Windows_NT-x64-9a0b1c2d-e574a365", "2026-11-28T01:08:00.5Z"),
            entry(f"v0-rust-release-{MANIFEST}-Windows_NT-x64-15d0a0c1-7ad20924", "2026-11-20T23:36:34.208229Z"),
        ]
        # The only entry of its set and system stays, however old.
        only_darwin = entry(f"v0-rust-release-{MANIFEST}-Darwin-arm64-30c62bd1-7ad20924", "2026-11-20T10:00:00Z")
        # Another set on the same system is its own group.
        quick = entry(f"v0-rust-quick-{MANIFEST}-Linux-x64-770ca156-10e4678e", "2026-11-24T00:00:00Z")
        old_zig = entry(f"vt-zig-v1-Linux-X64-{PINS}", "2026-11-01T00:00:00Z")
        new_zig = entry(f"vt-zig-v1-Linux-X64-{NEXT_PINS}", "2026-11-02T00:00:00Z")
        inputs = entry(f"vt-inputs-v1-Linux-X64-{PINS}", "2026-11-01T00:00:00Z")
        others = [
            entry("node-cache-Linux-x64-pnpm-9c2493ba", "2026-11-01T00:00:00Z"),
            entry("node-cache-Linux-x64-pnpm-e05b5c2e", "2026-11-28T00:00:00Z"),
            # Another ref's entry is not main's generation.
            entry(f"v0-rust-workspace-{MANIFEST}-Linux-x64-770ca156-00000000", "2026-11-28T02:00:00Z",
                  ref="refs/pull/3105/merge"),
        ]
        stale = generations.replaced([new_linux, *windows, old_linux, only_darwin, quick, new_zig, old_zig,
                                      inputs, *others])
        self.assertEqual([item["key"] for item in stale], [
            f"v0-rust-release-{MANIFEST}-Windows_NT-x64-15d0a0c1-10e4678e",
            f"v0-rust-release-{MANIFEST}-Windows_NT-x64-15d0a0c1-7ad20924",
            f"v0-rust-workspace-{MANIFEST}-Linux-x64-770ca156-10e4678e",
            f"vt-zig-v1-Linux-X64-{PINS}",
        ])

    def test_refuses_a_dependency_key_whose_shape_it_cannot_read(self):
        # A key under rust-cache's or vt-cache's prefix in an unknown shape
        # could belong to any group, so nothing is deleted on a guess.
        with self.assertRaises(generations.UnknownKey):
            generations.replaced([entry("v0-rust-workspace-Linux-x64-770ca156-e574a365", "2026-11-28T00:00:00Z")])


if __name__ == "__main__":
    unittest.main()
