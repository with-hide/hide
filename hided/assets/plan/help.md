# hide plan
Put up the plan of your own work so the operator sees where you are: Hide draws it as a map beside your pane and reminds you of your step on every prompt. A step is something you deliver or a point to check (a spec, a change, a PR, a verification); research, failures and retries are not steps.

```
hide plan set (--file <plan.json> | --json <plan>) [--ask [why]] [--reason <text>]
hide plan node <key> [--status planned|working|done|dropped] [--line <text>] [--pr <number> | --file <path>]
hide plan approve        hide plan drop        hide plan show [<agent>]        hide plan help
hide agent spawn --parent here --plan-node <key> --name <name> --kind <kind> --repo <path> --branch <branch> --brief <text>
```

```json
{"nodes":[{"key":"spec","title":"Write the spec","depends_on":[],"status":"done","output":"file","file":"docs/spec.md"},
 {"key":"core","title":"Core events","depends_on":["spec"],"status":"working","line":"tests next"},
 {"key":"web","title":"Web map","depends_on":["spec"],"status":"planned","delegate":true},
 {"key":"pr","title":"Open the PR","depends_on":["core","web"],"status":"planned","output":"pr"}]}
```

- Each node needs `key`, `title`, `depends_on` (other keys, no loop) and `status`; `line`, `output` (`pr`, `file`, `none`) and `delegate` are optional; at most 64 steps.
- Mark a step `working` when you start it and `done` when it is finished; neither is taken before the steps it waits on are done or dropped (`plan_order_refused`). Only the status is refused: your edits are never blocked.
- A step ends by its output: `pr` with `--status done --pr <number>`, a pull request made from a branch of your checkout or of a child you handed a step to; `file` with `--file <path>`, a file inside your checkout (relative to its root); `none` with nothing. In `set` a done step carries the same as `"pr": <number>` or `"file": "<path>"`. Hide checks the pull request with GitHub and the file on your machine; `plan_pr_unconfirmed` means GitHub could not be asked now, so run the same command again later, and tell the operator if it lasts.
- When you plan, tell the operator once how you will verify the work (which steps check what, and how) and let them confirm; if they say to just go on, go on as you judge. No verification step is required: size it to the work.
- `--ask` puts the plan up for the operator's review: until you run `hide plan approve`, writes, `git commit` and `git push` are refused. Use it when a skill or the operator says to have the plan reviewed, or when you change the goal or scope (`--ask "<why>"`).
- Approval is spoken: when the operator agrees in the conversation, run `hide plan approve`; asked to change it, send the fixed plan with `set --json '<plan>' --ask` (file writes are refused while it waits); told not to do it, `hide plan drop`.
- Replanning on your own is fine: `set` again with `--reason`; changed steps are marked, the approval stays, and a done step keeps its evidence and the agent that did it.

## Handing a step to another agent
- When the operator asks for the work split across agents, or a step is worth it: a step that delivers something (a spec, a change, a PR, a review, a verification) and can be done without your context goes to another Hide agent; research and searches stay with your own subagents, which are not on the map. Do most steps yourself.
- Hand it with `hide agent spawn --parent here --plan-node <key> ...` and a brief that says what to deliver, where, and to report when done. Mark a step you mean to hand with `"delegate": true` so the map shows it as one to hand. The spawn is refused while the steps it waits on are unfinished, while the plan waits for review (`plan_review_pending`), and for an agent outside Settings > Conductor's worker candidates (`plan_candidate_refused`). Running the same spawn again returns the same agent.
- For a review or a verification, prefer a candidate from another company than yours; with none installed, use a new agent of your own company.
- The child works on its own branch: for a `pr` step it opens its own pull request; for a change it commits and you take it in; a review or a verification ends with its report and no commit.
- The child tells you by letter: `hide request send <parent> --kind report` when the step is done (the step shows 보고 옴), `--kind block` when it is stuck or cannot do it (막힘). A report finishes nothing: read it, take the work in if it is a change, then mark the step done. A handed step is done only after its current child's report (`plan_report_missing`); a `pr` step also needs that child's `--pr <number>`, while a `file` step needs no `--file`, since the file is on the child's branch until you take it in.
- 막힘 lifts when the child sends progress or a report, or when you answer it. With a blocked child, or one whose pane closed (닫힘), choose: answer it (blocked only), hand the step again with the same spawn command (a new agent, and an earlier agent's report no longer counts), do it yourself (`hide plan node <key> --status working`), or drop it (`--status dropped`). A child stopped by an expired login reaches the operator by itself; the step stays as it is.
- Once you mark a handed step done, or take it back, Hide puts its child to sleep when it rests off screen.
- Your plans' children that have not ended or gone to sleep number at most 3 (`plan_delegate_full`), and a letter that would wake a sleeping child past that is refused too; wait for one to finish, or do the step yourself.
- An agent a plan or a mission started does its own work and starts no agents (`plan_delegate_not_permitted`).
- When the work needs an independent verification record, the merge order of several pull requests, or a lead that could be replaced, ask the operator whether to make it a mission ("미션으로 올릴까요?"). Only ask: a refusal ends it, and a full cap alone is no reason to ask.
