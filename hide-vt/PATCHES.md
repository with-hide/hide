# hide-vt: where it comes from and what Hide changed

hide-vt is Herdr's `crates/ghostty-vt`, the Rust binding to libghostty-vt, taken from Herdr v0.9.5 (commit `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`) and renamed.
The Rust sources are copied into this crate; libghostty-vt itself is not, and its build downloads it from the same Herdr release.
`build-inputs.json` pins every downloaded input by digest, and docs/BUILD.md, "The terminal parser and Zig", owns how they are prepared and built.

## Upstream versions

| Input | Version | Pinned by |
| --- | --- | --- |
| Herdr | v0.9.5, commit `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01` | the source archive's sha256 in `build-inputs.json` |
| libghostty-vt | source dist `libghostty-vt-1.3.2-HEAD-+44f2a44df` of Ghostty commit `44f2a44df7e8c4a0c6df3f7d872ef3d7ead88e51`, as Herdr vendors it in `vendor/libghostty-vt` | the same archive; its `vendor/libghostty-vt.vendor.json` is pinned by sha256 too |
| Zig | 0.16.0 | each system's archive sha256, from ziglang.org's download index |
| Zig packages | the eight libghostty-vt's build reads | Zig's own content hash of each, in `build-inputs.json` |

## Herdr's patches to libghostty-vt

Herdr's tree is the source dist with these patches already applied; Hide applies none of its own.
`scripts/prepare-vt-inputs.py` checks each patch's sha256 and that it applies in reverse to the extracted tree, so a tree that is not the dist plus exactly these patches is refused.
Herdr's `vendor/libghostty-vt.patches.md`, pinned in the same archive, gives each one's reason, files and removal condition.

| Patch | What it adds |
| --- | --- |
| `0002-expose-modify-other-keys-mode.patch` | a terminal-data query for xterm modifyOtherKeys mode 2 |
| `0004-fix-hosted-wuffs-builds.patch` | Wuffs builds for Windows targets and hosted non-SIMD builds |
| `0005-bounded-word-selection.patch` | a bounded, wrap-aware word selection for link activation |
| `0006-clear-screen-preserving-cursor-line.patch` | a screen clear that keeps the cursor's line |
| `0007-experimental-png-retention.patch` | an opt-in mode that keeps encoded PNG uploads |

## Hide's changes to Herdr's crate

Each source file starts with its origin, Herdr's copyright and license, and what Hide changed in it.

| File | Change |
| --- | --- |
| `Cargo.toml` | Renamed `ghostty-vt` to `hide-vt`; `tracing` without its default features; `serde_json` and `sha2` for the build script; the edition stays Herdr's 2021 so the sources stay as Herdr wrote them. |
| `build.rs` | Reads the source and Zig from the prepared inputs folder instead of `vendor/`, checks the Zig version before building, runs Zig with `--system` so it fetches nothing, keeps Zig's caches in the worktree, and drops Herdr's `LIBGHOSTTY_VT_OPTIMIZE`, `LIBGHOSTTY_VT_SIMD`, `LIBGHOSTTY_VT_ZIG_SYSTEM_DIR` and `LIBGHOSTTY_VT_WINDOWS_LIBC` switches, which no Hide build sets. |
| `src/lib.rs` | Reads the kitty placeholder table from the prepared source (`HIDE_VT_SOURCE_DIR`, set by the build script) instead of a path into `vendor/`; Herdr's name is gone from comments and test paths; two pixel loops use `as_chunks` and `kitty_placement_payload` carries a reasoned `too_many_arguments` allow, for this repository's Clippy (Herdr allows eleven arguments). |
| `src/pane_graphics_files.rs` | The log target is `hide_vt::pane_graphics_files`, and the runtime folders are `hide-vt-native-sources-<uid>` and `hide-vt-pane-graphics-<uid>`. |
| `src/native_source_tests.rs` | The test folder name no longer carries Herdr's name. |
| `src/bindings.rs`, `src/native_image_sources.rs`, `src/native_source.rs`, `src/png_forward_tests.rs`, `src/test_allocations.rs` | Unchanged but for the header. |
| `tests/it/` | Hide's own: the contract a holder relies on, on every system. |

## Moving to a newer Herdr

Change `build-inputs.json` to the new release's archive, its digest, its patch list and the packages its libghostty-vt build reads, compare Herdr's `crates/ghostty-vt` with these sources, and carry Hide's changes above across.
The packages a build reads are found by building once with `--system` on an empty folder: Zig names each missing package, and its `build.zig.zon` entry gives the URL.
