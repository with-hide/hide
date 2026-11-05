#!/usr/bin/env python3
"""Say which green pull request merges first, and where a pull request should stand.

CONTRIBUTING.md owns the rules: "Starting work beside open pull requests" says
when work stacks on an open pull request, and "Merging into main" the order the
green ones merge in. This computes both from git and GitHub as they are now. It
only reads: it fetches the open pull requests' heads, and writes nothing to
GitHub or to any branch.

Every open pull request is merged with main in memory (`git merge-tree`), and
those merges are merged with each other in pairs, so a conflict is the one the
second pull request meets after the first merges. A conflict touches code, or
only documents and design files, which are resolved once right before the merge
and never stack anything.

Usage:
  python3 scripts/merge-order.py [--json]                  the green pull requests in merge order
  python3 scripts/merge-order.py <number> [--json]         where that pull request should stand
  python3 scripts/merge-order.py --branch <name> [--json]  the same for a branch, with or without a pull request

Exit status: 0 answered; 2 git or GitHub could not answer, and the output says why.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
import itertools
import json
import sys

from pull_requests import BASE, MERGE_IDENTITY, ROOT, Undecided, gh_json, git, latest_verify, merge_tree, verify_runs

# Pairs grow with the square of the open pull requests. A pair took 46 ms when 20
# pull requests' 190 pairs were merged on 2026-10-16, so 60 (1,770 pairs) stays
# near a minute and a half; past that the design, not the limit, has to change.
OPEN_LIMIT = 60
# A stack this tall takes no new layer: each change to a layer runs `verify` again
# on every layer above it.
STACK_LIMIT = 3
# A conflict only in these is resolved once, right before the merge.
DOCUMENT_SUFFIXES = (".md", ".pen")
DOCUMENT_FOLDERS = ("site/",)
# A pull request that changes one of these stays off every stack: it plans every
# lane or changes what every other change builds against, so its failure would
# hold every layer above it.
RISKY_PATHS = ("Cargo.lock", "contracts/", ".github/workflows/")
# The number a device helper and the core compare at Hello.
PROTOCOL_FILE = "hide-node-link/src/protocol.rs"
PROTOCOL_CONSTANT = "PROTOCOL_VERSION"
# GitHub lookups run side by side; each is one `gh` process.
LOOKUPS_AT_ONCE = 8


@dataclass(eq=False)
class Pull:
    """An open pull request, or a branch that has none yet (`number` None)."""
    number: object
    branch: str
    base: str
    head: str
    title: str = ""
    draft: bool = False
    # green, running, red, or none before a `verify` run starts
    verify: str = "none"
    green_at: object = None
    risky: tuple = ()
    conflicts_with_main: tuple = ()
    # The open pull request this one's base branch is, when it stacks on one.
    below: object = None
    depth: int = 1
    stray_base: bool = False
    # The pull request merged with main, the commit its pairs are merged from.
    commit: object = field(default=None, repr=False)

    def __str__(self):
        return f"#{self.number}" if self.number is not None else self.branch


def chain(pull):
    """The layers below a pull request, nearest first."""
    layers = []
    while pull.below is not None:
        pull = pull.below
        if any(layer is pull for layer in layers):
            raise Undecided(f"the base branches from {pull} down form a loop.")
        layers.append(pull)
    return layers


def bottom(pull):
    layers = chain(pull)
    return layers[-1] if layers else pull


def link_stacks(pulls):
    """Point each pull request at the open pull request its base branch is."""
    by_branch = {}
    for pull in pulls:
        if pull.branch in by_branch:
            raise Undecided(f"{by_branch[pull.branch]} and {pull} both come from branch {pull.branch}, "
                            "so the stacks on it cannot be told apart.")
        by_branch[pull.branch] = pull
    for pull in pulls:
        if pull.base != BASE:
            pull.below = by_branch.get(pull.base)
            pull.stray_base = pull.below is None
    for pull in pulls:
        pull.depth = len(chain(pull)) + 1


def stack_height(pull, pulls):
    root = bottom(pull)
    return max(other.depth for other in pulls if bottom(other) is root)


def is_document(path):
    return path.endswith(DOCUMENT_SUFFIXES) or path.startswith(DOCUMENT_FOLDERS)


def kind(paths):
    """`code`, or `docs` when every conflicting path is a document or design file."""
    return "docs" if all(is_document(path) for path in paths) else "code"


def merge_with_main(main, pulls, repo=ROOT):
    """Merge each pull request with main into a commit the pairs are merged from.

    A conflict in code leaves no commit, since its code is not what will merge;
    one in documents only still compares the code with the other pull requests.
    """
    for pull in pulls:
        tree, conflicts = merge_tree(main, pull.head, repo)
        pull.conflicts_with_main = tuple(conflicts)
        if conflicts and kind(conflicts) == "code":
            continue
        pull.commit = git("commit-tree", tree, "-p", main, "-p", pull.head, "-m", f"merge-order: {pull} with main",
                          cwd=repo, env=MERGE_IDENTITY).stdout.strip()


def pair_conflicts(pulls, repo=ROOT, target=None):
    """{(a, b): paths} for two pull requests whose merges with main conflict with each other.

    Two layers of one stack line are left out: the upper one holds the lower, and
    merging the layer below is how it takes the lower's changes. With `target`,
    only the pairs that include it are merged.
    """
    merged = [pull for pull in pulls if pull.commit is not None]
    pairs = {}
    for a, b in itertools.combinations(merged, 2):
        if target is not None and target is not a and target is not b:
            continue
        if a in chain(b) or b in chain(a):
            continue
        _, conflicts = merge_tree(a.commit, b.commit, repo)
        if conflicts:
            pairs[(a, b)] = pairs[(b, a)] = tuple(conflicts)
    return pairs


def conflicts_of(pull, pulls, pairs, wanted):
    """The pull requests `pull` conflicts with in `code`, or in `docs` only, and the paths."""
    return [(other, pairs[(pull, other)]) for other in pulls
            if (pull, other) in pairs and kind(pairs[(pull, other)]) == wanted]


def breaks(pull, pulls, pairs):
    """The open pull requests merging `pull` would send back through `verify` for a code conflict.

    A red one is left out: it has to push again anyway.
    """
    return [other for other, _ in conflicts_of(pull, pulls, pairs, "code") if other.verify != "red"]


def green_layers(root, pulls):
    """The layers of a stack that may merge one after another now: green, on green layers."""
    return [pull for pull in pulls if bottom(pull) is root and pull.verify == "green"
            and all(layer.verify == "green" for layer in chain(pull))]


def merge_order(pulls, pairs):
    """The bottom layers that may merge now, the fewest open pull requests broken first.

    A tie goes to the stack with more layers ready to merge, then to the pull
    request green longest, so one that conflicts with an already green pull
    request goes after it. How much the work matters is the merger's to weigh.
    """
    ready = [pull for pull in pulls if pull.below is None and not pull.stray_base and pull.verify == "green"
             and not pull.conflicts_with_main]
    return sorted(ready, key=lambda pull: (len(breaks(pull, pulls, pairs)), -len(green_layers(pull, pulls)),
                                           pull.green_at, pull.number))


def green_first(a, b):
    """Whether `a` turned green before `b`, so `b` goes on top of it."""
    return a.verify == "green" and (b.verify != "green" or a.green_at < b.green_at)


def stack_command(target, onto):
    edit = (f"gh pr edit {target.number} --base {onto.branch}" if target.number is not None
            else f"open the pull request with --base {onto.branch}")
    return f"{edit}; git fetch origin {onto.branch} && git merge origin/{onto.branch}, then resolve and push"


def verdict(target, other, pulls):
    """What the target does about a code conflict with `other`, as (step, sentence)."""
    if other.risky:
        return "alone", f"{other} is risky ({', '.join(other.risky)}): do not stack; after it merges, merge main once."
    if target.risky:
        return "alone", f"{target} is risky ({', '.join(target.risky)}): it stays alone; after {other} merges, merge main once."
    if target.below is not None:
        return "wait", f"{target} is already a layer on {target.below}: run this again after {bottom(target)} merges."
    if stack_height(other, pulls) >= STACK_LIMIT:
        return "full", (f"{bottom(other)}'s stack already has {STACK_LIMIT} layers: do not stack; "
                        f"run this again after {bottom(other)} merges.")
    if green_first(other, target):
        return "stack", f"{other} turned green first: stack on it."
    if green_first(target, other):
        return "below", f"{target} turned green first: stay; {other} stacks on {target} when it turns green."
    return "wait", f"neither {target} nor {other} is green yet: stay; whichever turns green first goes below, so run this again then."


def placement(target, pulls, pairs):
    """The target's code and documents-only conflicts, a verdict for each code one, and the one thing to do."""
    if target.conflicts_with_main and kind(target.conflicts_with_main) == "code":
        files = short(target.conflicts_with_main)
        if target.below is None:
            do = (f"{target} conflicts with main in {files}: git fetch origin {BASE} && git merge origin/{BASE}, "
                  "resolve, push, and run this again.")
        else:
            do = (f"{target} conflicts with main in {files}: the bottom layer {bottom(target)} merges main, "
                  f"then each layer merges the one below it; {target} merges origin/{target.base} only.")
        return {"code": [], "docs": [], "do": do}
    code = [(other, files, *verdict(target, other, pulls))
            for other, files in conflicts_of(target, pulls, pairs, "code")]
    docs = conflicts_of(target, pulls, pairs, "docs")
    stack_on = [other for other, _, step, _ in code if step == "stack"]
    # Stacking on the higher of two layers in one line takes the lower one too.
    tops = [other for other in stack_on if not any(other in chain(higher) for higher in stack_on)]
    if tops:
        onto = min(tops, key=lambda pull: pull.green_at)
        do = f"Stack on {onto}: {stack_command(target, onto)}."
        left = [str(other) for other, *_ in code if other is not onto and other not in chain(onto)]
        if left:
            do += f" It still conflicts with {', '.join(left)}: run this again after {bottom(onto)} merges."
    elif code:
        do = "Stay as you are. " + " ".join(sentence for *_, sentence in code)
    else:
        do = "Stay as you are: no open pull request conflicts with it in code."
    if docs:
        do += (f" Documents or design only with {', '.join(str(other) for other, _ in docs)}: "
               "resolve that once, right before the merge.")
    if target.conflicts_with_main:
        when = "right before the merge" if target.below is None else f"once {target} is the bottom layer"
        do += f" Documents or design only with main in {short(target.conflicts_with_main)}: merge main once, {when}."
    return {"code": code, "docs": docs, "do": do}


def risk(pull, main, repo=ROOT):
    """What makes a pull request risky: the paths, and the protocol number, it changes."""
    paths = git("diff", "--name-only", f"{main}...{pull.head}", cwd=repo).stdout.splitlines()
    reasons = sorted({prefix.rstrip("/") for path in paths for prefix in RISKY_PATHS
                      if path == prefix or (prefix.endswith("/") and path.startswith(prefix))})
    if PROTOCOL_FILE in paths:
        diff = git("diff", "-U0", f"{main}...{pull.head}", "--", PROTOCOL_FILE, cwd=repo).stdout
        if any(line.startswith(("+", "-")) and not line.startswith(("+++", "---")) and PROTOCOL_CONSTANT in line
               for line in diff.splitlines()):
            reasons.append("protocol number")
    return tuple(reasons)


def short(paths, limit=3):
    paths = list(paths)
    return ", ".join(paths[:limit]) + (f" and {len(paths) - limit} more" if len(paths) > limit else "")


def open_pulls():
    listed = gh_json("pr", "list", "--state", "open", "--limit", str(OPEN_LIMIT + 1),
                     "--json", "number,title,headRefName,headRefOid,baseRefName,isDraft")
    if len(listed) > OPEN_LIMIT:
        raise Undecided(f"more than {OPEN_LIMIT} pull requests are open, past what this compares in pairs.")
    return [Pull(number=item["number"], branch=item["headRefName"], base=item["baseRefName"],
                 head=item["headRefOid"], title=item["title"], draft=item["isDraft"]) for item in listed]


def read_verify(pulls):
    """Each pull request's latest `verify` run on its pushed head, which the merge queue requires to have passed."""
    with ThreadPoolExecutor(LOOKUPS_AT_ONCE) as pool:
        latest = list(pool.map(lambda pull: latest_verify(verify_runs(pull.head)), pulls))
    for pull, run in zip(pulls, latest):
        if run is None:
            pull.verify = "none"
        elif run["status"] != "completed":
            pull.verify = "running"
        elif run["conclusion"] == "success":
            pull.verify, pull.green_at = "green", run["updated_at"]
        else:
            pull.verify = "red"


def fetch(pulls, branch=None):
    """Fetch main and every open head in one fetch; raise when a head moved while it was read."""
    refs = [BASE] + [f"pull/{pull.number}/head" for pull in pulls] + ([branch] if branch else [])
    git("fetch", "--quiet", "--no-tags", "origin", *refs)
    for pull in pulls:
        if git("cat-file", "-e", f"{pull.head}^{{commit}}", check=False).returncode != 0:
            raise Undecided(f"{pull}'s head {pull.head[:12]} was not fetched; it moved while being read, so run again.")
    return git("rev-parse", f"origin/{BASE}").stdout.strip()


def local_head(name):
    result = git("rev-parse", "--verify", "--quiet", f"{name}^{{commit}}", check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def branch_target(name, head, pulls):
    """The branch as a target, with its open pull request's facts when it has one."""
    branch = name.removeprefix("origin/")
    pull = next((pull for pull in pulls if pull.branch == branch), None)
    if pull is None:
        pull = Pull(number=None, branch=branch, base=BASE, head=head)
        pulls.append(pull)
    pull.head = head
    return pull


def describe(pull):
    notes = [pull.verify]
    if pull.draft:
        notes.append("draft")
    if pull.risky:
        notes.append("risky: " + ", ".join(pull.risky))
    if pull.stray_base:
        notes.append(f"merges into {pull.base}, which no open pull request is")
    return ", ".join(notes)


def pull_record(pull):
    return {"number": pull.number, "branch": pull.branch, "base": pull.base, "title": pull.title,
            "draft": pull.draft, "verify": pull.verify, "green_at": pull.green_at, "risky": list(pull.risky),
            "below": pull.below.number if pull.below is not None else None, "depth": pull.depth,
            "conflicts_with_main": list(pull.conflicts_with_main)}


def order_record(main, pulls, pairs):
    order = merge_order(pulls, pairs)

    def row(pull):
        broken = breaks(pull, pulls, pairs)
        return {"number": pull.number, "verify": pull.verify,
                "breaks": {"green": sum(1 for other in broken if other.verify == "green"),
                           "running": sum(1 for other in broken if other.verify != "green")},
                "code": [{"number": other.number, "verify": other.verify, "files": list(files)}
                         for other, files in conflicts_of(pull, pulls, pairs, "code")],
                "docs_only": [{"number": other.number, "files": list(files)}
                              for other, files in conflicts_of(pull, pulls, pairs, "docs")],
                "layers": [row(layer) for layer in pulls if layer.below is pull]}

    shown = [pull for pull in pulls if any(bottom(pull) is root for root in order)]
    waiting = [pull for pull in pulls if pull.verify == "green" and not any(pull is seen for seen in shown)]
    return {"main": main, "pulls": [pull_record(pull) for pull in pulls], "order": [row(pull) for pull in order],
            "green_not_ready": [{"number": pull.number, "why": why_not_ready(pull)} for pull in waiting]}


def why_not_ready(pull):
    if pull.conflicts_with_main:
        return f"conflicts with main in {short(pull.conflicts_with_main)}; it merges main first"
    if pull.stray_base:
        return f"merges into {pull.base}, which no open pull request is"
    root = bottom(pull)
    return f"its bottom layer {root} is not ready ({root.verify if root.verify != 'green' else why_not_ready(root)})"


def print_order(record, pulls):
    by_number = {pull.number: pull for pull in pulls}
    states = [pull.verify for pull in pulls]
    print(f"main {record['main'][:12]}, {len(pulls)} open: green {states.count('green')}, "
          f"running {states.count('running') + states.count('none')}, red {states.count('red')}")
    if not record["order"]:
        print("No green pull request may merge now.")

    def show(row, indent, lead):
        pull = by_number[row["number"]]
        counted = row["breaks"]["green"] + row["breaks"]["running"]
        print(f"{indent}{lead} {pull}  breaks {counted} (green {row['breaks']['green']}, running {row['breaks']['running']})"
              f"  [{describe(pull)}]  {pull.title[:60]}")
        for entry in row["code"]:
            print(f"{indent}     breaks #{entry['number']} ({entry['verify']}) in {short(entry['files'])}")
        for entry in row["docs_only"]:
            print(f"{indent}     documents only with #{entry['number']} in {short(entry['files'])}")
        for layer in row["layers"]:
            show(layer, indent + "   ", f"layer on {pull}:")

    for position, row in enumerate(record["order"], start=1):
        show(row, "", f"{position}.")
    for entry in record["green_not_ready"]:
        print(f"green, not ready: #{entry['number']} {entry['why']}")
    counts = [row["breaks"]["green"] + row["breaks"]["running"] for row in record["order"]]
    if len(set(counts)) < len(counts):
        print("Rows that break as many: the more important work goes first; the order above then prefers "
              "the stack with more green layers, then the one green longest.")


def print_placement(main, target, answer):
    name = f"{target} {target.branch}" if target.number is not None else target.branch
    print(f"{name} into {target.base} [{describe(target)}], main {main[:12]}")
    for heading, entries in (("Code conflicts:", answer["code"]), ("Documents or design only:", answer["docs"])):
        if entries:
            print(heading)
            for other, files, *_ in entries:
                print(f"  {other} ({other.verify}) in {short(files)}")
    print(f"Do: {answer['do']}")


def placement_record(main, target, answer):
    return {"main": main, "target": pull_record(target),
            "code": [{"number": other.number, "verify": other.verify, "files": list(files), "step": step, "say": sentence}
                     for other, files, step, sentence in answer["code"]],
            "docs_only": [{"number": other.number, "files": list(files)} for other, files in answer["docs"]],
            "do": answer["do"]}


def run(arguments):
    pulls = open_pulls()
    head = local_head(arguments.branch) if arguments.branch is not None else None
    main = fetch(pulls, branch=arguments.branch if arguments.branch is not None and head is None else None)
    read_verify(pulls)
    target = None
    if arguments.branch is not None:
        target = branch_target(arguments.branch, head or git("rev-parse", f"origin/{arguments.branch}").stdout.strip(), pulls)
    elif arguments.number is not None:
        target = next((pull for pull in pulls if pull.number == arguments.number), None)
        if target is None:
            raise Undecided(f"#{arguments.number} is not an open pull request.")
    link_stacks(pulls)
    for pull in pulls:
        pull.risky = risk(pull, main)
    merge_with_main(main, pulls)
    pairs = pair_conflicts(pulls, target=target)
    if target is None:
        record = order_record(main, pulls, pairs)
        if arguments.json:
            print(json.dumps(record, indent=1))
        else:
            print_order(record, pulls)
        return
    answer = placement(target, pulls, pairs)
    if arguments.json:
        print(json.dumps(placement_record(main, target, answer), indent=1))
    else:
        print_placement(main, target, answer)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("number", nargs="?", type=int, help="where this open pull request should stand")
    parser.add_argument("--branch", help="where this branch should stand, with or without a pull request")
    parser.add_argument("--json", action="store_true", help="print the answer as JSON")
    arguments = parser.parse_args(argv[1:])
    if arguments.number is not None and arguments.branch is not None:
        parser.error("give a pull request number or --branch, not both")
    try:
        run(arguments)
    except Undecided as reason:
        print(f"undecided: {reason}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
