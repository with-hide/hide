# Command line conventions

This file covers the `hide` command under `hided/src/`: its words, shape, options and help (`cli.rs`, the `*_cli.rs` modules, `cli_contract.rs`, `guide.rs`), and the retired-words table every reader of a `hide` command shares (`hide-shell/src/retired.rs`).
Read the repository `AGENTS.md` for architecture and [docs/delivery.md](../docs/delivery.md#the-contract-a-calling-tool-checks) for the contract a calling tool checks.
Do not copy those contracts here.

## One name per thing

- Before adding a word, look for the thing in the table below and in [docs/LOCALIZATION.md](../docs/LOCALIZATION.md#glossary); a second name for a thing that has one is a review finding.
- A new thing adds its row here in the same pull request.
- This table rules command words and options only.
  An answer field, a refusal code or a variable that still carries an older word (`machine`, `access_*`, `HIDE_ACCESS`, the envelope key `plan`) moves with its command in a release that changes the contract's format, never before it.

| Thing | Word | Not, for a new command or option |
| --- | --- | --- |
| A machine that runs panes | topic `device`, option `--machine <device id>` | `--device`, which `device accept-ssh` uses between two Hides for the asking machine's id |
| An agent family | `--kind <kind>` | `--agent`, retired on `skill` and `mcp` `on`/`off` |
| The text an agent starts with | `--brief <text>` | `--task`, retired on `automation add` and `edit` |
| The checkout a pane works in | topic `checkout` (`show`, `purpose`, `remove`) | `workspace`, the engine's word, which only a hook's own commands keep (`workspace info --hook`, `status --hook`, `bootstrap`, `memory`) |
| The project a command works in | `--repo <path>`, cleared by `--no-repo` | `--folder` and `--own-folder`, retired on `automation add` and `edit` |
| A plan document given on the command line | `--inline <plan>` | `--json <plan>`, retired on `mission plan` and `task set` |
| A session's own plan and its steps | topic `task`, `node`, option `--plan-node` | a second topic for it; the glossary's `task` is the sidebar's unit of work, and this topic keeps the word it has |

## Shape

- A command is `hide <topic> <verb> [<subject>] [options]`: the topic is a noun and the verb comes second (`agent profile` is a topic inside a topic, `SUBTOPICS` in `cli_contract.rs`).
  A bare top-level word is for a person at a terminal (`status`, `stop`, `version`, `ps`, `guide`).
- Every new top-level word hides a folder of the same name from `hide <path>`, so add one only for a new topic, and say it in the pull request.
- A verb comes from the verb registry `VERBS` in `cli_contract.rs`, one line of meaning each.
  Reach first for `list`, `show`, `add`, `edit`, `remove` and the pairs `start`/`stop`, `pause`/`resume`, `on`/`off`, `join`/`leave`; a new command that changes a thing's fields is `edit`, and `update` fetches a newer version of it.
  A new verb adds its line to the registry, with what none of the existing ones says.
- An option is never a verb for a new command: `ask-human --clear` predates the rule, and `inbox --ack` is retired for `inbox ack`.
  `--clear` unsets what the command sets, and `--no-<option>` unsets one field.
- A command lives under the topic of the thing it changes, and its code in that topic's module; `hide agent read` in the pane module is the shape to avoid.

## Options

- An option name takes one value type across the contract table.
  The exceptions are a list in the test (`VALUE_TYPE_EXCEPTIONS`), each with its reason; a new exception needs a reason a reviewer accepts, and an exception that no longer differs is taken out.
- `--json` prints the answer as JSON and means nothing else; the plan document of `mission plan` and `task set` is `--inline`.
- `--intent <key>` is the retry key: the same key converges on the first result.
- A command that answers data takes `--json`; one that always prints JSON takes it too and changes nothing.

## Help

- Usage text and the contract table hold each other: every command and option of the table is in its topic's `--help`, and every `hide` line and `--option` in a usage is in the table.
  A test fails either way, so change both in the same commit.
- `hide <topic> <verb> --help` prints the usage of the parser that reads the command (`command_usage` in `cli.rs`), which for `agent profile` and `agent read` is not the `agent` topic's own.

## Commands a program calls

- The verbs one Hide runs on another machine's account are marked hidden in the contract (`BETWEEN_HIDES`) and not printed by `hide --help`.
- A command a program calls that the usage leaves out is listed, with its caller, in the usage test's `PROGRAM_ONLY`.
- Before adding one, look for a caller of the ones that exist; a command nothing calls is deleted, not kept.

## Renaming

- A renamed command or option keeps its old spelling for good.
  No runtime notice is printed and no removal date is set: session files, an operator's edited skill and their own notes keep the old spelling past any build.
- The old spelling goes in the retired-words table (`hide-shell/src/retired.rs`), keyed by command (a row names the words a command starts with, from its topic), with an option's arity and whether it is renamed only when a value follows.
  `hided`'s parser, `hide-session`'s thread reader and `hide-desk`'s step reader all read a command line through `hide_shell::retired::current`, and none keeps its own copy.
- The contract only grows: the retired command stays an entry marked with the command it is now (`retired_for`), a retired option stays in its command's options, and a retired topic's envelope key stays, because a calling tool looks them up by the word it types and refuses a build where they are gone (`docs/delivery.md`).
  `cli_contract.rs` writes those entries from the table, so a rename changes no entry by hand.
  An argument's name or a value's type does not change outside a release that changes the contract's format.
- A retired (command, word) is never used again for something else.
- The same pull request changes what teaches the word: the guide, the help texts, the skill stubs and their versions, the session-start note, Desk's instructions, the core's `next_action` texts and answer values that are commands, `site/src/data/cli-notes.json`, and the documents that name it.

## What fails when this drifts

| Rule | Check |
| --- | --- |
| An old spelling reads as the command it is now, in the parser and in both readers; a retired (command, word) is not used by a current command | `each_retired_spelling_parses_as_the_command_it_is_now` (`cli_contract.rs`), `a_retired_spelling_is_read_as_the_command_it_is_now` (`hide-session/src/thread/hide_call.rs`), `a_retired_spelling_counts_as_the_command_it_is_now` (`hide-desk/src/permission/tests.rs`) |
| A retired command keeps its contract entry and its envelope | `a_retired_command_keeps_its_entry_and_its_envelope` |
| One value type per option name | `an_option_name_takes_one_value_type_across_the_table` |
| Every verb has its line | `every_verb_of_the_table_is_in_the_registry` |
| Usage and table agree both ways | `the_usage_and_the_table_name_the_same_commands_and_options` |
| What an agent is taught names contract commands and no retired spelling | `every_contract_command_an_agent_is_taught_is_in_the_contract` (`guide.rs`) |
| No retired spelling in the core's texts, `docs/`, `site/` or `web/src` | `python3 scripts/check-retired-spellings.py` |

## Before a pull request

- `contracts/hide-cli.json` is regenerated, and its diff only adds.
- A new word has its row in "One name per thing", a new verb its line in the registry.
