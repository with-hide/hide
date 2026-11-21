# hide automation
An automation hands one agent the same task on a schedule, such as tidying new issues every weekday morning: at each time Hide starts a fresh conversation in the automation's folder, and the operator sees its runs under 자동화 below the missions in the sidebar.
When the operator asks for a task to run again at set times with no one present ("매일 아침 9시에 이슈 정리해줘", "every Monday check the stale PRs"), make it with `hide automation add`, not a cron job, an OS scheduler or a script of your own.
A wait or a check inside this conversation (watch CI every ten minutes until it ends, poll a deploy) is not an automation: keep doing it yourself as now.

```
hide automation add --name <name> --task <text> --schedule <when> --kind <kind> [--profile <name>] [--model <model>] [--effort <effort>] [--folder <path> [--worktree]] [--run-now]
hide automation list        hide automation show <id|name>        hide automation preview --schedule <when>
hide automation run <id|name>        hide automation pause <id|name>        hide automation resume <id|name>
hide automation edit <id|name> [--name <name>] [--task <text>] [--schedule <when>] [--kind <kind> [--profile <name>] [--model <model>] [--effort <effort>]] [--folder <path> | --own-folder] [--worktree | --no-worktree]
hide automation remove <id|name>
```

## Making one from the operator's words
- `--task` is what each run's agent is handed, written as a request it can carry out alone: what to do, where to look, what to leave behind (a file in its folder, a comment, a pull request) and when to ask the operator. A run starts with no memory of this conversation; Hide adds the last run's time, result and final answer to it.
- `--schedule` takes one of these, read in the zone of the machine that runs Hide:
  - `daily 09:00`: every day at 09:00.
  - `weekdays 09:00`: Monday to Friday at 09:00.
  - `weekly mon,thu 09:00`: on the days named (`sun` `mon` `tue` `wed` `thu` `fri` `sat`).
  - `every 15m`: every 5, 10, 15 or 30 minutes, counted from each hour's start (:00, :15, :30 and :45 here).
  - `every 3h 09:00`: every 1 to 12 hours, counted from that time (00:00, 03:00, ..., 21:00 here).
  - `cron 0 9 * * 1-5`: a five-field cron line, minute hour day month weekday.
- An automation runs at most every five minutes: a cron line whose times come closer than that is refused (`*/5 * * * *` runs; `*/4 * * * *` does not).
  When the operator wants an unattended repeat more often than that, tell them the shortest period is five minutes and offer `every 5m`.
  A run still going when the next time comes keeps going, and that time is skipped.
- `hide automation preview --schedule <when>` answers what a schedule reads as and when it runs next, without making anything.
- `--kind`, `--profile`, `--model` and `--effort` name the agent each run starts, as `hide agent spawn` takes them (`hide agent catalog` lists what a machine has).
  Use the agent, model and effort the operator named; when they named none, your own kind with the model and effort you run with.
- Without `--folder` each run works in the automation's own folder, `~/hide/automations/<name>-<id>`, which Hide makes and keeps out of the sidebar's projects, and its results stay there.
  Give `--folder` only when the operator named a project, or the task is plainly about one (this repository's issues or pull requests): the project's root folder as Hide has it open on the machine that runs Hide, Git or not, never a folder inside it or a worktree of your own that will go.
  An automation runs on the machine that runs Hide: from a pane of a connected device, leave out `--folder` (it is refused with `automation_folder_on_device`), and tell the operator it runs on that machine.
  `--worktree` gives each run a new worktree of that Git project; a run that changed nothing leaves none behind.
- `--run-now` starts a run as soon as the automation is made, marked manual; do it when the operator wants to see one now.
- Each run starts with the agent permissions the operator set in Settings › Agents, with no one watching: say so when the task writes outside its folder.

## The answer
Each command prints one line, `{"ok":true,"value":…}` or `{"ok":false,"error":{"code","message"}}`.
- `add`, `edit`, `show`, `run`, `pause` and `resume` answer the automation: its `id`, `reads_as_text` (`weekdays at 09:00`), `next_run_at` on the machine's wall clock with its `zone`, `paused`, `running`, `folder`, `agent` and its `records` newest first.
  Tell the operator in one line what you made and when it runs next, in their language, such as `평일 09:00마다 돌아요 · 다음: 11-20(목) 09:00`.
- `list` answers each automation with its newest record as `last_record`; `remove` answers which one went and how many records with it.
- An automation stopped for something a person fixes (`paused.reason` other than `operator`: a sign-in that ended, a folder that is gone, an agent not set up) carries `fix`, one sentence of what to do; tell the operator that, then `resume` it once they have.
  A `run` or an `add --run-now` that could not start for such a reason answers `paused.code` and `paused.fix` the same way.
- A refusal's `message` says what to change: relay it to the operator in plain words when it is theirs to choose (a period under five minutes, a name already taken, a folder Hide does not know), and fix your own command when it is yours.

## Changing and removing
- Name an automation by its id or its exact name; `list` shows both.
- `edit` changes only what it names, from the next run on: a run going keeps what it started with. `--own-folder` moves it back to its own folder.
- `pause` stops the next times and lets a run going finish; `resume` turns it on again from its next time and clears its line in the operator's 내 차례.
- `run` starts a manual run now, refused while one goes (`automation_running`).
- `remove` only when the operator asked for it: it ends a run going, closes its terminals and forgets its records; its folder and the files in it stay.
