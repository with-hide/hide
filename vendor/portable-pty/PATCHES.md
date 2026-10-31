# portable-pty: where it comes from and what changed

This is `portable-pty` 0.9.0 from crates.io (part of wezterm, Copyright (c) 2018 Wez Furlong, MIT, `LICENSE.md`), taken as Herdr v0.9.5 vendors it (commit `2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`, `vendor/portable-pty`), with Herdr's four patches applied, plus Hide's own changes.
The pane holder (`hide-holder`) is the only crate that uses it; the root manifest's `[patch.crates-io]` puts this copy in place of the published crate, so it is not a workspace member and keeps its upstream edition.
Each source file starts with its origin, its license, the Herdr patches it carries and what Hide changed in it.

## Herdr's patches

Herdr's `vendor/portable-pty.patches.md`, in the same release, gives each one's issue, reason and removal condition.

| Patch | Files | What it does |
| --- | --- | --- |
| `0001-control-conpty-loading` | `src/win/psuedocon.rs`, `Cargo.toml` | Loads ConPTY only from the pinned runtime beside the program, by absolute path, after checking the DLL and both OpenConsole hosts against their sha256 and refusing reparse points and unexpected files. |
| `0002-windows-raw-command-tail` | `src/cmdbuilder.rs` | Lets a Windows command carry a raw command-line tail, for `cmd.exe /d /c`. |
| `0003-reject-malformed-windows-environments` | `src/cmdbuilder.rs` | Leaves out Windows environment values that cannot form one `name=value` record. |
| `0004-skip-directories-when-resolving-windows-programs` | `src/cmdbuilder.rs` | Skips a `PATH` directory whose name matches the program. |

## Hide's changes

| File | Change |
| --- | --- |
| `Cargo.toml` | Written by hand instead of crates.io's normalized form: `publish = false`, no examples or their dev-dependencies, no `lazy_static`, and the `winapi` feature the job start needs (`jobapi2`). |
| `src/win/psuedocon.rs` | The pinned runtime is required: Herdr's fallback to the ConPTY in `kernel32.dll` when the runtime is absent, its `HERDR_WINDOWS_CONPTY=system` switch and its fallback on other architectures are gone, so a pane runs only on the ConPTY Hide tested, and only on x86_64. A missing, changed or unloadable runtime is an error kept once and answered to every caller (`verify_runtime`), not a panic, and its words name the runtime's file list rather than the file Herdr's bundle keeps it in. `spawn_command_in_job` starts the child suspended, puts it in the caller's job and only then resumes it, so nothing it starts escapes the job. |
| `src/win/conpty.rs` | `ConPtySlavePty` implements `spawn_command_in_job`. |
| `src/win/mod.rs` | Exports `verify_runtime`. |
| `src/lib.rs` | `SlavePty::spawn_command_in_job` (Windows), which refuses by default. |
| every other file | Unchanged but for the header. |

## The ConPTY runtime

The runtime itself is not in this repository.
It is Microsoft's `Microsoft.Windows.Console.ConPTY` 1.24.260710001 (MIT), the `conpty/` folder the pinned Herdr release ships beside `herdr.exe`, unchanged; `CONPTY_FILES` in `src/win/psuedocon.rs` pins each file's sha256, and the holder's version folder carries a copy (docs/ARCHITECTURE.md, The pane holder).

## Moving to a newer upstream

Compare the new `portable-pty` (or the new Herdr release's `vendor/portable-pty` and its patch notes) with these sources, carry Hide's changes above across, and update the headers and this file in the same change.
When the pinned Herdr moves its ConPTY runtime, `CONPTY_FILES` moves with it.
