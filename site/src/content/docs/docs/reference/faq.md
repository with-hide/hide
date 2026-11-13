---
title: FAQ
description: Short answers to what people ask before and just after they install hide.
sidebar:
  order: 4
---

## Is hide a wrapper around my agent?

No. Your agent runs in a terminal pane as its own program, with its own sign-in.
hide does not bundle authentication: every agent CLI keeps its own sign-in state and credentials, and hide never starts a different agent in place of one you chose.

What hide does add is a small piece for each agent, so it can see what the agent does inside its own process, such as the subagents it starts:

- Hide's entries in the agent's own settings file, for example `~/.claude/settings.json` for Claude Code and `~/.codex/hooks.json` for Codex. Other agents get a hook file, plugin or extension of their own.
- A `hide` skill, plus `hide-mission` and `hide-issue`, that the agent can call.

hide appends one entry per event and touches nothing else in the file, so entries other tools put there stay.
On a computer where hide has never run, it holds off until you answer which agents to set up in the first-run wizard.
When you uninstall, these pieces stay and do nothing; the [install guide](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#uninstall) lists what to delete.

## Which agents work with it?

Claude Code, Codex, Grok, OpenCode, Pi, omp and Antigravity.
Install the agent's CLI and sign in to it first; hide starts it, it does not sign in for you.
Cursor is retired: you can still read its existing conversations, but hide does not start it.

Grok has three differences today:

- A letter to Grok is rung only while Grok is at rest; while it works, Grok takes due letters at its next tool call.
- Grok does not receive Project Memory cards, because its adapter has no route for them yet.
- Desk lists Grok dimmed and never starts it, because hide cannot hold Grok in a mode that asks before it acts.

## How do I connect my phone?

1. On your computer, open Settings > Mobile and turn on **Open hide from your phone**.
2. Wait until the connection row reads `Connected · Hide relay (Tokyo)`, then press **Show QR**.
3. On an iPhone, install [Hide from the App Store](https://apps.apple.com/app/hide-remote-coding-agents/id6832780181) and choose **Scan QR code**. On any other phone, scan the code with the camera; it opens `app.withhide.dev`; tap **Connect**.
4. In a browser, you can keep it as an app with Share, then Add to Home Screen.

You need no Hide account, payment or Tailscale installation.
The code works for five minutes, and one phone uses it once.
Keep your computer awake and connected: when it sleeps or goes offline the phone says it cannot connect and retries when the computer returns.
Each computer allows four paired phones; Settings > Mobile lists them under **Connected phones** and lets you **Revoke** any.
To get notifications, allow them in the phone app, then choose when the computer pushes them in Settings > Notifications: `Off`, `Only when the app is closed` or `Always`.
The phone is told only when an agent needs your answer, a question, an approval or a block, and an agent that settles on its question within a few seconds is one notification.
Tailscale remains as an advanced direct option under `Advanced · Connect directly with Tailscale`.
If the connection does not work, see [Troubleshooting](/docs/reference/troubleshooting/).

## Can the relay read my conversations?

No. Your phone and your computer encrypt the messages before they leave either one, and the relay only passes them along.
It cannot read your conversations, commands or phone credentials.
It can see routing identifiers, when connections happen and how much data passes through, and it does not keep messages for later delivery.
Your agents, the core and Hide AI still run on your own machines, using the CLIs you signed in to; those CLIs talk to their providers as they do without hide.

## What does hide send over the internet?

- Your phone's messages, through the encrypted relay described above, only when Mobile is on.
- Usage counts, only while Settings > Privacy > **Share usage statistics** is on, which it is by default. It collects counts of connections, agent and mission starts, device and phone connections and practice chapters, with the app version and operating system. It does not send conversations or code. Turn the switch off and no new counts are collected.
- A look at hide's GitHub releases for updates ([Does hide update itself?](#does-hide-update-itself)).

## Which systems does hide run on?

Apple Silicon Macs with macOS 14 or later, Windows x64 and Linux x64 (`.deb` and `.rpm` packages).
An Intel Mac and a Windows ARM PC are not supported.
Windows 11 with Smart App Control turned on blocks unsigned programs, so hide does not run there.
You can also use a Mac, Windows PC or Linux machine as a device: connect it from Settings > Devices over SSH.

## Why do Windows and Linux warn me when I install?

The macOS app is signed and notarized.
The Windows installer and the Linux packages are not signed, so Windows SmartScreen warns about the installer, and apt and dnf install a local file you checked yourself.
Every download has a `.sha256` file beside it: check it before you install.
The steps for each system are in the [install guide](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#install-a-release), and the SmartScreen steps are in [Troubleshooting](/docs/reference/troubleshooting/#windows-says-windows-protected-your-pc).

## Does hide update itself?

Yes. A packaged release looks for a new release shortly after it opens, every six hours after that, and when you return to the app after an hour or more away.
It downloads in the background and shows nothing until the update is ready: a dot appears on the `?` button in the sidebar.
Choose **Restart** in the Help menu, or **Restart now** in Settings > About, to install it.
macOS and Windows also install it the next time you quit; Linux installs it only when you choose Restart.
A download whose digest does not match the release's update file is discarded.
Your state in `~/.hide` and your panes and agents keep running through an update.

## Does hide keep running after I close the window?

The window is the app; the daemon, `hided`, keeps your panes and agents running.
On macOS the app stays open with no window until you quit it; on Windows and Linux, closing the last window quits the app.
Either way `hided` keeps running while a pane runs.
With no pane running and no window connected, it exits after ten minutes.

## Where does hide keep my files?

Everything hide owns on a computer is under `~/.hide`, and every folder it makes there is private to your account.
Quitting hide and moving it to the Trash removes the app and keeps `~/.hide` and the app's profile; delete both only when you want to reset hide's saved state.

## What are the keyboard shortcuts?

The full list, for macOS, Windows and Linux, is on [Keyboard shortcuts](/docs/reference/shortcuts/).
In the app, open it with `⌘/` on macOS or `Ctrl+Shift+/` on Windows and Linux.
