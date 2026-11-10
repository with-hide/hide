<p align="center">
  <img src="design/brand/hide-icon-blinking.svg" width="120" alt="hide owl app icon">
</p>

<h1 align="center">hide</h1>

<p align="center">
  <strong>Run more agents. Hold less.</strong><br>
  The multi-agent IDE for Claude Code, Codex and whatever comes next.
</p>

<p align="center">
  <a href="https://github.com/with-hide/hide/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/with-hide/hide?label=release&color=B9FF66&labelColor=0D191B"></a>
  <a href="LICENSE"><img alt="Functional Source License 1.1" src="https://img.shields.io/badge/license-FSL--1.1--ALv2-blue"></a>
  <img alt="macOS, Windows and Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey">
  <a href="https://discord.gg/h6zme5kybD"><img alt="Join the hide Discord" src="https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white"></a>
</p>

<p align="center">
  <a href="https://withhide.dev"><strong>Download</strong></a> ·
  <a href="https://withhide.dev">Website</a> ·
  <a href="docs/README.md">Docs</a> ·
  <a href="https://discord.gg/h6zme5kybD">Discord</a> ·
  <a href="https://github.com/with-hide/hide/discussions">Discussions</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src="site/assets/video/kick-conductor.gif" width="960" alt="A Conductor plans a six-step mission, then each step flies out to a worker on This Mac, linux-box or mac-mini: Claude Code, Codex, Antigravity and Grok. One agent asks a question, you answer it, and every step ends in a pull request.">
</p>

You run several coding agents at once, and someone has to keep track of them.
hide puts every agent, task and pull request in one view, lets the agents start, watch and write to each other, and brings up only what needs a person.
Close the laptop and the work keeps going.

It works with the agent CLIs you already signed in to: no new API key, no hosted service.

<p align="center">
  <kbd>Claude Code</kbd> <kbd>Codex</kbd> <kbd>Antigravity</kbd> <kbd>OpenCode</kbd> <kbd>Grok</kbd> <kbd>Pi</kbd> <kbd>omp</kbd>
</p>

## Install

[withhide.dev](https://withhide.dev) picks the installer for your system, or download it from the [latest release](https://github.com/with-hide/hide/releases/latest):

| System | Package | First launch |
| --- | --- | --- |
| macOS 14+, Apple Silicon | `hide-v<version>-macos-arm64.dmg` | Drag hide to Applications; it opens like any app |
| Windows x64 | `hide-v<version>-windows-x64-setup.exe` | Installs for you only; unsigned, so SmartScreen asks once: **More info**, **Run anyway** |
| Debian, Ubuntu (x64) | `hide-v<version>-linux-x64.deb` | `sudo apt install ./hide-v<version>-linux-x64.deb`, then hide is in the app menu |
| Fedora and other rpm systems (x64) | `hide-v<version>-linux-x64.rpm` | `sudo dnf install ./hide-v<version>-linux-x64.rpm`, then hide is in the app menu |

Check the installer against its `.sha256` file before you open it.
After that one install, hide updates itself.
Install and sign in to the agent CLIs you want hide to start; hide never asks for their credentials.
hide runs its own terminals: each pane lives in a small holder process that keeps running across an app restart.

[Install hide](docs/INSTALL.md) covers checksums, Gatekeeper, each system's steps, updates, uninstalling and troubleshooting, and [What each system supports](docs/INSTALL.md#what-each-system-supports) records where each system stands.

<details>
<summary><strong>Build from source</strong></summary>

<br>

You need Rust with 2024 edition support, Node.js 22.12.0 or later and pnpm 10.

```sh
git clone https://github.com/with-hide/hide.git
cd hide
pnpm install --frozen-lockfile
pnpm --dir desktop package
```

On macOS this writes an ad-hoc signed `hide.app` under `desktop/out/`, and the dmg, the zip the updater reads, their SHA-256 files and the update file under `desktop/out/release/`; install the app with:

```sh
sudo /usr/bin/ditto --rsrc --extattr --qtn desktop/out/hide-darwin-arm64/hide.app /Applications/hide.app
open /Applications/hide.app
```

A local build is named after the newest release before it and its commit, such as `1.1.1-main.6afb9543`; leave `HIDE_VERSION` unset, since it is the release workflow's switch.
Windows and Linux source builds are in [Build and install from source](docs/INSTALL.md#build-and-install-from-source).

To look at hide in a browser without installing the app, `bash scripts/web-dev.sh` builds the daemon, starts it and opens the address ([Try hide in a browser](docs/INSTALL.md#try-hide-in-a-browser)).

</details>

## Features

<table>
<tr>
<td width="46%" valign="middle">

### Hide AI does the reading

Hide AI reads what each agent did and writes what a person needs: a title, why it stopped, what you asked and what you missed.
Agents sort themselves into **Needs You, Unread, Working and Seen**, and the ones waiting on you sit on top of every screen.

Hide AI runs on the agent CLI you already signed in to, and every status still works with it switched off.

</td>
<td width="54%">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="site/assets/shots/rows-on-dark.webp">
  <img src="site/assets/shots/rows-on-light.webp" width="100%" alt="The Tasks list with Hide AI on: Approve #431 and #436? and #433: CSV or XLSX for the export? under Needs You, a fixed test under Unread, the release notes and a starred task under Revisit, and the agents still working.">
</picture>
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### Every agent at a glance

The sidebar shows the same work three ways: **Projects** as a tree of checkouts and the agents in them, **Tasks** grouped by who acts next, and **Pull requests** sorted by what waits on you.

The Overview draws the whole night as one map: which agent started which, the letters between them, and who is waiting on you.

</td>
<td width="54%">
<img src="site/assets/video/kick-focus.gif" width="100%" alt="The Overview map of 36 agents across four projects narrows to My turn: two agents wait on you, Approve #431 and #436? and Retry failed payments.">
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### Agents start, watch and write to each other

A lead starts workers, a worker starts a helper, on this machine or another, and they write to each other in letters.
Mix vendors freely: one agent writes, another checks.

The agent that started a child watches it: it hears within a minute when the child stops on a question, and after 20 quiet minutes when it stalls.
For a set of pull requests, a Conductor agent runs a **mission**: it plans the work, starts the workers and reports back.

</td>
<td width="54%">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="site/assets/shots/record-lane-dark.webp">
  <img src="site/assets/shots/record-lane-light.webp" width="100%" alt="A session's record: working, one question waiting on you, its requests grouped as Plan the export and Check each pull request, and the pull requests its workers opened.">
</picture>
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### Memory every agent shares

**Project Memory** turns what you decide in a session into a card, checks it against the repository and hands it to the next agent, whichever vendor runs it.
Tell Codex once on Monday; Antigravity already knows on Thursday.

A rule worth keeping comes back as one pull request to your docs.

</td>
<td width="54%">
<img src="site/assets/video/kick-memory.gif" width="100%" alt="Two fresh agents on Thursday: one starts from zero and has to be told again; Antigravity in hide starts with the card Use pnpm, never npm, said to Codex on Monday.">
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### Your agent drives the browser

An agent opens the page it changed beside its pane, clicks through it and reads what it shows, so it checks its own work before it asks you.
A file or diff it names opens next to it too.

```sh
hide browser open localhost:5173/invoices
hide browser click browser-1 --text "Export CSV"
hide file open src/export/csv.ts
```

</td>
<td width="54%">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="site/assets/shots/agent-click-dark.webp">
  <img src="site/assets/shots/agent-click-light.webp" width="100%" alt="A Codex pane ran hide browser click browser-1 --text Export CSV beside the invoices page it opened, downloaded 128 invoices, and reports that the export matches the spec.">
</picture>
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### Pull requests, waiting on you first

The ones waiting on you come first, with their checks; the ones an agent is still working on sit below.
One click hands a pull request to an agent.

</td>
<td width="54%">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="site/assets/shots/prs-list-dark.webp">
  <img src="site/assets/shots/prs-list-light.webp" width="100%" alt="The sidebar's PR list: #431 and #436 under Needs You, ready to merge with checks passed; #438 and #432 under Agent is working, one with failed checks.">
</picture>
</td>
</tr>
<tr>
<td width="46%" valign="middle">

### One core, any screen

Put hide's core on a machine that never sleeps.
Projects, letters and titles live there; your laptop becomes a screen, so closing it stops nothing.

Add any machine by its SSH alias and its agents join the same lists, and pair your phone by QR to answer the one that needs you, over your own Tailscale.

</td>
<td width="54%" align="center">
<img src="site/assets/owls/owl-devices.svg" width="70%" alt="The hide owl with a laptop, the core and a phone">
</td>
</tr>
</table>

### One agent, several accounts

Give an agent more than one account: a work and a personal Claude Code, or a model gateway behind a wrapper program.
Each profile is its own account folder with hide's hooks installed there; pick one in the start panel, per project, for a mission's workers or for Hide AI.

> [!TIP]
> You don't have to fill in the form. Ask any agent in hide, "add a Claude Code profile for ~/.claude-work", and it sets the profile up for you.
> If you also start a wrapper by hand in a terminal, have it set the profile's folder (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`), so hide shows that start as the profile too.

### Still an IDE

- **Real terminals.** Agents run in real terminal panes with their layout, scrollback and focus, drawn in place.
- **Files and diffs.** An editor with a VS Code style Explorer, and each checkout's changes one click from its tab.
- **A worktree per task.** GitHub issues and local notes sit in one backlog; starting one opens a worktree, an agent and its first prompt.
- **Keyboard first.** `⌘K` goes to any agent, task, issue or pull request, and `⌘P` opens any file.
- **Guardrails.** Guard the base branch, working folders, files and risky Git commands, per project.
- **Limits and cost in view.** How much of each agent's week is left, and each project's last seven days at API prices.

> [!NOTE]
> **Software Factory is coming.** Queue issues with clear requirements; workers take them in order, verify and merge them, and bring back only what needs a person.
> Email [hello@withhide.dev](mailto:hello@withhide.dev) to join the list, or watch this repository's releases.

## hide, Orca and Herdr

All three run the agent CLIs you already use, side by side, across more than one machine.
hide is built for the part after that: knowing which of them needs you, and why.

| | hide | [Orca](https://github.com/stablyai/orca) | [Herdr](https://github.com/herdrdev/herdr) |
| --- | :---: | :---: | :---: |
| Form | Desktop app | Desktop app | Terminal app, one Rust binary |
| Runs the agent CLIs you signed in to | ✓ | ✓ | ✓ |
| Agents start and prompt each other | ✓ | ✓ | ✓ |
| Agents on several machines in one window | ✓ | ✓ | ✓ |
| Marks the agent that waits on you | ✓ | ✓ | ✓ |
| Writes why it stopped and what it asks, read from its session | ✓ | | |
| A parent agent is told when its child goes quiet | ✓ | | |
| Decisions carried to the next agent, whichever vendor | ✓ | | |
| Answer from your phone | ✓ | ✓ | |
| License | FSL-1.1-ALv2, Apache 2.0 after two years | MIT | Apache 2.0 |

<sub>The Orca and Herdr columns come from each project's README on 2026-11-10; an empty cell means its README does not describe the feature, not that it cannot be done. Something wrong? [Tell us](https://github.com/with-hide/hide/discussions). Parts of hide's terminal layer come from Herdr, and [NOTICE](NOTICE) names each file.</sub>

## Your agents drive hide too

hide installs a skill and hooks into each agent, so an agent can do from its pane what you do from the window.

```sh
hide agent spawn --parent here --name export-csv --intent export-csv --kind codex \
  --repo ~/billing-app --branch feat/export-csv --brief "Export invoices as CSV"   # a worker it watches
hide request send <agent> --intent review --body "Check #431 before I merge"   # a letter to another agent
hide mission help                                                     # plan and run a set of pull requests
hide device add workbox                                               # another machine, by SSH alias
```

`hide guide` prints how an agent works inside hide, and `hide --help` lists every command.

## Start with one task

1. Install hide from [withhide.dev](https://withhide.dev) or the [latest release](https://github.com/with-hide/hide/releases/latest), and sign in to the agent CLIs you use.
2. Add a project, start an agent and give it a task; ask it to split the work across a few workers.
3. Watch the sidebar's Tasks list and the Overview map, and answer only what reaches **Needs You**.

## Your machines, your sign-ins

- **Nothing hosted.** The app, the core and Hide AI run on machines you own, so your code goes only where your agent CLIs already send it.
- **Credentials stay with their tools.** Agent CLIs and SSH keep their own sign-in state.
  The one thing hide reads is Codex's `auth.json` access token and account ID, which it sends to Codex's usage endpoint to show your usage.
- **Your phone stays in your tailnet.** It reaches your machines over your own Tailscale.

## How it works

```text
pane holders ─ one per pane: its PTY, program and parsed terminal
     │  terminal output + process facts
     ▼
hide-core (Rust) ─ the one owner of hide's state, with its own engine over the holders
     │  typed events in, snapshot deltas out
     ▼
hided ─ the daemon and the `hide` CLI: loopback HTTP + a token WebSocket
     │
     ▼
web shell (React) ─ in the Electron app, or a browser on the same machine
```

The core owns the state behind one snapshot boundary, and the shell only renders it and sends user events back, so the screen never keeps a competing model of the session.
[ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the boundaries and their reasons.

## Performance

<details>
<summary><strong>What one machine measured</strong></summary>

<br>

On an M4 Pro Mac, the 2026-10-04 candidate measured **16.3 ms p95** from a renderer keydown to a complete nine-character echo in the terminal buffer, and its whole process tree held **549 MiB** idle and **622 MiB** with three busy terminals (RSS p95).
These are one machine's numbers on macOS, taken without quiescing it: they leave out key-to-photon latency and Windows and Linux, and no other app ran the same workload, so they claim nothing about relative speed.
The method, the distributions and every limit are kept in `docs/performance-results.md` in with-hide/hide-private.

</details>

## Documentation

| Guide | What it covers |
| --- | --- |
| [Install](docs/INSTALL.md) | Releases, source builds, devices, moving the core, updates and troubleshooting |
| [UI behavior](docs/UI_BEHAVIOR.md) | What every screen does |
| [Status model](docs/status-model.md) | How an agent's group and read state are decided |
| [Agent hooks](docs/agent-hooks.md) | What hide writes for each agent, and where |
| [Browser displays](docs/BROWSER_DISPLAYS.md) | `hide browser` and the pages drawn beside the work |
| [Architecture](docs/ARCHITECTURE.md) | The core, its engine and pane holders, the daemon and the desktop host |
| [All docs](docs/README.md) | Every current guide and what owns it |
| [llms.txt](https://withhide.dev/llms.txt) | The same guides as one index for an agent to read |

## Contributing

hide is built in the open.
Report a bug you reproduced with the [bug form](https://github.com/with-hide/hide/issues/new?template=bug.yml), ask for a feature with the [feature request form](https://github.com/with-hide/hide/issues/new?template=feature_request.yml), and bring open-ended ideas to [Discussions](https://github.com/with-hide/hide/discussions/categories/ideas).
Pull requests are open only to maintainers and the people in [`.github/APPROVED_CONTRIBUTORS`](.github/APPROVED_CONTRIBUTORS); others are closed automatically.
[CONTRIBUTING.md](CONTRIBUTING.md) owns that policy and lists the `verify` gates and their local commands, and [AGENTS.md](AGENTS.md) holds the rules every change follows.

```sh
pnpm install --frozen-lockfile
bash scripts/verify-cargo.sh test   # Rust suites
bash scripts/verify-web.sh          # web and desktop typecheck, lint, test and build
pnpm --dir desktop dev              # the desktop app, unpackaged, against this checkout's build
```

<details>
<summary><strong>Repository map</strong></summary>

<br>

| Path | What lives there |
| --- | --- |
| `desktop/` | The Electron desktop host, the app that ships |
| `web/` | The React web shell |
| `hided/` | The daemon and the `hide` CLI |
| `hide-core/` | The platform-neutral Rust runtime that owns hide's state |
| `hide-platform/` | What differs between macOS, Windows and Linux, written once |
| `hide-agent-hooks/` | The only code that writes an agent's configuration file |
| `hide-ai/` | Background AI features on the operator's own agent CLIs |
| `hide-kit/` | The install kit and every path under `~/.hide` |
| `hide-*/` | The other supporting crates: sessions, memory, devices, projects |
| `contracts/` | The daemon's wire schema and the other pinned contracts |
| `design/` | The Pen design library and `tokens.json` |
| `site/` | [withhide.dev](https://withhide.dev) |

</details>

## Community and support

- **Chat with the team and other users:** [Discord](https://discord.gg/h6zme5kybD)
- **Questions:** [Discussions Q&A](https://github.com/with-hide/hide/discussions/categories/q-a)
- **Feature requests:** [the feature request form](https://github.com/with-hide/hide/issues/new?template=feature_request.yml)
- **Open-ended ideas:** [Discussions Ideas](https://github.com/with-hide/hide/discussions/categories/ideas)
- **Bugs you reproduced:** [the bug form](https://github.com/with-hide/hide/issues/new?template=bug.yml)
- **Security:** report privately as [SECURITY.md](SECURITY.md) describes, never in a public issue
- **Teams and everything else:** [hello@withhide.dev](mailto:hello@withhide.dev)

## License

hide is fair source under the [Functional Source License 1.1, ALv2 Future License](LICENSE) (FSL-1.1-ALv2).
You may use, change and redistribute it for any purpose except offering others a commercial product or service that competes with hide, and using it in your own work, including at a company, is always allowed.
Each release becomes available under the Apache License 2.0 two years after it is published, and releases up to and including 1.1.1 remain under the Apache License 2.0.
[NOTICE](NOTICE) carries the copyright notice and the files derived from other projects, which keep their own licenses.
Third-party components keep their own licenses, listed in [THIRD_PARTY_NOTICES](desktop/resources/THIRD_PARTY_NOTICES).
