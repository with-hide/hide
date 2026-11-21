---
title: Concepts
description: The six parts of the screen you will see all day, and how to read each one.
sidebar:
  order: 6
---

These are six parts of hide, drawn live from the product's own components.
Each card says how to read the picture and where to learn more.

## Task row

<iframe src="/embed/docs-embed.html?part=task-row&theme=dark" title="A task row, live component" height="140" loading="lazy"></iframe>

A row is one agent's work in the sidebar. The mark on the left is its state, the line under the name says what it is doing, and the badge on the right counts helpers or questions.

More: [Tasks and status](/docs/features/tasks-status/).

## State colors

<iframe src="/embed/docs-embed.html?part=status-colors&theme=dark" title="The four state colors, live component" height="220" loading="lazy"></iframe>

Yellow is your turn: an agent asks you something. Green is confirmed done and not looked at yet, blue is working, and grey is seen, with nothing waiting.

More: [Tasks and status](/docs/features/tasks-status/).

## Turn card

<iframe src="/embed/docs-embed.html?part=turn-card&theme=dark" title="The turn card, live component" height="560" loading="lazy"></iframe>

Press the turn line under an agent's pane header and the card opens under it: your request, then the agent's answer. A question the agent waits on is your turn, marked **Question** in yellow, and a finished turn is **Done** in green.

More: [Tasks and status](/docs/features/tasks-status/).

## Pane header

<iframe src="/embed/docs-embed.html?part=pane-header&theme=dark" title="A pane header, live component" height="180" loading="lazy"></iframe>

The top row holds the agent's state mark, its logo and its title. A helper's header leads back to the agent that started it, and the second row lists the helpers this agent started and what each is doing.

More: Workspace and terminal (the page is planned as `workspace-terminal`).

## PR card

<iframe src="/embed/docs-embed.html?part=pr-card&theme=dark" title="A pull request card, live component" height="640" loading="lazy"></iframe>

One pull request: its number and state, its branch, the checks GitHub reports and a **Merge** button. The Timeline lists the sessions that worked on it, and **Changed files** opens each diff.

More: PR list (the page is planned as `pr-list`).

## View slot

<iframe src="/embed/docs-embed.html?part=view-slot&theme=dark" title="The lane's tabs for files, diffs and pages, live component" height="420" loading="lazy"></iframe>

A file, a diff or a web page that you or an agent opens shows as a tab in the lane on the right. Pin the lane and it stays beside the agents when the window has room; unpinned, or in a narrow window, it is drawn over them. A pull request's card or a session's record opens in the lane's **View** slot, in front of those tabs.

More: Files and editor (the page is planned as `files-editor`).

## Next

Put the parts to work in [First 10 minutes](/docs/start/first-10-minutes/), or see them move in [Practice](/docs/start/practice/).
