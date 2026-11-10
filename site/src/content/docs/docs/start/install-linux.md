---
title: Install on Linux
description: Install hide from a deb or rpm package, sign in to your agents, and keep it up to date.
sidebar:
  order: 4
---

## Requirements

- Linux on an x64 machine: Debian or Ubuntu for the `.deb`, Fedora or another rpm system for the `.rpm`.
- A desktop distribution. hide needs the libraries every Electron app needs (GTK 3, NSS, ALSA), which the package declares and a desktop system already has.
- Optional: an agent CLI (Claude Code, Codex, Grok, OpenCode, Pi, omp or Antigravity), installed and signed in, if you want hide to start it.
- Optional: GitHub's CLI `gh` 2.72.0 or later, signed in, if you want pull requests in hide.

## Install

1. Download the package and its `.sha256` file from [withhide.dev](https://withhide.dev) or the [Releases page](https://github.com/with-hide/hide/releases): `hide-v<version>-linux-x64.deb` or `hide-v<version>-linux-x64.rpm`.
2. In the folder that holds both, check the download and install it with your package manager. Continue only when `sha256sum` prints `OK`.

   ```sh
   sha256sum -c hide-v<version>-linux-x64.deb.sha256
   sudo apt install ./hide-v<version>-linux-x64.deb
   ```

   ```sh
   sha256sum -c hide-v<version>-linux-x64.rpm.sha256
   sudo dnf install ./hide-v<version>-linux-x64.rpm
   ```

3. Open hide from your applications menu.

## First launch

The package installs hide into `/opt/hide`, adds it to the applications menu, and makes `/usr/bin/hide` the `hide` command.
It also sets up Electron's sandbox helper and, on Ubuntu 24.04 and later, the AppArmor profile the app needs, so there is no manual sandbox step.
The packages are not signed: apt and dnf install a local file that you checked yourself.

On the first launch a short setup opens over the app: **Choose your agents**, then (only if your ssh config lists other machines) a step for those machines, then **Know when an agent needs you**, then **How do you want to start?**
hide adds its own entries to the config of each agent you turn on, so it can tell when that agent needs you.
Nothing changes until you choose **Continue**.

## Sign in to your agents

hide never asks for an agent's credentials.
Each CLI keeps its own sign-in, so install the CLI and sign in to it in a terminal before you launch hide.
In the **Choose your agents** step each agent shows **Ready**, **Not signed in** or **Not installed**.
For pull requests, sign in to `gh` the same way (`gh auth login`).

## The `hide` command

A new terminal's `hide` is the CLI, so `hide .` in a folder opens it as a project.
The first launch also links `~/.local/bin/hide`.
hide never edits your `PATH`: if your shell does not already include `~/.local/bin`, add `export PATH="$HOME/.local/bin:$PATH"` to your shell's startup file and open a new terminal.
Run `hide status --json` to check.

## Updates

hide updates itself when it was installed from the deb or rpm.
It checks shortly after it opens, then every six hours, and downloads in the background.
When an update is ready, a ↑ button appears and a card says **Update ready**.
On Linux an update installs only when you choose **Restart now**, because the package manager asks for your password; if you cancel the prompt, the update stays ready.
Your agents and panes keep running through a restart.
**Check for updates…** in the Help menu looks at once, and Settings > About shows the state.
The packages are unsigned, so an update is checked only against the release's SHA-512 digest.

## Uninstall

Run `hide stop`, then remove the package:

```sh
sudo apt remove hide
sudo dnf remove hide
```

Use the line for the package manager you installed with, then remove `~/.local/bin/hide`.
Your saved state in `~/.hide` and `~/.config/hide-desktop` stays until you delete it, and so do the entries hide added to your agents' configs (the full list is in [Uninstall](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#uninstall)).

## What Linux does differently

Closing the last window quits the app on Linux; `hided`, the background daemon, keeps running while a pane does.
The package, daemon and pane rows of [What each system supports](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#what-each-system-supports) are marked supported on Linux.
The gaps that matter on a first day:

- Signing is not supported. The packages are unsigned.
- An update of the deb is checked in CI; an rpm update is unverified.
- Connecting your phone directly through Tailscale is not supported on Linux; use the relay instead.
- The Needs You count on the app icon is unverified: it shows only on a desktop that draws launcher counts.

Next: [First 10 minutes](/docs/start/first-10-minutes/).
