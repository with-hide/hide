# Build output and worktrees

This document owns where build output goes and why: every build inside the worktree that asked for it, the release binaries at their fixed path, a debug build kept small, one Rust version named by the repository, and the machine's toolchain reused rather than reinstalled.
The scripts named here are the executable authority; `scripts/tests/test_toolchain_reuse.py`, `test_rust_toolchain_pin.py`, `test_ci_gate_portability.py`, `test_verification_builds.py` and `test_debug_build_size.py` assert the parts a workflow depends on.

## One rule: build output lives in the worktree

Cargo writes to `target/`, the web shell to `web/node_modules/` and `web/dist/`, and the desktop app to `desktop/node_modules/`, `desktop/dist/` and `desktop/out/`, all at their default locations inside the checkout and all ignored by Git.
Nothing a checkout builds is written anywhere else, so `git worktree remove` is the whole cleanup, and no cache can outlive the work that produced it.

Two facts make the default location the only correct one.

Cargo names a workspace member's artifacts by its path relative to the workspace root, so two checkouts sharing one target directory read each other's build as fresh and run the other checkout's test binary.
Cargo also holds an exclusive lock on the directory, so sharing serializes the parallel builds the worktree layout exists to allow.

The release binaries are `target/release/{hided,hide,hide-agent-hooks}`, and `desktop/scripts/package.mjs` reads every one of them from that fixed relative path.
Redirecting a release build with `CARGO_TARGET_DIR`, `--target-dir` or `--build-path` leaves the packager reading a path nothing wrote.
`scripts/verify-cargo.sh` therefore sets `CARGO_TARGET_DIR` to the worktree's own `target/` on every invocation, whatever the caller carried.

The cost of this layout is one full build cache per worktree, which is why a worktree is removed when its branch lands rather than kept around.

A release `hided` carries `web/dist` inside the binary: `hided/build.rs` embeds every file under it when the profile is `release` and fails the build when `web/dist/index.html` is missing, so a release build is always `pnpm --dir web build` first, then `cargo build --release --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks` (`scripts/verify-cargo.sh release`).
`desktop/scripts/package.mjs` performs that build before staging the release binaries into the app bundle, so a packaged app's SessionStart probe has a matching `hide-agent-hooks` executable even without a separate CLI on `PATH`.
A debug `hided` embeds nothing and reads `web/dist` from disk at run time (`HIDED_UI_DIR` overrides the lookup), so a rebuilt web shell shows up without a cargo rebuild.
The web measurement (`MEASURE_SCENARIO=multi HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure/<attempt> bash scripts/web-shell-measure/run.sh`, `docs/PERFORMANCE_TESTING.md`) needs that release `hided` at `target/release/hided` inside the worktree it measures; it is never redirected, for the same reason as the release binaries.
On 2026-09-09 one abandoned worktree held 5.3 GB, over half of the 10 GB across all eight.
An earlier design sent test builds to `/tmp/hide-verify-<uid>/<hash>/`, keyed by checkout path so a run judging the tree would not dirty it; the reason had lapsed once the harness scored only tracked files, and the caches it left behind reached 7.5 GB with no worktree owning them.
Hide's own merged-worktree cleanup had grown a step that forked `bash` to compute and delete that path, which this layout makes unnecessary.

`[profile.dev] incremental = false` in the workspace manifest is deliberate, not a leftover.
An agent worktree is built a few times and discarded, which never repays an incremental cache; what it does instead is grow one per worktree, and those had reached 1.5 GB.
Debug output is what makes a stale worktree expensive, because nothing strips it: a debug `hided` measures about 165 MB, with no debug info, against 24 MB for the release binary.
The SSH transport's crates (`russh`, `russh-sftp` and the ciphers and hashes it runs) are built at `opt-level = 3` in dev builds too: a development daemon installs its own debug `hided` on a device over SSH, and unoptimized those crates moved about 4 MiB a second, so the remote mailbox lane's device was not ready within its bound; optimized, its install takes about 6 seconds.

## A debug build is kept small

A worktree's `target/` is paid once per worktree, so its size multiplies by the number of parallel worktrees; on 2026-10-08 five of them held 2 to 10 GB each, and the disk ran out under the builds of the others.
Three causes made most of a debug build, and each has a rule below.
`scripts/verify-cargo.sh hakari` in the rust lane and `scripts/tests/test_debug_build_size.py` in the policy lane fail a change that brings one back.

The measurement started each time from an empty `target/` in a clone of `main` at `6472cb38`, on an Apple silicon Mac with the toolchain `rust-toolchain.toml` pins.
It ran the commands one worktree's work runs, in order: `cargo test --locked --workspace --no-run`, `cargo build --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks` (`verify-cargo.sh cli`), `cargo test --locked -p herdr-core --no-run`, `cargo test --locked -p hide-platform -p hide-herdr-client --no-run` and `cargo clippy --locked --workspace --all-targets`, and read `du -sm target` after each.
Each row adds its change to the row above it.

| Build | After the workspace tests | After all five | Compile time, all five |
| --- | --- | --- | --- |
| Before | 4,968 MB | 8,086 MB | 173 s |
| One feature set per dependency | 4,946 MB | 7,023 MB | 148 s |
| No debug info | 3,095 MB | 4,192 MB | 136 s |
| No member feature only tests turn on | 3,095 MB | 3,419 MB | 93 s |
| One test binary per crate | 2,400 MB | 2,710 MB | 83 s |

Before, the four commands after the first added 3.1 GB, nearly all of it crates built again for a different selection of members; now they add 310 MB, which is Clippy's own metadata for every crate and the small subtree of the two crates the OS contract lanes build alone.
The workspace lists the same 3,326 tests (17 ignored) before and after.

### Debug builds carry no debug info

`[profile.dev] debug = false` covers the workspace's crates and every dependency.
Debug info was most of what a debug build wrote: 214 of the 331 MB of object code in the 20 largest dependency libraries was DWARF, herdr-core's library was half DWARF even at `line-tables-only`, and on macOS any debug info keeps every object file a binary linked beside it in `target/debug/deps`, because the debugger reads the DWARF from there (2.2 GB of the build above).
A panic still prints its file, line and column, which are compiled into the binary as data rather than read from debug info; a backtrace names its functions without lines.
For a debugger or a backtrace with lines, turn debug info on for one build, `CARGO_PROFILE_DEV_DEBUG=line-tables-only cargo test -p <crate>` (`true` for variables too); every crate builds again under another hash beside the default build, so that worktree pays for both until it is removed.
The change invalidates CI's cached dependency builds once; the first run on `main` after it rebuilds and saves them.

### One feature set per dependency

Cargo resolves a dependency's features from the members a command selects, so `-p herdr-core`, `--workspace` and `--bins` each built tokio, hyper and serde_json with a different set, under a different hash, and every crate above them again beside the first copy.
`workspace-hack` is a member with no code whose manifest turns on, for each third-party crate, the union of the features any member asks of it; every other member depends on it, so every command resolves one set.
[cargo-hakari](https://docs.rs/cargo-hakari) writes it from `.config/hakari.toml` for the systems the workspace is built and packaged for.
Cargo's own `resolver.feature-unification = "workspace"` does the same without a crate, but it is unstable in the pinned toolchain ([rust-lang/cargo#14774](https://github.com/rust-lang/cargo/issues/14774)); once it is stable it replaces `workspace-hack`.

After adding or changing a dependency or its features, run `cargo hakari generate` and `cargo hakari manage-deps --yes` and commit what they write; `bash scripts/install-hakari.sh` installs the pinned release.
It writes each version as a semver range rather than the members' exact pins, since `Cargo.lock` decides the version either way, so a patch or minor bump such as Dependabot's weekly group leaves it unchanged; a major bump or a changed feature set needs the two commands.
The rust lane runs `scripts/verify-cargo.sh hakari`, which fails when either command would change a file.
`hide-platform` and `hide-herdr-client` stay off the shared set (`final-excludes`): the OS contract lanes build and test exactly those two, which would otherwise compile tokio, hyper and schemars for nothing.
Their own dependencies still count toward the set, and a command that selects them alone builds their small subtree once more.

hakari cannot unify a member's own features, so no member has a feature only a test build turns on.
`hide-host`'s `call-log` and `hide-node`'s `test-support` were enabled by other members' dev-dependencies, which built those crates and every crate above them twice, once for a test build and once for a `--bins` build.
The git call log is now a recorder the core's tests turn on at run time (`hide_host::worktrees::record_git_calls`), which records nothing in a running daemon, and `RemoteHost::detached` is always compiled.
`test_debug_build_size.py` refuses a dev-dependency that sets a member's features.

### One integration test binary per crate

Cargo builds every `tests/*.rs` file as a binary of its own, and each one links the crate and all its dependencies: there were 56, and each of hided's thirteen took up to 177 MB.
A crate's integration tests are modules of one binary, `tests/it/main.rs`, which declares each module and the fixtures they share (`hided/tests/it/support/`), so a new test file is a new `mod` line there.
A module's tests are selected by name: `bash scripts/verify-cargo.sh test-scoped -p hided --test it handshake::` locally, and `-E 'binary_id(hided::it) & test(/^handshake::/)'` under nextest.
A test that runs its own binary again to play a role names that test with its module, as in `--exact process::child_role`.
`cargo test` runs a binary's tests as threads of one process, so the modules now share process-wide state they did not share before: every hide-ai test that sets a `FAKE_*` variable, or starts a fake that reads one, holds `FAKE_ENV` from `hide-ai/tests/it/main.rs`.
nextest still runs each test in a process of its own.
`test_debug_build_size.py` refuses a `tests/*.rs` file.

## The toolchain is never copied

`scripts/toolchain-env.sh` is sourced by every script that calls cargo, and it resolves `CARGO_HOME` and `RUSTUP_HOME` from the cargo shim's own location so an isolated HOME reuses the machine's installed toolchain.

Without it a verification runner pays for a whole toolchain and keeps it.
rustup reads `RUSTUP_HOME` with a default of `$HOME/.rustup`, and a runner HOME makes that an empty directory; rustup does not fail there, it downloads and installs into it and reports the fact as a warning while exiting 0.
That exit 0 is why two earlier workarounds never ran: each had diagnosed the missing toolchain correctly, and each guarded its recovery behind a cargo invocation failing.
The cost was 1.3 GB of `.rustup` plus 128 MB of `.cargo` per run, 9.1 GB across eleven run directories, duplicating a toolchain already on the machine.

## One toolchain version

`rust-toolchain.toml` at the repository root names the exact Rust version, with rustfmt and Clippy, and every build uses it: each CI job on Linux, macOS and Windows, and every workstation checkout.
rustup reads the file on the first `rustc` or `cargo` call and installs that version if it is missing, so no workflow has a toolchain step and no runner image's own Rust builds anything.
Before the file existed, CI used whatever stable the runner image carried; on 2026-10-06 the ubuntu image moved to a new stable that deprecated one method and added a Clippy finding, and code already on `main` failed `-D warnings` in every pull request that touched Rust, and in `main` itself, with no change in this repository.

The version is written only in that file; `scripts/tests/test_rust_toolchain_pin.py` fails when a workflow, script or document restates it, or when a script reaches a toolchain by its directory rather than through rustup, which would build with another version.
`scripts/verify-cargo.sh` prints `rustc --version` and `cargo --version` on stderr before it runs cargo, so each CI job's log names the toolchain that built; the list of installed toolchains `rust-cache` prints does not say which one ran.
A verification run under its own HOME still sources `scripts/toolchain-env.sh`, so rustup looks for the pinned version in the machine's `~/.rustup`: when it is installed there, nothing is downloaded; when it is not, rustup installs it there once, never under the runner HOME.

Moving to a new version is a pull request that changes the file together with whatever the new compiler and Clippy ask of the code, proven by that pull request's CI; a finding is fixed, not silenced with `allow`.
Dependabot proposes that pull request (`.github/dependabot.yml`), checking every Monday for a newer stable release; whoever takes it up pushes the fixes to it.

## Two entrypoints

`scripts/verify-cargo.sh` and `scripts/verify-web.sh` are the only way a check script or the PRD harness builds this repository.
The Cargo entrypoint uses the portable `C` locale for its subprocesses, so an unavailable caller locale cannot crash system helpers before a test starts.
`scripts/tests/test_verification_builds.py` exercises this boundary with the real system Perl under a caller's `LANG=C.UTF-8`.
The harness runs a verify command with no shell, so an `ENV=value cargo ...` binding fails with ENOENT at verify time, when a sealed run can no longer be amended; a script is the only place that environment decision can live.
A check script calls these scripts rather than cargo or pnpm directly, so the target directory and toolchain decisions are made once.

| Binding | Command | Output and proof |
| --- | --- | --- |
| test | `bash scripts/verify-cargo.sh test` | `target/debug`; locked workspace tests |
| lint | `bash scripts/verify-cargo.sh lint` | `cargo fmt --check` then `cargo clippy -D warnings` over every target |
| hakari | `bash scripts/verify-cargo.sh hakari` | nothing written; fails when `workspace-hack` is out of date ([One feature set per dependency](#one-feature-set-per-dependency)) |
| release | `bash scripts/verify-cargo.sh release` | `target/release/{hided,hide,hide-agent-hooks}`, the binaries the desktop packager ships |
| cli | `bash scripts/verify-cargo.sh cli` | `target/debug/{hide,hided,hide-agent-hooks}` for isolated CLI, daemon, SessionStart and install kit checks |
| web | `bash scripts/verify-web.sh` | `pnpm install --frozen-lockfile`, then typecheck, lint, test and build for both `web` and `desktop`: `web/dist` and `desktop/dist` |

The Cargo `test` mode forwards trailing test arguments, so an explicitly configured live probe can run as `verify-cargo.sh test <test-name> -- --ignored` without bypassing worktree isolation or toolchain ownership.
The no-argument `test` mode remains the full locked workspace gate.
Focused delivery and session-activity filters and their nonzero-test prerequisite are listed in [delivery.md: Verification](delivery.md#verification).
Each compiler or test process's failure reaches the caller.

CI uses scoped Cargo modes `test-scoped`, `check`, `build` and `clippy` with trailing Cargo arguments, for example `bash scripts/verify-cargo.sh test-scoped -p hide-platform -p hide-herdr-client`.
Each adds `--locked`, reuses the installed toolchain, clears inherited Herdr and legacy coordination overrides, and fixes output to this worktree's `target/`.
Scoped modes accept at most 128 arguments and refuse `--target-dir`, `--manifest-path` and `--config`; an unknown mode exits 2.
The sealed `test`, `lint`, `release` and `cli` invocations keep their existing behavior.
`verify-web.sh install [--ignore-scripts]` locks dependency installation; `verify-web.sh <web|desktop> <typecheck|lint|test|build>` runs one package step.
`web build` includes its typecheck before Vite bundles, so the no-argument gate and CI's web checks run it once before lint and unit tests, with no separate web typecheck over the same inputs.
Desktop build uses esbuild without typechecking, so its separate typecheck remains required.
Standalone package `build` and `e2e` commands keep their preparation: web build typechecks, web package e2e builds the shell, and desktop e2e builds the host.
`web e2e` through this wrapper runs Playwright against the web output already built; `desktop e2e` always rebuilds the desktop host before Playwright, including a direct local invocation.
Both forward the test arguments and their exit status, so a missing test filter fails the caller.
`<web|desktop> e2e-list [args...]` only discovers tests (`playwright test --list`); it needs no app build and cannot execute tests.
The desktop CI job uses it for the complete and filtered shard lists, then uses `desktop e2e` for execution: one fresh host build per job instead of one for each list and another for execution.
There is no skip-build flag, freshness marker or reuse of a previous job's desktop output.
`playwright-install` installs Chromium for web or desktop; `desktop package` packages this runner's app.
`desktop package compile` builds only the web shell and the release binaries and records them in `desktop/out/release-binaries.json` (commit, version, build order and each binary's SHA-256); `desktop package assemble` builds the host and packages exactly those binaries, refusing a record of another commit, version or order and any binary that is missing, linked or changed.
`package.yml`'s macOS lane runs the two halves as two jobs of one run at one identity (see [Device helper release bundles](#device-helper-release-bundles)), the one place a release binary crosses a job; nothing reuses a release binary from another run or source.
An invalid package/action pair or more than 128 trailing arguments exits 2; the no-argument gate retains both packages' typecheck, lint, unit tests and build coverage.
`test_ci_verification_entrypoints.py` checks this external command boundary without building or installing; it complements the real build tests below.

In `web-e2e.yml`, a lane with several shards builds the web shell once in a job parallel to its daemon build job.
The shell build job and each shard check out the same workflow commit and use Node 22, pnpm 10 and the frozen workspace lockfile, with the same web build command and environment.
The build uploads `web/dist` as `web-dist-<lane>` with one-day retention and an error on missing output; each shard downloads that named artifact from its own workflow run into `web/dist` before Playwright.
The lane suffix separates simultaneous reusable calls, as it does for `hided-debug-<lane>`; there is no cross-run lookup or cache of built shell output.
The HTML, CSS and JavaScript need no executable bits; the daemon binaries still restore theirs after download on Unix.
The shards wait for both builds; a failed or cancelled build stops them and a missing artifact fails the download step, without rebuilding or accepting stale output.
A single-shard lane skips both shared build jobs and retains its in-place daemon and web builds, including the nightly macOS and Windows web lanes.
Every shard still installs dependencies for Playwright and test discovery; test selection and reporting remain shard-owned.

The build regression tests in `test_verification_builds.py` use a tiny real Cargo workspace, not compiler mocks.
They check that a caller's `CARGO_TARGET_DIR` cannot move the release binaries, that output stays in the checkout, warm build reuse, a core value change, a changed failing test, and compiler and prerequisite failure propagation.
They require macOS with Cargo.


The Windows/Linux package smoke uses an isolated HOME and Herdr socket to check the command from a fresh shell, both configured agent hooks and the completed retirement row.
It then uses a second complete package fixture with an executable overlay that changes the daemon hash, proving replacement, refreshed command and hook paths, same-build reuse and standalone refusal.
All test daemons and temporary files are cleaned up on failure as well as success.

## Product analytics build input

`HIDE_POSTHOG_PROJECT_TOKEN` is an optional public ingestion token embedded into hided at build time, validated by `hided/build.rs` and listed in `hided/src/env.rs` and `hided/.env.example`.
It must start with `phc_`, contain only ASCII letters, digits or underscores, and have 20 to 128 bytes; invalid values fail the build without printing the value.
An absent or empty value produces a build with no external analytics transport.
The disable-only `~/.hide/product-analytics-disabled` marker suppresses transport even in a configured build; shared browser/desktop fixtures and package smoke homes write it on every system, without granting the privileged device fixture's service-control rights.
`package.yml` supplies the repository Actions variable of the same name on every packaging system; a personal PostHog API key must never be used.
Local builders export the variable before the build; the example file is not auto-loaded, and the running binary does not read a runtime override.
See [product usage statistics](ARCHITECTURE.md#product-usage-statistics) for the events and privacy boundary.

## The desktop app

`desktop/` is a pnpm workspace member; `pnpm install` at the root installs it with the web shell.
Electron downloads its runtime into `desktop/node_modules/electron/dist/` on the first launch rather than at install, so a lane that only typechecks never fetches it.
`desktop/scripts/package.mjs` turns off Electron's `GrantFileProtocolExtraPrivileges` fuse in the built app before it is signed (`desktop/scripts/fuses.mjs`) and reads it back from the result, and the desktop end-to-end suite runs on a copy of Electron flipped the same way, made once into `desktop/dist/electron-fused/` before its tests.
CI acquires that lock-resolved runtime once with `bash scripts/verify-web.sh desktop electron-install` before desktop or packaged-app fixtures, using the dependency's own checksum-verifying installer within a five-minute step.
An acquisition failure blocks the suite at that prerequisite and retains the upstream error instead of retrying the download in each test.

| Command | Does |
| --- | --- |
| `pnpm --dir desktop dev` | Bundles `desktop/dist/` with esbuild and launches the app unpackaged; it finds this worktree's `target/{debug,release}/hide` itself |
| `pnpm --dir desktop typecheck`, `lint`, `test` | The desktop CI lane |
| `pnpm --dir desktop e2e` | Playwright `_electron` against a private hided and pinned Herdr; needs `web/dist`, `target/debug/hide` and `hided`, and the pinned `herdr` as the web e2e does |
| `pnpm --dir desktop package` | `desktop/scripts/package.mjs`: builds the release binaries and fetches the pinned Herdr, bundles `hided`, `hide`, `hide-agent-hooks` and `herdr` into `Contents/Resources` and the notification permission helper into `Contents/MacOS/hide-notification-status`, signed under the app's identifier, signs `desktop/out/hide-darwin-<arch>/hide.app` (bundle id `me.grab.hide.desktop`), writes the release version into the app's own `package.json` (the version the app and its updater report) and puts `desktop/resources/app-update.yml`, the updater's download folder name, in its resources; `desktop/scripts/installers.mjs` then wraps the finished app with electron-builder (`--prepackaged`, so the app and its signature are what was checked) into `desktop/out/release`: `hide-v<version>-macos-<arch>.dmg` and the `.zip` the updater installs, `latest-mac.yml` and the zip's `.blockmap`, each installer beside a `.sha256` checksum; nothing is installed. The signature is ad-hoc unless `HIDE_MACOS_SIGNING=developer-id`, which the official macOS publisher sets: every executable in the bundle, Herdr included, is then signed with the Developer ID certificate under the hardened runtime with the exceptions in `desktop/resources/entitlements/`, Apple notarizes the app and the ticket is stapled, and the run fails unless Gatekeeper accepts the app expanded from the zip as notarized; the dmg is signed, notarized and stapled too, and must pass `spctl -t open`. That mode needs `HIDE_SIGN_IDENTITY`, `HIDE_NOTARY_KEY_PATH`, `HIDE_NOTARY_KEY_ID` and `HIDE_NOTARY_ISSUER`, which `desktop/scripts/macos-signing.mjs` defines and checks together before the build. On Windows x64 and Linux x64 the same command builds that system's package, unsigned: the folder `desktop/out/hide-win32-x64/` or `hide-linux-x64/` with the same binaries (`.exe` on Windows, with Herdr's `conpty/` beside `herdr.exe`) in its `resources/`, wrapped into `desktop/out/release` as the per-user NSIS installer `hide-v<version>-windows-x64-setup.exe` with `latest.yml` and its `.blockmap` (`desktop/resources/windows/installer.nsh` stops the daemon on uninstall), or as `hide-v<version>-linux-x64.deb` and `.rpm` into `/opt/hide` with one `latest-linux.yml` (`desktop/resources/linux/` holds the package scripts that link `/usr/bin/hide` to the CLI and set up the sandbox), each installer beside a `.sha256`. `node desktop/scripts/installers.mjs <app folder> <version> <out>` wraps an already packaged Windows or Linux app again under another version, which the CI update test uses for its two builds; on Linux these test builds make only the deb, its checksum and update file, while release packaging still makes both the deb and rpm. Each system packages its own desktop host, and a machine of another architecture than its host Herdr asset is refused; the Mac app also requires the foreign device helper artifacts described below |
| `node desktop/scripts/smoke-package.mjs <installed app folder>` | On Windows or Linux only: takes the folder the system's installer put the app in, prepares its runtime (on Windows the copy under the private home that `hide runtime prepare` makes), and with a private home (stand-in `claude` and `codex` programs in `~/.local/bin`, so both agents are installed) and state folder runs the bundled `herdr --version`, the device helper, `hide-agent-hooks doctor`, and `hide connect`, `/health`, the embedded shell, first-launch CLI and hooks, a command from a fresh shell, the completed retirement row, replacement by a second package fixture, refreshed paths, same-build reuse, standalone refusal and `hide stop`, then waits for each daemon pid it started to end (ten seconds, a named failure beyond that); on Windows it also checks that the next prepare removed the first build's runtime copy; `.github/workflows/package.yml` runs it after installing each package, and `desktop/scripts/smoke-windows-installer.mjs` checks the Windows installer's install, reinstall while a Herdr server runs, and uninstall. It refuses macOS, where first-launch kit behavior is covered by the isolated desktop suite |

Every build of `hide` carries the version and commit `hide version` reports (`hided/build.rs`): `HIDE_VERSION` if it was set for the build, else the crate's version, and `HIDE_COMMIT`, else for a release build the checkout's `HEAD`.
A release passes `HIDE_VERSION` from its tag; a local package leaves it unset and `package.mjs` names the build after the newest `v<version>` tag behind it and its commit, `0.4.0-main.<commit>` (`.dirty` when tracked files differ), so a local build never claims a release, even on a tagged commit, and its version is no release number the app would update from.
`package.mjs` sets both to what it ships and refuses to package when it cannot read the commit; a debug build given no `HIDE_COMMIT` reports none, because following `HEAD` would rebuild `hided` after every commit.

The app attaches to whatever daemon the environment names: without `HIDE_STATE_DIR` it is the operator's own at `~/.hide/state`.
For QA, set `HIDE_STATE_DIR`, `HOME`, `HERDR_SOCKET_PATH` and `HIDE_DESKTOP_USER_DATA_DIR` to private paths, as `desktop/e2e/fixture.ts` does and refuses to launch without.
Use `web/e2e/platform-fixture.ts` for the native executable and tool paths and for Windows `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` inside that private home.
The fixture C shim needs the runner's native compiler, `cc` on Unix or `clang.exe` on Windows, which Playwright's `globalSetup` runs once before any test (`web/e2e/shims/build.ts`); this is a fixture prerequisite, not an installed product requirement.
A packaged app does not need `hide` on `PATH`: it ships its own CLI and Herdr, and only falls back to a login-shell PATH search and the well-known install directories when its own bundled CLI is somehow missing (see `docs/ARCHITECTURE.md`, The desktop host).
macOS may refuse the unsigned app's first launch until it is opened once with Open from the context menu.

The desktop e2e runs beside the operator's own work on the same Mac (issue 232).
Every launch through `desktop/e2e/fixture.ts` passes `--hide-show-inactive`, so the host shows its window behind every other window without activating the app and a second launch never focuses it, and `--disable-backgrounding-occluded-windows`, so the window keeps painting behind the operator's and a capture by window id is current.
The fixture also preloads `desktop/e2e/focus-guard.cjs` into the app, which records into a per-test file each time this app becomes active or a window takes the keyboard, from launch to quit, and fails any test not tagged `@needs-focus` that recorded one; specs take `test` from the fixture, and `launch` refuses a test that did not.
The guard sees only this app, so a spec that could reach another program (a browser, Finder, the folder picker) stubs it, as the existing ones do.
A spec that needs the key window or native input (a page holding the keyboard, a pinch, a native drag) carries the `@needs-focus` tag (`NEEDS_FOCUS` in the fixture) and focuses the window itself, a browser page through `focusPage` once the host shows that page, which gives the page the keyboard only after the window's `focus` event says macOS made it the key window; `pnpm --dir desktop e2e --grep-invert @needs-focus` runs everything that leaves the operator's keyboard alone, and CI runs the whole suite.
The suite runs the focus tests after every other one (`desktop/playwright.config.ts` puts them in a project that depends on the rest): a focus test brings its app forward and quits it, and on a machine with no other app in front, a CI runner, macOS then activates the next app that opens, which failed every background test after the first focus test.

## Device helper release bundles

The Mac desktop host remains Apple Silicon; Linux x64 and Linux ARM64 devices download their kit on first connection.
The app carries only `Contents/Resources/device-bundles.json`, alongside its four native programs.
Foreign helper and Herdr binaries are separate release assets, never app resources.
The pinned release publishes both Linux Herdr assets; their upstream digests live only in `contracts/herdr-bundle.json`.
`scripts/bump-herdr.sh <release-tag>` verifies those assets with the host platforms, refusing a missing required asset or a changed digest of the same pinned release.
The fetch script verifies foreign bytes without executing them; matching native lanes also check the reported version.

`.github/workflows/device-helpers.yml` is the reusable producer.
Its required string inputs are `source-sha` (full public source commit), `version` (the app version, optionally prefixed with `v`) and `build-order` (decimal order, including `0` for a fixture).
It explicitly checks out the public repository.
The Linux runners build natively on `ubuntu-24.04` and `ubuntu-24.04-arm` using the existing package recipe after the web build.
No new cross-compilation toolchain is needed.
Targeted output stays in this worktree's `target/<triple>/release`; host release binaries stay at `target/release`.
Never redirect a release target directory.

| Artifact | Download into this checkout | Contents |
| --- | --- | --- |
| `hide-device-helper-linux-x86_64` | `desktop/out/device-helpers/linux-x86_64/` | `hided`, `hide`, `hide-agent-hooks`, `manifest.json` |
| `hide-device-helper-linux-aarch64` | `desktop/out/device-helpers/linux-aarch64/` | same four files |

Each producer manifest records platform, source commit, version, build order and each helper's SHA-256.
`package.yml` resolves one identity with `desktop/scripts/build-identity.mjs` and calls the producer on every call, beside the Windows and Linux packages; a [release candidate](#unsigned-release-candidates) keeps both artifacts for the private publisher.
Its macOS lane does not wait for them to compile: `compile (macos)` runs `desktop package compile` while the helpers build, and `package (macos)` needs both jobs, downloads the binaries and these artifacts from that run and runs `desktop package assemble`, so a failed helper skips the Mac app and fails the run.
A local Mac package needs the same two directories at its exact identity; the identity command prints the values to give the producer.
`desktop/scripts/device-helpers.mjs build <platform> <version> <source-sha> <build-order>` is the producer command on its matching runner after dependency installation and `rustup target add <triple>`.
Packaging refuses missing members, a different source/version/order, digest mismatches, links, files above 256 MiB and incorrect executable headers.
Downloaded artifacts need not retain executable mode bits.

Before building the native app, or before assembling it from compiled binaries, `prepareDeviceBundles` copies the helpers and pinned Herdr into private worktree staging.
Linux bytes remain unchanged; only the native Mac app follows the existing Developer ID signing and notarization path.
It hashes the final files and builds one gzip-compressed USTAR archive per platform, containing exactly four regular entries at its root: `hided`, `hide`, `hide-agent-hooks`, `herdr`.
No paths, links, extended attributes or PAX records belong in an archive.
The archive's name is `hide-device-<platform>-<app-version>.tar.gz`, without a leading `v` in the version.
The uncompressed four-file total and the compressed archive are each capped at 256 MiB.

`device-bundles.json` has `format: 1`, `version`, full `commit`, decimal string `order`, and `platforms` keyed by the two Linux platform names above.
Each platform records `asset`, archive `sha256`, archive `size` in bytes, and `files` keyed by the four program names, each with its final `sha256` and `size`.
Hashes are lowercase SHA-256 hex.
The app ships this small manifest; `desktop/out/release/` receives a copy and both archives alongside the installers and updater files.
`desktop/out/device-bundles/` keeps the intermediate output, never anything under `/tmp`.

The private publisher must take the two artifacts from the release candidate run it selected, as it takes the Linux and Windows packages and [the Mac binaries](#the-candidates-mac-build), require every helper's SHA-256 to equal the receipt's `device_helpers`, download them into the paths above, then run `desktop package assemble` with its Developer ID settings; it builds no helper and compiles no binary itself.
It must publish both `hide-device-<platform>-<app-version>.tar.gz` assets and `device-bundles.json` on the **same public `v<app-version>` release** as the Mac app.
Do not re-sign or repack an archive after the manifest is made, and do not substitute output from another source or run.
The public release validator and draft writer require 18 assets in total: the existing 15 installers, checksums, updater files and blockmaps, plus both Linux bundles and their manifest.
They verify each bundle's size and SHA-256 against the manifest; draft preparation also requires its source commit to match the release event before any GitHub request.
The private workflow is maintained outside this repository; the public producer and package run do not establish that publishing is connected or a release has shipped.

The connection worker fetches only `https://github.com/with-hide/hide/releases/download/v<app-version>/<asset>` with HTTPS, a 60-second request bound and at most five redirects.
It checks archive size and hash, then exact entry names, regular-file types, sizes, program hashes and executable headers before atomically publishing a cache slot.
The cache path belongs to `hide_kit::layout::device_bundle_cache`: `~/.hide/cache/device-bundles/`.
A five-second process lock serializes cache access; each of two platform slots holds one verified kit, and one staging folder holds the current download.
The maximum is 1024 MiB including the two 256 MiB kits, one 256 MiB staging kit and its 256 MiB compressed archive, plus the fixed lock file.
A reconnect verifies and reuses its cached bytes offline; an app update replaces mismatched slots, and a failed download leaves no usable partial kit.
Every exit explicitly cleans staging before releasing the cache lock; if cleanup fails, the device reason retains both failures and a structured diagnostic names the platform and release, while Drop makes a backup attempt.
Files in the cache are inert private data; upload gives the verified programs executable permission on the device.
Missing releases, network failures and verification errors reach the existing device failure and Retry path.
Local/main builds report that no matching device release exists; an existing developer artifact folder still takes precedence.
Physical device startup, the private publisher and official signing/notarization remain separate verification claims.

## Developer ID signing

A release's macOS app is signed with the team's Developer ID Application certificate and notarized with an App Store Connect team API key.
Both belong to the Apple Developer Program membership of the team that owns the certificate.
The account holder keeps the private key, certificate export and notary key outside the public repository.
Official signing credentials, their rotation and the CI environment that reads them are maintained privately.
Public CI prepares unsigned packages without those signing credentials.

### A local Developer ID build

On a Mac whose keychain holds the certificate with its private key, `security find-identity -v -p codesigning` lists the identity's full name, and this builds, signs and notarizes the release app:

```sh
HIDE_VERSION=<version> HIDE_MACOS_SIGNING=developer-id \
HIDE_SIGN_IDENTITY="Developer ID Application: <team name> (<team id>)" \
HIDE_NOTARY_KEY_PATH=<absolute path to the .p8> HIDE_NOTARY_KEY_ID=<key id> HIDE_NOTARY_ISSUER=<issuer id> \
pnpm --dir desktop package
```

`desktop/scripts/macos-signing.mjs` refuses before the build when a value is missing or malformed, the identity is in no keychain on the search list, or `notarytool` is not installed.
Apple usually answers within minutes; the run leaves no archive unless Apple accepts the app and Gatekeeper accepts the app expanded from the archive as notarized.
Once Apple accepts the app or the dmg, its ticket exists, so a stapler that cannot fetch it (exit 65 or 68, as when a GitHub macOS runner loses the connection to Apple's ticket service) tries again, six times over about five minutes, and the run fails after the last try; any other stapler failure fails at once.
`xcrun notarytool history` lists past submissions and `xcrun notarytool log <submission id>` says why one was refused, each with `--key`, `--key-id` and `--issuer` given the same three values.
The command installs nothing; [The Installed App Is The Operator's](../AGENTS.md#the-installed-app-is-the-operators) governs replacing `/Applications/hide.app` with what it built.

## Unsigned release candidates

`release-candidate.yml` is a manual preparation workflow on protected `main`.
It creates no tag or release and reads no signing credential.
Its four required string inputs are `public_sha` (a full lowercase commit SHA), `tag` (`vX.Y.Z` or `vX.Y.Z-rc.N`), `verification_run_id` (the `pr.yml` push/main run of that exact source, queued, running or passed), and `build_order` (a positive decimal integer no larger than 9007199254740991); `macos_runner` picks where [the Mac binaries](#the-candidates-mac-build) compile, `mini` unless the operator chooses `hosted`.
The source must already be an ancestor of main; a branch, fork, PR run or different source is refused before building, and so is a verify run that failed or still waits behind another main run (`pending`), which the next push to main replaces.
The candidate does not wait for that verify run: it may start while the source's push verify runs, and the publish, not the candidate, requires that the run it names concluded success.
The workflow revision is main's dispatch revision and may differ from the source revision it builds.
`package.yml`, `device-helpers.yml` and `agent-journey.yml` run their steps from that dispatch revision on the source's checkout, so the workflow refuses a source at which any of them differs from main; choose a newer source instead.

The workflow reuses `package.yml` for unsigned Linux and Windows installers, including their installation, packaged-window and update checks, and for both [Linux device helpers](#device-helper-release-bundles), built beside them on public runners.
Beside them it compiles the Mac app's release binaries on the Mac mini ([The candidate's Mac build](#the-candidates-mac-build)).
It runs the existing `agent-journey.yml` on the same source, with stand-ins on Linux and macOS, Windows form checks, and fake-model lanes on all three systems.
It signs no agent in: since 2026-10-27, by the operator's decision, a real login is measured only by the release's local run on the Mac ([CONTRIBUTING.md: Releases](../CONTRIBUTING.md#releases)), so the workflow summary records signed-in providers as **not measured**; package, stand-in and fake-model failures block the receipt.
A successful candidate does not establish that any provider answered.
Rerunning only the failed jobs does not make a selectable candidate: the receipt keeps the first attempt and the earlier attempt's artifacts stay beside the new ones, so a failed candidate is dispatched again with the same inputs.
The reusable workflows' optional `source-sha` defaults to the caller commit, and `package.yml`'s optional `build-order` defaults to its existing history-based ordering, so existing callers keep their behavior.
An explicit order allows separately prepared packages of one numeric version to share one build identity.
The operator assigns a strictly increasing order within that numeric version, including release-candidate to stable transitions; a private-only change takes a new patch version.

Only after packaging, both helpers, the Mac binaries and the selected journey succeed does the workflow upload `hide-release-candidate`, containing only `candidate.json`:

```json
{"schema":4,"repository":"with-hide/hide","public_sha":"<source SHA>","tag":"vX.Y.Z","build_order":"<decimal>","verification_run_id":"<decimal>","workflow_sha":"<workflow SHA>","run_id":"<decimal>","run_attempt":1,"live_agents":false,
 "device_helpers":{"linux-aarch64":{"hide":"<sha256>","hide-agent-hooks":"<sha256>","hided":"<sha256>"},"linux-x86_64":{"hide":"<sha256>","hide-agent-hooks":"<sha256>","hided":"<sha256>"}},
 "macos_binaries":{"hide":"<sha256>","hide-agent-hooks":"<sha256>","hided":"<sha256>"}}
```

The installer artifacts keep the names `hide-package-linux` and `hide-package-windows`, the helper artifacts `hide-device-helper-linux-x86_64` and `hide-device-helper-linux-aarch64`, and the Mac binaries `hide-macos-release-binaries`; candidate artifacts expire after seven days.
All receipt fields describe public facts, with IDs and order as decimal strings and `schema` and `run_attempt` as integers.
Schema 2 adds the boolean `live_agents`, which records whether provider checks were requested, not their per-cell verdicts; since 2026-10-27 it is always `false`, kept so the private consumer that requires it reads the receipt unchanged.
Schema 3 adds `device_helpers`, each Linux platform's helper digests, recorded by the receipt job only after it checked both artifacts' manifests, executable headers and digests against `public_sha`, the tag's version and `build_order` (`withDeviceHelpers` in `scripts/release-candidate.mjs`); from schema 3, `verification_run_id` names the verify run without proving it passed.
Schema 4 adds `macos_binaries`, the digests of `hided`, `hide` and `hide-agent-hooks` compiled for macOS, recorded only after the compile artifact's `release-binaries.json` named `public_sha`, the tag's version and `build_order` and every binary matched it (`withMacosBinaries`).
Consumers retaining schema 1 support must treat its missing selection as the original live-enabled workflow, never as evidence that all providers passed.
A consumer verifies the successful manual main run, its trusted workflow revision, current attempt, matching receipt and artifact inventory, and that the verify run the receipt names and its `verify` job concluded success, before consuming it; a receipt alone proves neither GitHub origin nor that a package is signed.
Run evidence and reports remain workflow artifacts, separate from distributable files.
Release approval must disclose unmeasured provider coverage; it does not waive signing, installation, update checks or version approval.
Candidate preparation creates no public tag or Release; only the official publisher creates the approved release tag and publishes the final assets.

### The candidate's Mac build

The candidate's `compile (macos)` job runs `desktop package compile` on the source and uploads `hide-macos-release-binaries`: `target/release/{hided,hide,hide-agent-hooks}` and `desktop/out/release-binaries.json`, the record [`package.mjs assemble`](#the-desktop-app) checks before it packages them.
The private publisher downloads that artifact from the candidate run, requires each binary's SHA-256 to equal the receipt's `macos_binaries`, puts them where `assemble` reads them, and assembles, signs and notarizes the app; it compiles nothing.
Waiting for a hosted macOS runner took 53 minutes at the 90th percentile of 40 pull request verify runs (October 2026), so the job runs on the operator's Mac mini, a self-hosted runner of this public repository labelled `hide-candidate`, which no other workflow names (`scripts/tests/test_release_candidate.py`).

A self-hosted runner of a public repository would run whatever a pull request's workflow asks of it, so the Mac mini takes only this job:

- **Only this workflow, on main.** The Team plan's runner groups can be limited to repositories but not to workflows, so the runner refuses the rest itself: its job-started hook (`scripts/candidate-runner/job-started.sh`) stops every job, before any of its steps runs, unless the job's repository, event, ref and workflow ref are this repository's `release-candidate.yml` dispatched on `refs/heads/main`. Failing the hook would still let the worker run `if: always()` and `if: failure()` steps and actions' `pre` steps, so the hook empties the work folder and kills the job's worker, its parent; the listener completes the job as failed and stays online. A hook sees the job's own `github` context and never a workflow's `env:`, so no pull request, fork or branch push can name those values. Pull requests from outside contributors also wait for a maintainer's approval before any workflow runs.
- **Its own account.** The runner runs as `hiderunner`, a standard account with no login and a password no one knows, from a LaunchDaemon, so it needs no one logged in; it cannot read the operator's home. Its hooks are root's, under `/usr/local/libexec/hide-candidate-runner`.
- **Nothing kept.** The job-completed hook (`scripts/candidate-runner/job-completed.sh`) empties the runner's work folder after every job, whatever its result: the checkout and its release build, the cargo registry (the daemon points `CARGO_HOME` there), setup-node's Node and pnpm with its store. The account keeps only rustup and the toolchain the source pins; the hook removes any other.

`scripts/candidate-runner/setup.sh` sets the Mac mini up; its administrator runs `sudo bash setup.sh` from a copy of that folder once and pastes the registration token GitHub shows under the repository's Settings, Actions, Runners, New self-hosted runner.
A second run keeps the account and the registration, installs the hooks again and replaces the LaunchDaemon only when it differs; each run fails unless the administrator's own launchd services, runner configuration and Rust toolchain are as it found them.
To retire it, `sudo launchctl bootout system/dev.withhide.candidate-runner`, remove the runner from the repository's settings, then delete the account.

When the Mac mini does not answer (a queued `compile (macos)` waits up to a day for it), cancel the candidate and dispatch it again with `macos_runner: hosted`, which compiles the same binaries on GitHub's `macos-15`, free for this public repository, and the receipt checks them the same way.
The private publisher's signing job has the same choice, but a hosted macOS minute of a private repository counts ten times against the plan and the organization's Actions budget is $0, so it starts only after the operator raises that budget.

## Release asset gate

Official release assets use a `vX.Y.Z` or `vX.Y.Z-rc.N` tag (`N` at least 1, no leading zeros) whose source commit is already an ancestor of protected `main`.
The tag is the one source of the release's kind: `releaseKind` in `scripts/check-release-assets.mjs` reads it for the workflow's tag check, the asset names, the draft's `prerelease` flag, every check of the draft, and the publish call, so no second input can disagree with it.
A `vX.Y.Z` tag is a stable release, published with `make_latest: "true"`; a `vX.Y.Z-rc.N` tag runs the same gates, the signed and notarized macOS build included, and is published as a pre-release with `make_latest: "false"`, which withhide.dev's download links ignore.
Its packages report the version without the `v`, such as `0.4.0-rc.1`, and are named for it, such as `hide-v0.4.0-rc.1-macos-arm64.zip`; the macOS bundle version and the Windows file version are the numeric prefix, `0.4.0`.
The publisher assembles all three systems' packages before `node scripts/release-draft.mjs <tag> <event-commit-sha> <directory>` prepares the matching draft.
The file-only entrypoint is `node scripts/check-release-assets.mjs <tag> <directory> [release-pages.json]`.
The directory must contain exactly that tag's eighteen files (`expectedAssetNames`): the macOS ARM64 dmg and zip, the Windows x64 `setup.exe`, the Linux x64 deb and rpm, one SHA-256 sidecar per installer, `latest-mac.yml`, `latest.yml`, `latest-linux.yml`, the zip's and the setup's `.blockmap`, both Linux device bundles and `device-bundles.json`.
The gate rejects a missing target, extra or mixed-version files, symbolic links, empty files, an incorrectly named sidecar, and a mismatched digest, and reads each update file strictly: its version must be the tag's, and every file it names must be in the release with the SHA-512 digest and size it states, since the app's updater refuses a download that differs; hashing streams bytes rather than retaining each package in memory.
Before any release write, the writer checks that the current tag still resolves to the event commit; the workflow passes its contents-write token as `GH_TOKEN`.
A missing credential, moved tag, or failed initial API read blocks preparation before writes; package workflow artifacts remain available.
An API failure after an append may leave a partial or complete draft, including an upload whose success response was lost.
The writer stops further preparation and preserves accepted assets; a retry validates those bytes before skipping them.
The file and draft helpers do not establish whether repository immutability is enabled; the official publisher separately requires that setting before publication.

The authenticated release and asset inventories are fully paginated, with a 100-page and 16 MiB limit; a tag lookup alone cannot establish that a draft is absent.
Multiple releases for the same tag, an invalid inventory, a published release, or a draft of the other kind (a pre-release draft for a stable tag, or the reverse) block the run.
A new draft records its event commit in its body; an existing draft must have that exact provenance record.
Legacy drafts are not automatically reused.
The provenance record describes this controlled writer's origin; it is not a cryptographic attestation or a defense against administrative bypass.

The writer binds to that checked release ID, uploads only missing names, and skips an existing asset only when its uploaded state, size and SHA-256 digest match the local file.
It never changes an existing release's metadata, deletes an asset, or overwrites a name.
Mismatched assets, incomplete `starter` uploads and duplicate-name races stop for review; a retry resumes a matching partial draft without replacing completed bytes.
The final read must still show the complete unpublished draft and matching bytes.
[GitHub immutability](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases) supplies, once the operator setting is on, the server protection if publication happens between a read and upload; a GET alone is not an atomic draft-state condition.
The operational contract excludes concurrent owner-policy disabling, manual changes to the draft, and tag moves during preparation and publication.
Repeated tag reads do not make prepublication provenance atomic; the writer reads the tag and the draft's source record again right before it publishes.
Each writer request has a 30-second bound and JSON responses are limited to 8 MiB.

This gate proves the asset set and digests, not installation, terminal input/output, native first launch, signing or notarization.
The Windows and Linux package smoke checks cover bundled tools, daemon startup, the embedded shell and daemon shutdown; the macOS packaged-app specs and the Windows packaged window open the app itself, and the tagged commit's main `verify` run covers the end-to-end suites.
`release-draft.mjs --publish` publishes the draft, as the latest release for a stable tag and as a pre-release for an `-rc.N` tag, only after that final read, and only when every one of those release gates passed ([CONTRIBUTING.md](../CONTRIBUTING.md#releases)); without `--publish` it prepares the draft only.
Its source guard does not establish that the repository setting is on; verify it before release work.
