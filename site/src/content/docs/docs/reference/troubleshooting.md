---
title: Troubleshooting
description: What you see when something goes wrong, and the fix for each.
sidebar:
  order: 3
---

Find the line you see on screen, then follow its fix.
The wording in quotes is what the app prints.
Most install entries come from the [install guide](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#troubleshooting), which has the full detail.

## Installing and opening

### macOS blocks hide the first time

Releases after v0.3.0 are signed and notarized, so macOS opens them like any other download.
If you installed v0.3.0 or an earlier release, Control-click `hide.app`, choose **Open**, and confirm the dialog.
If macOS still keeps the app in quarantine, check the download's `.sha256` file first, then run:

```sh
xattr -dr com.apple.quarantine /Applications/hide.app
open /Applications/hide.app
```

### Windows says "Windows protected your PC"

The Windows installer is not signed, so SmartScreen stops it the first time.
Choose **More info**, check that the dialog names `hide-v<version>-windows-x64-setup.exe` with an unknown publisher, then choose **Run anyway**.
Check the installer against its `.sha256` file before you run it.
In PowerShell, from the folder that holds both downloads, the first line printed must be the hash at the start of the second:

```powershell
(Get-FileHash -Algorithm SHA256 .\hide-v<version>-windows-x64-setup.exe).Hash.ToLower()
Get-Content .\hide-v<version>-windows-x64-setup.exe.sha256
```

### hide does not run on Windows 11

Windows 11 with Smart App Control turned on blocks every unsigned program, and hide is unsigned.
hide does not run there.
This is a known gap, listed in the install guide under "What each system supports".

### Settings > About says updates are off, or asks you to move the app

An app that runs from the disk image, or from a folder you cannot write to, cannot update itself.
On macOS, drag `hide` to Applications and open it from there.
Updates are also off for a development build, an app run from a source checkout, and a Linux app that was not installed from the `.deb` or `.rpm` package.

### An update is ready but not installed

When an update is ready, a dot appears on the `?` button at the bottom of the sidebar, and the Help menu ends with `hide <version> ready · Restart`.
Choose **Restart** there, or **Restart now** in Settings > About.
macOS and Windows also install it the next time you quit.
On macOS, closing hide's last window while an update is ready quits hide to install it.
Linux installs it only when you choose Restart, because the system asks for your password; if you cancel that prompt, the update stays ready.

### hide does not open after an update

A published release is never changed; a bad one is replaced by a higher version, which the app takes at its next check or at **Check now**.
Until then, install the previous release from the [Releases page](https://github.com/with-hide/hide/releases) by hand.
Your `~/.hide` folder and the app's profile stay as they are.

Read this before you install an earlier release by hand.
After you have used a profile in a mission's worker candidates or in Hide AI's agent items, version 1.2.6 or earlier cannot read those choices, and version 1.2.5 or earlier also drops a session's profile from its launch record when it saves.
Read the profile rows of [What each system supports](https://github.com/with-hide/hide/blob/main/docs/INSTALL.md#what-each-system-supports) in the install guide first.
The app never installs a lower version by itself.

An install cut off partway is finished by installing again: drag `hide` to Applications again on macOS, run the release's `setup.exe` again on Windows, or on Linux run `sudo dpkg --configure -a` and then `sudo apt install ./hide-v<version>-linux-x64.deb` (on an rpm system, `sudo dnf reinstall ./hide-v<version>-linux-x64.rpm`).

## When the app starts

### "Can't connect to hided"

The window shows `Starting Hide…` while it connects.
If it cannot reach the daemon, it shows this page with **Retry** and **Copy diagnostics**, and one line that names the cause.

| The line says | What to do |
| --- | --- |
| "The hide command was not found." | If you set `HIDE_CLI_PATH`, an unusable value ends the search: fix it or unset it. Otherwise the installed app uses the `hide` inside its own folder, so install the app again. |
| "hided could not start." | Press **Retry**. If it repeats, read the host log (below). |
| "hided did not respond." | Press **Retry**. If it repeats, read the host log (below). |
| "Another build of hided is running. Only the app can replace it." | Open the installed app. Only the installed app replaces a daemon of another build; a development or standalone `hide` refuses to. |
| "hided could not start because of this file, which was left unchanged." | The page names the file, and Hide did not change it. The reason is in the host log (below). Choose **Copy diagnostics**, then report it from the Help menu with **Report an issue**. |
| "Update this app" | The core on that machine runs a newer build than this app. Replace the app with the newer build and open it again. The core keeps running. |
| "The core was not updated" | The core still runs its previous build. Press **Retry**, or update Hide on that machine. |

The host log is `<profile>/logs/desktop.log`.
The profile is `~/Library/Application Support/hide-desktop` on macOS, `%APPDATA%\hide-desktop` on Windows and `~/.config/hide-desktop` on Linux.
A `HIDE_DESKTOP_USER_DATA_DIR` variable replaces it.
**Copy diagnostics** puts the build, the reason code and the log's place on the clipboard.

### The app opens but no pane opens

The daemon could not start the pane's holder.
Its diagnostic log, `Logs/core.jsonl` in the state folder, says why under a kind that starts with `pane_holder.`, such as `pane_holder.start_refused`.
The holder is the `hide-holder` program beside `hided` (in `hide.app/Contents/Resources` on macOS, in `resources` in the install folder on Windows and Linux); on Windows it also needs the `holder-conpty` folder beside it.
An install that is missing either cannot open a pane, so install the app again.

### An agent does not start

Check that the agent's command resolves, and complete that CLI's own sign-in:

- macOS and Linux, in your login shell: `command -v claude` or `command -v codex`.
- Windows, in PowerShell, the default shell of a Windows pane: `Get-Command claude` or `Get-Command codex`.

hide never starts the other agent in its place when the one you chose is missing.
On Windows, a pane takes its PATH from the system Path and your user Path, not from the environment Hide started in, so put the agent CLI's folder on one of them.

### "hide: command not found" in a new terminal

Hide never changes your PATH.
On Linux, add `export PATH="$HOME/.local/bin:$PATH"` to your shell's startup file and open a new shell.
On Windows, add `%USERPROFILE%\.local\bin` to your user Path with **Edit environment variables for your account**, then open a new terminal.
Run `hide status --json` to check that the command now resolves, or call `hide` by its full path.

### The `hide` command and the agent hooks are missing after the first launch

A first launch can stop before it changes anything, for example when Hide's own folder is unavailable or another Hide is changing the same account.
Until it passes, the `hide` command, the agents' hooks and letters are all missing.
On a Mac no pass has answered for, the first-run wizard shows what stopped it with **Try again** and **Not now**; on an install set up before, and on a device, the line sits under the machine in Settings > Devices with **Retry**.
**Try again** and **Retry** run the same pass again, so the line goes only once nothing stops it.
**Not now** closes the wizard with the agent question unanswered; it opens again for a different refusal or when Hide's daemon starts again.

Fix what the line names:

- A folder or file it names: make it a real folder of your account that no other account can change, then press **Try again** or **Retry**.
- Open work of an earlier Hide: finish or close it, then try again.
- Too many registered projects: remove the ones you no longer use, then try again.
- The kit record, `~/.hide/kit/installed.json`: one a newer Hide wrote belongs to that build, so update Hide. One no Hide can read can be moved aside; the next pass starts a new record and asks the agent choice again.
- A state folder chosen by `HIDE_STATE_DIR` or `XDG_STATE_HOME`, which the line names: set the variable to a folder only your account can change, or unset it so the state returns to `~/.hide/state`.
  On Windows a folder below a drive root such as `D:\` whose permissions let every signed-in account change it is refused.
  On macOS and Linux a folder that is group- or world-writable, another account's, or reached through an account's link is refused.

The app hands its own environment to the `hide connect` it runs, and that connect starts `hided` on the folder the environment chooses.
`hided` keeps that folder until it stops, so quitting the app leaves it running as long as a pane runs.
A launch whose environment chooses the same folder attaches to it again, and one that chooses another folder starts a second daemon beside it.
So on the machine that runs the app, the line also names the command that stops the daemon on the refused folder, with that folder and the app's own `hide` written out: `HIDE_STATE_DIR='<folder>' '<app>/hide' stop`, or the PowerShell form on Windows.
Quit Hide, run that command, change the variable where the app's launch reads it, and open Hide again.
A device's line names no command: its helper reads the variable when it starts there.

Where the app's launch reads the variable:

- macOS: an app opened from the Dock, Finder or Spotlight gets launchd's environment, not a shell profile's, so `launchctl setenv` and `launchctl unsetenv` change it for the next launch. An app opened with `open` from a terminal gets that terminal's variables.
- Windows: an app started from the Start menu gets the account's environment variables (System Properties or `setx`), which Explorer passes to every app it starts after the change.
- Linux: an app started from the desktop's menu gets the session's environment, set at login (`~/.profile`, `environment.d`), so a change there reaches it at the next login. One started from a terminal gets that terminal's.
- `hide connect` run from a terminal reads that terminal's variables.

### Pull requests and GitHub issues are missing

Hide needs GitHub's CLI `gh` 2.72.0 or later, signed in.
An older `gh` lacks fields Hide asks for, so Hide reads nothing with it and its GitHub row says to update it.

## Other machines and your phone

### A machine does not connect over SSH

Check that SSH works outside hide, with the same alias the machine was added with:

```sh
ssh <host-alias> true
```

hide runs that same `ssh`, so what fails there fails in hide too.
The diagnostic log, `Logs/core.jsonl` in the state folder, keeps ssh's own words.
The machine's row in Settings > Devices says what to do: **Sign in**, check a changed server key with `ssh <alias>` in a terminal, or **Retry** once the cause that ssh's last line names is fixed.
**Test** runs the SSH, authentication, protocol, PTY and Git stages one after another and names the first one that needs attention.
What ssh asks, such as a password or a key's passphrase, is answered on the machine's row; hide does not store, log or copy an answer.

### The phone says "The code expired"

The QR code works for five minutes, and one phone uses it once.
On your computer, open Settings > Mobile, press **Show QR**, and scan the new code.

### The phone says "You can connect up to 4 phones"

Each computer allows four paired phones.
On your computer, open Settings > Mobile, find the phone under **Connected phones**, and press **Revoke** on one you no longer use.

### The phone shows "This phone's connection was revoked"

A phone that stays away for seven days is revoked, and so is one you revoked on the computer.
Open a new QR code on your computer and scan it again.

### The phone shows "<computer> has Hide turned off or is asleep · Retrying"

The phone connects through your computer, so the computer has to be awake, connected and running Hide.
The phone retries by itself and connects when the computer answers; the relay does not keep messages for later.
If it never does, check Settings > Mobile on the computer: its row should read `Connected · Hide relay (Tokyo)`.
When it reads "Relay unreachable · Retrying", press **Retry now**.
When it reads "This Hide is too old", press **Update**, which opens Settings > About.

### The phone says "Camera unavailable"

Allow camera access for the page, or paste the QR link instead.
