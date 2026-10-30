# Rust core working conventions

This file covers conventions already established under `herdr-core/`.
Read the repository `AGENTS.md` for architecture, ownership, the runtime mutex, the Herdr contract, the snapshot wire, and performance invariants.
Read `CONTRIBUTING.md` for required gates.
Do not copy those contracts here.

## Placement and visibility

- Give a source file one named responsibility and register its module in `src/lib.rs`.
- Keep `lib.rs` as the module and export map; behavior belongs in the owning module.
- Default modules and helpers to private or `pub(crate)`.
- Add a public module or item only when a crate consumer needs that surface.
- Keep the owner-thread handle hided drives in `handle.rs` and re-export only `Core` from `lib.rs`; the crate has no C ABI.
- Speak to Herdr only through the `Engine` trait in `src/engine/`; keep conversions to and from the generated Herdr contract in `src/engine/herdr/wire.rs`, which only the engine's Herdr implementation calls.
- Before changing that boundary, follow the Herdr API procedure in the root `AGENTS.md`.
- Extend the existing feature owner instead of adding a second module for the same decision.

## Results and failures

- Return `Result` from work that can fail and add action-specific context where the error crosses a boundary.
- Reuse the owning module's error family.
- Feature-local action pipelines commonly use `Result<T, String>`; transport and service boundaries use their existing typed errors.
- Preserve distinct failure categories when a caller branches on them.
- Reject missing, malformed, or refused state explicitly rather than converting it to an empty success.
- Use `?`, `ok_or_else`, and `map_err` to keep the failing operation visible in the returned message.
- Emit operational diagnostics with `diagnostic!(json!(...))` and a stable `kind` field.
- Include the relevant workspace, tab, pane, checkout, request, or generation identifier when it is already available.
- Keep diagnostic sink and rotation behavior in `diagnostics.rs`.
- Do not add direct printing inside library behavior; stderr writes belong to the diagnostic fallback or an explicit process boundary.

## Tests

[docs/TESTING.md](../docs/TESTING.md) owns what a test asserts, which layer it belongs to, and the flaky policy; this section only places tests inside this crate.

- Put focused unit tests at the end of the owning module in `#[cfg(test)] mod tests`.
- Use a separate `*_tests.rs` file only for a large regression suite that the owning module explicitly includes.
- Put crate-boundary coverage in `herdr-core/tests/it/`, as a module of the crate's one integration test binary, and stable input data in `herdr-core/tests/fixtures/`.
- Name tests as snake_case statements of the observable result or rejected transition.
- Keep test doubles inside the test module.

## Change discipline

- Do not add style rules that `rustfmt` or Clippy can enforce.
- Keep comments for ownership, safety, protocol gaps, or non-obvious failure behavior.
- When behavior changes, update the current owning guide named by `docs/README.md` in the same change.

`CLAUDE.md` beside this file is a symlink to this file so both supported runtimes load the same nested instructions.
