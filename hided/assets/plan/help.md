# hide plan

Put up the plan of your own work so the operator sees where you are: Hide draws it as a map beside your pane and reminds you of your step on every prompt.
A step is something you deliver or a point to check (a spec, a change, a PR, a verification); research, failures and retries are not steps.

```
hide plan set (--file <plan.json> | --json <plan>) [--ask [why]] [--reason <text>]
hide plan node <key> [--status planned|working|done|dropped] [--line <text>]
hide plan approve        hide plan drop        hide plan show [<agent>]        hide plan help
```

```json
{"nodes":[{"key":"spec","title":"Write the spec","depends_on":[],"status":"done","output":"file"},
 {"key":"core","title":"Core events","depends_on":["spec"],"status":"working","line":"tests next"},
 {"key":"pr","title":"Open the PR","depends_on":["core"],"status":"planned","output":"pr"}]}
```

- Each node needs `key`, `title`, `depends_on` (other keys, no loop) and `status`; `line` and `output` (`pr`, `file`, `none`) are optional; at most 64 steps.
- Mark a step `working` when you start it and `done` when it is finished; a step cannot be done before the steps it waits on are done or dropped.
- `--ask` puts the plan up for the operator's review: until you run `hide plan approve`, writes, `git commit` and `git push` are refused. Use it when a skill or the operator says to have the plan reviewed, or when you change the goal or scope (`--ask "<why>"`).
- Approval is spoken: when the operator agrees in the conversation, run `hide plan approve`; asked to change it, fix it and `set --ask` again; told not to do it, `hide plan drop`.
- Replanning on your own is fine: `set` again with `--reason`; changed steps are marked and the approval stays.
