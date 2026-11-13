---
title: Install on Windows
description: Download the Windows installer, get past SmartScreen, sign in to your agents, and keep hide up to date.
sidebar:
  order: 3
---

## Requirements

- Windows on an x64 PC. Windows on ARM is not supported, because hide's Windows programs are built for x64 only.
- Windows 11 with **Smart App Control** turned on cannot run hide. Smart App Control blocks every unsigned program, and hide is unsigned.
- Optional: an agent CLI (Claude Code, Codex, Grok, OpenCode, Pi, omp or Antigravity), installed and signed in, if you want hide to start it.
- Optional: GitHub's CLI `gh` 2.72.0 or later, signed in, if you want pull requests in hide.

## Install

1. Download `hide-v<version>-windows-x64-setup.exe` and its `.sha256` file from [withhide.dev](https://withhide.dev) or the [Releases page](https://github.com/with-hide/hide/releases).
2. In PowerShell, in the folder that holds both, check the download:

   ```powershell
   (Get-FileHash -Algorithm SHA256 .\hide-v<version>-windows-x64-setup.exe).Hash.ToLower()
   Get-Content .\hide-v<version>-windows-x64-setup.exe.sha256
   ```

   Continue only when the first line printed is the hash at the start of the second.
3. Open the installer. It installs hide for your account without asking for administrator access, adds hide to the Start menu and to Settings > Apps, and opens it.

## First launch: SmartScreen

The installer is not signed, so Windows SmartScreen stops it the first time with **Windows protected your PC**.

1. Choose **More info**.
2. Check that the dialog names `hide-v<version>-windows-x64-setup.exe` with an unknown publisher.
3. Choose **Run anyway**.

After install, a short setup opens over the app: **Choose your agents**, then (only if your ssh config or WSL lists other machines) a step for those machines, then **Know when an agent needs you**, then **How do you want to start?**
hide adds its own entries to the config of each agent you turn on, so it can tell when that agent needs you.
Nothing changes until you choose **Continue**.

## Sign in to your agents

hide never asks for an agent's credentials.
Each CLI keeps its own sign-in, so install the CLI and sign in to it before you launch hide.
In the **Choose your agents** step each agent shows a state, for example **Checking…**, **Ready**, **Not signed in**, **Not installed** or **Update needed**. **Installed** means the CLI is found but its sign-in has not been confirmed, so sign in to it yourself.
For pull requests, sign in to `gh` the same way (`gh auth login`).

A pane takes its `PATH` from the system Path and your user Path, not from the environment hide was started in.
Put each agent CLI's folder on one of them, or hide's panes will not find it.

## The `hide` command

The first launch adds `%USERPROFILE%\.local\bin\hide.cmd`, which runs hide's command line.
hide never edits your `Path`.
Add `%USERPROFILE%\.local\bin` to your user Path with **Edit environment variables for your account**, open a new terminal, and run `hide status --json` to check.
Then `hide .` in a folder opens it as a project.

## Updates

hide updates itself.
It checks shortly after it opens, then every two hours, and downloads in the background.
When an update is ready, a ↑ button appears and a card says **Update ready**: choose **Restart now** to install it, or **Later**.
Without a restart, the update installs when you quit hide.
Your agents and panes keep running through an update.
**Check for updates…** in the Help menu looks at once, and Settings > About shows the state.
The installer is unsigned, so an update is checked only against the release's SHA-512 digest, not against a publisher.

## Uninstall

Remove hide in Settings > Apps.
That removes the app and its Start menu entry and stops the daemon, but the programs running in your panes keep running.
Close your panes in hide before you uninstall.
Your saved state in `%USERPROFILE%\.hide` and `%APPDATA%\hide-desktop` stays until you delete it, and so do the entries hide added to your agents' configs and `%USERPROFILE%\.local\bin\hide.cmd` (the full list is in [Uninstall](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#uninstall)).

## What Windows does differently

Closing the last window quits the app on Windows; `hided`, the background daemon, keeps running while a pane does.
The package, daemon and self-update rows of [What each system supports](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#what-each-system-supports) are marked supported on Windows.
The gaps that matter on a first day:

- Signing is not supported. SmartScreen warns once, and Smart App Control blocks hide outright.
- Missions are unverified: no mission has run on Windows yet.
- Windows on ARM is not supported.

Next: [First 10 minutes](/docs/start/first-10-minutes/).
