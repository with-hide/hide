# Release state folders

`generate.py` makes the state folder a published release writes, for the upgrade test `hide-core/src/node_migration/upgrade_tests.rs`, and the files its kit leaves in the device's account, for `hide-kit/src/upgrade_tests.rs`.
Every release adds its folder, and the release checklist runs it from a checkout of public `main`:

```
python3 scripts/release_state/generate.py v1.2.4
```

It needs Docker, and takes about five minutes.
It downloads the release's Linux device kit from the release's public download address for the Docker machine's own architecture (the same `hided`, `hide`, `hide-agent-hooks` and `hide-holder` the release's packages carry, checked against the release's `device-bundles.json`), so nothing runs under emulation.
The core and one SSH device it dials each run in their own container, with made-up machine ids and accounts; nothing touches this machine's `~/.hide`, `~/.claude`, the installed app or a real device.
`scenario.py` drives the old core through its `/ws` and its own CLI typed into stand-in agents (`agent.py`, started as `claude` by `claude.c`); each of its steps stores something that names the device or that a later build reads back about an agent (its launch, sleep and archive records, the project settings, the kit's record and hooks).
Both accounts start with the operator's own Claude Code settings (a permission and a Stop hook of another tool), which the kit installs beside.
The containers, network, image and work folder are removed on every exit; the old core's log stays at `agents/runs/release-state-<version>.log`.

## Where the folder goes

- `hide-core/tests/fixtures/release-state/<tag>/`: only what a start reads (the JSON stores, the SQLite stores with their logs folded in, `node.json`, the engine folder and the mission events). Logs, holders, sockets, scrollback, pane credentials, backups and pairing secrets are left behind.
- `hide-kit/tests/fixtures/release-home/<tag>/device/`: what the release's kit left in the device's account, at its place in the home: the kit's record (`.hide/kit/installed.json`), Claude Code's `settings.json` with Hide's hooks beside the operator's, and the folder trust Hide recorded for a worktree it spawned into (`.claude.json`, `.hide/agent-hooks/claude-trust.json`). The core has none: it runs outside a desktop package, whose kit installs nothing.
- `hide-core/tests/fixtures/release-state/<tag>.json`: the release's commit, the made-up ids, and each scenario step with `ok` true, false, or null when the release itself refuses the step (`why` says what it answered).

The upgrade tests pick up every folder there by themselves; nothing else is registered.
A release adds its folder in a pull request of its own, `test: add the <version> release state folder`.

## Before committing

- Run `scripts/check-no-workstation-identity.sh --scope index` after `git add`, and read its answer: it must pass with the new folder staged.
- The folders may hold only the made-up accounts `me` (core) and `remote` (device), the made-up machine ids `c0de0000000000000000000000000001` and `de510000000000000000000000000002`, the host names `core` and `device`, and the `example.invalid` git email. A path under `/Users/`, another account name, or a real machine id means the run was not isolated: do not commit it.
- Read `<tag>.json`: a step that is false where the previous release's is true is a scenario that broke, and is fixed before the folder is committed.
- Run the upgrade tests: `scripts/verify-cargo.sh nextest -p hide-core --lib -E 'test(/upgrade_tests/)'` and `scripts/verify-cargo.sh nextest -p hide-kit --lib -E 'test(/upgrade_tests/)'`.

## When a folder comes out

A folder stays while an operator can still upgrade from its release to the current build.
It comes out in the pull request that removes the migration code a start needs for it (for example the conversion of a folder written before node ids, or the renaming of a device registered under its SSH alias), never only to save space.

## When the scenario changes

A change that stores a device or machine id in a store adds a step to `scenario.py` that makes the release write it, in the same pull request (`docs/TESTING.md`, Upgrading a released state folder).
Releases that predate the feature record the step as not done; the release that ships it adds the first folder that carries it.
When the field already exists in released builds, the change regenerates their folders with the new step.
