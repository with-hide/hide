# Dev Runtime: Which App Is Actually Running

The single biggest time sink so far is verifying a change against the wrong process.
Read this before running or screenshotting the app.
For responsiveness, rendering, CPU, or memory checks, also read [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md) in full.

## One instance, per profile

`hide` is the Electron desktop host (`desktop/`); three kinds of instance can exist on this machine:

- an installed packaged bundle, normally at `/Applications/hide.app`, built and ad-hoc signed by `pnpm --dir desktop package`
- an unpackaged instance from `pnpm --dir desktop dev`, which runs the checkout's own `electron .` rather than a bundled app
- a Playwright `_electron` instance from `pnpm --dir desktop e2e` (`desktop/e2e/fixture.ts`), which refuses to launch without its own private `HIDE_STATE_DIR`, `HOME`, `HERDR_SOCKET_PATH` and `HIDE_DESKTOP_USER_DATA_DIR`

Every instance keeps one Electron `requestSingleInstanceLock()` per profile directory (`desktop/src/main/index.ts`): a second launch that resolves to the *same* profile does not start a second process, it wakes the first one's window (`second-instance` -> `host.reopen`) and quits itself.
The profile defaults to `~/Library/Application Support/hide-desktop` for every instance, packaged or not, unless `HIDE_DESKTOP_USER_DATA_DIR` names a different directory.
That means an unpackaged `pnpm --dir desktop dev` run started while the operator's installed app is already running does not open a second window at all: it silently focuses the operator's live app and exits, because both share the default profile.
Never rely on "it opened a window" as proof that your build is the one running; always start a dev or QA instance with its own `HIDE_DESKTOP_USER_DATA_DIR`, so it cannot collide with, focus, or in any way touch the operator's instance.

Before any visual check:

```sh
pgrep -fl 'hide.app/Contents/MacOS/hide'          # a packaged instance
pgrep -fl 'electron/dist/Electron.app/Contents/MacOS/Electron'   # an unpackaged dev or e2e instance
```

Identify every matching PID and its profile before proceeding: read the host log's first line at `<profile>/logs/desktop.log` (or `$HIDE_DESKTOP_USER_DATA_DIR/logs/desktop.log` for an isolated instance), a JSON line whose `event` is `host.start` and whose fields carry `packaged` (`true` for an installed bundle, `false` for `pnpm --dir desktop dev`) and `version`.
Quit only an instance you started under your own private profile; coordinate with the operator before quitting anything running under the default profile, and never quit, restart, focus, or otherwise manipulate the operator's own instance for QA (see [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md) for the native isolation and foreground-interaction boundaries this extends to).

## Verify against a rebuilt package, not a live dev instance

A source fix is invisible to an already-running app, packaged or not, because Electron loads the bundled `desktop/dist/` once at launch.
Repeatedly "fixing" something the user still sees broken usually means they are looking at a build from before the fix.

For any change the user will confirm visually in the installed app:

```sh
pnpm --dir desktop package   # prints the new hide.app path and archive under desktop/out/
```

Reinstall that bundle (or point the operator at the new `desktop/out/release/hide-v<version>-macos-<arch>.dmg`), relaunch, and confirm with a real screenshot.
For a faster loop that does not need reinstalling anything, `pnpm --dir desktop dev` picks up a rebuilt `desktop/dist/` and this worktree's own `target/{debug,release}/hide` on its next launch; state explicitly which build (dev or packaged, and its `host.start` version) the user is looking at when reporting a fix.
The operator's own state is `~/.hide/state`; a dev daemon started under the operator's HOME without `HIDE_STATE_DIR` attaches to that state, and on its first connect moves a legacy `~/.local/state/hide` there.

## Bundled artwork ownership

The production identity is `hide` (`me.grab.hide.desktop`).
The app bundles `desktop/resources/hide.icns` as its icon (`desktop/scripts/package.mjs`).
The approved mark and its visual rules live in [BRAND.md](BRAND.md), with the full square source at `design/brand/hide-mark.png`.
The app's rounded 1024-pixel PNG, `.icns` and the Windows `hide.ico` are derivatives of that source; the `.ico` holds that PNG resized to 16, 24, 32, 48, 64, 128 and 256 pixels as PNG entries, and a Linux package carries no icon of its own; the mobile web icons under `web/public/m/` use the same mark.
Agent marks (`agent-claude.png`, `agent-codex.png`, and the other agents' logos under `web/src/assets/agents/`) live in `web/src/assets/`, drawn by the web shell itself rather than bundled as native app resources.
The interface faces, Inter 4.1 and Pretendard 1.3.9 (both OFL 1.1, unmodified), live in `web/src/assets/fonts/` and are declared in `web/src/index.css`; each notice in `desktop/resources/THIRD_PARTY_NOTICES/` names its release and SHA-256.
File and folder marks are the Material Icon Theme's SVGs (MIT), taken from the pinned `material-icon-theme` package at build time rather than copied into the repository; `web/src/fileIcons.ts` resolves a name to one.
Keep packaging aligned with `desktop/resources/THIRD_PARTY_NOTICES/`; bundled artwork is not a grant of trademark permission.
Use [UI_BEHAVIOR.md](UI_BEHAVIOR.md) for shell behavior.

## Pet, deep links and other retired native-only features

The removed native shell had a pet window, its own deep-link scheme, global shortcuts, a menu bar item and Dock badges; none of that exists in this host.
See the electron backlog issue (issue 184) for what of it, if anything, is still wanted.
