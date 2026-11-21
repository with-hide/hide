---
title: Answer only what needs you
description: Many agents can run at once. hide brings up only the ones waiting on a person, and tells you the difference between your turn and done.
features: [tasks-status, project-memory, letters-watches]
---

**Use when** several agents are working and you do not want to keep checking each terminal.

<iframe src="/embed/docs-embed.html?part=turn-card&theme=dark" title="The turn card: your request, the agent's answer, and what waits on you" height="560" loading="lazy"></iframe>

`Needs You` is your turn: a question, an approval, a block, or a file the agent opened for you to look at. `Unread` only means you have not read a result yet.

## Steps

1. Start your agents and work on something else. Rows that need no one stay quiet.
2. To hear about it, turn on `Notify me when an agent needs me` in Settings, under Notifications (desktop app). The app icon badge counts the questions and files waiting on you.
3. When the Overview or the sidebar shows `Needs You`, press the first row. It takes you to the agent that waits.
4. Press the turn row under the pane's title. The turn card opens with your request, the agent's answer and a `Waiting on you` list. `Show all` opens the whole answer. `Open record` shows the full session.
5. Answer in the agent's terminal.
6. The row leaves `Needs You` when the question is answered. A question you read but did not answer keeps its row, and a file the agent opened keeps it until you look at the file.
7. Open each `Unread` row and read the result. `Unread` is not a promise that the work is done: a stopped or unfinished agent is there too, and a read question that is still unanswered can wait in `Seen`. Check the turn card's `Waiting on you` list and the files the agent opened before you leave.

If the agent needs you to sign in, unlock something or allow a permission, its row shows a key instead of the `!`. The line reads `Sign-in` or `Permission`, then the place to act. Do the step, then press `Done` on the row or the pane. The agent gets a letter and goes on. Sending your own message to it also ends the wait; a message you typed but did not send does not.

When an answer is a rule you will repeat, ask the agent to keep it as a card with `hide memory add <text>`.

## Paste to your agent

Agents started inside hide already read this from `hide guide`. Paste it for an agent that does not use it:

```text
When the next step is one only I can take, run
`hide agent ask-human auth <where>` for a sign-in, a re-authentication or an
unlock, or `hide agent ask-human permission <where>` for an app connection or an
OS permission. Then stop and end your turn. Put in <where> the page or setting
where I act, for example `hide agent ask-human auth https://github.com/login`.
If you get past the step yourself, run `hide agent ask-human --clear`.
Give any command that waits for my approval a timeout, so you can declare and
stop instead of hanging. Ask me anything else, a decision included, in this
conversation.
```

## What you will see

- A key in the warning badge, named `A person's turn`, and a line like `Sign-in · https://github.com/login`.
- `Done` next to the line, on the sidebar and Tasks rows, the pane head and the Overview.
- After `Done`, the key and the line go everywhere at once, and the agent continues.
- A one-time code in the place stays visible until you press `Done`.

## If it goes wrong

- **`Done` is disabled.** The agent runs on a device that is not connected. Reconnect the device first.
- **A row stays in `Needs You` after you answered in the terminal.** The row leaves when the question is answered, not when it is read. Check the terminal for a second question, or for `Waiting on you` items on the turn card.
- **The line is still there after `Done`.** The agent declared a second step. The row shows the one declared first, with the rest counted behind it, and `Done` settles only that one. Do the next step and press `Done` again.

## Read more

- [Tasks and status](/docs/features/tasks-status/) explains the rows, marks and groups.
- [Project Memory](/docs/features/project-memory/) explains the cards.
- [Letters and watches](/docs/features/letters-watches/) explains the letter an agent gets when you press `Done`.
- Next guide: [Run a mission](/docs/guides/run-a-mission/).
