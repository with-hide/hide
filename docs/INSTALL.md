# Install hide

hide runs on Apple Silicon Macs with macOS 14 or later, on Windows x64 and on Linux x64.
You can install a published release when one is available or build the same app from source today.
Each release carries a signed macOS app and unsigned Windows and Linux packages, built from the same source; [Windows and Linux](#windows-and-linux) says how to install those, and [What each system supports](#what-each-system-supports) records where each system stands.

## Requirements

- An Apple Silicon Mac with macOS 14 or later, or a Windows x64 or Linux x64 machine for the unsigned packages.
- No Xcode is required to install or run hide.
- For source builds only: a current stable Rust toolchain with Rust 2024 edition support, Node.js 22, and pnpm 10.
- For source builds on Windows only: the Rust MSVC toolchain with the Visual Studio C++ build tools, Git for Windows, and PowerShell 7 (`pwsh` fetches the pinned Herdr); run the build from Git Bash, so `bash` is Git's and not the WSL launcher in `System32`.
- For source builds on Linux only: `zsh`, `jq`, and `curl`, `shasum` and `awk` under `/usr/bin`, which the Herdr fetch uses.
- For source builds only: network access for the pinned Rust crates, the npm packages, the Electron runtime download, and the pinned Herdr asset.
- An agent CLI Hide supports (Claude Code, Codex, Grok, OpenCode, Pi, omp or Cursor), installed and signed in, only if you want hide to launch that agent.

Check the build tools before a source install:

```sh
rustc --version
cargo --version
node --version
pnpm --version
```

## Install a release

Open the [hide Releases page](https://github.com/with-hide/hide/releases) or [withhide.dev](https://withhide.dev), which links the latest stable release's files for your system.
If there are no releases yet, continue with [Build and install from source](#build-and-install-from-source).
Every installer has a `.sha256` file beside it naming only that file; check it before installing.
After this one install, hide updates itself ([Update](#update)).

| System | Download | Installs to |
| --- | --- | --- |
| macOS, Apple Silicon | `hide-v<version>-macos-arm64.dmg` | `/Applications/hide.app` |
| Windows x64 | `hide-v<version>-windows-x64-setup.exe` | `%LOCALAPPDATA%\Programs\hide`, for this user only |
| Debian, Ubuntu (x64) | `hide-v<version>-linux-x64.deb` | `/opt/hide` |
| Fedora and other rpm systems (x64) | `hide-v<version>-linux-x64.rpm` | `/opt/hide` |

The release also carries `hide-v<version>-macos-arm64.zip`, the update files `latest-mac.yml`, `latest.yml` and `latest-linux.yml`, and `.blockmap` files; those are what the app's updater reads, and you do not need them to install.

### Install on macOS

Check the disk image from the folder that holds both downloads:

```sh
shasum -a 256 -c hide-v<version>-macos-arm64.dmg.sha256
```

Continue only when the command prints `OK`.
Open the disk image and drag `hide` onto the Applications folder beside it, then open hide from Applications.
Run it from Applications, not from the disk image: an app on the disk image or in a folder you cannot write cannot update itself, and Settings > About says to move it.

Releases after v0.3.0 are signed with hide's Developer ID and notarized by Apple, so macOS opens them like any other downloaded app.
v0.3.0 and earlier are ad-hoc signed and not notarized: on the first launch, Control-click `hide.app`, choose **Open**, and confirm the Gatekeeper dialog.

If macOS still keeps one of those in quarantine, remove the quarantine attribute only after confirming the downloaded checksum:

```sh
xattr -dr com.apple.quarantine /Applications/hide.app
open /Applications/hide.app
```

### Install on Windows

Check the installer in PowerShell from the folder that holds both downloads:

```powershell
(Get-FileHash -Algorithm SHA256 .\hide-v<version>-windows-x64-setup.exe).Hash.ToLower()
Get-Content .\hide-v<version>-windows-x64-setup.exe.sha256
```

Continue only when the first line printed is the hash at the start of the second.
Open the installer.
It installs hide for your account without asking for administrator access, adds hide to the Start menu and to Settings > Apps, and opens it.

The installer is not signed, so Windows SmartScreen warns about it the first time, and the launch stops at "Windows protected your PC".
Choose **More info**, check that the dialog names `hide-v<version>-windows-x64-setup.exe` with an unknown publisher, then choose **Run anyway**.
Windows 11 with Smart App Control turned on blocks unsigned programs wherever they are, and hide and its Herdr are unsigned, so hide does not run there ([What each system supports](#what-each-system-supports)).

The install folder holds the Electron app at its top and, in its `resources` folder, the same `hided`, `hide`, `hide-agent-hooks` and pinned Herdr the macOS app carries, as `.exe` files, with Herdr's ConPTY runtime in the `conpty` folder beside `herdr.exe`.
The app does not run the daemon or the Herdr server from there: at each launch it copies those programs into `%USERPROFILE%\.hide\runtime\<build>` and starts them from the copy, so installing a new version, by hand or by an update, never stops the Herdr server or the agents in it.
A copy no program runs from any more is removed at a later launch, and at most four are kept; one that cannot be removed is recorded in the app's log (`runtime.copy_remove_failed`, `runtime.copies_over_cap`).

### Install on Linux

Check the package from the folder that holds both downloads, then install it with the system's package manager:

```sh
sha256sum -c hide-v<version>-linux-x64.deb.sha256
sudo apt install ./hide-v<version>-linux-x64.deb
```

```sh
sha256sum -c hide-v<version>-linux-x64.rpm.sha256
sudo dnf install ./hide-v<version>-linux-x64.rpm
```

Continue only when `sha256sum` prints `OK`.
The package installs hide into `/opt/hide`, adds hide to the applications menu, and makes `/usr/bin/hide` the `hide` command, so a new terminal's `hide` is the CLI.
It also sets up Electron's sandbox helper (`/opt/hide/chrome-sandbox`) and, where AppArmor restricts user namespaces (Ubuntu 24.04 and later), the profile the app needs, so there is no manual sandbox step.
The packages are not signed: apt and dnf install a local file you checked yourself.
The app needs the libraries every Electron app does (GTK 3, NSS, ALSA), which the package declares and a desktop distribution already has.

### Moving from a zip or tar.gz build

A Windows zip folder or a Linux tar.gz folder from before this change cannot update itself.
Install the new package once as above; its first launch moves the `hide` command and every agent hook to the installed app, as an update does.
Then remove the old folder: run its `hide stop` (`resources\hide.exe stop` on Windows, `resources/hide stop` on Linux), stop its Herdr server with `herdr server stop` from the same folder's `resources` when no agent you need is running in it, and delete the folder.
The new app starts its own daemon and Herdr server at its next launch, and `~/.hide` with all of hide's state stays where it is.

### What differs from the macOS app

- Closing the last window quits the app on Windows and Linux, as other apps there do; on macOS the app keeps running with no window until you quit it.
  Either way `hided` keeps running after the app is gone, as [First launch](#first-launch) describes.
- The app's profile is `%APPDATA%\hide-desktop` on Windows and `~/.config/hide-desktop` on Linux, instead of `~/Library/Application Support/hide-desktop`.
- First launch installs the `hide` command and, once the first-run wizard's agent choice is answered, each chosen agent's hook, plugin or extension and skill, as on macOS ([What the first launch installs](#what-the-first-launch-installs)).
  Settings > Devices shows their installation results on this computer's row.
- Linux also installs `~/.local/bin/hide` as a link to the package's `resources/hide`, beside the package's own `/usr/bin/hide`.
  Windows installs `%USERPROFILE%\.local\bin\hide.cmd`, which runs the runtime copy's `hide.exe` through a `.hide-kit` directory junction beside the command; neither Administrator access nor Developer Mode is needed.
- Opening a newer package replaces a running daemon of another build and refreshes the command and hook paths, as on macOS.
  A development or standalone `hide` still refuses to replace a packaged daemon.
- A package's `hided` is also the node it installs on a device, for its own system only, as the macOS app does: a Linux package installs it on a Linux x64 device, while no device runs the Windows one yet (Windows devices are not supported), so from Windows a device's files and Git stay unavailable.
- On Windows the installer adds the Start menu entry (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\hide.lnk`) that notifications are attributed through, and the first launch adds a per-user COM class key under `HKCU\Software\Classes\CLSID`, so a notification clicked after Hide quit can start it.
- An update installs when you quit the app on Windows, as on macOS; on Linux it installs only when you choose Restart, because the package manager asks for your password ([Update](#update)).

### What each system supports

Every agent Hide supports and every feature works on macOS, Windows and Linux: that is the bar, and a feature is not finished until it meets it on all three.
This table is the one record of where each system stands.
A cell that is not supported says why and what would change it, and a pull request that changes what a system does updates its row in the same change.
The behavior itself is owned by the guide each row links to.

Supported means the package is built from the release, the feature is written for that system, and CI exercises it there.
Unverified means the feature is written to work there, but no check on that system has proved it.
Not supported is a gap, never a design choice.

| | macOS (Apple Silicon, 14 or later) | Windows x64 | Linux x64 |
| --- | --- | --- | --- |
| Package | supported: a dmg holding the app bundle, both signed with hide's Developer ID and notarized ([BUILD.md](BUILD.md#developer-id-signing)); the package checks open the dmg and start the app the update zip holds | supported: a per-user installer, which the package checks install, install again while a Herdr server runs from the runtime copy, and uninstall | supported: a deb and an rpm into `/opt/hide`, which the package checks install on Ubuntu and in a Fedora container and start |
| Signing | supported (Developer ID) | not supported: the installer and the programs are unsigned because signing has to stay free for now; SmartScreen warns on the first install, an update's publisher is not checked (only the release's SHA-512 digest is), and Windows 11 with Smart App Control on blocks hide and its Herdr outright (the upstream `herdr.exe` is unsigned too, herdrdev/herdr#4380); changes with Azure Artifact Signing or SignPath (issue #193) | not supported: the packages are unsigned for the same reason, and an update's publisher is not checked (only the release's SHA-512 digest is); changes with a signed package repository |
| The app updates itself ([Update](#update)) | supported: the choice of release, the states and the download are unit tested; a real update from one release to the next is unverified until two Developer ID builds exist, the first two release candidates | supported: CI installs a release-numbered build and updates it to the next one from a local feed through Help's Restart, keeping the Herdr server and the daemon | supported for the deb: the same CI update runs on Ubuntu; an rpm update is unverified (the rpm is installed and started in Fedora, not updated) |
| Daemon, Herdr, install kit | supported | supported (CI runs them against the pinned Herdr) | supported (the web shell's whole e2e suite runs against the pinned Herdr) |
| The eight agents: Claude Code, Codex, Grok, OpenCode, Pi, omp, Cursor and Antigravity, each with its hook, plugin or extension and skill ([agent-hooks.md](agent-hooks.md#support-table)) | supported; agy's tool-permission question reaches Needs You as an approval (seen in the app on Herdr 0.9.3, 2026-10-16), its folder-trust question is unmeasured, and agy has no bell until that question is shown to read as blocked too; agy asks its own permission question before each `hide` command as before any command, because agy 1.4.0 has no command allow list and a hook's answer does not skip that question; it closes when agy offers a command allow list ([agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block)) | supported: each agent's Windows form is read from its own source or package and run on the Windows runner (`hide-agent-hooks/tests/it/windows_hook_command.rs`, including agy's `cmd` tool command when its helper is gone, fails or answers, and `windows_scripts.rs`), and the `agy contract` lane installs the vendor's agy there and checks it loads Hide's `cmd /c` block; the kit's read of whether Codex has its shared server, which every Codex start Hide makes waits for, runs a `codex.cmd` that starts Node there (`hide-agent-hooks/src/codex_daemon.rs`), where a read in an environment built from nothing had left every Codex start refused; a hook's request reaches the daemon's bootstrap listener over a named pipe and reads its answer in the `windows check` lane (`hided/src/workspace_cli.rs`), which no hook could do until the pipe took a write timeout, so no Windows prompt got Project Memory or its letters; a real Grok, Cursor or agy session is unverified, and agy has the same bell gap as macOS | supported: the same entries as macOS, checked by the workspace tests on Linux and by the nightly `agy contract` leg; a real Grok or Cursor session is unverified, a real agy turn passes on the nightly `agy contract` lane (Hide's hooks, a letter and a refused start), and agy has the same bell gap as macOS |
| A delegated Claude Code, Codex or agy starts in the worktree Hide made without its folder trust question ([agent-hooks.md](agent-hooks.md#folder-trust-for-the-worktrees-hide-makes)) | supported: the agent's own trust of that worktree is recorded on its machine before it starts, and the real Claude Code 2.1.302 and Codex 0.163.0 read it in the nightly `agent trust contract` | supported: written in each agent's Windows spelling (`/` between names for Claude Code, lower case for Codex), checked by the unit tests on the Windows runner and with the real agents by the nightly contract's Windows leg | supported: the unit tests and Codex's stand-in in `rust`, and the real agents in the nightly contract's Linux leg |
| Letters and watches, agent registration and spawning, and agent sleep, whose records the daemon keeps in its state folder's durable files ([delivery.md](delivery.md#persistence-clocks-and-limits)) | supported | supported: the state folder's directory barrier has its Windows form (a folder is flushed through a handle allowed to add an entry to it, [ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)), so the ledger opens and agent sleep saves the record of an agent it closes, where every letter, `hide agent register` and `hide agent spawn` had answered `ledger_unavailable` and agent sleep could close no agent; Claude Code, Codex and agy, which sleep (and Reopen) in their own pane, are ended there as on the other systems: Hide presses the agent's own quit keys first, and the pane's node ends the agent's process tree only when it does not quit, where it had refused as unsupported and left them awake ([ARCHITECTURE.md](ARCHITECTURE.md#agent-sleep)); a known gap remains: an agent that does not quit on its keys (a draft in its input) cannot put back its terminal modes once ended, Herdr then wraps the wake's start line in a bracketed paste the Windows prompt does not read, and that wake fails with `The agent did not become ready in time.` at the start's deadline, until Herdr stops wrapping a line for a shell that holds the terminal ([herdrdev/herdr#5371](https://github.com/herdrdev/herdr/issues/5371), whose close is when this gap and the quit keys' reason for Windows go); the `windows check` lane opens the ledger from a real state folder, registers two agents, sends a letter and reads it back from the inbox and from disk, saves a sleep record, ends a stand-in agent and the tool it started until the pane's shell holds the terminal, and checks that the daemon on the pinned Herdr saved its ledger at startup; a real agent registering, reading a letter, sleeping and resuming on a Windows machine is unverified, and closes with the agent journey's Windows lane | supported: the same code, checked by the workspace tests on Linux |
| Letters to an agent while it works ([delivery.md](delivery.md#letters-while-the-agent-works)), one row per agent; the macOS cells were measured with the agent logged in on a Mac, through `scripts/agent-letters-check.py` against this build's daemon and a private Herdr, and the figures are in the pull request that landed it (#1115): Claude Code | supported: the doorbell types the letter's line while the agent works and the agent reads it after the running tool, in the same turn (measured: `--now` and the five-minute wait) | supported: the doorbell and the prompt hook are the same code, and the hook's Windows form runs on the Windows runner (`hide-agent-hooks/tests/it/windows_hook_command.rs`); a real agent reading a line typed mid-turn is unverified, since no agent is logged in on the runner, and closes with a logged-in agent on a Windows machine | supported: the same code, checked by the workspace tests on Linux; a real agent reading a line typed mid-turn is unverified for the same reason |
| Codex | supported: the same path as Claude Code (measured with `--now`; Codex 0.163.0 needs `[features] codex_hooks = true` and the hook trust answered, which the install kit writes and the operator answers once) | same as Claude Code | same as Claude Code |
| Grok | supported: its `PreToolUse` hook hands the letters over at the next tool call, any tool (measured with `--now`) | supported: the hook's PowerShell form runs on the Windows runner (`hide-agent-hooks/tests/it/windows_hook_command.rs`) and the core's rules in every system's tests; a real session is unverified, since the agent needs an account, and closes with a logged-in agent on a Windows machine | supported: the same code, checked by the workspace tests on Linux; a real session is unverified for the same reason as Windows |
| OpenCode | supported on OpenCode 1.18, where the plugin's `tool.execute.after` appends the letters to the tool result (the fake-host test on the 1.18.30 event capture), and on OpenCode 2.0, where the server half of Hide's plugin appends them to the tool result and the terminal half, in the pane that shows the session, asks for them (the fake-host test on a stand-in for 2.0's shared server; a real 2.0.29 session repeated a letter that reached it at a tool result, 2026-10-18) | supported: the plugin's or extension's tool-result handler runs on the Windows runner against the real helper, which reaches for `hide` and records that none answered (`hide-agent-hooks/tests/it/windows_scripts.rs`), OpenCode 2's two halves included, and a letter reaching a tool result is proved by the Unix fake-host test (`opencode_plugin.rs`, `pi_extension.rs`); a real agent reading one is unverified, since no agent is logged in on the runner, and closes with a stand-in daemon on the Windows lane or a logged-in agent there | supported: the same code, checked by the workspace tests on Linux, where the fake-host test proves a letter reaching a tool result; a real agent reading one is unverified for the same reason as Windows |
| Pi | supported: the extension's `tool_result` adds the letters to the result (the fake-host test on the 1.0.4 event capture); a real session is unverified, since the Mac's Pi login could not be used (its OpenAI token refresh answered `invalid_grant`), and closes with a logged-in Pi | supported: the plugin's or extension's tool-result handler runs on the Windows runner against the real helper, which reaches for `hide` and records that none answered (`hide-agent-hooks/tests/it/windows_scripts.rs`), and a letter reaching a tool result is proved by the Unix fake-host test (`opencode_plugin.rs`, `pi_extension.rs`); a real agent reading one is unverified, since no agent is logged in on the runner, and closes with a stand-in daemon on the Windows lane or a logged-in agent there | supported: the same code, checked by the workspace tests on Linux, where the fake-host test proves a letter reaching a tool result; a real agent reading one is unverified for the same reason as Windows |
| omp | supported: the extension's `tool_result` adds the letters to the result, read before the next tool (measured with `--now`) | supported: the plugin's or extension's tool-result handler runs on the Windows runner against the real helper, which reaches for `hide` and records that none answered (`hide-agent-hooks/tests/it/windows_scripts.rs`), and a letter reaching a tool result is proved by the Unix fake-host test (`opencode_plugin.rs`, `pi_extension.rs`); a real agent reading one is unverified, since no agent is logged in on the runner, and closes with a stand-in daemon on the Windows lane or a logged-in agent there | supported: the same code, checked by the workspace tests on Linux, where the fake-host test proves a letter reaching a tool result; a real agent reading one is unverified for the same reason as Windows |
| Cursor | supported: its `preToolUse` hook hands the letters over at the next tool call, any tool (the hook's payload tests); a real session is unverified, since the Mac's Cursor plan refuses a named model and ran out of requests on `auto`, and closes with a Cursor plan that can run a turn | supported: the hook's PowerShell form runs on the Windows runner (`hide-agent-hooks/tests/it/windows_hook_command.rs`) and the core's rules in every system's tests; a real session is unverified, since the agent needs an account, and closes with a logged-in agent on a Windows machine | supported: the same code, checked by the workspace tests on Linux; a real session is unverified for the same reason as Windows |
| Antigravity | supported: its `PreInvocation` hook runs before every model call of a turn, and on a call after the prompt's first Hide answers it with the letters due while the agent works as an injected system message, which the model reads before its next step; a real session is unverified for mid-turn delivery, since agy runs only in the operator's own home (its sign-in lives in the keychain of that home) and the measurement never writes a hook into the operator's own agent configuration; closes with a run the operator allows in their home, or an agy that reads a private configuration folder | supported: the hook's Windows form runs on the Windows runner (`hide-agent-hooks/tests/it/windows_hook_command.rs`) and the core's rules in every system's tests; a real session is unverified for the same reason | supported: the same code, checked by the workspace tests on Linux; a real session is unverified for the same reason |
| Project guardrails ([agent-hooks.md](agent-hooks.md#project-guardrails)) | supported: shell rules in all eight agents, file writes in all eight, file reads in all but Codex, which has no read tool, so a protected path is caught only in its shell words; a real refusal was seen in Claude Code, Codex, Cursor, OpenCode, Pi and omp, and Grok's and agy's rest on their payload tests | supported, with one gap: the same rules for the same agents, from the same code, and the decision's tests run on the Windows runner (`hide-agent-hooks` library tests); a shell call's words are read as a POSIX shell reads them, so in a PowerShell command a backslash path (`.\.env`, `C:\repo`) is not resolved and the files protection and a `git -C`/`cd` aimed by one miss it, while file tools and forward-slash paths are guarded; this closes when a payload names the shell it runs in and the guardrails read PowerShell's words for it; a real agent's refusal is unverified | supported: the same rules for the same agents, checked by the workspace tests on Linux; a real agent's refusal is unverified |
| GitHub through `gh` for the PRs tool and PR card (reads, Merge, branch update; [UI_BEHAVIOR.md](UI_BEHAVIOR.md#the-prs-tool-and-the-pr-card)) | supported | unverified: the allowlist, the head-conditioned command lines, the card parse and the grouping are platform-neutral Rust and run in every system's tests, but the `gh` process boundary test (`hide-host/src/gh.rs`) runs on Unix only and the web e2e runs on Linux; closing it means running that boundary test against a Windows `gh` shim and tagging the PRs journey `@platform` | supported (the boundary test and the web e2e run on Linux) |
| Forking an OpenCode 2 session: the node asks OpenCode's server with `opencode api` and the fork's pane opens the session it made ([ARCHITECTURE.md](ARCHITECTURE.md#agent-adapters)) | supported: a real fork was made and opened on OpenCode 2.0.29 by the agent journey's fork cell (#1356), and the `opencode api` process test (`hide-host/src/opencode_fork.rs`) runs on Unix | unverified: the route by database schema and the answer's parse are platform-neutral Rust that runs in every system's tests, but the `opencode api` process test runs on Unix only, and an `opencode` installed as an npm `.cmd` shim has not been started this way; closes when the agent journey's fake-model lane (`scripts/agent-journey-check.py --fake-model`, #1333) runs the OpenCode fork cell on Windows and it passes, after which this cell becomes supported | supported for the process (the `opencode api` process test runs in the Linux Rust lane); a real fork is unverified, and closes when the same fake-model lane runs the OpenCode fork cell on Linux and it passes |
| Desktop app window and menu | supported (desktop e2e) | supported (the desktop `@platform` tests in every pull request that can change them, and the packaged `hide.exe`'s window in the package checks) | supported (the desktop `@platform` tests in the nightly, under a virtual display) |
| Keyboard shortcuts | supported | supported by registry tests; the table is in [UI_BEHAVIOR.md](UI_BEHAVIOR.md#keyboard-shortcuts-per-system) | same as Windows |
| Agent notifications ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#first-run)) | supported: the wizard's notifications step asks macOS and reads its answer back; delivery on a signed build is unverified | supported: the app registers itself for toasts at launch, and the package checks require its Start menu entry; delivery and a click from the Action Center after Hide quit are unverified | supported where a notification service runs; without one the wizard and Settings say so and keep the switch off; delivery is unverified |
| Needs You count on the app icon | supported: a count on the Dock once macOS lets Hide notify | supported: a dot on the taskbar button, with the count in its description | unverified: drawn only by a desktop that shows launcher counts for the app's `.desktop` entry, which the packages install; no desktop has been tried |
| Device node (a remote machine's files and Git) | supported | not supported: no device runs the Windows `hided` yet, so from Windows a device's files and Git stay unavailable | supported |
| The Herdr server follows an app update ([Update](#update)) | supported | not supported: Herdr hands a running server to another Herdr on Unix only, so the server keeps running from the previous build's runtime copy until it is stopped, which an update never does; changes when Herdr supports live handoff on Windows | supported (the remote mailbox lane hands a pinned server over) |
| A connected device's Herdr follows an app update ([Connect another Mac](#connect-another-mac-over-ssh)) | supported for an Apple Silicon Mac device, whose system the Mac app carries (the device-kit e2e on an isolated SSH device; the remote mailbox lane's macOS nightly hands a Herdr 0.9.1 server to the pinned one) | not supported: no device runs the Windows `hided`, and Herdr hands a server over on Unix only; changes with a Windows device node and Windows live handoff | supported for a Linux x64 device, whose system the Linux package carries (the remote mailbox lane hands a Herdr 0.9.1 server to the pinned one); a Mac app's Linux device keeps its own Herdr until a package carries the Linux helper |
| Move the core to another machine ([Move the core](#move-the-core-to-another-mac)) | supported | not supported: the core's login item is written for launchd only (`hide_platform::user_agents`), with no Windows or systemd form yet; the build a core update writes on the core's machine is written durably on Windows too (its programs are private files there), so the login item is what an update or a move there still waits for | not supported: same as Windows |
| agy's own log kept to one file per kind in `~/.hide/ai/antigravity-logs` for every agy child Hide starts: a Hide AI request, its model list and the usage read ([AI_PROVIDERS.md](AI_PROVIDERS.md#antigravity)) | supported: `--log-file` measured on agy 1.4.0 (2026-10-18) for `agy models` and `-p /usage`, which left nothing in `~/.gemini/antigravity-cli/log`; a request passes the same flag first and was not run live, to spend no model turn; the stand-in tests check every kind's argument and emptying | unverified: the same code and stand-in tests, but `--log-file` has never been run against agy on Windows, where the path is a Windows path; closes with an agy model list or usage read on the Windows runner, which the `agy contract` lane already installs agy on | unverified: as on Windows; closes with the same read on the nightly `agy contract` lane's Linux leg |
| Reporting a Memory card stale ([ARCHITECTURE.md](ARCHITECTURE.md#a-card-an-agent-reports-stale)) | supported for Claude Code, Codex, OpenCode 1.18 and 2.0, Pi, omp and agy, whose held card ends when Memory analyzes the reporting session past the report (OpenCode's counted in messages, the rest in bytes); OpenCode 2.0's prompt gets its card from the terminal half of Hide's plugin, in the pane that shows the session, and Memory reads its sessions from its own tables (`session_v2` and `session_message`, read with 2.0.29's schema); not for Grok and Cursor, which have no Memory hook, so no prompt gives them a card to report, which changes when they get one | same as macOS (the store and command are platform-neutral Rust run on every system's tests; a real report there is unverified) | same as Windows |
| Comments on a page in Comment mode ([BROWSER_DISPLAYS.md](BROWSER_DISPLAYS.md#comment-mode)) | supported, from the window on the machine whose daemon holds the page; a screen of a linked node answers `comment_unavailable_here` because its daemon relays the core and cannot reach the page's gateway; changes when the node relays the gateway too | same as macOS (unverified: the overlay is the same Chromium code, not yet run on a Windows CI lane) | same as macOS (unverified on Linux the same way) |

Unverified on Windows and Linux (the macOS assumptions below are left as they are, and no check on those systems has proved them):

- Paths outside the `$HOME` boundary, and drive letters and UNC paths on Windows.
- Opening a path in the system's file manager (Reveal), and opening a file or link in the default app.
- Move to Trash through the system's trash.
- Terminal programs that expect macOS text-editing keys; Windows and Linux send `Ctrl+U`, `Home` and `End` as typed.
- Whether Alt+Shift also switches the input language on a Windows or Linux layout; hide's chord still runs.
- A real Grok or Cursor session: each needs an account, so their Windows and Linux hook forms rest on each CLI's source and package, not on a run.
- A real agy turn on Windows: the nightly `agy contract` lane's signed-in turn does not run there, because its stand-in `hide` is a shell script and the helper looks for `hide.exe` beside itself; it closes when the test builds a portable stand-in.
  Signed in on Windows, agy lists Hide's four events.
  On Linux and macOS one real turn passes (run 38887836027 on 2026-10-16): Hide's hooks fire, a letter reaches the conversation and is confirmed after agy recorded it, and a direct `herdr agent start` ends as agy's tool error with Hide's reason.
  The lane signs agy in on the CI account's key (`HIDE_AGY_API_KEY`, handed to agy as `GEMINI_API_KEY` with `"modelProvider": "gemini"` in its settings, the API-key mode agy 1.4.0's own changelog names; `GEMINI_API_KEY` alone asked for a browser sign-in), runs the turn on `gemini-3.8-flash-medium` (`HIDE_AGY_MODEL` in `agy-contract.yml`), the model the key's free tier serves (agy's API-key default `gemini-3.1-pro` has a quota of 0); the refusal needs that model to make the call it is asked for, so a renamed model or a model that declines reads as a failed nightly, not a Hide fault; that free tier allows 20 requests a day per model across the project, of which a nightly spends 4, so hand runs beside the nightly can spend it, and a turn the provider then answers nothing because of the per-day request quota (agy's `429 RESOURCE_EXHAUSTED`, run 38958746826, macOS; `contracts/provider-quota.json`) still checks the hooks and the letter and reports the refusal in a `Not measured` warning instead of failing ([TESTING.md: A real agent on CI](TESTING.md#a-real-agent-on-ci)); it says "not configured" without the key.
- An agent in the agent journey on Windows ([VERIFICATION.md: Agent journey](VERIFICATION.md#agent-journey)): the harness runs there, and the journey's `windows forms` job proves its Windows forms on `windows-latest`, but no lane starts an agent there yet.
  The keyless lane runs on Linux and macOS only, and the signed-in lane, which runs on Windows, starts an agent only when the CI account's key for it is set.
  It closes when the keyless lane starts its agents on `windows-latest` and passes.
- A notification reaching the screen, on any system: the desktop e2e replaces the system's notification, and a moved Windows folder has not been tried.

### Make the command available in a new shell

Hide never changes PATH; on Windows the only registry key is the notification registration the first launch makes ([What differs from the macOS app](#what-differs-from-the-macos-app)).
On Linux, if your shell does not already include `~/.local/bin`, add `export PATH="$HOME/.local/bin:$PATH"` to your shell's startup file and open a new shell.
On Windows, add `%USERPROFILE%\.local\bin` to your user Path using **Edit environment variables for your account**, then open a new terminal.
Run `hide status --json` to check that the installed command resolves.
You can also call the command by its full path without changing PATH.

## Build and install from source

Clone the repository and build the release bundle:

```sh
git clone https://github.com/with-hide/hide.git
cd hide
pnpm install --frozen-lockfile
HIDE_VERSION=<version> pnpm --dir desktop package
```

`HIDE_VERSION` sets the version the packaged app reports.
Omit it to build from a checkout that a Git tag matching `v[0-9]*` already describes; the packaging script fails rather than ship a version nothing was released under.

Packaging builds the web shell, builds the release `hided`, `hide` and `hide-agent-hooks` binaries, fetches and digest-verifies the pinned Herdr binary, and stops with a named error and no app if any of those is missing or not executable.
It then packages everything into `desktop/out/hide-darwin-arm64/hide.app`, ad-hoc signs it, verifies the signature, and writes the release files into `desktop/out/release`: `hide-v<version>-macos-arm64.dmg` and the `.zip` the updater installs, each with its `.sha256`, `latest-mac.yml` and the zip's `.blockmap`.
A release signs with the Developer ID certificate and notarizes instead; [BUILD.md](BUILD.md) gives the switch and what it needs.
The same command on Windows x64 writes the app folder `desktop/out/hide-win32-x64/` and, in `desktop/out/release`, `hide-v<version>-windows-x64-setup.exe` with `latest.yml` and its `.blockmap`; on Linux x64 it writes `desktop/out/hide-linux-x64/` and `hide-v<version>-linux-x64.deb` and `.rpm` with one `latest-linux.yml`; each installer has its `.sha256`, the Windows and Linux ones are unsigned, and each system builds only its own package ([BUILD.md](BUILD.md)).

Install the app into the system Applications directory and launch the installed bundle:

```sh
sudo /usr/bin/ditto --rsrc --extattr --qtn desktop/out/hide-darwin-arm64/hide.app /Applications/hide.app
open /Applications/hide.app
```

## Verify the installed app

Verify the bundle signature:

```sh
/usr/bin/codesign --verify --deep --strict --verbose=2 /Applications/hide.app
```

For a release after v0.3.0, also confirm Gatekeeper accepts it as notarized; the answer ends with `accepted` and `source=Notarized Developer ID`:

```sh
/usr/sbin/spctl --assess --type execute --verbose=4 /Applications/hide.app
```

After opening hide, confirm the installed executable is the one running:

```sh
pgrep -fl '/Applications/hide.app/Contents/MacOS/hide'
```

Exactly one matching process should be active before checking the UI; a second match, at another path, is a dev or worktree build still running.
The app's bundle id is `me.grab.hide.desktop`.

## First launch

The app runs its own bundled `hide` CLI, which starts `hided` beside it.
`hided` serves the web shell inside the app's window.
It keeps running after the window closes for as long as the Herdr server it follows answers, so agent labels stay current with no window open; it exits after ten minutes with no client connected and no answering Herdr server.
The app passes its bundled Herdr binary to that CLI as `HERDR_BIN_PATH` unless the environment names one as an explicit override, one that arrives without `HERDR_PANE_ID`; the value a Herdr pane exports is replaced, so opening the app from a pane behaves like opening it from the Dock.
The CLI search order, and `HIDE_CLI_PATH`, are documented in [ARCHITECTURE.md](ARCHITECTURE.md), "The desktop host".

Panes attach to a Herdr server.
When none answers on the socket hide uses, the app starts the Herdr it bundles (`hide.app/Contents/Resources/herdr` on macOS, `resources/herdr` on Linux, `resources\herdr.exe` on Windows) there before it connects, and that server restores the saved workspaces and resumes the agents Herdr can resume, as a server `herdr` starts from a terminal does.
The default socket is the one Herdr itself uses: `$XDG_CONFIG_HOME/herdr/herdr.sock` when that is set, otherwise `%APPDATA%\herdr\herdr.sock` on Windows and `~/.config/herdr/herdr.sock` elsewhere; set `HERDR_SOCKET_PATH` to an absolute path before launching to use another socket.
A server already running on that socket is used as it is; hide never stops or replaces it, and a server the app started keeps running after the app quits.

Authentication is not bundled: SSH, Herdr and every agent CLI keep their own sign-in state and credentials.
If you want to start agents from hide, install and sign in to the relevant CLI before launching the app.

### What the first launch installs

Every launch of the installed app installs Hide's kit on this computer without asking, and puts back only what a newer app needs.
The one exception is a computer the kit has never run on: it holds Claude Code's, Codex's and agy's hook entries, skills, Herdr integrations and Codex trust (the three agents on by default; the other five start off until switched on) until the operator answers the first-run agent choice, the first step of the first-run wizard (UI_BEHAVIOR.md, First run); the three are recorded off in the hold, installed or not, so an agent installed later also starts off, and the rest below is installed as usual:

- `~/.local/bin/hide`, a link to the app's `hide` command, unless a `hide` that is not Hide's is already there;
- Hide's entries in `~/.claude/settings.json` and `~/.codex/hooks.json`, for each of Claude Code and Codex whose folder (`~/.claude`, `~/.codex`) is on this Mac, next to whatever other tools put there, among them a `PreToolUse` entry on the shell tool that refuses a direct `herdr agent start` and points to `hide agent spawn` ([agent-hooks.md](agent-hooks.md#the-spawn-guard));
- for each of the other agents that is on and installed, its own piece, in the file [agent-hooks.md](agent-hooks.md#what-is-written-and-where) lists: Grok's `~/.grok/hooks/hide.json`, Hide's entries in Cursor's `~/.cursor/hooks.json`, OpenCode's plugin `~/.config/opencode/plugins/hide.js`, Pi's and omp's extension `hide.ts` under `~/.pi/agent/extensions` and `~/.omp/agent/extensions`, and Hide's `hide` block in agy's `~/.gemini/config/hooks.json` with a trusted folder for Hide AI under `~/.hide/ai`, with each agent's `hide-browser` skill stub; an agent counts as installed when its program is found ([agent-hooks.md](agent-hooks.md#installed-means-the-program-is-found)), while Claude Code's and Codex's entries follow their folders;
- Codex's trust for those Codex entries: Codex asks for a review of every new or changed hook, so Hide takes installing it as your agreement to its own hooks and has Codex record them as trusted (through `codex app-server`, never by editing `~/.codex/config.toml`), only for an entry whose command is exactly the one Hide wrote, and for the entry Herdr's Codex integration wrote when the kit installed it in the same pass (the kit records what Herdr wrote and trusts exactly that), so a Mac the kit set up starts Codex with no review screen; another tool's hook, and a Herdr integration you installed before Hide, still open Codex's review screen (only Codex asks; a survey on 2026-10-07 found no hook approval screen for the other agents, [agent-hooks.md](agent-hooks.md#which-agents-ask-for-a-review)), and switching Hide's hook off in Codex's hook list keeps it off ([agent-hooks.md](agent-hooks.md#codex-trusts-hides-own-hooks));
- A one-release retirement stage removes the former coordination installation after its read-only preflight succeeds; see Coordination retirement below;
- for each of the eight supported agents that is on and installed (Claude Code, Codex, Grok, OpenCode, Pi, omp, Cursor and Antigravity), Herdr's own integration, installed with the bundled Herdr (`herdr integration install <agent>`) so Herdr learns the agent's session; one the operator installed before is left as it is, and Hide removes only what it installed;
- a one-time removal of what an earlier Hide put down for fourteen agents it no longer supports (GitHub Copilot CLI, Amp, Factory Droid, Kiro, Qwen Code, Goose, Cline, Kilo Code, Crush, Junie, Augment, Kimi Code, Mistral Vibe and Gemini CLI): only its marked skill and hook entries come out, and a file you wrote stays;
- the Codex daemon setting is not touched by an install: a Mac where an earlier Hide ran `codex features disable daemon_auto_start` keeps it off, and Hide reads it so every Codex it starts gets `--no-daemon`.
  Only the operator's confirmed "Turn off Codex's shared server…" in a not connected Codex pane changes it, and that also stops the shared daemon running now, which disconnects every Codex attached to it until it is continued with `codex resume`.

### Coordination retirement

Merging this change and installing its build performs the transition.
The operator chooses a time with no active sasu runs, open requests or active watches.
The kit retains this stage for one release on each machine, with its result shown in Settings > Devices.

Before changing anything, the stage checks the legacy ledgers, Hide's delivery ledger and the sasu run registry and registered checkout run state.
Both the legacy v1 and current Hide-only v2 registries are inspected read-only; a selected revision that disappears causes a refusal and a retry instruction, never an empty-registry assumption.
An open item, an unreadable or unknown record, or a capacity limit stops the pass, names the reason and recovery action, and leaves the machine unchanged.
An acknowledged letter with an explicit unconfirmed hook receipt is still open; a legacy acknowledgement with no receipt and no answer wait remains closed for compatibility and provides no confirmation evidence, while malformed or contradictory receipts refuse the pass.
Every local directory below the selected HOME or registered checkout must be a real directory owned by the account and unwritable by other accounts; an alias at or above that selected root remains valid.
An outside-HOME `HIDE_STATE_DIR` or `XDG_STATE_HOME` remains supported on every system only after its entire namespace is inspected, including before a missing ledger is treated as empty; protected system directories and authenticated system aliases are allowed, while account-controlled links and directories another account can modify are refused without changing them.
A system folder others may only add entries of their own to is allowed by the sticky rule ([ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)): `/tmp` on Unix, and on Windows a drive's root such as `D:\`, where every signed-in account may make a folder; a folder made directly in that root inherits every signed-in account's right to change it and is refused, so a state folder there is made by Hide, whose private folders inherit nothing, or given the same protection.
A relocated legacy home must stay below HOME and cannot overlap the active Hide state or kit; an indexed run outside HOME needs its owning checkout registered before inspection.
On a device, the existing SSH/SFTP connection performs this read-only check against that device's registered checkouts before uploading a helper, creating its folders or changing its current link.
It shares the local ledger and run predicates, needs no remote interpreter, and stops at the existing 15-second SSH operation deadline.
The kit never stops an old test server by name without an ownership receipt.
Before the transition, the operator checks earlier test servers and refuses automatic cleanup of any whose ownership cannot be confirmed.
After that check succeeds, the ordered steps stop the old daemon, unload its LaunchAgent and delete the plist, remove only Hide-owned command and plugin links, remove the kit copy, and rename the old `~/.hide/hcoord` and `~/.hcoord` folders to siblings ending in `.retired-YYYY-MM-DD`.
When Herdr's server is unavailable, the link step strictly inspects its owned `plugins.json` registry at the platform layer's pinned Herdr config location: `$XDG_CONFIG_HOME/herdr`, otherwise `~/.config/herdr` on Unix or `%APPDATA%\herdr` on Windows.
A missing registry or absent coordination entry needs no server or Herdr command; a registered entry is removed through Herdr's offline `plugin uninstall`, then its absence is checked again.
Foreign plugin entries and locally linked source remain intact; an unreadable, untrusted or oversized registry, a missing command for a registered entry, or unconfirmed removal keeps the link step failed for retry.
A socket confirmed absent, or an owned socket that refuses a connection, is already stopped; a missing socket does not need its name validated by the local transport.
A linked, foreign or unreadable endpoint is refused, and other connection failures remain visible at the stop step.
The old ledger is never imported into Hide or deleted.
There is no compatibility command and no rollback.

The private `~/.hide/kit/coordination-retirement.json` receipt records completion or the failed step.
An interrupted pass retries idempotently at the next kit pass or Reinstall, with completed removals staying removed.
A failed retirement step stays visible while the ordinary kit parts continue their pass; a preflight refusal stops the whole pass before it changes anything.
A completed stage does no more work, and the operator decides when a later release removes this stage after every device has passed it.
Retirement is tested only with private homes, fixture ledgers and injected service control.

### When a kit pass stops before changing anything

A preflight refusal, like the kit's own folder or lock being unavailable, leaves nothing installed by that pass, so on a new install the `hide` command, the agents' hooks and letters are all missing until it passes.
It reaches the operator once, as what stopped it and what to change, never as a reason copied onto every part (`hide_kit::KitRefusal`): a folder or file another account can change or a link in the way (`unsafe`), an earlier Hide's open work or a record Hide cannot read, its own kit record (`~/.hide/kit/installed.json`) among them, unreadable or written by a newer build (`record`), more registered projects than one pass checks (`projects`), or another Hide changing the same account (`busy`), with the folder or file at fault; a cause a newer helper names that this build does not know reads as `other` and says only to update Hide.
On a Mac no pass has answered for, the first-run wizard opens on its agents step with that line, `Try again` and `Not now`; on an install set up before, and on a device, the line sits under the machine in Settings > Devices with `Retry` (UI_BEHAVIOR.md, First run and Settings: each machine's install kit).
`Try again` and `Retry` run the same pass again, and a read while Settings is open runs the same check, so the line goes only once nothing stops the pass; `Not now` closes the wizard with the agent question unanswered, and it opens again for a different refusal, when Hide's daemon starts again, or with the question once a pass runs.
The kit's own words go to the diagnostic log when a refusal appears or changes (`kit.refused`, and `refused` on `apply.completed`).
To recover:

- A state folder chosen by `HIDE_STATE_DIR` or `XDG_STATE_HOME`, refused on any system (on Windows a folder below a drive root such as `D:\` whose permissions let every signed-in account change it; on macOS and Linux one that is group- or world-writable, another account's, or reached through an account's link): the line names the variable; set it to a folder only your account can change, or unset it so the state returns to `~/.hide/state`.
  The app hands the `hide connect` it runs its own environment, that connect starts `hided` on the folder the environment chooses, and `hided` keeps that folder until it stops: quitting the app leaves it running as long as its Herdr server answers, a launch whose environment chooses the same folder attaches to it again, and one that chooses another folder starts a second daemon beside it.
  So on the machine that runs the app the line also names the command that stops the daemon on the refused folder, with that folder and the app's own `hide` written out (`HIDE_STATE_DIR='<folder>' '<app>/hide' stop`, or the PowerShell form on Windows); quit Hide, run it, change the variable where the app's launch reads it, and open Hide again.
  A device's line names no command: its helper reads the variable when it starts there.
- Where the app's launch reads the variable:
  - macOS: an app opened from the Dock, Finder or Spotlight gets launchd's environment, not a shell profile's, so `launchctl setenv` and `launchctl unsetenv` change it for the next launch; an app opened with `open` from a terminal gets that terminal's variables.
  - Windows: an app started from the Start menu gets the account's environment variables (System Properties or `setx`), which Explorer passes to every app it starts after the change.
  - Linux: an app started from the desktop's menu gets the session's environment, set at login (`~/.profile`, `environment.d`), so a change there reaches it at the next login; one started from a terminal gets that terminal's.
  - `hide connect` run from a terminal reads that terminal's variables.
- Any other folder or file it names: make it a real folder of your account that no other account can change, then press `Try again` or `Retry`.
- Open work of an earlier Hide: finish or close it, then try again.
- The kit record: one a newer Hide wrote is that build's, so update Hide; one no Hide can read can be moved aside, and the next pass starts a new record and asks the agent choice again.
- Too many projects: remove the ones you no longer use, then try again.

`hide connect` and `hide open` do not print the refusal: the pass runs after the connection, on its own worker and for up to three minutes, so waiting for it would delay every window, and each way to use Hide after them, the desktop window or a browser tab, which shows the wizard's agents step alone, shows the line.

### Where Hide keeps its files

Everything Hide owns on a machine is under `~/.hide`:

| Folder | What it holds |
| --- | --- |
| `~/.hide/state` | The daemon's state: registered projects, screen layout, labels, phone pairing, the retired session search index (`session-search.sqlite3`, read only for each project's record retention and never written), the link record (`links.sqlite3`), Project Memory (`project-memory.sqlite3`, which only the core opens; the agent hooks ask the core for it), what the node last measured of each repository's worktrees (`worktree-facts/`), logs (`HIDE_STATE_DIR` or a set `XDG_STATE_HOME` choose another folder) |
| `~/.hide/kit` | The kit record and one-release retirement receipt |
| `~/.hide/agent-hooks` | The hook helper's per-pane counters, each with the hook version, agent and session that wrote it (hided reads them to put a pane's tokens back after a Herdr handoff or restart), the last report, the spawn guard's and the project guardrails' log (`spawn-guard.log`, capped, one older copy) and the lock beside it, and the project guardrails' policy (`guardrails.json`), which the core writes for this machine and the hooks read |
| `~/.hide/host-helper` | On a device: the `hided` builds that serve it as a node |
| `~/.hide/ai` | Hide AI's folder for agy: `antigravity-read-only`, the tool-free folder a request runs in, and `antigravity-logs`, the logs of the last agy children of each kind Hide started ([AI_PROVIDERS.md](AI_PROVIDERS.md#antigravity)) |
| `~/.hide/shell-integration` | The zsh and bash startup files a tab Hide makes for an agent starts its shell with, which read your own startup files and then mark the first prompt; one folder per build, another build's removed once unused for an hour |

Every folder Hide makes there is private to your account (0700; on Windows an access list naming only your account).
The daemon makes its state folder private each time it starts: a folder of your account that an earlier build left open, as a device's helper left `~/.hide/state` at 0755, is closed and the log records `state.made_private`; a state folder that is a link or belongs to another account is refused, no daemon starts on it, and `hide connect` answers `start_failed` naming the folder.

Outside it stay only what another program reads at a place it chose: each agent's hook entry, plugin, extension and skill stub ([agent-hooks.md](agent-hooks.md#what-is-written-and-where)), the `hide` link in `~/.local/bin` (`hide.cmd` and its junction on Windows), and on Windows the Start menu entry and notification registration.
macOS's own places (`~/Library/Application Support/hide-desktop`, `~/Library/Application Support/hide` with the AI settings), the Home folder `~/hide`, and the label generator lock beside the Herdr socket are not moved.

A build from before this layout kept the same files in `~/.local/state/hide`, `~/.local/share/hide`; the retired coordination folders are handled by the stage above.
The first launch of a newer app moves them once:

- the state folder is renamed to `~/.hide/state` as a whole after the daemon running from it is stopped; a `~/.hide/state` that already exists is used, the old folder is left untouched, and the daemon's log records `state.legacy_left` with both paths;
- `~/.local/state/hide-plugin-upgrade`, `~/.local/share/hide/agent-context-labels`, and then `~/.local/share/hide` if it is empty, are removed; `~/.local/state` and `~/.local/share` stay.

A build that names this machine by its own id rather than `local` converts the state folder once, when its daemon first starts (see [ARCHITECTURE.md](ARCHITECTURE.md#the-cores-node-id)).
The files it rewrites are copied first to `~/.hide/state/node-migration-backup/<time>/`, the Project Memory among them, except the session search index, which is rebuilt from the session files; `~/.hide/state/node.json` records the machine the folder belongs to.
An older app started on a converted folder reads this Mac's projects as another device's; to go back, stop the daemon and copy the backup's files over the converted ones.
A state folder whose `node.json` names another machine is not started, nor is one with a store that cannot be converted: the daemon stops, the app's window names the file, and the log records the reason with both machines.
A Mac whose machine id changed (a new Mac set up from this one, or a reinstall that changes it) reads its own old folder as another machine's; until a core can read another node's state, start it with a fresh state folder and keep the old one.


Settings > Devices shows each of these on This Mac's row, with where it is or why it is not.
A part you remove by hand stays removed; Reinstall on that row puts it back.
Each agent Hide knows also has a switch on that row, and Claude Code and Codex are on until you turn them off.
An agent that is on gets a `hide-browser` skill stub in the folder it reads skills from, and, where its documentation supports one, a `SessionStart` hook entry; every other agent is off until you switch it on, and an agent that is not installed on the machine has no switch there.
An agent is installed when Hide finds its program where your terminal would: on your login shell's `PATH` and in the usual install folders; a folder the agent creates does not count.
Switching an agent off takes out only the entries and stubs Hide wrote, and [agent-hooks.md](agent-hooks.md#other-agents-skill-and-guidance-hook) lists every agent, its skill folder, and why a hook is or is not written for it.
A `hided` run from neither the installed app nor a build Hide placed in `~/.hide/host-helper` (which a moved core runs from), such as a development build, installs nothing on this Mac and says so there.
Agent labels are made by hide's own daemon, so there is no labels part to install.
A machine that still has the old agent labels Herdr plugin, `hide.agent-context-labels`, loses it on the first kit pass after an update: the plugin is unlinked from Herdr (a copy installed from GitHub through `herdr plugin uninstall`), any watcher process holding its lock is stopped, and its copy under `~/.hide/kit/plugins/` and its state folder `~/.local/state/hide.agent-context-labels` are removed.
On this Mac the first launch reads that state folder once, before it is removed, so a pane whose session the plugin had already labeled keeps that label.
Herdr itself, your agent sessions, Herdr's own plugin configuration folder and your `config.toml` are not touched.
If a part cannot be taken out, for example because Herdr is not running, it stays, the diagnostic log records `plugin.retire_incomplete`, and the next kit pass tries again.

## Connect another Mac over SSH

hide can show and drive a Herdr server on another machine.
Open Settings, choose Devices, and add the machine with a label and the alias `~/.ssh/config` already knows it by; that alias is the only thing hide stores about it.
Authentication stays with SSH: the alias's `IdentityFile`, or the running SSH agent, is what hide signs in with, and hide never asks for or keeps a password.

The remote machine needs Herdr installed where a non-login shell finds it (`~/.local/bin`, Homebrew, or the system paths) and a running `herdr server` the first time; hide does not install Herdr or start a server there.
hide asks that machine `herdr status server --json` to learn where the server socket is, so nothing about the remote user or home directory is configured on this side.
After that the machine's Herdr follows this app: when its system and processor are ones the app carries (an Apple Silicon Mac from the Mac app, a Linux x64 machine from the Linux package), each connection puts this app's pinned Herdr in `~/.hide/host-helper/herdr`, once per Herdr version, and moves the Herdr server on the registered socket onto it when that server runs an older Herdr release, with its panes and the agents in them still running.
A server on another socket, one already on that version or newer, and a preview build are left as they are, and the `herdr` you installed there is never changed.
Hide's own Herdr calls on that machine use the pinned copy, and an older copy is removed once no server there runs it.
When the server cannot be moved, the machine's row says its Herdr is older than Hide's and what to do: restart Herdr there when its work is safe, and Hide moves it on the next connection, or update that machine's Herdr when it is too old to hand its panes over; a move that failed is not tried again for the same server.
Adding the machine installs the same kit a first launch installs here, under that account's home, with the form listing each part and where it goes: Hide's helper and every part's files in `~/.hide/host-helper`, the `hide` link in `~/.local/bin`, the Claude Code and Codex hook entries.
A machine allowed for the older `~/.local/share/hide/host-helper` is not asked again: its next connection installs in `~/.hide/host-helper`, re-points the `hide` link and the hook entries there, and then removes the old helper folder, the old `~/.local/state/hide/workspace-bridges`, and `~/.local/share/hide` and `~/.local/state/hide` when they are left empty.
Once the helper runs from `~/.hide/host-helper`, a later step that fails (a hook entry or a link) shows on that machine's row while the old helper folder is kept as long as anything still names it, and the next connection finishes it; removing the old folders is retried on each connection and logged, not shown.
Every connection brings the kit up to this Hide's version, and a part you removed there stays removed until Reinstall on that machine's row.
A pane on that machine can then run `hide file open`, `hide diff open` or `hide browser open http://localhost:3000`, and the result opens in this Hide, with `localhost` meaning that machine; that shell's `PATH` has to include `~/.local/bin` for a bare `hide` to be found.
Its agent panes show labels and Workspace guidance as panes on this Mac do, with the labels made on this Mac from the device's conversations, which the helper reads and sends in memory only (this Mac keeps on disk only the session proof, the hashes of the turns already analyzed and the label the model wrote, so a restart neither asks the model about them again nor loses the label, and no request, reply or path of the conversation).
A device's session gets its Project's Memory from this Mac's store through the device's helper: the index of cards when the session starts and the cards that fit each prompt, while the device is connected and Hide's hook is installed there.
`hide memory` in a device's pane asks that same store, so its answers are this Mac's: `memory_off` until you turn Memory on for the Project in Hide.
The store stays on this Mac and is never copied to the device.
A machine allowed by an earlier version of Hide gets the whole kit on its next connection without asking again; a machine added without the helper installs nothing until you press Allow and install on its row.
Only machines on the platform this build carries get the kit; another platform shows why on its row and is only viewed and driven.
Removing a machine while it is connected takes Hide's hook entries, the `hide` link and the helper folder `~/.hide/host-helper` off it, and leaves `~/.hide/kit` and `~/.hide/agent-hooks`; removing it while it is not connected leaves them there, where they do no harm, and adding it again replaces them.
When a Herdr server on that machine still runs the Herdr copy Hide moved it to, the helper folder stays with that copy and the removal says so; delete `~/.hide/host-helper` there once that server has stopped.
Each device row in Settings shows whether the remote session is connected and, when it is not, the reason in the words the connection failed with; `Test` runs the SSH, authentication, Herdr, protocol, PTY, SFTP, and Git stages one after another and lists the first one that needs attention on that host.

## Move the core to another Mac

The core is the part of `hided` that keeps your projects, layout, labels, letters, Factories and phone pairing; the app's window and Herdr's panes stay where they are.
It can run on an always-on Mac you connected over SSH, so agents and phones keep working while this Mac sleeps.
Open Settings, choose Devices, open that machine's `⋯` menu and choose **Move the core to this device…**; the item is dimmed until the machine is connected.

Before anything changes, the move checks both machines and lists every check that fails, each with what to do about it; hide does not sign in, start a session or change power settings for you.
The machine taking the core needs:

- the same device you connected, running this app's build of `hided` (connecting installs it) and a running Herdr server;
- `gh` signed in, because the core asks GitHub from its own machine;
- someone logged in to its desktop, since the core runs as a login item in that session (turn on automatic login if the machine restarts unattended);
- sleep turned off on power (`sudo pmset -c sleep 0`);
- no core and no other move in its `~/.hide/state`, Hide AI settings that are absent or the same as this Mac's, and the agent Hide AI asks signed in there.

This Mac needs that device connected, no other machine linked to this core, its Herdr server running and Hide AI set up; an agent of this Mac asleep with its pane closed has to be woken first.

The move stops the core here, copies its state to the other machine and starts it there; the window keeps its address, refuses input for those seconds and then reconnects as a window of that core.
Phones scan the core machine's QR once again in Settings > Mobile.
If a step fails, the core starts again here and the window shows which step failed.
If the other machine stops answering after it may have taken the core, the window waits for it and offers **Stop waiting**: this Mac's core starts again as it was, and once this Mac reaches the other machine it stops that machine's core of the move and sets its state aside rather than merging it.
If the other machine is only cut off, not gone, its core keeps running until this Mac reaches it.
Moving the core back checks this Mac the same way, including that the agent the core's Hide AI asks is signed in here; if the core's machine does not answer once the copy is here, the window offers **Take it here**, and this Mac's core starts from that copy while the other machine's core stays stopped until this Mac reaches it and retires it.
A move back whose copy has not reached this Mac yet only waits: while the core's machine never returns, this Mac runs no core.
A move back that fails waits the same way until the core's machine says its core runs again, keeping the copy it placed here; when that machine answers but cannot start its core, the window shows Waiting with the reason, and the move back goes on once that reason is fixed.
A machine the core moved back from runs no hided afterwards: its `core-handover.json` says the core retired there, so every start of its state folder, by the login item, a window or `hide connect`, ends at once and successfully.
From then on this Mac is a node of that core: its panes, files and terminals stay here, and the rail marks the core's machine with a crown.
Hide keeps its kit current on both machines as before: on the core's machine from the build the move placed in `~/.hide/host-helper`, at every start of its core, and on this Mac from this app, every time it links to the core; each machine's row in Settings > Devices shows its kit with Reinstall, and Settings > Agents lists both.
This Mac's own `⋯` menu moves the core back, or disconnects from the core: while disconnected the core refuses this Mac's panes, files and browser, and **Reconnect** in the window's strip restores them.

What the move leaves on each machine:

| Where | File | What it is |
| --- | --- | --- |
| This Mac | `~/.hide/state/core-placement.json` | Where the core runs (the SSH alias, the machine, the core's program and state folder), and `disconnected` while you disconnected from it; present only while the core is elsewhere |
| This Mac | `~/.hide/state/moved-out/<move>/` | The state this Mac held before its core moved away, the latest move's only |
| The machine giving the core | `~/.hide/state/core-move.json` | The move's record, written before each step that cannot be undone; a move cut short resumes or undoes itself from it at the next start |
| The machine giving the core | `~/.hide/state/core-move-abandoned.json` | The moves you stopped waiting for whose other machine has not been reached yet to stop or retire its core, at most 8; present only while one is left |
| The core machine | `~/.hide/state/core-handover.json` | Whether its core waits for the move's first link, runs, or stopped or retired for a move back |
| The core machine | `~/Library/LaunchAgents/dev.withhide.core.plist` | The login item that runs `hided core-login` at login and keeps the core running |
| The core machine | `~/.hide/state/Logs/core.log` | What the login item's core prints, set aside as `core.log.1` once it passes 1 MiB, replacing the one before; the running core looks at its size every minute and goes on in a new file |
| The core machine | `~/.hide/state/move-incoming/` | A copy being received; empty once the move ends |
| The core machine | `~/.hide/state/moved-out/<move>/` | The state of the core of a move you stopped waiting for, set aside there when this Mac reached it |

Hide AI's settings move with the core and come back with it.

## Update

A packaged release of hide updates itself from [with-hide/hide's releases](https://github.com/with-hide/hide/releases).
It looks shortly after it opens and every six hours after, takes the highest release whose files for this system are all uploaded, and downloads it in the background.
Nothing shows while it looks or downloads.
When the update is ready, a dot appears on the `?` button at the bottom of the sidebar, and the last line of the Help menu reads `hide <version> ready · Restart`.

- **Restart** in the Help menu, or **Restart now** in Settings > About, closes the window, installs the update and opens the new version.
- Without that, macOS and Windows install it the next time you quit hide, and the next launch is the new version.
- Linux installs it only when you choose Restart, after the system asks for your password; cancel the prompt and the update stays ready.

Settings > About shows the update row under the hide version: up to date with when it last looked and **Check now**, looking, downloading with its progress, ready with **Restart now**, or that it could not look or download with **Check again**; the reason is in the app's log.
**Check for updates…** in the app menu on macOS, or the Help menu on Windows and Linux, opens that row and looks at once.
A download whose SHA-512 digest does not match the release's update file is discarded and nothing is installed; on macOS the update must also carry the same Developer ID signature as the app.
Windows and Linux releases are unsigned, so there the digest in the release's update file is the only check of where an update came from.

The **Get preview versions (rc)** switch in About, off at first, also takes release candidates (`v<version>-rc.<n>`) from the next check on.
Turning it off never installs a lower version: an app on a release candidate waits for the next stable release above it.
A development build (a version that is not a release number, such as `0.4.0-main.<commit>`), an app run from a source checkout, and a Linux app not installed from the deb or rpm never look for updates, and About says updates are off.

The window is the only thing an update closes.
The core and the agents keep running; the new app replaces a `hided` still running from the previous build with one of its own build, and a `hided` that will not stop makes the app show its start failure instead of attaching to the old one.
On macOS and Linux it also moves the Herdr server the previous app started onto its own Herdr, with the panes and the agents in them still running, so nothing keeps running from the previous app's folder; a Herdr terminal you attached yourself is asked to reconnect, and a Herdr server you started from your own Herdr install is left as it is.
On Windows the server keeps running from its runtime copy, which the installer never touches, until you stop it ([What each system supports](#what-each-system-supports)).
State under `~/.hide/state` and the desktop profile (`~/Library/Application Support/hide-desktop` on macOS, `%APPDATA%\hide-desktop` on Windows, `~/.config/hide-desktop` on Linux) both persist across an update.
Files the previous Swift app left under `~/Library/Application Support/hide/` are not read by the current app, apart from `state.json`'s shortcut bindings, which are imported once, and can be deleted by hand.
Each packaged launch replaces an outdated part of the kit on this machine, and each device connection does the same there, hook entries included; a part you removed stays removed.
A connected device whose system the package carries also gets the app's pinned Herdr, and its Herdr server moves onto it when it runs an older release ([Connect another Mac](#connect-another-mac-over-ssh)); you do nothing on the device.
When the core runs on another machine, a newer app updates it to its own build before it connects, once per connection, while agents keep running; if that fails, the window shows both builds with Retry, and the core goes on with its previous build.
An updated app on the core's own machine updates the core the login item runs in the same way, and an older app there is told to update.

### When an update goes wrong

A published release is never changed; a bad one is replaced by a higher version, which the app takes at its next check or at **Check now**.
If the app does not open after an update, install the previous release from the Releases page by hand, as in [Install a release](#install-a-release); `~/.hide` and the desktop profile stay as they are.
The app never installs a lower version by itself.
An install cut off partway is finished by installing again:

- macOS: drag `hide` from the disk image to Applications again.
- Windows: run the release's `setup.exe` again; it replaces what is there and keeps your state.
- Linux: run `sudo dpkg --configure -a`, then `sudo apt install ./hide-v<version>-linux-x64.deb` again; on an rpm system, `sudo dnf reinstall ./hide-v<version>-linux-x64.rpm`.

## Uninstall

Quit hide and move `/Applications/hide.app` to the Trash.
This removes the application but keeps `~/.hide` and the desktop profile, `~/Library/Application Support/hide-desktop`.
If the core runs on another machine, move it back first; its login item otherwise keeps running there until you remove it with `launchctl bootout gui/$(id -u)/dev.withhide.core` and delete `~/Library/LaunchAgents/dev.withhide.core.plist` on that machine.
Delete both directories only when you deliberately want to reset hide's saved state.
Each agent's piece stays and does nothing once the app is gone ([agent-hooks.md](agent-hooks.md#what-is-written-and-where) lists them): delete the Claude Code, Codex and Cursor entries whose command carries `hide-subagents@`, Grok's `~/.grok/hooks/hide.json`, the OpenCode plugin and the Pi and omp extensions named `hide`, and the `hide-browser` skill stubs; Codex's trust records stay in `~/.codex/config.toml` and match only Hide's own command, the folder trust Hide recorded for its own worktrees stays in `~/.claude.json`, `~/.codex/config.toml` and agy's `settings.json` and names only those folders, and a Herdr integration Hide installed stays until `herdr integration uninstall <agent>`.
`~/.local/bin/hide` stays as well until you remove it.
On Windows, remove hide from Settings > Apps.
That removes the app, its Start menu entry and its Apps entry, and stops `hided`; the Herdr server and the agents in its panes keep running, so no work is ended without you.
Stop them yourself when you are done with them: `herdr server stop`, run from the newest folder under `%USERPROFILE%\.hide\runtime`, ends the server and every pane in it.
Then delete `%USERPROFILE%\.hide\runtime`, `%USERPROFILE%\.local\bin\hide.cmd` and the `.hide-kit` junction beside it (the junction itself, not the directory it leads to), and the `HKCU\Software\Classes\CLSID` key whose default value is `Electron Notification Activator` and whose `LocalServer32` names hide's `hide.exe`.
On Linux, run `hide stop`, then `sudo apt remove hide` or `sudo dnf remove hide`, and remove `~/.local/bin/hide`.
On every system the agents' pieces and `~/.hide` remain until you delete them.

### Earlier builds and the transition

An older build reads only the old places, so opened after the move it starts empty.
The coordination transition has no rollback; the preserved old folders remain available for the operator to inspect or delete.

## Troubleshooting

### The window shows a connection failure instead of the app

The desktop host shows `Starting Hide…` while it is connecting, and a failure page with `Retry` and `Copy diagnostics` (the app's build, the reason code and the host log's place) when it cannot reach the daemon: `cli_missing` when no `hide` executable was found, `start_failed` when the CLI could not start it, `no_response` when it started but never answered, and `state_refused`, with the file's path under the reason, when the daemon stopped on a stored file it could not make this machine's.
Read the host log at `<profile>/logs/desktop.log` (the profile is `~/Library/Application Support/hide-desktop`, or `HIDE_DESKTOP_USER_DATA_DIR`) for the detail behind whichever reason the page shows.

### No Herdr session

If the app connects but no workspace, terminal, or agent can start, no `herdr server` is running on the socket hided uses.
The packaged app starts one on launch, Retry, a second launch or a Dock click; the host log records `herdr.server_started`, or `herdr.status_failed` or `herdr.server_start_failed` with the reason it could not.
A development host, or a launch with `HERDR_BIN_PATH` set as an explicit override, starts none; run that Herdr once to start its server.
The daemon logs `herdr_bin.missing` when it cannot find a herdr binary at all: check `HERDR_BIN_PATH`, then that `herdr` runs from a new terminal of the same account, whose login shell's `PATH` the daemon searches too.
When `HERDR_BIN_PATH` names a file that no longer exists, `hide connect` and the daemon refuse to start and print that path: the Herdr server was started from an app bundle that has since been replaced, and every pane it opens still carries the old path.
The packaged app replaces that pane value with its own bundled `herdr`, but the `hide` CLI run directly in such a pane, and an unpackaged development host, still inherit it; unset `HERDR_BIN_PATH` there, or hand the server off to the new bundle's `herdr`.

<!-- herdr-provenance:start -->
hide distributes the [upstream Herdr release v0.9.5](https://github.com/herdrdev/herdr/releases/tag/v0.9.5).
The bundled binary's code is not modified by hide; a macOS release replaces only its ad-hoc signature with hide's Developer ID one, which notarization requires.
The daily `herdr-update.yml` workflow proposes upstream stable releases with `--repo herdrdev/herdr`; updates must pass contract and runtime checks.
<!-- herdr-provenance:end -->

### An agent cannot start

Run `command -v claude` or `command -v codex` in your login shell and complete that CLI's own sign-in flow.
hide never substitutes the other agent when the selected executable is missing.

### Remote workspaces do not connect

Confirm the SSH host works outside hide first, with the same alias the device was added with:

```sh
ssh <host-alias> true
ssh <host-alias> 'PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH" herdr status server --json'
```

The second command is what hide runs to find the remote socket.
`command not found` means Herdr is not installed there or not on a non-login shell's `PATH`; `"running":false` means the server is stopped, and running `herdr` once on that machine starts it.
The device row in Settings carries the same answer, and `Test` names the stage that failed.
hide delegates authentication to the existing SSH configuration and agent.
It does not display, copy, or store a password.
