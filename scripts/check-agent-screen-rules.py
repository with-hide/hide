#!/usr/bin/env python3
"""A built-in agent's screen rules change only with a higher version and a
screen they are proven against (PRD engine-bundle-3-core-owns D-19, D-20).

Hide reads screen rules from the app's own definition files
(`hide-agent-adapter/agents/<id>.toml`, `[screen]`) and the operator's, never
from a download, so a rule change ships with the release that carries it and
this check is its one gate. For each built-in file the base already has
whose `[screen]` differs there (a new file adds an agent and is reviewed as
that):

- `[screen] version` is higher than the base's, so a release, the catalog
  and an operator's override can tell the rules apart;
- the agent has a captured screen under
  `hide-agent-adapter/tests/fixtures/screens/<id>/`, which the Rust suite's
  `screens::every_captured_screen_reads_as_the_state_the_operator_saw`
  evaluates the rules against.

The base is the first parent when HEAD is a merge commit (a pull request's
merge result, or a push to main), else where HEAD leaves `origin/main`;
`--base <rev>` names another.
"""
import argparse
import pathlib
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
AGENTS = "hide-agent-adapter/agents"
SCREENS = "hide-agent-adapter/tests/fixtures/screens"


def version_key(version):
    parts = [int(part) for part in str(version).split(".")]
    while parts and parts[-1] == 0:
        parts.pop()
    return parts


def problems(base, head, captured):
    """`base` and `head` map a file name to its parsed TOML (a file the base
    lacks is new); `captured` is the agent ids with a captured screen."""
    found = []
    for name, definition in sorted(head.items()):
        # A new file adds an agent, which its own pull request brings with
        # whatever proves it (D-41); this gate is for rules that change.
        if name not in base:
            continue
        screen = definition.get("screen")
        before = base[name].get("screen")
        if screen is None or screen == before:
            continue
        agent = definition.get("id", name.removesuffix(".toml"))
        if before is not None and version_key(screen.get("version", "0")) <= version_key(
            before.get("version", "0")
        ):
            found.append(
                f"{AGENTS}/{name}: [screen] changed but its version {screen.get('version')} "
                f"is not higher than {before.get('version')}; raise it"
            )
        if agent not in captured:
            found.append(
                f"{AGENTS}/{name}: [screen] changed but {SCREENS}/{agent}/ has no captured "
                "screen; add one named <idle|working|blocked>-<what>.ansi from the real agent"
            )
    return found


def git(*args):
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def default_base():
    parents = git("rev-list", "--parents", "-n", "1", "HEAD").split()
    if len(parents) > 2:
        return "HEAD^1"
    return git("merge-base", "origin/main", "HEAD").strip()


def read_base(base):
    files = {}
    listing = git("ls-tree", "--name-only", f"{base}:{AGENTS}") if has_tree(base) else ""
    for name in listing.split():
        if name.endswith(".toml"):
            files[name] = tomllib.loads(git("show", f"{base}:{AGENTS}/{name}"))
    return files


def has_tree(base):
    return (
        subprocess.run(
            ["git", "cat-file", "-e", f"{base}:{AGENTS}"], cwd=ROOT, capture_output=True
        ).returncode
        == 0
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base")
    args = parser.parse_args()
    base = read_base(args.base or default_base())
    head = {
        path.name: tomllib.loads(path.read_text(encoding="utf-8"))
        for path in sorted((ROOT / AGENTS).glob("*.toml"))
    }
    screens = ROOT / SCREENS
    captured = {
        folder.name for folder in screens.iterdir() if folder.is_dir() and any(folder.iterdir())
    }
    found = problems(base, head, captured)
    for problem in found:
        print(problem, file=sys.stderr)
    if found:
        return 1
    print(f"agent screen rules ok ({len(head)} built-in definitions)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
