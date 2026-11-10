---
title: Install on macOS
description: Download hide for Apple Silicon, open it for the first time, sign in to your agents, and keep it up to date.
sidebar:
  order: 2
---

## Requirements

- An Apple Silicon Mac with macOS 14 or later. No Xcode is needed.
- Optional: an agent CLI (Claude Code, Codex, Grok, OpenCode, Pi, omp or Antigravity), installed and signed in, if you want hide to start it.
- Optional: GitHub's CLI `gh` 2.72.0 or later, signed in, if you want pull requests in hide. An older `gh` shows nothing, and hide says to update it.

## Install

1. Download `hide-v<version>-macos-arm64.dmg` and its `.sha256` file from [withhide.dev](https://withhide.dev) or the [Releases page](https://github.com/with-hide/hide/releases).
2. In the folder that holds both, check the download. Continue only when it prints `OK`.

   ```sh
   shasum -a 256 -c hide-v<version>-macos-arm64.dmg.sha256
   ```

3. Open the disk image and drag `hide` onto the Applications folder beside it.
4. Open hide from Applications, not from the disk image. An app that runs from the disk image cannot update itself, and Settings > About tells you to move it.

## First launch

hide is signed with a Developer ID and notarized by Apple, so macOS opens it like any other downloaded app.
No Control-click is needed.
That holds for every release after v0.3.0; v0.3.0 and earlier are not notarized, and for those you Control-click `hide.app`, choose **Open** and confirm the dialog.

If macOS still keeps the app in quarantine, check the download's checksum first, then run:

```sh
xattr -dr com.apple.quarantine /Applications/hide.app
open /Applications/hide.app
```

On the first launch a short setup opens over the app: **Choose your agents**, then (only if your ssh config lists other machines) a step for those machines, then **Know when an agent needs you**, then **How do you want to start?**
hide adds its own entries to the config of each agent you turn on, so it can tell when that agent needs you.
Nothing changes until you choose **Continue**.
macOS asks for permission to show notifications when you choose **Turn on notifications**.

## Sign in to your agents

hide never asks for an agent's credentials.
Each CLI keeps its own sign-in, so install the CLI and sign in to it in a terminal before you launch hide.
In the **Choose your agents** step each agent shows **Ready**, **Not signed in** or **Not installed**.
For pull requests, sign in to `gh` the same way (`gh auth login`).

## The `hide` command

The first launch links `~/.local/bin/hide` to the app's `hide` command, so `hide .` in a terminal opens that folder as a project.
hide never edits your `PATH`.
If a new terminal does not find `hide`, add `~/.local/bin` to your shell's `PATH`, open a new terminal, and run `hide status --json` to check.

## Updates

hide updates itself.
It checks shortly after it opens, then every six hours, and downloads in the background.
When an update is ready, a ↑ button appears and a card says **Update ready**: choose **Restart now** to install it, or **Later**.
Without a restart, the update installs the next time you quit hide.
Your agents and panes keep running through a restart.
**Check for updates…** in the hide menu looks at once, and Settings > About shows the state.

## Uninstall

Quit hide and move `/Applications/hide.app` to the Trash.
This keeps your saved state in `~/.hide` and `~/Library/Application Support/hide-desktop`; delete both only when you want to reset hide.
The entries hide added to your agents' configs, and `~/.local/bin/hide`, stay until you remove them (the full list is in [Uninstall](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#uninstall)).

## What macOS supports

The package, signing, self-update, daemon and pane rows of [What each system supports](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#what-each-system-supports) are all marked supported on macOS.
macOS is the only system where the app is signed and notarized.

Next: [First 10 minutes](/docs/start/first-10-minutes/).
