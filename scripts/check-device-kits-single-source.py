#!/usr/bin/env python3
"""A device platform is named once: in contracts/device-kits.json.

The core (hide-node/src/ssh/kit/) and the packaging scripts
(desktop/scripts/device-helpers.mjs and its readers) take every platform, its
programs and its header from that file, so adding a platform, such as Windows
ARM later, is one entry there and a runner, with no code branch
(with-hide/hide-private#501). This check fails when a platform name appears
in that code anywhere else.

Tests may name platforms as data: `*_tests.rs`, a crate's `tests.rs` and
`tests/` folders, `*.test.mjs` and `scripts/tests/`, and in other Rust files
everything from the first `mod tests` on. Workflow files are not covered yet:
the release producers that build each kit derive their matrix from the
contract in a later change. Product analytics keeps its own enumerated
vocabulary (hide-core/src/product_analytics.rs) and is not kit code.

Searches use `git grep`, never `rg`: ripgrep is not on the CI runner.
"""

import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "contracts" / "device-kits.json"
# The code that reads or produces device kits.
SCOPE = ["hide-node/src", "desktop/scripts", "scripts"]
TEST_MODULE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod tests\s*\{")


def is_test_file(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return (
        name.endswith("_tests.rs")
        or name == "tests.rs"
        or name.endswith(".test.mjs")
        or "/tests/" in path
        or path.startswith("scripts/tests/")
    )


def test_region_start(path: str) -> int:
    """The first line of a Rust file's inline test module, or past its end."""
    lines = (ROOT / path).read_text(encoding="utf-8").splitlines()
    for number, line in enumerate(lines, start=1):
        if TEST_MODULE.match(line):
            return number
    return len(lines) + 1


def main() -> int:
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    names = sorted(set(contract["kits"]) | set(contract["refused"]))
    command = ["git", "grep", "--untracked", "-n", "-F"]
    for name in names:
        command += ["-e", name]
    command += ["--", *SCOPE]
    found = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if found.returncode not in (0, 1):
        sys.stderr.write(found.stderr)
        return 2
    offenders = []
    starts: dict[str, int] = {}
    for line in found.stdout.splitlines():
        path, number, text = line.split(":", 2)
        if is_test_file(path):
            continue
        if path.endswith(".rs"):
            if path not in starts:
                starts[path] = test_region_start(path)
            if int(number) >= starts[path]:
                continue
        offenders.append(f"{path}:{number}: {text.strip()}")
    if offenders:
        print(
            "A device platform is named outside contracts/device-kits.json; "
            "read it from the contract instead:",
            file=sys.stderr,
        )
        for offender in offenders:
            print(f"  {offender}", file=sys.stderr)
        return 1
    print(f"device kit platforms are named only in the contract ({len(names)} names)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
