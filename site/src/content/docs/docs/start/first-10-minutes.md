---
title: First 10 minutes
description: From download to a first pull request, with three agents on the same task.
sidebar:
  order: 5
---

By the end of this page you will have three agents working on the same task in your own repository, and a pull request from the one you like best.
You need a Git repository on your machine, at least one agent CLI that you have signed in to, and `gh` signed in if you want to see the pull request in hide.

## Four things to look at

Once an agent runs, the window has four places to look.

1. **Left: what exists.** The sidebar's Tasks list has one row per task. The mark on the row tells you if it needs you.
2. **Middle: the agent.** Each agent is the real CLI you already know, in its own pane.
3. **Right: what it made.** Files, diffs, pull requests and pages open in the lane on the right.
4. **Yellow: your turn.** A yellow `!` means an agent is asking you something. It is the only state you have to answer.

<iframe src="/embed/docs-embed.html?part=task-row&theme=dark" title="A task row, live component" height="140" loading="lazy"></iframe>

<iframe src="/embed/docs-embed.html?part=status-colors&theme=dark" title="The four state colors, live component" height="220" loading="lazy"></iframe>

Look at the row first, then the colors under it: yellow is your turn, green is finished and not looked at yet, blue is at work, and grey is seen.
The list groups the rows the same way, under **Needs You**, **Unread** and **Working**.

## Then do this

1. **Install hide** for your system, and sign in to the agent CLIs you use: [macOS](/docs/start/install-macos/), [Windows](/docs/start/install-windows/) or [Linux](/docs/start/install-linux/).
2. **Open a folder as a project.** In a terminal inside your repository, run `hide .`. Or press `⌘⇧N` (`Alt+Shift+N` on Windows and Linux) for **Add project**, then choose **Browse folder**.
3. **Start three agents on the same task.** Press `⌘N` (`Ctrl+Shift+N` on Windows and Linux) to open the start panel. Write the task under **What should the agent do?**, choose the agent, and in the branch menu choose **Start on a new branch…** and name it. Press Enter. Do it three times with three branch names.
4. **Watch the left list, keep the best one, and open its pull request.** Answer any yellow row, then tell the agent you like: `Commit your work and open a pull request.` Close the other two from their row's menu with **Close session**.

### Notes on each step

**Step 2.**
`hide .` adds the folder first if it is not a project yet, then brings that project forward.
If your terminal does not find `hide`, see the install page for your system, or use **Add project**.
The first project you add ends the setup and opens the start panel once on that project, with the cursor in the text box.

**Step 3.**
The start panel's agent menu lists Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity.
Next to it, `! Not signed in` means the CLI is not signed in yet; sign in to it in a terminal first.
**Start on a new branch…** makes a new worktree for that agent, so the three agents do not edit the same files.
The branch menu shows for a Git project.
Each start opens in a tab of its own.

**Step 4.**
If the sidebar shows Projects, choose **Tasks** in its list switch (Projects, Tasks, PRs).
When an agent opens a pull request, a PR mark with its number appears on its row.
Press the mark to open the pull request's card in the lane.
`⇧⌘R` (`Alt+Shift+R`) opens the PR list for every project on this device.
Pull requests need `gh` 2.72.0 or later, signed in.

## What you will see

- Three rows in the Tasks list, blue while the agents work.
- A row turns yellow when its agent asks you something, and green when it finishes and you have not looked yet.
- After you ask for it, the row you like carries a PR mark and number.

## If it goes wrong

- **`hide` is not found in a terminal.** hide never edits your `PATH`. Add the folder that holds the `hide` command, as your system's install page says, or use **Add project**.
- **No agent starts, and the Home says "Add a project to start an agent."** Add a project first, as in step 2.
- **No pull request shows.** Check that `gh` is 2.72.0 or later and signed in. An older `gh` shows nothing, and the GitHub row in Settings says to update it.

## Next

Learn the six parts of the screen in [Concepts](/docs/start/concepts/), or try the steps on a practice screen first in [Practice](/docs/start/practice/).
