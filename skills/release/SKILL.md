---
name: release
description: Cut a hide release by the procedure CONTRIBUTING.md owns, with the agent journey run on the Mac first and its table shown in the version approval question. Use when the user invokes "/release", or says "릴리스해줘", "릴리스", "버전 올려서 배포", "release", "cut a release", "tag a release". Not for packaging a local build to try; that is `pnpm --dir desktop package` (docs/BUILD.md).
---

# release

The procedure belongs to `CONTRIBUTING.md`, "Releases", and `docs/BUILD.md`, "Release asset gate"; the agent journey belongs to `docs/VERIFICATION.md`, "Agent journey".
Read them there when a case is not below.
This skill says the order and where to stop; it never pushes a tag the operator did not approve in this session.

## 1. Pick the commit

Take the newest `main` commit whose own `verify` push run passed, never a branch:

```sh
gh run list --workflow verify --branch main --event push --limit 10 --json headSha,conclusion,url
```

A run that was cancelled or failed is not that commit's pass: rerun it, or take a later commit whose run passed.
Read the latest release tag (`gh release list --limit 5`) and propose the next version: `vX.Y.Z` for a stable release, `vX.Y.Z-rc.N` for a signed build to try first.

## 2. Build it in its own worktree

```sh
git worktree add ../release-<version> <sha>
cd ../release-<version>
bash scripts/verify-cargo.sh cli
bash scripts/verify-cargo.sh build -p hide-session --example journey_conversation
zsh scripts/fetch-herdr-runtime.sh
```

The build is that worktree's own; never point it at another checkout's `target/`.

## 3. Run the agent journey on the Mac

From that worktree, with no `HERDR_*` or `HIDE_*` variable from the launching pane:

```sh
(unset $(env | sed -En 's/^((HERDR|HIDE)_[A-Za-z0-9_]*)=.*/\1/p')
 python3 scripts/agent-journey-check.py --run-dir agents/runs/release-<version>-journey)
```

Every declared cell runs; no `--agents` narrows it.
Run no cargo command while it runs.
The table stays in `agents/runs/release-<version>-journey/report.md`, never in a commit.

| The table has | Do |
| --- | --- |
| A `fail` cell, or exit 1 or 2 | Stop before the tag. Tell the operator which cell failed and the product's answer it carries. Go on only when the operator says, in this session and for this version, to release past it; write their words to `agents/runs/release-<version>-journey/override.md` first. |
| `not measured` cells, exit 3 | Not a stop by itself. List each with its reason in the approval question below. |
| `excluded` cells | Show them in the table; they are not in the list of blanks. |
| A report marked `fixture` | Not release evidence. Run the journey on the Mac with the signed-in agents. |

## 4. Ask the operator to approve the version

One question, in the operator's language: the version, the commit and its `verify` run, the journey table, the `not measured` list with each reason, and any failure passed with their words.
The tag waits for their answer to that question in this session; an approval in another session, a PRD or a plan is not one.

## 5. Push the tag

```sh
git tag -a <version> <sha> -m "<version>"
git push origin <version>
```

`release.yml` then runs the gates `CONTRIBUTING.md` lists, the agent journey's CI lanes among them, and publishes when every one passes.
Watch it to the end and report the release's URL, or the gate that refused it.
A published release is final: a fault the gates missed is fixed by the next version.
Remove the release worktree afterwards with `git worktree remove ../release-<version>`.
