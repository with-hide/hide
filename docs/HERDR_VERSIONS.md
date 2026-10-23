# Herdr versions

This page is the overview of which Herdr runs under Hide and what each version of an operator's own Herdr gets.
It states the policy in one place and points at the sections that own each rule; when a detail here and an owning section differ, the owning section is right and this page is fixed.

## One server, one protocol

Hide and the operator's own `herdr` share one Herdr server, on the socket Herdr itself uses ([INSTALL.md: First launch](INSTALL.md#first-launch)).
There is never a second, Hide-only server: a separate socket was studied on 2026-10-23 and dropped, because Herdr shares its configuration folder across sockets and a named session is still stopped and updated by the operator's `herdr update`.
"Hide's Herdr" is a program file, not a server: the binary the app carries, pinned in `contracts/herdr-bundle.json` and nowhere else ([ARCHITECTURE.md: The bundled Herdr runtime](ARCHITECTURE.md#the-bundled-herdr-runtime)).
Hide starts a server from that file only when none answers on the socket; otherwise it uses the server that is there.

Hide speaks exactly one Herdr protocol, the pin's (`HERDR_PROTOCOL_REVISION`, [ARCHITECTURE.md: The Herdr wire boundary](ARCHITECTURE.md#the-herdr-wire-boundary)).
There is no supported range and no floor: a server on another protocol refuses Hide's requests, so Hide holds its whole screen until that server is moved, restarted or caught up.
Herdr's own CLI checks the protocol the same way: a `herdr` command on another protocol than the server is refused (measured for `agent list`, `pane read`, `agent prompt` and `workspace list`), and Herdr keeps a few recovery commands such as `server stop` working across protocols.

The protocol each release answers in `ping`, measured on isolated servers on 2026-10-23: 0.5.12 is 7, 0.6.2 is 10, 0.6.3 to 0.6.5 are 11, 0.6.10 is 13, 0.7.5 is 17, 0.8.2 is 20, and 0.9.4 and 0.9.5 are 22.
Herdr's release notes do not state the protocol, so a bump is read from the pinned binary's schema (`scripts/check-herdr-contract.sh`), not from the notes.

## What each server gets

| The server on the socket | What Hide does | Owner |
| --- | --- | --- |
| None | Starts one from its own Herdr; nothing is asked | [ARCHITECTURE.md: The bundled Herdr runtime](ARCHITECTURE.md#the-bundled-herdr-runtime) |
| Hide's version | Uses it | same |
| Another version on Hide's protocol, older | Uses it; the first-run wizard offers the move with `Not now` | [UI_BEHAVIOR.md: First run](UI_BEHAVIOR.md#first-run) |
| Another version on Hide's protocol, newer | Uses it and changes nothing | [ARCHITECTURE.md: The operator's own older server](ARCHITECTURE.md#the-operators-own-older-server) |
| An older protocol, and the server can hand off (0.6.3 and later, macOS and Linux) | Holds the screen and offers Move: the server hands its panes to Hide's Herdr live, so running agents and commands go on | [UI_BEHAVIOR.md: First run](UI_BEHAVIOR.md#first-run) |
| An older protocol that cannot hand off (0.6.2 and earlier, and every older server on Windows) | Holds the screen and offers Restart: workspaces, tabs, panes and their folders come back, running commands end | same |
| A newer protocol | Holds the screen, touches nothing, and offers the update check; Hide catches up with a release | same |
| A server an earlier Hide app started | Hands it to this app's Herdr on connect without asking (macOS and Linux) | [ARCHITECTURE.md: The bundled Herdr runtime](ARCHITECTURE.md#the-bundled-herdr-runtime) |
| A connected device's server on an older stable release | Handed to the pinned copy on connect without asking; the device row says what to do when it cannot be | [INSTALL.md: Connect another Mac over SSH](INSTALL.md#connect-another-mac-over-ssh) |

Measured on 2026-10-23 against real servers: Move from 0.6.3, 0.6.4, 0.6.5, 0.6.10, 0.7.5, 0.8.2 and 0.9.4; Restart from 0.5.12 and 0.6.2; 0.9.5 is the pin and needs neither.
A move loses the screen contents behind a full-screen program on every release, Herdr's handoff behaviour, while plain scrollback stays.
Windows has only the restart, and that restart has not been run against a real Windows server ([INSTALL.md: What each system supports](INSTALL.md#what-each-system-supports)).

## The terminal's `herdr` after a move

A move changes the program the server runs, not the operator's own `herdr` file, so that file can stay on the old protocol and its commands are refused.
Settings › About shows the server's version and, when the terminal's `herdr` speaks another protocol, its version with a command to copy: `install -m 755` of Hide's Herdr over that file, or `brew upgrade herdr` for a Homebrew install.
Hide never runs that command itself, because it replaces a file the operator owns.
The advice never says `herdr update --handoff`: that command always installs Herdr's latest release and hands the server to it, which moves the server past Hide's pin whenever Herdr is ahead.
Agents in Hide's panes call Herdr through `HERDR_BIN_PATH`, which names the server's own binary, so an older `herdr` on the operator's `PATH` does not break them.

## When Herdr releases

`.github/workflows/herdr-update.yml` looks for a new stable release daily and opens a pull request made by `scripts/bump-herdr.sh`, with the contract that binary reports; it never merges.
A release on the same protocol changes nothing for operators until Hide ships it.
A release on a new protocol means an operator who updates Herdr before Hide sees the held screen with the update check, so that bump is merged and released quickly.
`scripts/check-herdr-contract.sh` compares the pinned binary's `herdr api schema --json` with `contracts/herdr-api.schema.json`, and the wire types are generated from that contract in `herdr-core/src/wire.rs`.
