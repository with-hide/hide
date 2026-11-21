<!--
Above the fold, only what a person decides; below it, folded, what a reviewer opens when needed.
Keep the text above the folded blocks under about 30 lines, one fact per bullet (120 characters).
Delete a line, section or block that does not apply; do not fill it with "N/A". The gates are listed in CONTRIBUTING.md.
Title: `<type>: <imperative sentence>` in English, like the commits (types: CONTRIBUTING.md, Commit messages and attribution).
-->

Closes #<!-- the issue this PR finishes; for a piece of a large issue, close that piece's sub-issue. Delete the line if none. -->
Related: <!-- PRD path, prerequisite or dependent PRs. Delete the line if none. -->

## Summary

### Problem

<!--
3-5 lines in words a first-time reader can follow:
the situation (who was doing what, and what went wrong), why it matters (who it reaches, how often, what it costs them),
how you know it is real (reproduced, with the version and steps, or who asked for it), and any context a reader needs (an earlier attempt, a related decision or pull request).
-->

### Before → After

| | Before | After |
| --- | --- | --- |
| What the user gets | | |
| Screen | | |
| Flow | | |

<!--
Before and After side by side; keep the rows that changed, delete the others, and add one line under the table saying where to look.
What the user gets: one line per side. A change only to the CLI, an API or saved data: a short output or request/response per side.
Screen: one image per side, both with the same crop, size, data and state, one box around what changed; up to three image rows, further captures in the folded Evidence block.
An image is required only when the diff touches a screen (web/src, site/, design/); a reason to show none goes in one line `No screenshot: <reason>`, or verify's policy check blocks the PR.
Upload command: AGENTS.md, Before Opening A Pull Request. Only GitHub user-attachments or committed repo paths render; crop out workstation names.
Flow: the steps joined by →, the changed step in bold.
Nothing a user notices changes (a refactor, tests, CI): delete the table and write one line `Same for the user: <reason>`.
-->

## Decide

<!-- Calls only a person makes, one question per line: product, copy, a tradeoff, and for a screen change the one visual judgment the image asks for. Delete the section if none. -->

## Hard to undo

<!--
What is hard to take back once it ships or reaches every user, in plain words, and what the operator must do:
a saved state, settings or install format; a public contract; security or permissions; a place code cannot clean up (shell profile, installed clones, the user's home);
resident or hot-path work every user pays for (added work per input or tick, notification fan-out, wait bounds and caps, owner and teardown: PERFORMANCE_TESTING.md#resident-work-cost-review).
Delete the section if none.
-->

## Not verified

<!-- What you did not check, and why. Never empty. Failed checks, required checks not run and major risks stay here, not folded. -->

## Accountability

<!-- Asked of a contributor outside the maintainers; maintainers delete this section. Check a box only when it is true: a box left empty tells the reviewer where to look. -->

- [ ] The problem is real: I hit it, or Summary links who asked for the change.
- [ ] I tried the After myself, in the app or the CLI, not only through tests.
- [ ] This description says only what I saw; what I did not see is under Not verified.

<details><summary>Evidence</summary>

<!-- Name what was measured and how; `agents/runs/<slug>/` stays local, so do not link it. -->

- **Verified**: <!-- Not "tests pass": what, how many, under which conditions. -->
- **Generated**: <!-- A large script-made diff such as `design/hide.pen`: one line, pointing at the result image. Delete the line if none. -->

<!-- Pen comparisons, per-state captures, technical logs, long command output, one caption line each. -->

</details>

<details><summary>Tests</summary>

<!-- Delete this block if the diff adds or changes no test. Criteria: docs/CODE_REVIEW.md perspective 6 and docs/TESTING.md, Reviewing a pull request that adds or changes a test. -->

- **Layer**: <!-- unit / integration / e2e. For e2e: the user flow it protects and why a lower layer cannot observe it. -->
- **Catches**: <!-- The plausible bug that makes it fail, and the planted defect you made to see it fail. -->
- **Repeat**: <!-- Changed tests x runs, all green, with the link to the repeat job. -->
- **Waits**: <!-- Each new wait, timeout or poll and the state it waits for, or "none". -->

</details>

<details><summary>Risky files</summary>

<!-- `path` - why, one per line. Pick from where this repository was burned: work under the runtime mutex, snapshot wire, engine/core ownership, calls outside the engine contract, failure path, high-frequency path. Delete the block if none. -->

</details>

<details><summary>Verification record</summary>

<!-- PRD path, base/head SHAs, suites run and results, reviewer findings and their disposition, registered evidence, changed paths. Filled by the `ship` skill: keep this block, and open or update the PR through `ship`. -->

</details>
