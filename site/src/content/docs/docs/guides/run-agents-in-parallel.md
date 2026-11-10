---
title: Run agents in parallel
description: Start several agents on one project at once, give each its own branch, and see what runs at a glance.
features: [tasks-status]
---

**Use when** you have two jobs for one project, or one job you want two agents to try, and you do not want them in each other's way.

<iframe src="/embed/docs-embed.html?part=task-row&theme=dark" title="A task row: the dot shows the state, the line says what the agent is doing" height="140" loading="lazy"></iframe>

One row is one agent's work. Read the dot first: it tells you whether the agent needs you.

## Steps

1. Press `⌘N` (Windows and Linux: `Ctrl+Shift+N`). The Start panel opens over the window. In a browser tab `⌘N` stays the browser's: press `⌘K` (`Ctrl+Shift+K`) and type `Start an agent…` instead.
2. Write the job in the box that asks "What should the agent do?".
3. Look at the row above the box. It reads project, then branch. Open the branch menu and choose `Start on a new branch…`, then type a name in `New branch name`. Hide makes a worktree for that branch, which is its own folder, and starts the agent in it. The branch menu offers this for a Git project.
4. Under the box, pick the agent and the model, then press `Start` (or `Enter`). `Shift+Enter` adds a new line.
5. Press `⌘N` (`Ctrl+Shift+N`) again for the next job. Each start opens a tab of its own, so you can start the next one at once.
6. To race agents on one task, start it again with the same words, a different branch name and a different agent. Give each branch its own name, such as `try-a` and `try-b`.
7. When they finish, open each result and keep the one you like. To drop the others, right-click that branch's row in the Projects list and choose `Delete worktree…`. The dialog asks whether to delete the branch too (`Also delete branch`).

To see everything at once, switch the sidebar list from `Projects` to `Tasks`. In the desktop app, `⌥1` to `⌥9` (`Alt+1` to `Alt+9`) jump to the first nine agents.

## Paste to your agent

When an agent is already working with you and the job splits, ask it to hand the parts out:

```text
Split this into independent parts. Start one agent for each part with
`hide agent spawn --parent here`, each on its own branch, and collect their
reports. Tell me when all of them are done.
```

## What you will see

- Each agent gets a row. A dot or ring shows the state: working, waiting, finished and unread, or yours to answer.
- The lists group rows as `Needs You`, `Unread`, `Working` and, in the agent lists, `Seen`. Only `Needs You` is waiting on a person.
- A new agent starts in its new branch's worktree, and the Projects list shows that checkout under the project.
- When a start goes through, the center moves to the new agent's pane. If it is refused, the reason shows inside the Start panel and your text stays.

## If it goes wrong

- **The panel says the device is not connected.** The target is a device that stopped answering. The panel keeps that target and holds `Start`. Use `Reconnect`, or choose a project on this machine.
- **`Start` does nothing for a long time.** A start with no answer in 90 seconds says so in the panel. Your text stays, so press `Start` again.
- **The agent fails to start after its tab opened.** Your text goes back into the draft with the reason, and the next `⌘N` (`Ctrl+Shift+N`) shows both. Fix the cause and start again.

## Read more

- [Tasks and status](/docs/features/tasks-status/) explains the rows, dots and groups.
- Next guide: [Answer only what needs you](/docs/guides/answer-only-what-needs-you/).
