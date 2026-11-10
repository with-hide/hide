<!--
Write in the order a reviewer reads: what changed, what needs a human, what is proven.
Keep the text above the folded blocks under about 40 lines, one fact per bullet (120 characters). Longer material goes in a folded block.
Delete a line or section that does not apply; do not fill it with "N/A". The gates are listed in CONTRIBUTING.md.
Title: `<type>: <imperative sentence>` in English, like the commits (types: CONTRIBUTING.md, Commit messages and attribution).
-->

Closes #<!-- the issue this PR finishes; for a piece of a large issue, close that piece's sub-issue. Delete the line if none. -->
Related: <!-- PRD path, prerequisite or dependent PRs. Delete the line if none. -->

## Summary

<!-- 3-5 lines: the problem, then the change, in words a first-time reader can follow. -->

<!--
Screen change: one before/after image and one caption line saying where to look. Further images go in the folded block under Evidence.
Upload command: AGENTS.md, Before Opening A Pull Request. Only GitHub user-attachments or committed repo paths render; crop out workstation names.
A screen change with no image needs one line `No screenshot: <reason>`; verify's policy check blocks the PR without either.
No screen change: no screenshot. Optionally one short code block with the changed behavior, request/response or repro.
-->

## Review

<!-- Where the reviewer's time should go. A test already proves the rest; delete a line with nothing to decide. -->

- **Decide**: <!-- product, copy and tradeoff calls only a human makes, one per line. For a screen change, the one visual judgment the image asks for. -->
- **Risky files**: <!-- `path` - why. Pick from where this repository was burned: work under the runtime mutex, snapshot wire, engine/core ownership, calls outside the engine contract, failure path, high-frequency path. -->
- **Cost**: <!-- Resident or hot-path change: added work per input or tick, notification fan-out, wait bounds and missing caps, owner and teardown (PERFORMANCE_TESTING.md#resident-work-cost-review). -->
- **Questions**: <!-- Only ones that need an answer. Delete the line if none. -->

## Tests

<!-- Delete this section if the diff adds or changes no test. Criteria: docs/CODE_REVIEW.md perspective 6 and docs/TESTING.md, Reviewing a pull request that adds or changes a test. -->

- **Layer**: <!-- unit / integration / e2e. For e2e: the user flow it protects and why a lower layer cannot observe it. -->
- **Catches**: <!-- The plausible bug that makes it fail, and the planted defect you made to see it fail. -->
- **Repeat**: <!-- Changed tests x runs, all green, with the link to the repeat job. -->
- **Waits**: <!-- Each new wait, timeout or poll and the state it waits for, or "none". -->

## Evidence

<!-- Name what was measured and how; `agents/runs/<slug>/` stays local, so do not link it. -->

- **Verified**: <!-- Not "tests pass": what, how many, under which conditions. -->
- **Not verified**: <!-- What and why. Never empty. Failed checks, required checks not run and major risks stay here, not folded. -->
- **Generated**: <!-- A large script-made diff such as `design/hide.pen`: one line, pointing at the result image. Delete the line if none. -->

<details><summary>More evidence</summary>

<!-- Pen comparisons, per-state captures, technical logs, long command output, one caption line each. Delete the block if none. -->

</details>

## Breaking change

<!--
What breaks (install, state folder, saved settings, public contract) and what the operator must do.
List any place code cannot clean up (shell profile, installed clones, the user's home).
Delete the section if nothing breaks.
-->

<details><summary>Verification record</summary>

<!-- PRD path, base/head SHAs, suites run and results, reviewer findings and their disposition, registered evidence, changed paths. Filled by the `ship` skill: keep this block, and open or update the PR through `ship`. -->

</details>
