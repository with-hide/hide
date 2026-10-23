<p align="center">
  <img src="design/brand/hide-icon-blinking.svg" width="120" alt="hide owl app icon">
</p>

<h1 align="center">hide</h1>

<p align="center">
  <strong>Run more agents. Hold less.</strong><br>
  The agent development environment for Claude Code, Codex and whatever comes next.
</p>

<p align="center">
  <a href="https://github.com/with-hide/hide/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/with-hide/hide?label=release&color=B9FF66&labelColor=0D191B"></a>
  <a href="LICENSE"><img alt="Apache License 2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue"></a>
  <img alt="macOS, Windows and Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey">
  <a href="https://herdr.dev"><img alt="Built on Herdr" src="https://img.shields.io/badge/built%20on-Herdr-B9FF66?labelColor=0D191B"></a>
</p>

<p align="center">
  <a href="https://withhide.dev">Website</a> ·
  <a href="#install">Install</a> ·
  <a href="docs/README.md">Docs</a> ·
  <a href="https://github.com/with-hide/hide/discussions">Discussions</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="site/assets/shots/hero-light.webp">
    <img src="site/assets/shots/hero-dark.webp" alt="The hide window. Two agents that need an answer are raised above the project tree. A lead splitting the work and the agent asking to merge two pull requests run side by side, and the PRs tool lists the two ready to merge above the ones agents are still working on.">
  </picture>
</p>

Your team runs several coding agents at once, and someone has to watch them.
hide lets them work as one team: they start helpers, watch them and write to each other.
It keeps the team in one view and keeps it going after you close the laptop.
You get the decisions; the rest stays out of the way.

It works with the agent CLIs you already signed in to: **Claude Code, Codex, Antigravity, OpenCode, Grok, Pi and omp**.
No new API key, no hosted service.

## Features

### Agents start, watch and write to each other

A lead starts workers, a worker starts a helper, on this machine or another, and they write to each other in letters.
Mix vendors freely: one agent writes, another checks.
A child runs in its own tab and reports to its parent, not to you, and the tree shows who started whom on every machine.
The agent that started a child watches it: it hears within a minute when the child stops on a question, and after 20 quiet minutes when it stalls.
Claude Code and Codex can end a turn and wait; the letter they wait for wakes them.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="site/assets/shots/agents-light.webp">
    <img src="site/assets/shots/agents-dark.webp" alt="The hide window with the lead's tree open. The sidebar shows the lead with Codex workers and OpenCode helpers under it, and a popover from the lead's pane lists the helpers it started, one of them asking.">
  </picture>
</p>

### Only what needs you comes up

Hide AI reads what each agent did and gives it a title and a one-line summary.
Agents sort themselves into four groups, **Needs You, Unread, Working and Seen**, and the ones waiting on you sit on top of every screen.
Hide AI runs on the agent CLI you already use.
Each task keeps a record of what it is doing now and what you asked, ticked off as it goes.

### One core, any screen

Put hide's core on a machine that never sleeps.
Projects, letters and titles live there; your laptop becomes a screen, so closing it stops nothing.
Add any machine by its SSH alias and its agents join the same groups, and pair your phone by QR to answer the one that needs you.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="site/assets/shots/core-light.webp">
    <img src="site/assets/shots/core-dark.webp" alt="Settings in hide. Devices lists the core machine, a laptop that dials in and a Linux server, all connected. Mobile shows Tailscale ready and one paired phone.">
  </picture>
</p>

### Still an IDE

- **Real terminals.** Agents run in Herdr panes with their real layout, scrollback and focus, drawn in place.
- **Files beside the work.** A path an agent names opens next to it, in an editor with a VS Code style Explorer.
- **Diffs and pull requests.** Each checkout's changes are one click from its tab, and the PRs tool sorts pull requests by who acts next.
- **A browser your agents can drive.** `hide browser open <url>` shows a page beside the agent, which can click, fill and screenshot it.
- **Keyboard first.** `⌘K` goes to any agent, session, issue or pull request, and `⌘P` opens any file.

### Around the agents

- **Project Memory.** What one session decided reaches the next agent, and a card that disagrees with your docs comes to you as a question.
- **Backlog.** GitHub issues and local notes sit in one list; starting one opens a worktree, an agent and its first prompt.
- **Pull requests.** The ones waiting on you come first, and one click hands a pull request to an agent.

> [!NOTE]
> **Software Factory is coming.** Queue issues with clear requirements; workers take them in order, verify and merge them, and bring back only what needs a person.
> Email [hello@withhide.dev](mailto:hello@withhide.dev) to join the list, or watch this repository's releases.

## Install

Download the package for your system from the [latest release](https://github.com/with-hide/hide/releases/latest):

| System | Package | First launch |
| --- | --- | --- |
| macOS 14+, Apple Silicon | `hide-v<version>-macos-arm64.dmg` | Drag hide to Applications; it opens like any app |
| Windows x64 | `hide-v<version>-windows-x64-setup.exe` | Installs for you only; unsigned, so SmartScreen asks once: **More info**, **Run anyway** |
| Debian, Ubuntu (x64) | `hide-v<version>-linux-x64.deb` | `sudo apt install ./hide-v<version>-linux-x64.deb`, then hide is in the app menu |
| Fedora and other rpm systems (x64) | `hide-v<version>-linux-x64.rpm` | `sudo dnf install ./hide-v<version>-linux-x64.rpm`, then hide is in the app menu |

Check the installer against its `.sha256` file before you open it.
After that one install, hide updates itself.
Install and sign in to the agent CLIs you want hide to start; hide never asks for their credentials.
hide brings its own [Herdr](https://herdr.dev), and when a Herdr server is already running it opens on top of it as it is.

[Install hide](docs/INSTALL.md) covers checksums, Gatekeeper, each system's steps, updates, uninstalling and troubleshooting, and [What each system supports](docs/INSTALL.md#what-each-system-supports) records where each system stands.

<details>
<summary><strong>Build from source</strong></summary>

<br>

You need Rust with 2024 edition support, Node.js 22.12.0 or later and pnpm 10.

```sh
git clone https://github.com/with-hide/hide.git
cd hide
pnpm install --frozen-lockfile
HIDE_VERSION=0.0.0-local pnpm --dir desktop package
```

On macOS this writes an ad-hoc signed `hide.app` under `desktop/out/`, and the dmg, the zip the updater reads, their SHA-256 files and the update file under `desktop/out/release/`; install the app with:

```sh
sudo /usr/bin/ditto --rsrc --extattr --qtn desktop/out/hide-darwin-arm64/hide.app /Applications/hide.app
open /Applications/hide.app
```

`0.0.0-local` marks a local build; set the version you mean when packaging any other.
Windows and Linux source builds are in [Build and install from source](docs/INSTALL.md#build-and-install-from-source).

To look at hide in a browser without installing the app, `bash scripts/web-dev.sh` builds the daemon, starts it with the pinned Herdr and opens the address ([Try hide in a browser](docs/INSTALL.md#try-hide-in-a-browser)).

</details>

## Your machines, your sign-ins

- **Nothing hosted.** The app, the core and Hide AI run on machines you own, so your code goes only where your agent CLIs already send it.
- **Credentials stay with their tools.** Agent CLIs, SSH and Herdr keep their own sign-in state.
  The one thing hide reads is Codex's `auth.json` access token and account ID, which it sends to Codex's usage endpoint to show your usage.
- **Your Herdr is left alone.** A Herdr server that answers is used as it is and never stopped or replaced; when none answers, hide starts its bundled one, which keeps running after the app quits.
- **Your phone stays in your tailnet.** It reaches your machines over your own Tailscale.

## How it works

```text
Herdr server ─ panes, terminals and agent lifecycle
     │  versioned snapshots + terminal output
     ▼
herdr-core (Rust) ─ the one owner of hide's state
     │  typed events in, snapshot deltas out
     ▼
hided ─ the daemon and the `hide` CLI: loopback HTTP + a token WebSocket
     │
     ▼
web shell (React) ─ in the Electron app, or a browser on the same machine
```

The core owns the state behind one snapshot boundary, and the shell only renders it and sends user events back, so the screen never keeps a competing model of the session.
[ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the boundaries and their reasons.

### Built on Herdr

<!-- herdr-provenance:start -->
hide distributes the [upstream Herdr release v0.9.5](https://github.com/herdrdev/herdr/releases/tag/v0.9.5).
The bundled binary's code is not modified by hide; a macOS release replaces only its ad-hoc signature with hide's Developer ID one, which notarization requires.
The daily `herdr-update.yml` workflow proposes upstream stable releases with `--repo herdrdev/herdr`; updates must pass contract and runtime checks.
<!-- herdr-provenance:end -->

## Performance

On an M4 Pro Mac, the 2026-10-04 candidate measured **16.3 ms p95** from a renderer keydown to a complete nine-character echo in the terminal buffer, and its whole process tree held **549 MiB** idle and **622 MiB** with three busy terminals (RSS p95).
These are one machine's numbers on macOS, taken without quiescing it: they leave out key-to-photon latency and Windows and Linux, and no other app ran the same workload, so they claim nothing about relative speed.
[PERFORMANCE_RESULTS.md](docs/PERFORMANCE_RESULTS.md) has the method, the distributions and every limit.

## Documentation

| Guide | What it covers |
| --- | --- |
| [Install](docs/INSTALL.md) | Releases, source builds, devices, moving the core, updates and troubleshooting |
| [UI behavior](docs/UI_BEHAVIOR.md) | What every screen does |
| [Status model](docs/status-model.md) | How an agent's group and read state are decided |
| [Agent hooks](docs/agent-hooks.md) | What hide writes for each agent, and where |
| [Browser displays](docs/BROWSER_DISPLAYS.md) | `hide browser` and the pages drawn beside the work |
| [Architecture](docs/ARCHITECTURE.md) | The core, the daemon, the desktop host and the Herdr boundary |
| [All docs](docs/README.md) | Every current guide and what owns it |

## Contributing

hide is built in the open.
Report a bug you reproduced with the [bug form](https://github.com/with-hide/hide/issues/new?template=bug.yml) and bring ideas to [Discussions](https://github.com/with-hide/hide/discussions/categories/ideas).
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
| `herdr-core/` | The platform-neutral Rust runtime that owns hide's state |
| `hide-platform/` | What differs between macOS, Windows and Linux, written once |
| `hide-agent-hooks/` | The only code that writes an agent's configuration file |
| `hide-ai/` | Background AI features on the operator's own agent CLIs |
| `hide-kit/` | The install kit and every path under `~/.hide` |
| `hide-*/` | The other supporting crates: sessions, memory, devices, projects |
| `plugins/` | Herdr plugins shipped from this repository |
| `contracts/` | The pinned Herdr API contract |
| `design/` | The Pen design library and `tokens.json` |
| `site/` | [withhide.dev](https://withhide.dev) |

</details>

## Community and support

- **Questions:** [Discussions Q&A](https://github.com/with-hide/hide/discussions/categories/q-a)
- **Ideas and feature requests:** [Discussions Ideas](https://github.com/with-hide/hide/discussions/categories/ideas)
- **Bugs you reproduced:** [the bug form](https://github.com/with-hide/hide/issues/new?template=bug.yml); issues are only for reproduced bugs
- **Security:** report privately as [SECURITY.md](SECURITY.md) describes, never in a public issue
- **Teams and everything else:** [hello@withhide.dev](mailto:hello@withhide.dev)

## License

hide is licensed under the [Apache License 2.0](LICENSE); [NOTICE](NOTICE) carries its copyright notice.
The bundled Herdr and other third-party components keep their own licenses, listed in [THIRD_PARTY_NOTICES](desktop/resources/THIRD_PARTY_NOTICES).
