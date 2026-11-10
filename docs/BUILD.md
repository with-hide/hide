# Build output and worktrees

This document owns where build output goes and why: every build inside the worktree that asked for it, the release binaries at their fixed path, a debug build kept small, one Rust version named by the repository, the machine's toolchain reused rather than reinstalled, and the terminal parser's Zig build from pinned inputs.
The scripts named here are the executable authority; `scripts/tests/test_toolchain_reuse.py`, `test_rust_toolchain_pin.py`, `test_ci_gate_portability.py`, `test_verification_builds.py`, `test_debug_build_size.py`, `test_prepare_vt_inputs.py` and `test_vt_cache.py` assert the parts a workflow depends on.

## One rule: build output lives in the worktree

Cargo writes to `target/`, the web shell to `web/node_modules/` and `web/dist/`, and the desktop app to `desktop/node_modules/`, `desktop/dist/` and `desktop/out/`, all at their default locations inside the checkout and all ignored by Git.
Nothing a checkout builds is written anywhere else, so `git worktree remove` is the whole cleanup, and no cache can outlive the work that produced it.

Two facts make the default location the only correct one.

Cargo names a workspace member's artifacts by its path relative to the workspace root, so two checkouts sharing one target directory read each other's build as fresh and run the other checkout's test binary.
Cargo also holds an exclusive lock on the directory, so sharing serializes the parallel builds the worktree layout exists to allow.

The release binaries are `target/release/{hided,hide,hide-agent-hooks,hide-holder}` (`RELEASE_PROGRAMS` in `desktop/scripts/release-binaries.mjs`), and `desktop/scripts/package.mjs` reads every one of them from that fixed relative path.
Redirecting a release build with `CARGO_TARGET_DIR`, `--target-dir` or `--build-path` leaves the packager reading a path nothing wrote.
`scripts/verify-cargo.sh` therefore sets `CARGO_TARGET_DIR` to the worktree's own `target/` on every invocation, whatever the caller carried.

The cost of this layout is one full build cache per worktree, which is why a worktree is removed when its branch lands rather than kept around.

A release `hided` carries `web/dist` inside the binary: `hided/build.rs` embeds every file under it when the profile is `release` and fails the build when `web/dist/index.html` is missing, so a release build is always `pnpm --dir web build` first, then `cargo build --release --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks -p hide-holder --bin hide-holder` (`scripts/verify-cargo.sh release`), which prepares hide-vt's Zig inputs first because the pane holder links it ([The terminal parser and Zig](#the-terminal-parser-and-zig)); every job that builds release binaries restores the vt lane's entries (`.github/actions/vt-cache`).
`desktop/scripts/package.mjs` performs that build before staging the release binaries into the app bundle, so a packaged app's SessionStart probe has a matching `hide-agent-hooks` executable even without a separate CLI on `PATH`.
A debug `hided` embeds nothing and reads `web/dist` from disk at run time (`HIDED_UI_DIR` overrides the lookup), so a rebuilt web shell shows up without a cargo rebuild.
The web measurement (`MEASURE_SCENARIO=keys HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure/<attempt> bash scripts/web-shell-measure/run.sh`, `docs/PERFORMANCE_TESTING.md`) needs that release `hided`, with `hide-holder` beside it, at `target/release/` inside the worktree it measures; it is never redirected, for the same reason as the release binaries.
On 2026-09-09 one abandoned worktree held 5.3 GB, over half of the 10 GB across all eight.
An earlier design sent test builds to `/tmp/hide-verify-<uid>/<hash>/`, keyed by checkout path so a run judging the tree would not dirty it; the reason had lapsed once the harness scored only tracked files, and the caches it left behind reached 7.5 GB with no worktree owning them.
Hide's own merged-worktree cleanup had grown a step that forked `bash` to compute and delete that path, which this layout makes unnecessary.

`[profile.dev] incremental = false` in the workspace manifest is deliberate, not a leftover.
An agent worktree is built a few times and discarded, which never repays an incremental cache; what it does instead is grow one per worktree, and those had reached 1.5 GB.
Debug output is what makes a stale worktree expensive, because nothing strips it: a debug `hided` measures about 165 MB, with no debug info, against 24 MB for the release binary.
`sha2` is built at `opt-level = 3` in dev builds too: a development daemon installs its own debug `hided` and `hide` (about 140 MB each) on a device over SSH and takes each file's SHA-256 here first, and unoptimized that digest took longer than the remote mailbox lane waits for a ready device; the SSH itself is the system `ssh`, which no build compiles.
`miniz_oxide` and `crc32fast` are too, for the same install's gzip packing (with-hide/hide-private#582): unoptimized, packing one 276 MiB debug `hided` took 10.6 s, optimized 0.8 s.

## A debug build is kept small

A worktree's `target/` is paid once per worktree, so its size multiplies by the number of parallel worktrees; on 2026-10-08 five of them held 2 to 10 GB each, and the disk ran out under the builds of the others.
Three causes made most of a debug build, and each has a rule below.
`scripts/verify-cargo.sh hakari` in the rust lane and `scripts/tests/test_debug_build_size.py` in the policy lane fail a change that brings one back.

The measurement started each time from an empty `target/` in a clone of `main` at `6472cb38`, on an Apple silicon Mac with the toolchain `rust-toolchain.toml` pins.
It ran the commands one worktree's work runs, in order: `cargo test --locked --workspace --no-run`, `cargo build --locked -p hided --bins -p hide-agent-hooks --bin hide-agent-hooks` (`verify-cargo.sh cli`), `cargo test --locked -p hide-core --no-run`, `cargo test --locked -p hide-platform -p hide-herdr-client --no-run` and `cargo clippy --locked --workspace --all-targets`, and read `du -sm target` after each.
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
Debug info was most of what a debug build wrote: 214 of the 331 MB of object code in the 20 largest dependency libraries was DWARF, hide-core's library was half DWARF even at `line-tables-only`, and on macOS any debug info keeps every object file a binary linked beside it in `target/debug/deps`, because the debugger reads the DWARF from there (2.2 GB of the build above).
A panic still prints its file, line and column, which are compiled into the binary as data rather than read from debug info; a backtrace names its functions without lines.
For a debugger or a backtrace with lines, turn debug info on for one build, `CARGO_PROFILE_DEV_DEBUG=line-tables-only cargo test -p <crate>` (`true` for variables too); every crate builds again under another hash beside the default build, so that worktree pays for both until it is removed.
The change invalidates CI's cached dependency builds once; the first run on `main` after it rebuilds and saves them.

### One feature set per dependency

Cargo resolves a dependency's features from the members a command selects, so `-p hide-core`, `--workspace` and `--bins` each built tokio, hyper and serde_json with a different set, under a different hash, and every crate above them again beside the first copy.
`workspace-hack` is a member with no code whose manifest turns on, for each third-party crate, the union of the features any member asks of it; every other member depends on it, so every command resolves one set.
[cargo-hakari](https://docs.rs/cargo-hakari) writes it from `.config/hakari.toml` for the systems the workspace is built and packaged for.
Cargo's own `resolver.feature-unification = "workspace"` does the same without a crate, but it is unstable in the pinned toolchain ([rust-lang/cargo#14774](https://github.com/rust-lang/cargo/issues/14774)); once it is stable it replaces `workspace-hack`.

After adding or changing a dependency or its features, run `cargo hakari generate` and `cargo hakari manage-deps --yes` and commit what they write; `bash scripts/install-hakari.sh` installs the pinned release.
It writes each version as a semver range rather than the members' exact pins, since `Cargo.lock` decides the version either way, so a patch or minor bump such as Dependabot's weekly group leaves it unchanged; a major bump or a changed feature set needs the two commands.
The rust lane runs `scripts/verify-cargo.sh hakari`, which fails when either command would change a file.
`hide-platform` stays off the shared set (`final-excludes`): the OS contract lanes build and test exactly that crate, which would otherwise compile tokio, hyper and schemars for nothing.
`hide-vt`, `hide-holder` and `hide-holder-protocol` stay off it too: a pane's holder runs for as long as its pane, so the program carries only the few dependencies it names.
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

## The release profile

`[profile.release]` links with thin LTO in sixteen codegen units, where it used one.
The release jobs build from a clean target, so the build is most of their time, and sixteen units let rustc optimize one crate on several cores.
Turning LTO off with one unit was also measured and was slower on macOS (19.2 min), so thin LTO stays.

Three clean builds of `hided`, `hide` and `hide-agent-hooks` per profile on hosted runners, from the same source, gave these medians:

| System | One unit | Sixteen units | Three binaries |
| --- | --- | --- | --- |
| macOS arm64 | 22.7 min | 16.0 min (-29%) | +20.9 MB (+17%) |
| Windows x64 | 22.1 min | 17.0 min (-23%) | +10.7 MB (+6%) |
| Linux x64 | 15.1 min | 12.8 min (-15%) | +20.3 MB (+14%) |

The two builds of `hided` were also compared at run time on one macOS arm64 hosted runner, four alternating trials each of the `keys` scenario in `docs/PERFORMANCE_TESTING.md`.
Key echo and CPU did not move beyond the spread between trials, idle or with five panes printing.
Startup to a healthy `/health` took 168 ms against 134 ms, the median of four trials each, about 35 ms more.
`hided` holds about 4.7 MiB more resident memory (62 MiB against 57 MiB) in every trial, probably the larger binary's code.

## A pull request's release build

A pull request's package lane builds its release binaries at opt-level 1 without LTO, in 256 codegen units (`.github/actions/quick-release-build`).
That lane checks that the app packages, installs, starts and updates; none of that depends on how well the binaries are optimized, and the optimization was most of the lane's time.
Each of the three binaries runs thin LTO over the whole program at its link, so after the last hide crate started compiling, a Windows release build still took about 12 more minutes (2026-10-30, pull request #1913).
Measured on 2026-10-30 (pull request #1916) against the same lane's recent pull-request runs: `compile (windows)` went from 17-24 minutes to 7.6, `compile (macos)` from 15-21 to 8.8, and `package (linux)` from 16-22 to 12.1.
The Windows cargo build alone went from 14.7 minutes in #1913 to 6.2, both compiling every dependency from source, since this setting rebuilds them anyway.

The settings are cargo's own profile variables, set in the job's environment, so the binaries still land in `target/release`, the path `desktop/scripts/package.mjs` reads, and keep the release profile's name, strip and debug-assertion settings.
The cost is that a pull request's package never holds what ships.
Main's push run, the nightly, a candidate and a release build with the release profile itself, and main's runs are the only savers of the `release` dependency set, so that set always holds the release profile's dependencies; a pull request restores it and rebuilds its dependencies at its own settings, because cargo rebuilds a dependency whose profile differs.
`scripts/tests/test_quick_release_build.py` fails when another caller of `package.yml` turns the setting on, or a job that builds release binaries ignores it.

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

## CI's Rust caches

Every CI job that builds Rust restores its compiled third-party dependencies through `.github/actions/rust-cache`, which wraps [rust-cache](https://github.com/Swatinem/rust-cache) with one key per dependency set and system rather than one per job.
The repository has 10 GB of Actions cache, and on 2026-10-28 main's entries filled 9.8 GB with the same dependencies kept four or five times over, so GitHub evicted entries within a day of their last use.
Three things made the copies: a key named after the job, so the rust lane, the web end-to-end build, the mailbox lane and the journey each kept their own copy of one build; the installed toolchains in the key, where some Linux runners of the same image carried another stable beside the pinned version and each Linux job kept a copy per variant; and a changed lockfile, which saves a new key beside the old one.

| Set | What it holds | Saved by, on main only | Read by |
| --- | --- | --- | --- |
| `workspace` | The debug build of the dependencies, one feature set for every member (`workspace-hack`) | `rust workspace` on Linux, `windows check` on Windows, `desktop e2e` shard 1 on macOS | Every other lane that builds Rust, and a release tag's tests |
| `platform` | `hide-platform` built alone, outside that feature set | `os contract` on each system | Nothing else |
| `release` | The release profile's dependencies | `package.yml`'s Windows and Linux package jobs and its macOS compile | The packages of a pull request and a release tag, and the nightly measurement |

Each set is saved by one job per system, the one that builds the most of it, because a key that exists is never saved again: two savers would race and the loser's extra build would be lost every run.
A reader builds what the saver did not, which on macOS is the extra dependencies of the nightly lanes that test more than the daemon.
rust-cache's environment hash stays in the key; it hashes every installed toolchain, so the action first removes every toolchain but the one `rust-toolchain.toml` names, and the hash then depends on the pin and not on the runner.
rust-cache ends each key with its own 8-digit hash of `Cargo.lock`, every member's `Cargo.toml` and `rust-toolchain.toml`, and when that hash changes it restores the newest entry of the same set and system and builds only the dependencies that changed; it leaves out the root `Cargo.toml`, whose `[profile]` sections change every dependency's build, so the action puts that file's hash in the key before the system, and a change there misses once.
A changed lockfile thus saves a new key beside the old one, and nothing deletes the old one: GitHub deletes an entry no run has read for seven days, and past the repository's 10 GB it evicts the least recently read first.
Main's runs read the current entry of every set every day, so what ages out or is evicted first is a replaced one; one entry per set and system is about 3 GB, and a week of replaced sets beside it stays under the limit unless the lockfile changes several times that week, when eviction takes the oldest replaced sets.
Nothing in the repository prunes them: deciding which entry is current means computing rust-cache's own key, which changes whenever rust-cache does, and a copy of that computation that drifted would delete the current entries.
The keys from before the sets (`v0-rust-<job>-...`) are deleted by hand once, after each set's first entry is saved.
`scripts/tests/test_rust_cache.py` fixes the action's key, its dependency-only save and the toolchain step, and each saver's exact `save` condition, and fails when a job uses rust-cache directly or names another set.

### What a cached release dependency can change

The packages a candidate builds are what ships on Windows and Linux, and their release dependencies now come from the `release` set rather than from a cold build.
What a release build takes from the cache is limited to third-party dependencies, so every hide crate, `hided`'s build script (the embedded web shell, version, commit and build order) and the final link are compiled from the checkout in every run.
Two steps hold that: the action keeps rust-cache's `cache-workspace-crates: false`, whose normal cleanup deletes the workspace's own crates from `target/` before a save, and since that cleanup swallows its own errors and saves anyway, a release job also runs `cargo clean --release --workspace` right after the restore, which removes any workspace crate an entry still held and leaves the dependencies.
Cargo reuses a cached dependency only when its fingerprint matches what this checkout asks for: the same package and version, which `Cargo.lock` pins with the registry checksum it was downloaded against, the same features, profile, target and compiler; anything else is rebuilt from source.
A run reads entries saved by its own ref and by main, so the only entries a candidate or a release tag can read are those saved by runs on main, where a pull request's code arrives only through a reviewed merge; a pull request never saves (`save` is false off main), and an entry is never overwritten once saved.
The argument stops at the artifact itself: cargo does not hash a compiled dependency against its source, so a release dependency is as trustworthy as the jobs that run on main, since any of them can save an entry under a key that does not exist yet.
The macOS app the private publisher signs is built on the Mac mini from its own checkout and reads none of these caches, and the Linux device helpers (`device-helpers.yml`) still build cold.

## The terminal parser and Zig

`hide-vt` is the terminal parser the pane holder (`hide-holder`) links: Herdr's binding to libghostty-vt, renamed ([hide-vt/PATCHES.md](../hide-vt/PATCHES.md) lists its origin and every change).
libghostty-vt is Zig and C, 22 MB of source, and is never copied into this repository, so a build needs three inputs it downloads once: the Zig 0.16.0 compiler for the machine, the Herdr source archive whose `vendor/libghostty-vt` is the source dist with Herdr's patches applied, and the eight Zig packages that build reads.
`hide-vt/build-inputs.json` pins each one by digest; it is the only place the versions are written, and it pins the Herdr release the binding was taken from, because the parser moves only when someone moves it.

`scripts/prepare-vt-inputs.py` is the one preparation step, on every system and in every lane.
It downloads each input, refuses one whose sha256 differs from its pin, checks that every one of Herdr's patches applies in reverse to the extracted tree (so the tree is the dist plus exactly those patches), has Zig fetch the packages, which Zig checks against its own content hash, and moves the finished folder into place in one rename.
The `translate_c` package is fetched over git (`git+https://codeberg.org/...#<commit>`), not as codeberg's generated archive: on 2026-11-09 codeberg's archive endpoint stopped answering while its git service kept working, which failed every CI build that prepared the inputs, and Zig's content hash of the git checkout is the same as the archive's.
A refusal names the input, the digest it found and the one pinned, and leaves nothing behind.
The folder is `<cache>/vt/<pin digest>-<system>`, where `<cache>` is `HIDE_BUILD_CACHE`, else `~/.cache/hide-build`, else `%LOCALAPPDATA%\hide-build` on Windows; on macOS it holds about 470 MB, 415 MB of it the Zig compiler.
It is a download cache like the Rust toolchain, shared by every checkout and never written by a build, and the three most recently prepared entries are kept.
`scripts/toolchain-env.sh` points `HIDE_BUILD_CACHE` at the machine's own home, for the same reason it does so for rustup: a verification runner under its own HOME would otherwise download and keep a private copy.

`hide-vt/build.rs` reads only that folder and downloads nothing.
It stops with the reason and the command to run when the folder is not prepared, when Zig is missing, and when `ZIG` names a Zig of another version.
It runs Zig with `--system <packages>`, which turns package fetching off, and `-fno-sys` for each of libghostty's system integrations, so every package comes from the prepared folder and a missing one is a failure naming it, never a download.
Zig compiles into the worktree: its caches are about 260 MB per target, kept in the build's `OUT_DIR` for that build only and removed after it, unless `HIDE_VT_ZIG_CACHE` names a folder to keep, which only CI does.
A cold build on an Apple M-series Mac took 46 s with a peak of 1.07 GB in the Zig compiler (2026-10-31); the vt lane measures each system in CI.

`scripts/verify-cargo.sh` prepares the inputs before every mode that can build hide-vt (`test`, `lint`, `release`, and a scoped mode naming `--workspace`, `-p hide-vt` or `-p hide-holder`), which does nothing when they are ready.
When the preparation fails, as it does without a network the first time, the command stops before cargo with the reason and the command to run where ziglang.org, github.com, codeberg.org and deps.files.ghostty.org are reachable.
`release` builds the pane holder, which every package carries and which links hide-vt, so packaging prepares the inputs, Zig included, as a test does; `cli` builds no hide-vt and needs no Zig; a Linux device kit carries the pane holder, so `device-helpers.mjs build` prepares them too.
The Mac mini's release jobs therefore prepare the inputs as its verify job `vt (macos)` does, in each job's own hide-vt input folder ([The Mac mini CI runner](#the-mac-mini-ci-runner)), and a job that finds that folder empty downloads them first, about 415 MB with Zig, before it compiles.

In CI, `.github/actions/vt-cache` restores the prepared folder, keyed by the pin file, system and architecture, and Zig's caches, keyed by the pin file, `hide-vt/build.rs` (which fixes the target mapping and the `ReleaseFast` optimize mode), system and architecture.
Both live under the runner's temporary folder at fixed paths, because Zig's C-header cache keys include the compiler's installation path, and outside `target/`, which the workspace set's rust-cache entry holds.
The `vt` lane is each system's one saver, on main only; `rust workspace`, `windows check` and the release's workspace tests restore, and a miss costs a download, never a failure.
`scripts/tests/test_vt_cache.py` holds every job to that.

The `vt` lane (`.github/workflows/vt.yml`) builds hide-vt on Linux, macOS and Windows from empty Zig caches and again from kept ones, recording the caches' sizes, runs its tests and Clippy and the pane holder's ([TESTING.md](TESTING.md#the-pane-holder)), builds it again with the network denied on Linux (`unshare --net`) and macOS (`sandbox-exec`) after showing the wrapper denies it, and runs `scripts/check-vt-build-refusals.sh`, which shows a missing Zig, another Zig version and a download of other bytes each stopping with its reason.
On 2026-11-01 (main's run 41283189059) a build from empty Zig caches took 157 s with a largest process of 813 MiB on Linux, 108 s and 1,072 MiB on Windows and 45 s and 778 MiB on the Mac mini, and a build from kept caches 1.6 to 2.3 s and at most 300 MiB; the caches were 259 to 500 MiB and the prepared inputs about 450 MiB.
The lane no longer times its builds: nothing judged those numbers, and the script that took them was removed.
`scripts/tests/test_prepare_vt_inputs.py` covers the preparation's refusals without a network.

The holder restart lane (`.github/workflows/holder-restart.yml`, [TESTING.md](TESTING.md#the-holder-restart-lane)) builds `hide-holder` and hide-node's `holder_node` example and restores the vt caches without saving them.
The example is declared `test = false` in `hide-node/Cargo.toml`, so `cargo test` and the workspace tests never build it and only the lane and the measurements link it; Clippy, which `verify-cargo.sh lint` runs over every target, checks it without linking it: on the GitHub runners of 2026-10-31 the example took 17 to 44 s on top of the holder's own 6 to 109 s, which depend on how warm the rust-cache entry was.

## The vendored PTY crate

The pane holder opens its PTY with `portable-pty` 0.9.0 as Herdr v0.9.5 vendors it, with Herdr's four patches and Hide's own changes, in `vendor/portable-pty` ([PATCHES.md](../vendor/portable-pty/PATCHES.md) lists its origin and every change).
The root manifest's `[patch.crates-io]` puts that copy in place of the published crate for the whole workspace; it is not a member, so the workspace's Clippy and tests do not cover it, and the holder's tests are what check it.
Its Windows side loads only the pinned ConPTY runtime beside the holder, which this repository does not carry: Microsoft's `Microsoft.Windows.Console.ConPTY` package from nuget.org, at the version, package sha256 and file sha256s `contracts/conpty.json` pins.
`scripts/fetch-conpty.py` downloads it, refuses a package or a file of any other digest, and makes the runtime folder (`conpty.dll`, `x64/OpenConsole.exe`, `arm64/OpenConsole.exe` and the file list `conpty-files.json`) in a cache keyed by the package digest, with `--out <folder>` copying it to a folder of its own; `CONPTY_FILES` in `vendor/portable-pty/src/win/psuedocon.rs` pins the same file digests, and `scripts/tests/test_fetch_conpty.py` fails when the two disagree.
A node finds the runtime in `holder-conpty` beside its own program (`hide_kit::holders::CONPTY_SOURCE`), apart from the `conpty/` folder Herdr's own Windows asset ships beside `herdr.exe`; the holder's tests on Windows read it from the folder `HIDE_HOLDER_CONPTY` names, and the vt lane fetches it with `scripts/fetch-conpty.py`.
A newer ConPTY moves `contracts/conpty.json` and `CONPTY_FILES` together.
A change there plans the same lanes as a change to `hide-holder` (`scripts/ci-plan.py`).

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
| release | `bash scripts/verify-cargo.sh release` | `target/release/{hided,hide,hide-agent-hooks,hide-holder}`, the binaries the desktop packager ships |
| cli | `bash scripts/verify-cargo.sh cli` | `target/debug/{hide,hided,hide-agent-hooks}` for isolated CLI, daemon, SessionStart and install kit checks |
| web | `bash scripts/verify-web.sh` | `pnpm install --frozen-lockfile`, then typecheck, lint, test and build for both `web` and `desktop`: `web/dist` and `desktop/dist` |

The Cargo `test` mode forwards trailing test arguments, so an explicitly configured live probe can run as `verify-cargo.sh test <test-name> -- --ignored` without bypassing worktree isolation or toolchain ownership.
The no-argument `test` mode remains the full locked workspace gate.
Focused delivery and session-activity filters and their nonzero-test prerequisite are listed in [delivery.md: Verification](delivery.md#verification).
Each compiler or test process's failure reaches the caller.

CI uses scoped Cargo modes `test-scoped`, `check`, `build` and `clippy` with trailing Cargo arguments, for example `bash scripts/verify-cargo.sh test-scoped -p hide-platform -p hide-holder`.
Each adds `--locked`, reuses the installed toolchain, clears inherited Herdr and legacy coordination overrides, and fixes output to this worktree's `target/`.
Scoped modes accept at most 128 arguments and refuse `--target-dir`, `--manifest-path` and `--config`; an unknown mode exits 2.
The sealed `test`, `lint`, `release` and `cli` invocations keep their existing behavior.
`verify-web.sh install [--ignore-scripts]` locks dependency installation; `verify-web.sh <web|desktop> <typecheck|lint|test|build>` runs one package step.
`web build` includes its typecheck before Vite bundles, so the no-argument gate and CI's web checks run it once before lint and unit tests, with no separate web typecheck over the same inputs.
Desktop build uses esbuild without typechecking, so its separate typecheck remains required.
Standalone package `build` and `e2e` commands keep their preparation: web build typechecks, web package e2e builds the shell, and desktop e2e builds the host.
The desktop packager also runs the web build with its typecheck by default, for local builds and standalone CI calls alike.
The web package's `bundle` script prepares the relay WASM and owns the Vite command; both `build` after its typecheck and the packager's CI-only path call it.
`typecheck`, `test`, `dev` and `build:phone` also prepare their WASM prerequisite through `prepare:relay`; Cargo checks freshness on repeated calls, and a preparation failure stops the caller.
`package.yml` accepts `web-typechecked: true` only from a caller that requires the same source's verify web build before accepting or publishing its packages.
It passes that source as `HIDE_WEB_TYPECHECKED_SHA`; `package.mjs` requires GitHub Actions, an exact checkout commit match and unchanged web inputs and lockfile with no untracked web source, then runs Vite without repeating `tsc`.
The `GITHUB_ACTIONS=true` check guards against mistakes, not a security boundary; a caller that sets the environment still owns the verify dependency contract.
The same source pins `pnpm-lock.yaml`, and both verify and packaging install it frozen with Node 22 and pnpm 10.
The PR/main caller opts in only when its plan requires the `checks` lane, and the existing aggregate refuses a failed, cancelled or skipped planned check.
`release-candidate.yml` also opts in and may build in parallel with verify; its receipt names that exact source's verify run, whose success the final publication requires.
For macOS, the override reaches only `compile`, where the web shell is built; `assemble` uses the compiled binaries without another web build.
Until that gate passes, a built package is unverified; the opt-in is a dependency contract, not independent proof of success.
Other callers leave the input false, and setting the source override outside GitHub Actions or for another commit fails packaging.
`web e2e` through this wrapper builds the phone bundle and native relay fixture, then runs Playwright against the web output already built; `desktop e2e` always rebuilds the desktop host before Playwright, including a direct local invocation.
Both forward the test arguments and their exit status, so a missing test filter fails the caller.
`<web|desktop> e2e-list [args...]` only discovers tests (`playwright test --list`); it needs no app build and cannot execute tests.
Both web and desktop CI jobs use discovery for the complete and filtered shard lists, then use `e2e` for execution: preparation happens once per execution, never while listing tests.
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


The Windows/Linux package smoke uses an isolated HOME and state folder to check the command from a fresh shell, both configured agent hooks and the completed retirement row.
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
| `pnpm --dir desktop e2e` | Playwright `_electron` against a private hided and its pane holders; needs `web/dist`, `target/debug/hide`, `hided` and `hide-holder`, as the web e2e does |
| `pnpm --dir desktop package` | `desktop/scripts/package.mjs`: builds the release binaries, bundles `hided`, `hide`, `hide-agent-hooks` and `hide-holder` (the pane holder every pane runs in) into `Contents/Resources` and the notification permission helper into `Contents/MacOS/hide-notification-status`, signed under the app's identifier, signs `desktop/out/hide-darwin-<arch>/hide.app` (bundle id `me.grab.hide.desktop`), writes the release version into the app's own `package.json` (the version the app and its updater report) and puts `desktop/resources/app-update.yml`, the updater's download folder name, in its resources; `desktop/scripts/installers.mjs` then wraps the finished app with electron-builder (`--prepackaged`, so the app and its signature are what was checked) into `desktop/out/release`: `hide-v<version>-macos-<arch>.dmg` and the `.zip` the updater installs, `latest-mac.yml` and the zip's `.blockmap`, each installer beside a `.sha256` checksum; nothing is installed. The signature is ad-hoc unless one of two variables asks otherwise. An ad-hoc signature changes with every build, and macOS keys a privacy grant such as Screen Recording to the signature, so each install of an ad-hoc build asks for those grants again; set `HIDE_SIGN_IDENTITY` to a code-signing identity in your keychain (its name or SHA-1, such as an `Apple Development` or `Developer ID Application` certificate) and a local package is signed with it under the hardened runtime, not notarized, and keeps the grants across installs. `HIDE_MACOS_SIGNING=developer-id` is what the official macOS publisher sets: every executable in the bundle is then signed with the Developer ID certificate under the hardened runtime with the exceptions in `desktop/resources/entitlements/`, Apple notarizes the app and the ticket is stapled, and the run fails unless Gatekeeper accepts the app expanded from the zip as notarized; the dmg is signed, notarized and stapled too, and must pass `spctl -t open`. That mode needs `HIDE_SIGN_IDENTITY`, `HIDE_NOTARY_KEY_PATH`, `HIDE_NOTARY_KEY_ID` and `HIDE_NOTARY_ISSUER`, which `desktop/scripts/macos-signing.mjs` defines and checks together before the build. On Windows x64 and Linux x64 the same command builds that system's package, unsigned: the folder `desktop/out/hide-win32-x64/` or `hide-linux-x64/` with the same binaries (`.exe` on Windows, with the pane holder's pinned ConPTY runtime in `holder-conpty/`, which `scripts/fetch-conpty.py` fetches and checks against `contracts/conpty.json`, failing the package on any other digest) in its `resources/`, wrapped into `desktop/out/release` as the per-user NSIS installer `hide-v<version>-windows-x64-setup.exe` with `latest.yml` and its `.blockmap` (`desktop/resources/windows/installer.nsh` stops the daemon on uninstall), or as `hide-v<version>-linux-x64.deb` and `.rpm` into `/opt/hide` with one `latest-linux.yml` (`desktop/resources/linux/` holds the package scripts that link `/usr/bin/hide` to the CLI and set up the sandbox), each installer beside a `.sha256`. `node desktop/scripts/installers.mjs <app folder> <version> <out>` wraps an already packaged Windows or Linux app again under another version, which the CI update test uses for its two builds; on Linux these test builds make only the deb, its checksum and update file, while release packaging still makes both the deb and rpm. Each system packages its own desktop host, and a machine of another architecture than the package's is refused; no package carries or publishes a device kit, which a release candidate makes ([Device helper release bundles](#device-helper-release-bundles)) |
| `node desktop/scripts/smoke-package.mjs <installed app folder>` | On Windows or Linux only: takes the folder the system's installer put the app in, prepares its runtime (on Windows the copy under the private home that `hide runtime prepare` makes), and with a private home (stand-in `claude` and `codex` programs in `~/.local/bin`, so both agents are installed) and state folder checks the package carries no Herdr and, on Linux, that every ELF file it installs finds each library it links (which the Electron host, never started by these checks, would otherwise fail on at its first launch), then runs the device helper, `hide-agent-hooks doctor`, and `hide connect`, `/health`, the embedded shell, first-launch CLI and hooks, a command from a fresh shell, the completed retirement row, replacement by a second package fixture, refreshed paths, same-build reuse, standalone refusal and `hide stop`, then waits for each daemon pid it started to end (ten seconds, a named failure beyond that); on Windows it also checks that the next prepare removed the first build's runtime copy; `.github/workflows/package.yml` runs it after installing each package, and `desktop/scripts/smoke-windows-installer.mjs` checks the Windows installer's install, reinstall while the daemon runs, and uninstall. It refuses macOS, where first-launch kit behavior is covered by the isolated desktop suite |

Every build of `hide` carries the version and commit `hide version` reports (`hided/build.rs`): `HIDE_VERSION` if it was set for the build, else the crate's version, and `HIDE_COMMIT`, else for a release build the checkout's `HEAD`.
A release passes `HIDE_VERSION` from its tag; a local package leaves it unset and `package.mjs` names the build after the newest `v<version>` tag behind it and its commit, `0.4.0-main.<commit>` (`.dirty` when tracked files differ), so a local build never claims a release, even on a tagged commit, and its version is no release number the app would update from.
`package.mjs` sets both to what it ships and refuses to package when it cannot read the commit; a debug build given no `HIDE_COMMIT` reports none, because following `HEAD` would rebuild `hided` after every commit.

The app attaches to whatever daemon the environment names: without `HIDE_STATE_DIR` it is the operator's own at `~/.hide/state`.
For QA, set `HIDE_STATE_DIR`, `HOME` and `HIDE_DESKTOP_USER_DATA_DIR` to private paths, as `desktop/e2e/fixture.ts` does and refuses to launch without.
Use `web/e2e/platform-fixture.ts` for the native executable and tool paths and for Windows `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` inside that private home.
The fixture C shim needs the runner's native compiler, `cc` on Unix or `clang.exe` on Windows, which Playwright's `globalSetup` runs once before any test (`web/e2e/shims/build.ts`); this is a fixture prerequisite, not an installed product requirement.
A packaged app does not need `hide` on `PATH`: it ships its own CLI, and only falls back to a login-shell PATH search and the well-known install directories when its own bundled CLI is somehow missing (see `docs/ARCHITECTURE.md`, The desktop host).
macOS may refuse the unsigned app's first launch until it is opened once with Open from the context menu.

The desktop e2e runs beside the operator's own work on the same Mac (issue 232).
Every launch through `desktop/e2e/fixture.ts` passes `--hide-show-inactive`, so the host shows its window behind every other window without activating the app and a second launch never focuses it, and `--disable-backgrounding-occluded-windows`, so the window keeps painting behind the operator's and a capture by window id is current.
The fixture also preloads `desktop/e2e/focus-guard.cjs` into the app, which records into a per-test file each time this app becomes active or a window takes the keyboard, from launch to quit, and fails any test not tagged `@needs-focus` that recorded one; specs take `test` from the fixture, and `launch` refuses a test that did not.
The guard sees only this app, so a spec that could reach another program (a browser, Finder, the folder picker) stubs it, as the existing ones do.
A spec that needs the key window or native input (a page holding the keyboard, a pinch, a native drag) carries the `@needs-focus` tag (`NEEDS_FOCUS` in the fixture) and focuses the window itself, a browser page through `focusPage` once the host shows that page, which gives the page the keyboard only after the window's `focus` event says macOS made it the key window; `pnpm --dir desktop e2e --grep-invert @needs-focus` runs everything that leaves the operator's keyboard alone, and CI runs the whole suite.
The suite runs the focus tests after every other one (`desktop/playwright.config.ts` puts them in a project that depends on the rest): a focus test brings its app forward and quits it, and on a machine with no other app in front, a CI runner, macOS then activates the next app that opens, which failed every background test after the first focus test.

## Device helper release bundles

Every platform a device kit is made for is named once, in `contracts/device-kits.json`: the manifest's format and name, the archive name, the four programs, and for each platform its Rust target, the job that produces it, its program suffix, the executable header its programs must have and any runtime files beside them.
The core (`hide-node/src/ssh/kit/`) and the packaging scripts (`desktop/scripts/device-helpers.mjs`) read it, and `scripts/check-device-kits-single-source.py` fails when a platform name appears anywhere else in their code, so a new platform is one entry there and a runner.
It carries Linux x64, Linux ARM64, macOS ARM64 and Windows x64; an Intel Mac or a Windows ARM PC is refused at the probe, before anything is installed, with the reason the contract gives ([INSTALL.md](INSTALL.md#what-each-system-supports)).
No app carries a device kit or its manifest: a core of any system fetches `device-bundles.json` and the kits it names from its own release (with-hide/hide-private#501 decision 1).
The packager verifies foreign bytes without executing them, by their executable headers.

`.github/workflows/device-helpers.yml` is the reusable producer of the Linux kits.
Its required string inputs are `source-sha` (full public source commit), `version` (the app version, optionally prefixed with `v`) and `build-order` (decimal order, including `0` for a fixture).
It explicitly checks out the public repository.
The Linux runners build natively on `ubuntu-24.04` and `ubuntu-24.04-arm` using the existing package recipe after the web build.
No new cross-compilation toolchain is needed.
Targeted output stays in this worktree's `target/<triple>/release`; host release binaries stay at `target/release`.
Never redirect a release target directory.

| Artifact | Download into this checkout | Contents |
| --- | --- | --- |
| `hide-device-helper-linux-x86_64` | `desktop/out/device-helpers/linux-x86_64/` | `hided`, `hide`, `hide-agent-hooks`, `hide-holder`, `manifest.json` |
| `hide-device-helper-linux-aarch64` | `desktop/out/device-helpers/linux-aarch64/` | same five files |

Each producer manifest records platform, source commit, version, build order and each helper's SHA-256.
`package.yml` resolves one identity with `desktop/scripts/build-identity.mjs` and calls the producer on every call, beside the Windows and Linux packages.

The kits are made once per run by its `device bundles` job, with `desktop/scripts/device-helpers.mjs bundle <version> <source-sha> <build-order>`: every kit of the contract whose producer is not the private publisher (`makeCandidateKits`), today both Linux kits and the Windows kit.
It needs both helpers and `compile (windows)`, and reads each producer's output where it downloaded it (`candidateKitSources`):

| Producer | Downloaded into this checkout | Checked against |
| --- | --- | --- |
| `device-helpers` | the two helper artifacts, as above | each helper's `manifest.json`, its executable header and digest |
| `windows-compile` | `hide-windows-release-binaries` into `desktop/out/windows-compile/` (`target/release/*.exe`, `desktop/out/release-binaries.json`), and `scripts/fetch-conpty.py --out desktop/out/windows-compile/holder-conpty` | the binaries' record (`checkReleaseBinaries`), each program's PE header, and the ConPTY pin in `contracts/conpty.json`, which the fetch refuses any other digest of |

The Windows kit is therefore the bytes the Windows app packages, from the one Windows compile of the run, with no second build; Windows bytes stay unsigned.
The job keeps the three archives, and no manifest, as the `hide-device-bundles` artifact; it writes each kit's name, digest and size to the job summary.
No package waits for it or carries its output: each package needs only its own compile, and a failed helper or Windows compile skips the candidate's receipt.
A local package (no `HIDE_VERSION`) needs neither helpers nor kits: its `-main.` version has no published release, so a core of that build downloads no kit anyway.
`desktop/scripts/device-helpers.mjs build <platform> <version> <source-sha> <build-order>` is the Linux producer command on its matching runner after dependency installation and `rustup target add <triple>`.
Making a kit refuses missing members, a different source/version/order, digest mismatches, links, files above 256 MiB and incorrect executable headers.
Downloaded artifacts need not retain executable mode bits.

`makeDeviceKit(platform, folder, version, output)` makes one archive from a folder holding exactly the kit's files under their kit names and answers its manifest entry, read back from the archive by `readDeviceKit(platform, archive, version)`, the core's checks in JavaScript.
The bytes are taken as they are: no program is signed, stripped or rewritten on the way into an archive.
Each archive is one gzip-compressed USTAR archive holding exactly the kit's files as regular entries under their `/`-separated paths: the four programs, with the platform's suffix (`.exe` on Windows), and the runtime files the contract lists, such as the Windows holder's ConPTY runtime under `holder-conpty/`; a Linux or Mac kit is the four programs at its root.
A device is sent every file, the pane holder apart from the helper's version folder (PRD engine-bundle-3-core-owns D-22, `HelperPackages::payload_for`); no archive carries Herdr (PRD engine-bundle-4-herdr-removal D-2).
No links, folder entries, extended attributes or PAX records belong in an archive.
The archive's name is `hide-device-<platform>-<app-version>.tar.gz`, without a leading `v` in the version.
The uncompressed total and the compressed archive are each capped at 256 MiB.

`device-bundles.json` has `format: 4`, `version`, full `commit`, decimal string `order`, and `platforms` keyed by every platform of the contract.
Format 3 named only the two Linux kits, for the manifest each app carried; format 2 carried Herdr beside the holder and format 1 had no holder; a core reads only its own release's manifest, so no build reads two.
Each platform records `asset`, archive `sha256`, archive `size` in bytes, and `files` keyed by the kit's file names, each with its final `sha256` and `size`.
Hashes are lowercase SHA-256 hex.
`writeDeviceManifest(output, identity, entries)` writes it over one entry per contract platform, each an archive already in `output` with that size and digest, and refuses anything less.
`desktop/out/device-bundles/` keeps the intermediate output, never anything under `/tmp`.

The private publisher (with-hide/hide-private#501 N4) takes `hide-device-bundles` from the release candidate run it selected, as it takes the Linux and Windows packages and [the Mac binaries](#the-candidates-mac-build), and requires each archive's entry, read with `readDeviceKit`, to equal the receipt's `device_kits`.
It signs the Mac programs for the app, makes `hide-device-macos-aarch64-<app-version>.tar.gz` from those signed bytes with `makeDeviceKit`, and writes `device-bundles.json` over the three candidate archives, taken unchanged, and its own with `writeDeviceManifest`; it builds no helper and compiles no binary itself.
The Mac kit needs no notarization: files a core uploads over ssh carry no quarantine attribute, so Gatekeeper never assesses them.
It must publish the four archives and `device-bundles.json` on the **same public `v<app-version>` release** as the Mac app, which must be immutable.
Do not re-sign or repack an archive after the manifest is made, and do not substitute output from another source or run: a kit made again would not be the same bytes, and every kit download of that release would be refused.
The public release validator and draft writer require 20 assets in total: the existing 15 installers, checksums, updater files and blockmaps, plus a kit for every platform of the contract and their manifest.
They refuse a release short of a kit before anything else, verify that the manifest names this release and source and every platform of the contract with each kit's files, and verify each kit's size and SHA-256 against the manifest; draft preparation also requires its source commit to match the release event before any GitHub request.
The private workflow is maintained outside this repository; the public producer and package run do not establish that publishing is connected or a release has shipped.

### Which programs a device gets

Every core picks a device's programs by one rule (`HelperPackages`, `hide-node/src/ssh/kit/select.rs`), on the connection worker and never under the runtime mutex.
A developer's `<name>-<os>-<arch>` file in the build's folder comes first (`holder-conpty-<os>-<arch>` for the Windows runtime folder); for the core's own platform, its own programs under their system's names come next; for any other platform the contract carries, the kit of this build's release.
From a folder a device gets exactly the runtime files its kit names, the files a release kit carries, and a folder short of one is refused with its name; the Windows kit's runtime is held to `contracts/conpty.json` by hide-node's contract test, which a change to the pin runs.
A build that is not a published release (a `-main.` or `0.0.0-` version, one without a commit, or one no package numbered, such as a plain `cargo build --release`, whose build order is 0) downloads nothing, and its refusal names the developer folder: `node desktop/scripts/device-helpers.mjs stage <device-bundles.json> <archive> <folder>` checks a release or CI kit against its manifest and unpacks it there under those names.

The connection worker fetches `https://github.com/with-hide/hide/releases/download/v<app-version>/device-bundles.json`, then the kit's archive, over HTTPS with a 60-second request bound and at most five redirects; the manifest is capped at 64 KiB and a kit at 256 MiB.
Before any archive it refuses a manifest whose version, commit or build order differs from the ones compiled into the build (`hided::cli::RELEASE`), whose format is not the contract's, or which does not name exactly the contract's platforms and each kit's files.
It checks archive size and hash, then exact entry names, regular-file types, sizes, file hashes and the programs' executable headers before atomically publishing a cache slot.

The cache path belongs to `hide_kit::layout::device_bundle_cache`: `~/.hide/cache/device-bundles/`.
It holds the lock file, the release's manifest, `recent` (the slots from most to least recently used), one staging folder for the download in progress and one slot per platform a device of this account needed; any other entry is refused, never removed.
A five-second process lock serializes cache access, and a kit's bytes are read before the lock is released, so another build cannot evict a kit while it is read.
Everything in it, the staging kit and its archive included, stays within 1024 MiB: a download that would pass it first removes the least recently used slots.
A fixed slot per platform would have held four 256 MiB kits beside a 256 MiB staging kit and its archive, 1.5 GiB, so the four platforms share the old two-platform limit instead.
A reconnect verifies and reuses its cached bytes offline; a slot of another build or with changed bytes is replaced, and a failed download leaves no usable partial kit.
Every exit explicitly cleans staging before releasing the cache lock; if cleanup fails, the device reason retains both failures and a structured diagnostic names the platform and release, while Drop makes a backup attempt.
Files in the cache are inert private data; upload gives the verified programs executable permission on the device.
Missing releases, network failures and verification errors reach the existing device failure and Retry path.
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
Its four required string inputs are `public_sha` (a full lowercase commit SHA), `tag` (`vX.Y.Z` or `vX.Y.Z-rc.N`), `verification_run_id` (the `pr.yml` push/main run of that exact source, pending, queued, running or passed), and `build_order` (a positive decimal integer no larger than 9007199254740991); `macos_runner` picks where [the Mac binaries](#the-candidates-mac-build) compile, `mini` unless the operator chooses `hosted`.
The source must already be an ancestor of main; a branch, fork, PR run or different source is refused before building, and so is a verify run that failed.
A verify run that still waits behind another main run (`pending`) is read again every 30 seconds, for up to 45 minutes, until it starts, and the job summary says how long `validate` waited; main merges queue their verify runs behind each other, so a fresh merge's run is usually pending when a release starts.
When the next push to main replaces that pending run, it ends `cancelled` and the candidate is refused, so the source stays a commit whose verify actually runs; a run still pending after 45 minutes refuses the candidate too.
The candidate does not wait for that verify run to finish: it may build while the source's push verify runs, and the publish, not the candidate, requires that the run it names concluded success.
The workflow revision is main's dispatch revision and may differ from the source revision it builds.
`package.yml`, `device-helpers.yml` and `agent-journey.yml` run their steps from that dispatch revision on the source's checkout, so the workflow refuses a source at which any of them or the phone deployment script differs from main; choose a newer source instead.

The workflow first uploads and confirms [the hosted phone app](#the-hosted-phone-release), stopping desktop preparation if that fails.
It then reuses `package.yml` for unsigned Linux and Windows installers, including their installation, packaged-window and update checks, and for the [device kits](#device-helper-release-bundles) a candidate makes: both Linux kits, built beside them on public runners, and the Windows kit from the Windows app's own compile.
Beside them it compiles the Mac app's release binaries on the Mac mini ([The candidate's Mac build](#the-candidates-mac-build)).
It runs the existing `agent-journey.yml` on the same source, with stand-ins on Linux and macOS, Windows form checks, and fake-model lanes on all three systems.
It signs no agent in: since 2026-10-27, by the operator's decision, a real login is measured only by the release's local run on the Mac ([CONTRIBUTING.md: Releases](../CONTRIBUTING.md#releases)), so the workflow summary records signed-in providers as **not measured**; a package failure blocks the receipt, and a stand-in or fake-model failure fails the run, which no consumer takes.
A successful candidate does not establish that any provider answered.
The receipt waits for the builds and not for the journey, which uploads none of the artifacts a release takes, so when only journey jobs fail, **Re-run failed jobs** reruns them alone and the run's successful latest attempt is selectable: the private publisher takes every artifact from the one attempt that made them, and refuses the run if any job other than an `agent-journey / ` job ran after it.
When a phone, package, helper, Mac binary or receipt job fails, use **Re-run all jobs** or dispatch again with the same inputs; a partial rerun can mix two attempts' artifacts and is refused.
The reusable workflows' optional `source-sha` defaults to the caller commit, and `package.yml`'s optional `build-order` defaults to its existing history-based ordering, so existing callers keep their behavior.
An explicit order allows separately prepared packages of one numeric version to share one build identity.
The operator assigns a strictly increasing order within that numeric version, including release-candidate to stable transitions; a private-only change takes a new patch version.

Only after the phone upload and confirmation, packaging, the device kits and the Mac binaries succeed does the workflow upload `hide-release-candidate`, containing only `candidate.json`; the run succeeds only when the journey passes too, and a consumer takes only a successful run:

```json
{"schema":5,"repository":"with-hide/hide","public_sha":"<source SHA>","tag":"vX.Y.Z","build_order":"<decimal>","verification_run_id":"<decimal>","workflow_sha":"<workflow SHA>","run_id":"<decimal>","run_attempt":1,"live_agents":false,
 "device_kits":{"linux-aarch64":{"asset":"hide-device-linux-aarch64-X.Y.Z.tar.gz","sha256":"<sha256>","size":<bytes>,"files":{"hide":{"sha256":"<sha256>","size":<bytes>},"…":{}}},"linux-x86_64":{"…":{}},
  "windows-x86_64":{"asset":"hide-device-windows-x86_64-X.Y.Z.tar.gz","sha256":"<sha256>","size":<bytes>,"files":{"hided.exe":{"sha256":"<sha256>","size":<bytes>},"holder-conpty/conpty.dll":{"…":{}},"…":{}}}},
 "macos_binaries":{"hide":"<sha256>","hide-agent-hooks":"<sha256>","hide-holder":"<sha256>","hided":"<sha256>"}}
```

The installer artifacts keep the names `hide-package-linux` and `hide-package-windows`, the kits `hide-device-bundles`, the helper artifacts `hide-device-helper-linux-x86_64` and `hide-device-helper-linux-aarch64`, and the Mac binaries `hide-macos-release-binaries`; candidate artifacts expire after seven days, except `hide-windows-release-binaries`, which only this run's jobs read and which expires after one.
All receipt fields describe public facts, with IDs and order as decimal strings and `schema` and `run_attempt` as integers.
Schema 2 adds the boolean `live_agents`, which records whether provider checks were requested, not their per-cell verdicts; since 2026-10-27 it is always `false`, kept so the private consumer that requires it reads the receipt unchanged.
Schema 3 adds `device_helpers`, each Linux platform's helper digests, recorded by the receipt job only after it checked both artifacts' manifests, executable headers and digests against `public_sha`, the tag's version and `build_order` (`withDeviceHelpers` in `scripts/release-candidate.mjs`); from schema 3, `verification_run_id` names the verify run without proving it passed.
Schema 4 adds `macos_binaries`, the digests of the release binaries (`hided`, `hide`, `hide-agent-hooks` and, since the pane holder shipped, `hide-holder`) compiled for macOS, recorded only after the compile artifact's `release-binaries.json` named `public_sha`, the tag's version and `build_order` and every binary matched it (`withMacosBinaries`).
Schema 5 replaces `device_helpers` with `device_kits`: the manifest entry (`asset`, `sha256`, `size`, `files`) of every kit the candidate made, read from the `hide-device-bundles` archives with the core's checks, and recorded only after every program in each matched the build its producer recorded for `public_sha`, the tag's version and `build_order`, a Linux helper's manifest or the Windows compile's `release-binaries.json` (`withDeviceKits`); the publisher copies those entries into the release's manifest unchanged.
Consumers retaining schema 1 support must treat its missing selection as the original live-enabled workflow, never as evidence that all providers passed.
A consumer verifies the successful manual main run, its trusted workflow revision, current attempt, matching receipt and artifact inventory, and that the verify run the receipt names and its `verify` job concluded success, before consuming it; a receipt alone proves neither GitHub origin nor that a package is signed.
Run evidence and reports remain workflow artifacts, separate from distributable files.
Release approval must disclose unmeasured provider coverage; it does not waive signing, installation, update checks or version approval.
Candidate preparation creates no public tag or Release; only the official publisher creates the approved release tag and publishes the final assets.

### The candidate's Mac build

The candidate's `compile (macos)` job runs `desktop package compile` on the source and uploads `hide-macos-release-binaries`: `target/release/{hided,hide,hide-agent-hooks,hide-holder}` and `desktop/out/release-binaries.json`, the record [`package.mjs assemble`](#the-desktop-app) checks before it packages them.
The private publisher downloads that artifact from the candidate run, requires each binary's SHA-256 to equal the receipt's `macos_binaries`, puts them where `assemble` reads them, and assembles, signs and notarizes the app; it compiles nothing.
Waiting for a hosted macOS runner took 53 minutes at the 90th percentile of 40 pull request verify runs (October 2026), so the job runs on the operator's Mac mini, a self-hosted runner of this public repository labelled `hide-candidate`, which no other workflow names (`scripts/tests/test_release_candidate.py`).

Both the mini and hosted jobs explicitly prepare managed Python 3.12 before dependencies, the Zig inputs and the web build, then require `python3` to import the standard `tomllib` module (Python 3.11 or newer).
The web typecheck runs `prepare:relay`, whose WASM build reads `Cargo.lock` with `tomllib`; the runner's system Python is not a build prerequisite.
The pinned `astral-sh/setup-uv` action installs the pinned uv version in `runner.tool_cache` and activates a private environment on PATH, using only managed Python.
Python, that environment, the download cache and the tool binary directory stay under `runner.temp`, and Actions cache restore/save is disabled.
The existing work-folder cleanup owns the uv binary, and the runner empties its temporary directory at the start and end of the job.
Unlike this relocatable distribution, [`actions/setup-python` on self-hosted macOS](https://github.com/actions/setup-python/blob/6d4fd22b20465ccc0f7354476588e8df7c9ca0e0/docs/advanced-usage.md#macos) requires a fixed hosted tool-cache installation outside this cleanup boundary, so the candidate does not use it or change account permissions to accommodate it.

A self-hosted runner of a public repository would run whatever a pull request's workflow asks of it, so the Mac mini takes only this job:

- **Only this workflow, on main.** The Team plan's runner groups can be limited to repositories but not to workflows, so the runner refuses the rest itself: its job-started hook (`scripts/candidate-runner/job-started.sh`) stops every job, before any of its steps runs, unless the job's repository, event, ref and workflow ref are this repository's `release-candidate.yml` dispatched on `refs/heads/main`. Failing the hook would still let the worker run `if: always()` and `if: failure()` steps and actions' `pre` steps, so the hook empties the work folder and kills the job's worker, its parent; the listener completes the job as failed and stays online. The worker dies before it uploads the job's log, so GitHub shows the job failed with no log; the refusal is in the Mac mini's system log, `log show --last 1h --predicate 'process == "logger"'`. Where the hook's parent is not the worker, as an update of the runner could make it, the hook cannot stop that job's steps alone, so it stops the whole runner: it sets the registration aside as `~hiderunner/actions-runner/.runner.stopped` and kills the runner's worker and listener, and the LaunchDaemon, which has no `KeepAlive`, starts nothing more, so the runner stays offline until its administrator restores it. A hook sees the job's own `github` context and never a workflow's `env:`, so no pull request, fork or branch push can name those values. Pull requests from outside contributors also wait for a maintainer's approval before any workflow runs.
- **Its own account.** The runner runs as `hiderunner`, a standard account with no login and a password no one knows, from a LaunchDaemon, so it needs no one logged in; it cannot read the operator's home. Its hooks are root's, under `/usr/local/libexec/hide-candidate-runner`.
- **Nothing kept.** The job-completed hook (`scripts/candidate-runner/job-completed.sh`) empties the runner's work folder after every job, whatever its result: the checkout and its release build, the cargo registry (the daemon points `CARGO_HOME` there), setup-node's Node and pnpm with its store. The account keeps only rustup and the toolchain the source pins; the hook removes any other.

`scripts/candidate-runner/setup.sh` sets the Mac mini up; its administrator runs `sudo bash setup.sh` from a copy of that folder once and pastes the registration token GitHub shows under the repository's Settings, Actions, Runners, New self-hosted runner.
A second run keeps the account and the registration, installs the hooks again and replaces the LaunchDaemon only when it differs; each run fails unless the administrator's own launchd services, runner configuration and Rust toolchain are as it found them.
To retire it, `sudo launchctl bootout system/dev.withhide.candidate-runner`, remove the runner from the repository's settings, then delete the account.
After the runner updates itself (its version shows under the repository's Settings, Actions, Runners), check once that the hook's parent is still the worker: push a temporary branch whose only job asks for `hide-candidate` and has two harmless `if: always()` and `if: failure()` echo steps, see the job fail with neither step started and the system log say `refused` and not `is not Runner.Worker`, then delete the branch.
To bring the runner back after the hook stopped it, read why in the system log, then run `sudo -u hiderunner mv ~hiderunner/actions-runner/.runner.stopped ~hiderunner/actions-runner/.runner` and `sudo launchctl kickstart -k system/dev.withhide.candidate-runner`.

When the Mac mini does not answer (a queued `compile (macos)` waits up to a day for it), cancel the candidate and dispatch it again with `macos_runner: hosted`, which compiles the same binaries on GitHub's `macos-15`, free for this public repository, and the receipt checks them the same way.
The private publisher's signing job has the same choice, but a hosted macOS minute of a private repository counts ten times against the plan and the organization's Actions budget is $0, so it starts only after the operator raises that budget.

## The Mac mini CI runner

GitHub's hosted macOS runners are the queue of a pull request's verify run: the organization runs five macOS jobs at once across every repository, and on 2026-11-01 verify runs waited 25 minutes and more with every macOS job queued while their Linux and Windows jobs had finished.
So the macOS jobs of verify that need no screen, keychain or login session run on the operator's Mac mini when the repository variable `HIDE_MACOS_CI_RUNNER` says so: `vt (macos)`, `omp SDK (macos)` and `compile (macos)`.
`os contract (macOS)` stays hosted because its launchd test installs a login agent in the runner account's own session and refuses any runner that is not GitHub's; `desktop e2e`, `package (macos)`, the agent journey, the performance lane and every release job stay hosted because they start the app or sign it.
Moving the three helped little: the jobs that kept pull requests waiting were the hosted ones that need a screen or a login session.
Since 2026-11-01 a pull request runs no macOS job unless it changes a macOS path ([TESTING.md](TESTING.md#where-the-macos-runners-went)), so the runner serves main's push runs and the nightly's `verify` call, which run every macOS job, and those few pull requests.

- **One switch.** `pr.yml`'s `plan` job reads the variable once, as its `self-hosted-macos` output, and passes it to `vt.yml`, `omp-contract.yml` and `package.yml`, whose macOS job runs on `fromJSON(<input> || '"macos-15"')`. The value is a JSON `runs-on`, `["self-hosted","hide-ci"]`; unset, every one of those jobs runs on GitHub's `macos-15` again. Turn the runner off with `gh variable delete HIDE_MACOS_CI_RUNNER -R with-hide/hide` and on with `gh variable set HIDE_MACOS_CI_RUNNER -R with-hide/hide --body '["self-hosted","hide-ci"]'`; the next run follows it, and nothing else names the label (`scripts/tests/test_ci_runner.py`). Other callers of those workflows (the nightly's own `package.yml` call, a release) never pass it and stay hosted.
- **Never a fork's code.** The `plan` output is empty for a pull request whose head repository is not this one, so a fork's run asks for `macos-15`. A fork's pull request runs the fork's copy of the workflow, which could name the label itself, so the runner's job-started hook (`scripts/ci-runner/job-started.sh`) also refuses, before any step, a job of another repository, a pull request event whose head repository is not with-hide/hide, and a job whose event file it cannot read; like the candidate runner's hook, it empties the work folder and kills the job's worker, and stops the whole runner when its parent is not the worker. Pull requests from outside contributors also wait for a maintainer's approval before any workflow runs.
- **Out of the operator's files.** The runner runs under the operator's account from the LaunchAgent `dev.withhide.ci-runner`, at `nice` 10 so the Hermes gateways on the same Mac come first; it is not an account boundary, which is what the candidate runner's account is. Each job gets a HOME, TMPDIR, rustup home, cargo home and hide-vt input folder inside `~/actions-runner-hide-ci`, so a step such as `git config --global` or a pinned toolchain install never touches the account's own.
- **Nothing kept.** The job-completed hook (`scripts/ci-runner/job-completed.sh`) empties the work folder (the checkout, its target folder, the cargo registry, setup-node's Node), the job's HOME and its TMPDIR after every job, whatever its result, and removes any Rust toolchain but the one the source pins.
- **One job at a time.** The Mac mini has 10 cores and 16 GB and shares a 228 GB disk, about 12 GB free on 2026-11-01, with the Hermes gateways and the release runners, and a release build takes about 10 GB; a second runner would let two such builds fill the disk.

`scripts/ci-runner/setup.sh` sets the runner up; the operator's account runs `bash setup.sh` from a copy of that folder, with a registration token from `gh api -X POST repos/with-hide/hide/actions/runners/registration-token --jq .token` pasted or passed as `ACTIONS_RUNNER_INPUT_TOKEN`, and a second run reinstalls the hooks and replaces the LaunchAgent only when it differs.
To retire it, delete the variable, `launchctl bootout gui/$(id -u)/dev.withhide.ci-runner`, remove the runner whose name starts with `hide-ci-` from the repository's settings and delete `~/actions-runner-hide-ci`.
To bring it back after its hook stopped it, read why with `log show --last 1h --predicate 'process == "logger"'`, then `mv ~/actions-runner-hide-ci/.runner.stopped ~/actions-runner-hide-ci/.runner` and `launchctl kickstart -k gui/$(id -u)/dev.withhide.ci-runner`.

## Release asset gate

Official release assets use a `vX.Y.Z` or `vX.Y.Z-rc.N` tag (`N` at least 1, no leading zeros) whose source commit is already an ancestor of protected `main`.
The tag is the one source of the release's kind: `releaseKind` in `scripts/check-release-assets.mjs` reads it for the workflow's tag check, the asset names, the draft's `prerelease` flag, every check of the draft, and the publish call, so no second input can disagree with it.
A `vX.Y.Z` tag is a stable release, published with `make_latest: "true"`; a `vX.Y.Z-rc.N` tag runs the same gates, the signed and notarized macOS build included, and is published as a pre-release with `make_latest: "false"`, which withhide.dev's download links ignore.
Its packages report the version without the `v`, such as `0.4.0-rc.1`, and are named for it, such as `hide-v0.4.0-rc.1-macos-arm64.zip`; the macOS bundle version and the Windows file version are the numeric prefix, `0.4.0`.
The publisher assembles all three systems' packages before `node scripts/release-draft.mjs <tag> <event-commit-sha> <directory>` prepares the matching draft.
The file-only entrypoint is `node scripts/check-release-assets.mjs <tag> <directory> [release-pages.json]`.
The directory must contain exactly that tag's twenty files (`expectedAssetNames`): the macOS ARM64 dmg and zip, the Windows x64 `setup.exe`, the Linux x64 deb and rpm, one SHA-256 sidecar per installer, `latest-mac.yml`, `latest.yml`, `latest-linux.yml`, the zip's and the setup's `.blockmap`, the device kit of every platform `contracts/device-kits.json` carries and `device-bundles.json`.
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

## Mobile relay WebAssembly

`bash scripts/build-relay-wasm.sh` builds `hide-relay-protocol` for `wasm32-unknown-unknown` from source.
The compiler comes from the root `rust-toolchain.toml`; the script installs that toolchain's WASM standard library and the exact `wasm-bindgen-cli` version matching `Cargo.lock`, with `--locked` CLI dependencies.
Compiler output, binding tools and their build output all stay inside this worktree's `target/`.
In CI the installed tool, `target/relay-tools`, comes from `.github/actions/relay-tools`, keyed by that `wasm-bindgen` version and the system and saved on main by one job per system, so a web job skips the minute the install compiles for.
`scripts/tests/test_relay_tools_cache.py` reads every workflow: a job that builds the web shell, compiles the desktop package or calls `build-relay-wasm.sh` (or a script that does) must restore the tool before its first such step, and only the two `web-e2e.yml` jobs save it.
The release candidate's `phone` job is the one exception and compiles the tool itself, because it publishes to production with every action pinned to a commit and the restore is an unpinned `actions/cache` tag.
The crate retains `rlib` and `cdylib` outputs to use one standard Cargo build path for native and WASM; native verification also links the small cdylib, while consumers link the rlib.
Generated browser bindings and `relay_bg.wasm` land in ignored `hide-relay-protocol/pkg/`; no compiled binary or third-party CDN script is committed.
The library is excluded from hakari's native workspace feature union because the union includes operating-system dependencies that cannot compile for WASM; its own direct dependencies and target features remain locked.

A phone bundle first runs that script, then imports `hide-relay-protocol/web/index.ts` and loads the generated local `relay_bg.wasm` asset with its exported initializer.
The phone loader owns asset URL resolution, HTTP MIME/cache behavior and startup errors; the library owns only the shared ABI.
The web E2E lane prepares the locked binding tool and WASM in a separate bounded step before the existing five-minute shell build, including on a cold Windows runner.
Both hided and the browser therefore run one cryptographic implementation and one parser.
The public checkout builds with the pinned Rust toolchain and registry dependencies, without a private binary, credential or design tool.
`bash scripts/check-relay-protocol.sh` builds and tests both targets and their interoperability; after workspace dependency installation, `bash scripts/verify-web.sh relay-protocol typecheck` checks the TypeScript surface against the generated bindings.

## The hosted phone release

The release candidate first builds and uploads its source commit's phone app to `https://app.withhide.dev/p/<PHONE_PROTOCOL>/`.
Only `.github/workflows/release-candidate.yml`, manually dispatched on protected `main`, can perform this upload; a pull request, tag or workstation does not deploy it.
The source is an exact commit already on main, built with the frozen dependency lockfile by `bash scripts/verify-web.sh web build:phone`.
The phone job uses Vercel's [prebuilt deployment](https://vercel.com/docs/cli/deploy#prebuilt) and [Build Output API](https://vercel.com/docs/build-output-api/configuration); the host does not rebuild source or add analytics.
This is a production side effect of preparing a candidate, including an RC or a candidate that later fails desktop checks.
It creates no desktop tag or Release and does not replace the operator's installed app.

The order is enforced by job dependencies: validate the main source, deploy and check the phone site, then prepare the desktop packages and Mac binaries, then write the candidate receipt.
No phone step continues on error, and the receipt has the phone job as a required dependency under GitHub's normal success condition.
Missing secrets, a failed build, a failed retention read, upload failure or failed production check prevents those downstream jobs from running.
The publisher in `with-hide/hide-private` requires the selected candidate's current run to have succeeded, so it cannot publish a desktop release after this phone job fails.
Before the first relay release, review and update the private `release/policy.json` candidate-workflow pin to the merged main workflow containing this gate, following that repository's `release/README.md`.
Do not dispatch an older pinned candidate workflow to bypass a missing phone deployment credential.

### Deployment credentials and host setup

`scripts/phone-release.mjs` owns the required, server-only secret contract; `scripts/phone-release.env.example` contains placeholders, never credentials and never a default target.
The public repository's `phone-production` GitHub environment supplies exactly these secrets:

| Secret | Value supplied by the deployment owner |
| --- | --- |
| `PHONE_VERCEL_TOKEN` | Vercel deployment token restricted to the phone project |
| `PHONE_VERCEL_ORG_ID` | The existing Vercel team or account ID |
| `PHONE_VERCEL_PROJECT_ID` | The existing phone project ID, separate from the introduction site |

All are required; the credential check names every missing or invalid key and stops without printing values, installing dependencies or contacting the host.
Only the credential check and retention step receive all three secrets; upload receives only the token, and public confirmation receives none.
The phone job's external Actions are pinned to reviewed full commit SHAs with version comments, and neither those Actions nor the dependency installation or phone build receives deployment secrets.
The deployment owner creates the project, domain and secrets, restricts `phone-production` to main, and gives the token to that environment only.
Keep Git integration and deploy hooks disconnected, Web Analytics and Speed Insights off, and production deployment URLs publicly readable so CI can retain the previous static files.
Attach `app.withhide.dev` to this project before the first run; production confirmation uses that exact address and fails if DNS or the domain points elsewhere.
Keep the host account and GitHub account protected by two-factor authentication.
Creating resources, changing DNS and installing secrets are deployment work requiring the operator's deployment authority, separate from implementing or testing this workflow.
The scripts neither create the project nor configure its domain or secrets.

### Retention and confirmation

Every candidate's phone job holds the same `phone-production` concurrency group, with cancellation disabled, across reading the old site, upload and confirmation.
It reads the project's current production deployment through Vercel's API, then downloads from that deployment's fixed URL rather than a moving custom-domain alias.
`phone-site.json` records the source commit and every static file's size and SHA-256.
Every old `p/<n>/` and root `assets/` file is checked against that inventory and copied into the new deployment.
The new build refreshes the loader and its own protocol's entry pages and adds its assets.
Root assets include the hashed WASM and lazy JavaScript modules an already-open loader can request when the person connects after a newer deployment.
Previously referenced assets stay even when the same protocol is built again, and reusing an asset name for different bytes is refused.
`protocols.json` is recomputed as the sorted union of all retained protocol directories, each of which must have an entry page.
Only a project with no production deployment starts empty; a missing inventory, missing old asset, inconsistent protocol list or digest mismatch is a failure, never a reason to discard old bundles.
The previous source must be an ancestor of the candidate source, preventing a delayed older candidate from replacing a newer phone app.
There is no age-based deletion and no reliance on expiring Actions artifacts for retention.
The assembled site is capped at 4,096 files and 256 MiB, with at most 16 MiB per file and 30 seconds per HTTP read; exceeding a cap stops the release and asks for a host-limit review, without pruning protocols.

The existing phone build's CSP is checked before upload and also installed as an HTTP header, with `frame-ancestors 'none'`, `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`.
Each retained protocol bundle under `/p/<N>/` is served with the policy its own page was built with, so a newer build that changes the policy leaves older bundles' headers matching their pages; the root and shared assets take the new build's policy, and the confirmation reads every page's header against its own page.
The one exception is each protocol's `look-frame.html`, the frame an HTML View is drawn in: its own policy, which must keep `default-src`, `connect-src`, `form-action`, `frame-src` and `object-src` at `'none'` and name no `'self'`, is checked before upload and installed as its header with `frame-ancestors 'self'`, and the confirmation reads that header back ([ARCHITECTURE.md](ARCHITECTURE.md#the-mobile-companion)).
Scripts come only from the same site, with the narrow WASM compilation allowance; connections go only to the same site and `wss://relay.withhide.dev`.
There are no external scripts or analytics in the phone app, and an analytics-enabled or Git-linked host project is refused.
After upload, CI reads the inventory, protocol union and every listed file from `app.withhide.dev`, requiring the exact source, bytes and CSP headers before desktop preparation begins.
A failed confirmation can leave the newly uploaded phone site online, but the desktop release remains stopped; repair the cause and retry from main instead of publishing past it.
The 30-minute job limit bounds the whole operation, and Vercel upload has its own ten-minute limit.
The upload pins the npm-published Vercel CLI 65.10.0, whose deploy path accepts a project-scoped token when the project's GET succeeds but the owner team's GET returns `403 team_unauthorized`.
CLI 50.1.3 rejected that combination before upload with "Could not retrieve Project Settings".
Preparation still verifies the project's `id` and `accountId` before writing `.vercel/project.json`; no team-wide permission or Git integration is required.

`python3 -m unittest scripts.tests.test_phone_release scripts.tests.test_release_candidate` checks the release dependency graph, secret contract and real filesystem assembly against fixture Vercel HTTP responses.
It covers retained old files, same-protocol assets, the protocol union, missing secrets, upload prerequisites, refused rollback, corrupt or missing history, unsafe CSP and failed production confirmation, without any live upload.
The opt-in real-CLI regression installs no dependency itself and needs no credentials.
Install the two npm releases into ignored run directories with Node 22, then run:

```sh
npm install --prefix agents/runs/mission-27-phone-deploy/cli-new --ignore-scripts --no-audit --no-fund vercel@65.10.0
npm install --prefix agents/runs/mission-27-phone-deploy/cli-old --ignore-scripts --no-audit --no-fund vercel@50.1.3
PHONE_RELEASE_VERCEL_CLI="$PWD/agents/runs/mission-27-phone-deploy/cli-new/node_modules/vercel/dist/index.js" \
PHONE_RELEASE_OLD_VERCEL_CLI="$PWD/agents/runs/mission-27-phone-deploy/cli-old/node_modules/vercel/dist/index.js" \
python3 -m unittest scripts.tests.test_phone_release.PhoneReleaseCli
```

The fixture executes the workflow's deploy arguments against a loopback API using the CLI's `--api` option, with a private home and repository and a synthetic token.
It runs real preparation, verifies the current CLI matches the workflow pin, reproduces the old CLI's team-403 failure, and checks the new CLI uploads exactly the prepared file bytes and exits successfully.
A missing project, a different team error, and an upload refusal must still exit unsuccessfully; the ordinary workflow tests keep upload failure and confirmation ahead of packaging.
Without both installed CLI paths this external-binary case is explicitly skipped, while the ordinary release contract tests still run.
The existing phone browser journey checks that CSP blocks an external script.
These checks prove the release contract, not actual credentials, hosting, DNS, a live upload or a physical phone; those remain the deployment node's acceptance checks.

## The iOS app

`mobile/` is the Capacitor host that carries the phone web app inside an iOS app (bundle id `me.grab.hide.ios`, team `GH6F6ZA497`, iPhone only, iOS 15 or later, the lowest Capacitor 8 supports); [ARCHITECTURE.md](ARCHITECTURE.md#the-ios-app-host) owns what it is.
Xcode does not run on every Mac this repository is built on, so the app builds on the Mac mini and nothing about the build needs node or Rust there.

`bash mobile/scripts/build-ios.sh` is the one command.
It runs, on this machine:

1. `bash scripts/verify-web.sh web build:phone`, the same phone build the hosted site uses (`HIDE_IOS_SKIP_PHONE_BUILD=1` reuses `web/phone-dist`, for a run after only Swift changed);
2. `mobile/scripts/check-versions.mjs`, which refuses a Capacitor version that differs between `mobile/package.json`, `CapApp-SPM` and `HideShell`, and a plugin list (`BridgeGate.appPlugins`) that differs from the plugins the package defines;
3. `mobile/scripts/assemble.mjs`, which fills `mobile/www/` (ignored): the loader and the current protocol's bundle from step 1, and every older `p/<n>/` bundle that `mobile/retained-bundles.json` pins, then `protocols.json` as their union and no `sw.js`.
   The build trusts the pin and nothing the server says about itself: each pinned file is read from `https://app.withhide.dev` and must match its pinned size and SHA-256, so a file that differs, a pinned file or protocol the site no longer serves, a protocol the site serves that the pin lacks, or a page whose content policy is not the phone site's stops the build with the file named; the app never ships fewer bundles than the pin lists without someone changing the pin.
   The protocol this source speaks is never pinned: it comes from step 1;
4. `cap sync ios`, which copies that folder into the Xcode project;
5. an `rsync` of `mobile/ios/` to `~/hide-ios-build/<branch>/ios/` on the machine named by `HIDE_IOS_BUILD_HOST` (default `mini`), where `mobile/scripts/mini-ios.sh` is piped over `ssh` to run `xcodebuild` for the simulator with derived data and the Swift packages inside that folder, then two test runs on an iOS Simulator: the `HideShell` package's tests, and the `AppTests` target hosted in the app, whose test drives the app's own `ShellViewController` and proves a call from a sandboxed or same-origin frame never reaches a plugin while the main frame's does (delete the line that installs `BridgeGate` and it fails).
   A failing test fails the command.

`bash mobile/scripts/build-ios.sh run` installs the built app in a simulator of its own on the mini (`hide-ios-<branch>`, created if missing), launches it, waits `HIDE_IOS_SETTLE` seconds (default 15) and captures the screen.
With `HIDE_IOS_PAIR_LINK=<pairing link>` the debug build follows that link at launch and presses Connect, since a simulator has no camera (the hook is compiled out of a release build); the link travels on the standard input of the script, in front of it, so it is in no argument and in no file on the mini, and there is nothing to remove.
A pairing code lives five minutes and only while a Mobile page of that Hide stays open, so the page that showed it (or a client that sent `mobile_observe`) must still be connected when the simulator launches.
`HIDE_IOS_TAPS='<css selector>;;<css selector>'` presses those on the page, in order, once each is there (`[data-phone-host]` the saved computer, `[data-phone-tab="settings"]`, `[data-phone-setting="language"]`, `[data-phone-add]`, `[data-phone-scan]`), `HIDE_IOS_APPEARANCE=light|dark` sets the simulator's appearance, `HIDE_IOS_HOME=1` captures the home screen with the icon, and `HIDE_IOS_LAUNCH_SCREEN=1` holds the app on its launch screen (it starts and waits for a debugger that never comes) and captures that.
The taps, like the link, are read by the debug build only.
`HIDE_IOS_CAPTURE=<file.png>` copies the capture back; otherwise it is `~/hide-ios-build/<branch>/shots/run.png` on the mini.
`bash mobile/scripts/build-ios.sh clean` deletes that simulator and the branch's whole folder there.

Everything a command starts it ends: the simulator it created is shut down and deleted, and one it found off is shut down again, on every exit including a failure (a trap in `mini-ios.sh`).
`HIDE_IOS_KEEP_SIMULATOR=1` leaves the simulator booted for a check by hand and marks it; the next command of the branch that does not ask to keep it ends that simulator too, so `clean` is not the only way it goes.
The Swift tests of a `build` run on the branch's simulator and clear the pairing Keychain items they test, so a pairing kept for a check does not survive a later `build`: pair again after it.
The mini runs other work, and nothing prunes a branch it does not own, so each branch's state is one folder of about 0.5 GB (`~/hide-ios-build/<branch>/`) plus one simulator; once 4 other branches hold a folder there, the build refuses before it copies anything and prints the folders.
Run `bash mobile/scripts/build-ios.sh clean` from those branches, or `rm -rf` a folder it names after `xcrun simctl delete` of its `hide-ios-<branch>` simulator, then retry.
The build prints the machine's free disk when it ends.

`bash scripts/verify-web.sh mobile test` runs the node tests of `mobile/` (the assembler against a pin that differs or a protocol that is missing, the pin generator, the Capacitor version check, which also refuses a plugin package or product the app links without the fixed plugin list changing, and the release gate below), which `pr.yml`'s checks lane runs.
The Swift tests need Xcode, so only the build command above runs them; [Swift tests are not in CI](#swift-tests-are-not-in-ci) says why.
The simulator build is signed ad hoc (`CODE_SIGN_IDENTITY=-`, no account or certificate), not left unsigned, because the simulator gives the Keychain only to an app that carries its entitlements and answers -34018 to an unsigned one; [The release build](#the-release-build) signs and uploads one with the team's own certificate.

### The release build

`bash mobile/scripts/build-ios.sh release` is the one command that makes a signed build of the app and uploads it to TestFlight.
It refuses a working tree with tracked changes, `HIDE_IOS_SKIP_PHONE_BUILD` (a release builds the phone bundle from the commit it uploads) and a commit that already has an upload record ([Uploading a commit twice](#uploading-a-commit-twice)); it runs everything `build` runs (the phone build, the pin check, `cap sync`, the Swift tests on the mini, and a failing test stops it before anything is signed), and then, on the mini (`mobile/scripts/mini-release.sh`, piped over `ssh` like `mini-ios.sh`):

1. takes the machine's release lock and reads the secrets, checking all of them ([secrets](#the-releases-secrets));
2. archives the app in the Release configuration with signing off, with the version and build number given on the command line (never written into the repository);
3. exports the archive with automatic signing, which makes the App Store profile and signs with the team's cloud-managed Apple Distribution certificate, so no distribution private key exists on the mini and no keychain is read or changed;
4. unpacks the signed ipa, reads its property lists and signature, and runs the release checks ([Release checks](#release-checks)); one failed check stops the command with every failure named, before anything is uploaded;
5. validates and uploads that same ipa with `xcrun altool --validate-app` and `--upload-app`, then writes the upload record and prints the build's state once with the delivery id Apple gave the upload. `HIDE_IOS_NO_UPLOAD=1` stops after the validation, before anything is sent to TestFlight, which proves a change to the command without making a build.

It exits non-zero with the reason at the first failure (a step outside the tools names itself), prints the end of the log once the log has been searched for a secret, and ends each long step that outlives its bound (archive 20 minutes, export and validate 15, upload 30; a whole run took about 6).
Each step runs in a process group of its own, so a bound or a signal ends it with everything it started.
A whole run took about 6 minutes and the mini's free disk fell by about 1 GB (2026-11-10: 24 GB to 23 GB), most of the time in the Swift tests, Apple's validation and the upload.
The command stops before it starts only when the mini has less than 5 GB free, a few times what a run uses, so a run cannot fill the disk under the mini's other work; a run that runs out of space anyway fails at the step that ran out.

**Version and build number.**
The version is the numeric part of the repository's own (`HIDE_VERSION`, else the newest `v<version>` tag, as `desktop/scripts/build-identity.mjs` resolves it for the desktop app), so `0.4.0-rc.1` and `0.4.0-main.<commit>` are both `0.4.0`.
The build number is the UTC time of the machine that starts the command, taken after the Swift tests and just before the mini starts, `YYYYMMDDHHMMSS`: it rises with every run, a second run on the same commit uploads under a new number, and nothing is edited by hand.
App Store Connect refuses a build number that is not higher than every earlier build of the same version, so two runs that overlap are decided by the order they upload in, and a mini whose clock runs backwards is refused after the upload with Apple's own message.
A run that fails before the upload, or whose upload Apple refused, is run again as it is.
`bash mobile/scripts/build-ios.sh status <delivery id>` reads an upload's state again (`altool --build-status`, with the key from the same files) and changes nothing; it exits non-zero when altool fails or does not answer in 5 minutes.
The version is the last tag's until a release is tagged, so a run between releases uploads another build of the last version: once Apple has approved a version, a run from main that still carries it is refused by Apple's validation after the whole build, and the next tag or `HIDE_VERSION` is the way out.
Builds made between releases are development builds; expire them in App Store Connect before an external group or a review sees the build list.

#### What a release owns

`mini-release.sh` lists, in one place, everything a run creates, and one cleanup removes exactly that list on a normal end, a failed step, TERM, HUP or INT, and a closed standard output:

| What | Where | How it goes |
| --- | --- | --- |
| The machine's release lock | `~/hide-ios-build/.release.lock` | held by the kernel (`flock`) on a descriptor of the running shell, so it ends with the shell however that ends; its text (pid, start time, branch) only names the holder, and is emptied |
| The work folder | `~/hide-ios-build/<branch>/release/` | removed: the archive, derived data, the archive's result bundle and the one the export is given, the export, the unpacked ipa, the temporary folder and the private home the tools that reach App Store Connect run with |
| xcodebuild's distribution logs | `App_*.xcdistributionlogs` and `XcodeDistPipeline.*` in the account's temporary folder | deleted unread, by name, those made since the run started |

A dropped ssh connection sends the remote no hang-up and closes its output; the script ignores SIGPIPE, so its next write fails and it ends through the same cleanup, which never stops on its own output.
The step that was running when the connection dropped finishes first (an upload goes, and is recorded).
The lock is one per machine, not per branch, because what it guards is: the account's temporary folder, the disk floor, the Apple account and its build numbers.
While it is held a second release, of any branch, is refused naming the holder, and a `build` or `clean` of the holder's branch is refused too; `status` and `uploads` take no lock.
Only the lock's "held by another" answer reads as a running release; a lock that cannot be taken or read (no `perl`, a file that will not open) stops the command naming that.
The lock is taken when `mini-release.sh` starts, after `release`'s own build phase on the mini (the `ios/` copy, the Debug build and tests, the `release-inputs/` copy), which `busy` checks but does not hold: a second `release` or a `build` of the same branch started during that phase can still copy another tree under the first (a known gap; closing it means archiving from a copy made under the lock).
A run ended by SIGKILL or a restart skips the cleanup and leaves `release/`, whose private home holds altool's log (the key id and path) and xcodebuild's URL cache (tokens that expire in 20 minutes), until the next release of that branch or `clean` removes it; nothing else can run then to remove it.
xcodebuild writes its distribution logs, which hold a signed token and the key's ids, into the account's temporary folder, which it takes from the system (confstr) and not from `$TMPDIR` or the home, so they cannot be put in the work folder; the machine lock is what makes deleting them by name safe, and the one case it does not cover is an export another program starts on the mini during a release.
`xcodebuild -exportArchive`, `altool --validate-app`, `--upload-app` and `--build-status` run with `CFFIXED_USER_HOME` set to the work folder's private home: altool's logs (its command line with the key id and path) and xcodebuild's URL cache (signed App Store Connect tokens) land there and go with it.
The home's `Library/Caches` is made before the tools start: without it CFNetwork puts the URL cache in the account's cache folder (`getconf DARWIN_USER_CACHE_DIR`) instead, which a run on the mini showed.
On the mini (2026-11-10, commit 955d040723) the export still signed with the cloud certificate and got its App Store profile, and Apple's validation passed, that way; afterwards `~/Library/Logs/ContentDelivery/com.apple.itunes.altool/`, `~/Library/Caches/com.apple.dt.xcodebuild/`, the account's provisioning profile folders and the account's cache folder were unchanged, and no altool URL cache appeared in the cache folder.

What stays on the mini after a run: `~/hide-ios-build/<branch>/release.log`, redacted and searched; `~/hide-ios-build/uploads.log`, the upload record; the empty lock file (a lock file is never deleted, since a run that opened the old one and a run that made a new one could both hold a lock); and in the account's temporary folder (mode 700) what Apple's frameworks leave under names any program uses, so they are not deleted by name: per run about 20 to 30 `CFNetworkDownload_*.tmp` files of 1 to 2 KB from the export and validation, 3 `cms~~~*` files of about 5 KB, and 5 `ResultBundle_*` folders of 28 to 100 KB from the export, which writes these beside the one `-resultBundlePath` names.
On the mini none of those holds the key id or a token (counted, not read, 2026-11-10); nothing in this command removes them, and macOS clears the temporary folder of old files on its own schedule (the day's earlier runs' files were still there that evening).
`clean` removes the branch folder.

#### Uploading a commit twice

The mini's `~/hide-ios-build/uploads.log`, one for the machine, gets a line when an upload starts and another when it ends: time, commit, version, build number, delivery id (`started` has none yet), state and branch.
A caller whose connection dropped, which sends the remote no hang-up, may not know whether the upload went, and `build-ios.sh` then says to read the record; `bash mobile/scripts/build-ios.sh uploads` prints it, `status <delivery id>` asks Apple, and a `started` line with no `uploaded` line after it means look in App Store Connect before running again.
`release` refuses a commit that has any line in the record, under any branch name (main after a merge, a renamed branch), before the build and again on the mini, and says which; `HIDE_IOS_RELEASE_AGAIN=1` uploads it again on purpose (a new build number, the same commit).

**Why altool uploads.**
Xcode 26.6 also uploads from `xcodebuild -exportArchive` with `destination` set to `upload`, but that signs again while it uploads, so the bytes that were checked are not the bytes that went.
`xcrun altool --upload-app` (altool 26.40.1) sends the exact ipa the checks read, and `--validate-app` runs Apple's own validation on it first.
Apple's move from altool to notarytool is for notarization; the upload and validate commands are in 26.40.1's help.
Both tools put the key's id and issuer id on their command line (visible to the same account's `ps` on the mini) and read the key file by path; the id, the issuer id and the path are not the key, and the file's body is never an argument.

#### The release's secrets

Three values, registered in `mobile/release-env.json` (the key, its shape, and what a missing one costs), shown by `mobile/release.env.example`, and held equal by a test.
They are the App Store Connect Team API key's id, its issuer id and the path of its `.p8` file.
They live on the mini in `~/.config/hide-ios/` (mode 700): `AuthKey_<key id>.p8` and `release.env` (both mode 600), and in no checkout and no GitHub secret; the same three names set in the mini's environment work without the file.
The key needs the Admin role, since an App Manager key cannot make the cloud-managed distribution certificate and profiles.
The command checks every key before it builds, in one report: a missing key, a value of the wrong shape, a key file named for another key id or readable by another account each stop it, naming the key and the kind of problem and never a value.
A line of `release.env` that is not a registered name is refused, and the file is read as `KEY=VALUE` lines, never run.

The key file is read by Apple's tools from its path and by nothing else: the command checks its name, owner and mode and never opens, copies or hashes it, and never passes its body anywhere.
The ids and the path reach xcodebuild and altool as arguments (they are not the key); every pattern the command searches or replaces them with travels in a pipe, never in a file or another program's arguments.
Nothing runs under `set -x`, and every line xcodebuild and altool print passes through a filter that replaces the three values, in any letter case, with the names of their keys, and a whole token with `<token>`, before it reaches the log or the terminal.
Before a failed step shows the end of its log, and at every exit, the log is searched for the three values in any letter case, for anything shaped like a token or part of one (`eyJ` and a JSON header) and for a PEM private key header; if one is there the log is deleted, the end of it is not shown and the command fails.
The search cannot see a key's body printed without its PEM header; nothing in the command has a path to print it, since only Apple's tools read the file.
`mobile/scripts/mini-release.test.mjs` runs the script with stand-ins for the Apple tools and the macOS commands it asks questions of (so it runs in the `checks` lane, not only on a Mac): the stand-ins print their arguments and a token and write distribution logs where the real export does, and the tests prove that the script never reads the key file (it runs with the file unreadable), no copy of the key exists while the export runs, no value is in the arguments of a `sed` or `grep`, the log and the output hold none of the values, and the run leaves nothing behind after a failed export, a step outside the tools, a time bound (with the step's grandchildren), TERM during the export and a standard output closed mid-run; and that a second branch's release is refused while one runs, a lock whose text names a live process the kernel does not hold the lock for is not believed, and the upload record refuses the same commit under another branch name.
Rotating a key is replacing the two files; nothing in the repository changes.

#### Release checks

`mobile/scripts/check-release-app.sh` reads the signed app, not the project, so a setting that never reached the binary cannot pass.
It lists every failed check and the upload never starts after one.
`mobile/scripts/check-release-app.test.mjs` spoils each fact in turn and requires its check to fail; the signature's Apple Distribution authority is checked by `mini-release.sh` and tested in `mini-release.test.mjs`.

| The signed app | Must be |
| --- | --- |
| Identity | bundle id `me.grab.hide.ios`, the version and build number the command passed, iPhone only, minimum iOS 15.0, built with an iOS 26 SDK (Apple's upload rule since 2026-04-28) |
| Export compliance | `ITSAppUsesNonExemptEncryption` is in the app and `false`, and no `ITSEncryptionExportComplianceCode`; the value rests on the operator's two answers in App Store Connect's questionnaire (the app uses standard encryption algorithms, and is not distributed in France), so a build no longer waits at Missing Compliance; answer them again, and change the value, if the encryption method changes or distribution in France or the EU starts (a `true` without the code Apple issues after reviewing documents is refused at validation, error 90592) |
| Debug left out | Capacitor `loggingBehavior` none, `CAPACITOR_DEBUG` not on, no App Transport Security exception, no `server.url` or cleartext, no allowed navigation, web view inspection off, the debug pairing hook's name (`HIDE_DEBUG_PAIR_LINK`) absent from the binary and `BridgeGate` present in it |
| Privacy manifest | `PrivacyInfo.xcprivacy` in the app: no tracking, no tracking domains, nothing collected, and a reason for every required-reason API it declares; Capacitor 8.6.0's own two frameworks declare none and call none (read from the archive's symbols on 2026-11-10); the app's own code reads `UserDefaults` for the first-run marker that clears a Keychain item left by an earlier install, so its manifest declares `NSPrivacyAccessedAPICategoryUserDefaults` with reason `CA92.1`, and a node that adds code calling another required-reason API adds its declaration in the same change |
| Signature | signed by an Apple Distribution certificate, valid with `--strict --deep`; the profile is an App Store profile of team `GH6F6ZA497` for iOS, with no device list |
| Entitlements | exactly the list in `mobile/release-expect.json`; a node that adds a capability (`aps-environment` production with push, Associated Domains with universal links) adds its entitlement to that list in the same change, and a build with an entitlement nobody listed fails |
| Web bundle | every file of `public/` is in the manifest `mobile/scripts/bundle-manifest.mjs` made after `cap sync` (the assembled folder, every retained file as pinned, and the two Cordova files Capacitor adds), with its SHA-256, and none is missing |

The Release configuration of the project itself sets `CAPACITOR_DEBUG = false` and carries no `DEBUG` condition; the Debug configuration alone reads `ios/debug.xcconfig`.
The app icon is the placeholder the project carries until the native node's icon lands; App Store Connect's processing is what judges it.

#### Swift tests are not in CI

The Swift tests (`HideShell` and the app's own controller) run on the mini in `build` and again at the start of `release`, and nowhere else.
What closes this is a macOS lane for the paths under `mobile/ios/`: a `MACOS_PATHS` entry and its job in `scripts/ci-plan.py`, a lane in `pr.yml` and a runner for it, with the mini's `hide-ci` runner ([The Mac mini CI runner](#the-mac-mini-ci-runner)) the candidate because a hosted macOS runner queues for 53 minutes at the 90th percentile ([TESTING.md](TESTING.md#where-the-macos-runners-went)).
It also needs the operator's decision to share the mini's disk and one job at a time with a simulator build, and a change to a workflow plans every lane, so it is a separate change and not part of making the upload.
The release gate's own tests (the check script, the manifest, the secrets) are node tests and run in the `checks` lane with the rest of `mobile/`.

#### Later: from the desktop release

The release workflow of the private repository can run this command after it publishes a desktop release: `HIDE_VERSION=<tag> bash mobile/scripts/build-ios.sh release` on the operator's Mac, which reaches the mini the same way and needs the same three files there.
A workflow on the candidate runner is not possible as it stands: that runner accepts only `release-candidate.yml` on main, runs as an account with no access to the key, and a hosted macOS job would need the key as a GitHub secret.
The protocol order in [Retained bundles and the pin](#retained-bundles-and-the-pin) applies first: build the app from the main that carries the new protocol's bundle, with the previous one pinned.

#### When a node adds an extension or a capability

The release gate reads only the app. A node that adds a notification service extension adds, in the same change: its `aps-environment` (`production`, which the export turns the project's `development` into) in `mobile/release-expect.json`; the extension signed first with its own entitlements file before `embed entitlements` signs the app, which signs `App.app` alone; the same checks for each `PlugIns/*.appex` in `check-release-app.sh` (a bundle id under `me.grab.hide.ios.`, the app's version and build, its exact entitlements with no `get-task-allow`, an App Store profile of the team, the Apple Distribution authority, and a privacy manifest of its own when it reads `UserDefaults`); one manifest per target in the privacy-manifest test; and the extension's App ID capabilities (app group, keychain sharing) registered in the developer account, since automatic signing creates its second profile but not its capabilities.

### Retained bundles and the pin

An older phone protocol's bundle stays in the app so a desktop that still speaks it keeps working.
`mobile/retained-bundles.json` is the one place that says which: for each retained protocol, every file's path, size and SHA-256.
A human reads its diff in the pull request that changes it; nothing else decides what the app carries.

To change it, run `pnpm --dir mobile pin` from a checkout with network access.
It reads `https://app.withhide.dev` (`phone-site.json` and then each file of each protocol older than this source's), requires the downloaded bytes to agree with the site's own inventory, and rewrites the file with one line per file.
Open the diff and check that only what you meant moved: a protocol added when a new one was released and the previous one was retired from the site's `current`, a file whose hash changed only when you rebuilt a retained bundle on purpose, and no line removed unless a protocol is being dropped from the app.
Commit it on its own, so its review is only that.

The bundles in the app are chosen when the app is built, and the app never fetches code when it runs.
A desktop that speaks a newer phone protocol than any bundle inside the app makes the loader show its version screen worded for the app ("Update the Hide app"), and only a new app release clears it.
So a protocol bump and an app release take this order: raise `PHONE_PROTOCOL` and release the desktop, whose candidate uploads the new `p/<n>/` to the hosted site; build the app from that main (the assembler picks the new bundle up from source, and the previous protocol is pinned in a pull request of its own with `pnpm --dir mobile pin` before the app is built); submit it and wait for approval; and only then ship the desktop that speaks the new protocol to users, because until the app is approved an iOS phone stays on the version screen.
Raising a protocol number for an addition is not needed ([ARCHITECTURE.md](ARCHITECTURE.md#the-mobile-companion)), which keeps this order for the rare change an older side would misread.

## Mobile relay server

Installing, updating and operating the production relay is owned by a private operations document, not this guide.
The manual `relay-artifact.yml` producer builds only the relay from its exact protected-main dispatch SHA and retains source/run and SHA-256 evidence; it signs nothing and deploys nothing.

`bash scripts/verify-cargo.sh build -p hide-relay --bin hide-relay` builds the standalone memory-only server into `target/debug/hide-relay` (`.exe` on Windows).
Run `target/debug/hide-relay --listen 127.0.0.1:8787` for local clients; `/ws` accepts WebSockets and `/health` answers the uptime check.
For a deployment binary, use `bash scripts/verify-cargo.sh build --release -p hide-relay --bin hide-relay`, which keeps output in this worktree's `target/release/`.
It is a separate service and is not packaged into the desktop app.
`hide-relay --help` lists its entire configuration surface: a literal bind address and an optional trusted reverse-proxy IP.
It reads no environment override, configuration file, database, persistent state or credentials.
Its CLI parser is `hide-relay/src/config.rs`, budget authority is `limits.rs`, and [RELAY_PROTOCOL.md](RELAY_PROTOCOL.md) defines forwarding, proxy trust, log collection and reconnect semantics.
Ctrl+C on every system, or SIGTERM on Unix, drains the server; the operating service manager owns restarting it.
`bash scripts/verify-cargo.sh test-scoped -p hide-relay` starts private loopback listeners and real protocol clients and stops them on every exit path.
