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
| A message one agent sends another, kept with its answer | topic `letter` (`hide letter send\|reply\|show\|cancel\|ack`) | `request`, which names a letter's kind (`--kind request`) and the verb of `device request` and `access request`; the topic word `request` is the letters' retired spelling |
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
- `--json` prints the answer as JSON and means nothing else on a new command; the plan document of `mission plan` and `task set` is `--inline`.
  Those two keep `--json` as the retired option with its `json` value, so neither takes a bare `--json` within the contract's format.
- `--intent <key>` is the retry key: the same key converges on the first result.
- A new command that answers data takes `--json`; one that always prints JSON takes it too and changes nothing.
  Many existing commands that always print JSON (`inbox`, `letter send`, `task show`, `mission list`) do not take it, and `admit` refuses it there.

## Help

- `hide --help` (also `hide help` and `hide -h`) is one screen of 80 columns and 40 lines: the topics by group, each on one row with what it is for, and no command's syntax but `hide [open] [<path>]`.
  Every topic a person or an agent types is on it once, as a row or, for `open` and `guide`, in its first or last lines (`Shown::Around`); a hook's and the host's own commands are not topics, and a topic left off it says why (`Shown::Off`).
- `TOPICS` in `cli.rs` is the one list of topics: the first screen, `hide <topic> --help`, `hide help <topic>` and the guide's test of what an agent is taught all read it, so a new topic, or a change of the screen's wording or grouping, is an edit of that list.
- A topic's help is its own: `hide <topic> --help` prints that topic's commands and no other topic's, and a wrong command prints its reason, then that topic's usage.
- `--help` is a topic's commands and its `help` verb is its guide: `hide mission --help` lists the mission commands and ends with the line naming `hide mission help`; only `hide agent spawn --help` prints a guide.
- `hide help <topic> …` is read as `hide <topic> … --help` by the same parser, so the two print the same text; a retired topic word prints the help of the topic it is now, and a word that names no topic is refused with `hide --help`.
- Usage text and the contract table hold each other: every command and option of the table is in its topic's `--help`, and every `hide` line and `--option` in a usage is in the table.
  A test fails either way, so change both in the same commit.
- `hide <topic> <verb> --help` prints the usage of the parser that reads the command (`command_usage` in `cli.rs`), which for `agent profile` is not the `agent` topic's own.
- The site's CLI reference reads the topics and their groups from the first screen (a topic its last lines name, `guide`, joins the last group) and each topic's commands from its `--help` (`scripts/docs/cli-reference.mjs`), so a command added to a topic's help needs its note in `site/src/data/cli-notes.json`.

## Commands a program calls

- The verbs one Hide runs on another machine's account are marked hidden in the contract (`BETWEEN_HIDES`) and not printed by `hide device --help`.
- A command a program calls that the usage leaves out is listed, with its caller, in the usage test's `PROGRAM_ONLY`.
- Before adding one, look for a caller of the ones that exist; a command nothing calls is deleted, not kept.

## Renaming

- A renamed command or option keeps its old spelling for good.
  No runtime notice is printed and no removal date is set: session files, an operator's edited skill and their own notes keep the old spelling past any build.
- The old spelling goes in the retired-words table (`hide-shell/src/retired.rs`), keyed by command (a row names the words a command starts with, from its topic, and the longest row wins), with an option's arity and whether it is renamed only when a value follows.
  Write a row per command that moved (`workspace info` to `checkout show`) and a kept row for a longer spelling that stays (`workspace info --hook`), never a topic's row for a topic whose commands did not all move.
  A retired option never stays a current option of its command: the contract keeps it as the retired option with the value type it had.
  `hided`'s parser, `hide-session`'s thread reader and `hide-desk`'s step reader all read a command line through `hide_shell::retired::current`, and none keeps its own copy.
- The contract only grows: the retired command stays an entry marked with the command it is now (`retired_for`), a retired option stays in its command's options, and a retired topic's envelope key stays, because a calling tool looks them up by the word it types and refuses a build where they are gone (`docs/delivery.md`).
  `cli_contract.rs` writes those entries from the table, each right after the command it is now, and the table's rows as `retired_words`, so a rename changes no entry by hand.
  An argument's name or a value's type does not change outside a release that changes the contract's format.
- A retired (command, word) is never used again for something else.
- Text that must keep the spelling of its day (an earlier build's skill stub in `RETIRED_SKILLS`, which a kit pass recognises byte for byte) sits between `retired-spellings: kept from here: <why>` and `retired-spellings: kept to here`, and only that region is left out of the retired-spelling check.
- The same pull request changes what teaches the word: the guide, the help texts, the skill stubs and their versions, the session-start note, Desk's instructions, the core's `next_action` texts and answer values that are commands, `site/src/data/cli-notes.json`, and the documents that name it.

## What fails when this drifts

| Rule | Check |
| --- | --- |
| An old spelling reads as the command it is now, in the parser and in both readers; a retired (command, word) is not used by a current command | `each_retired_spelling_parses_as_the_command_it_is_now` (`cli_contract.rs`), `a_retired_spelling_is_read_as_the_command_it_is_now` (`hide-session/src/thread/hide_call.rs`), `a_retired_spelling_counts_as_the_command_it_is_now` (`hide-desk/src/permission/tests.rs`) |
| A retired command keeps its contract entry and its envelope | `a_retired_command_keeps_its_entry_and_its_envelope` |
| One value type per option name | `an_option_name_takes_one_value_type_across_the_table` |
| Every verb has its line | `every_verb_of_the_table_is_in_the_registry` |
| Usage and table agree both ways | `the_usage_and_the_table_name_the_same_commands_and_options` |
| The first screen is the topics by group on one screen, a topic's help is its own, `hide help <topic>` is the same text, and every topic of the table is in `TOPICS` | `the_first_screen_is_the_topics_by_group_on_one_screen`, `a_topics_help_names_only_its_own_commands`, `hide_help_topic_is_the_topics_help` (`cli.rs`), `every_topic_of_the_table_is_a_help_topic` (`cli_contract.rs`) |
| What an agent is taught names contract commands and no retired spelling | `every_contract_command_an_agent_is_taught_is_in_the_contract` (`guide.rs`) |
| No retired spelling in any other tracked text file: the core's texts, the other crates, `docs/`, `site/`, `web/src`, the root documents, `.claude/skills`, the scripts | `python3 scripts/check-retired-spellings.py` |

## Before a pull request

- `contracts/hide-cli.json` is regenerated, and its diff only adds.
- A new word has its row in "One name per thing", a new verb its line in the registry.
