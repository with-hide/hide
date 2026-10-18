# Herdr API contract

`herdr-api.schema.json` is what the pinned Herdr binary answers to `herdr api schema --json`, written by `scripts/bump-herdr.sh` in the same run that moves the pin recorded in `contracts/herdr-bundle.json`.
Herdr owns the schema and the protocol revision; `herdr-core/build.rs` derives `HERDR_PROTOCOL_REVISION` from this file, so no protocol number is maintained by hand anywhere in this repository.

<!-- herdr-provenance:start -->
hide distributes the [upstream Herdr release v0.9.5](https://github.com/herdrdev/herdr/releases/tag/v0.9.5).
The bundled binary's code is not modified by hide; a macOS release replaces only its ad-hoc signature with hide's Developer ID one, which notarization requires.
The weekly `herdr-update.yml` workflow proposes upstream stable releases with `--repo herdrdev/herdr`; updates must pass contract and runtime checks.
<!-- herdr-provenance:end -->

Never edit this file directly.
Move the pin and the contract together:

```sh
./scripts/bump-herdr.sh <release-tag>
```

Re-running the script for the tag already pinned rewrites the contract only when it drifted from what the binary reports, and is otherwise a no-op.

Before launching the IDE against a local runtime, verify that the contract, the CLI, and the running server agree:

```sh
./scripts/check-herdr-contract.sh --herdr-bin <path to the pinned binary>
```

# Snapshot wire enums

`snapshot-wire-enums.json` lists, for every string enum the core serializes into the shell snapshot, the exact values it emits.
The core is the writer of those strings.
`herdr-core/src/model.rs`'s `wire_enum_tests` pins the writing side: it fails to compile until a new variant is matched there, and then fails the test until that variant is added to this file.

Nothing in this repository checks an incoming wire value against this file from the reading side.
The web shell (`web/src/snapshot.ts`) types these fields with TypeScript, which is a compile-time check only and does not reject an unrecognized value at runtime; a field the core stops emitting, or emits a new value for, is not caught by a test today.

`workspace_view.tool` (`files`, `changes`, `work`, `memory`, or null while the lane is closed) is not listed here.
Neither is the View area layout inside `workspace_view`: a split's `axis` (`row`, `column`), a display's `kind` (`file`, `diff`, `browser`, `pull_request`), and a display's `state` (`open`, `opening`, `waiting`, `unavailable`).
Neither is `ui_state.theme` (`system`, `light`, `dark`); the web shell reads a value it does not know as Dark rather than failing.
List an enum here, with a Rust-side pin and a reading-side check for it, in the change that adds one.

# Agent adapters

`agent-adapters.json` is the web projection of `hide-agent-adapter`'s static declarations: support order, canonical ids and aliases, labels, Herdr kinds, Docs and Install links, logo ids and start/resume eligibility.
`web/src/agentAdapters.ts` is the shell's normalized reader; Settings, onboarding, desktop and phone pickers, resume requests and logo lookup derive their answers from it.
The Rust contract test compares the committed file with the current projection and checks its source links, logo manifest entries and support fixtures.
Do not maintain another web support table or edit this generated file by hand.
Build the exporter through the repository's verification entrypoint, then write its output:

```sh
bash scripts/verify-cargo.sh build -p hide-agent-adapter --example export_web_contract
target/debug/examples/export_web_contract > contracts/agent-adapters.json
```

# Agent journey

`agent-journey.json` is the agent journey's projection of the same declarations ([docs/VERIFICATION.md](../docs/VERIFICATION.md#agent-journey)): for each adapter its Herdr kind, its own folder in its HOME (`home_folder`, which the kit reads as the agent being set up), the flag that separates a spawn's first prompt, the model flag, the name a row shows for it while it has no task title (`untitled`), which journey cells it declares, how a letter reaches it (`letter`: `bell` wakes the pane, `next_prompt` rides the next prompt's hook, `tool` waits until the agent reads its inbox), how it wakes from sleep (`wake`: `pane` wakes the pane it sleeps in, `sleeping_session` reopens a closed session in a new pane), where the native title its `title` cell reads comes from (`title`: `automatic` when the agent titles its session itself, `{"named": "/name"}` when only the name the operator gives it with that command is one, as Pi's), and the most model requests a signed-in CI run may make for it (`request_budget`, three per turn the declared cells start).
`scripts/agent_journey` reads only this file for what an agent supports, so a cell an adapter does not declare reads N/A rather than a failure.
The Rust contract test fails when the file differs from the current projection; regenerate it, never edit it:

```sh
bash scripts/verify-cargo.sh build -p hide-agent-adapter --example export_journey_contract
target/debug/examples/export_journey_contract > contracts/agent-journey.json
```

# Agent CLI pins

`agent-clis.json` is the one place a CI agent CLI version is written: for each agent, by its Herdr kind, the version every CI lane installs, the installer (`npm` with its package, or the vendor's own installer for Grok, omp and agy), and, for an npm package whose program is not its `.bin` shim, the program's path in the package.
`scripts/install-agent-clis.py` reads it to install each CLI on a runner, and the agent journey reads it to say in its report whether the CLI it measured answered that version; a row whose installer takes no version (agy's) says so in `pinned`, and its version is checked, not installed.
Move a pin in a pull request that runs the lanes that install it; no workflow restates a version.

# hide CLI contract

`hide-cli.json` is what `hide contract --json` exports: the commands other tools call (`hide agent`, `request`, `inbox`, `watch`), their options, and the JSON Schema of each answer ([docs/delivery.md](../docs/delivery.md#the-contract-a-calling-tool-checks)).
The build is the writer; `hided/src/cli_contract.rs`'s `the_committed_contract_is_the_exported_one` fails until this file matches what it exports.
Regenerate it after a change to those commands, and review the diff as the change callers see:

```sh
target/debug/hide contract --json | python3 -m json.tool > contracts/hide-cli.json
```

# Provider quota

`provider-quota.json` names, for each provider a CI check signs an agent in with, the answer that means the key's requests for the day are spent: the error code and status of the agent's structured error report, and the quota ids that count.
A check that reads such an answer, with every quota it names among those ids and above 0, reports the step `Not measured` in a warning and passes; any other answer, a quota of 0 or another quota among them, fails ([docs/TESTING.md](../docs/TESTING.md#a-real-agent-on-ci)).
`hide-agent-hooks/tests/it/agy_binary.rs` reads it for the nightly agy turn; another signed-in check adds its provider's row here rather than a list of its own.
