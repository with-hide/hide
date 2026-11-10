---
title: Run a mission
description: Give one goal to a lead agent that splits it across several agents and several pull requests, and only come back for decisions.
features: [tasks-status]
---

**Use when** one goal needs planning first, several pull requests, or another agent to check the work, and you want to say it once.

<iframe src="/embed/docs-embed.html?part=mission-map&theme=dark" title="A mission map: the Conductor, workers side by side, a decision for you, checks, and main" height="460" loading="lazy"></iframe>

The map shows the Conductor, the parts running side by side, the checks, and the one node that waits on you.

## What a mission is

A mission is one goal carried by one lead agent, the Conductor. The Conductor plans the work with you, starts the agents that do each part, changes the plan when you say so, and reports at the end. Another model checks each part as it finishes. Only decisions come to you.

For the design behind missions, see `docs/missions.md` in the private with-hide/hide-private repository. This guide covers what you see and do.

## Steps

1. Say the goal to an agent in a hide pane. For complex work the agent asks one line: `Shall I run this as a mission, or do it directly?` It asks when the work needs planning or review first, two or more pull requests, another agent checking, or a long conversation.
2. Answer in words: `mission`, or `directly`. You can also call it yourself: type `/hide-mission` in Claude Code or `$hide-mission` in Codex.
3. Plan with the Conductor in the same conversation. Say what done means, and whether the agents stop at ready pull requests (`Up to ready PRs`) or go through the merge (`Through the merge`). Defaults live in Settings, under Mission.
4. Open the map. Press the `Mission` chip on the Conductor's pane, or the sidebar's `Missions` row. The `Missions` screen groups missions as `Your turn`, `In progress`, `Waiting to merge` and `Finished`. The command palette entry is `Open missions` and has no shortcut until you bind one.
5. Work on something else. When a node waits on you, the mission moves into `Your turn`, and its row or card shows the action: `Answer`, `Read letters`, `See what's stuck`, `Take a look`, `Merge` or `Join`.
6. Answer a decision. The card names the node and says `awaiting decision`. Press an option (the Conductor may mark one `Recommended`), or press `Write an answer…`, type, and press `Answer`. You can also answer in the Conductor's conversation.
7. To change course, say it to the Conductor: "stop", "resume", "drop node 3", "do this one first". The map is for looking, not editing.
8. To end it early, open `Mission menu` on the map, choose `End mission…`, then `Ask to end`. `Keep it` backs out.

## Paste to your agent

```text
Run this as a mission: <the goal in one sentence>.
Plan it with me first and show me the plan before you start any agent.
Stop at ready pull requests; do not merge. Another model should check each part.
Come back to me only for decisions.
```

## What you will see

- Each node is a card: its kind (`Change`, `Research`, `Verify together`, `External`), its state (`Waiting`, `In progress`, `Done`, `Dropped`) and its pull request.
- A node that waits on you has a thick border and `Your turn`.
- At the end the Conductor writes a report. The mission closes as `Finished`, or sits in `Waiting to merge` while a pull request is not in main yet. It moves to `Finished` once each is merged.
- A cancelled mission does not reopen.

## If it goes wrong

- **The row says `No Conductor`.** The lead agent's session is gone. Press `Resume command` on the row to take it over.
- **`Answer` says it could not send.** The card shows why, such as `Not sent · no Conductor`. Resume the Conductor, then press `Send again`.
- **A device does not answer.** The map keeps its last values dimmed, and `Resume command` is disabled until the device answers.

## Read more

- [Tasks and status](/docs/features/tasks-status/) explains the rows and marks the map reuses.
- Previous guide: [Answer only what needs you](/docs/guides/answer-only-what-needs-you/).
