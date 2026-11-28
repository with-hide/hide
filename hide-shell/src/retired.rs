//! The `hide` command line's retired words: every spelling a renamed
//! command or option had, and the one function that reads a command line's
//! words as the command they name today ([`current`]).
//!
//! Three readers decide which `hide` command a line ran, and each reads it
//! through [`current`] and keeps no copy of these rows: `hided`'s parser
//! (`parse_args`), `hide-session`'s thread reader (a session's shell call as
//! a Desk thread part) and `hide-desk`'s step reader (Desk's turn steps).
//! Session files keep the old spellings for good, so a row is never dropped,
//! and a retired spelling is never given to a new command
//! (`hided/AGENTS.md`, Renaming). `hided`'s contract carries each retired
//! command as an entry marked with the command it is now
//! (`docs/delivery.md`, The contract a calling tool checks).
//!
//! The rows are keyed by command, not by bare word: a row names the words a
//! command starts with, from the topic, so `request` as a letter's kind or as
//! `device request`'s verb is never read as a topic. An option row belongs to
//! one command and carries the old option's arity, and a rewrite skips every
//! option's value and everything after `--`.

/// A command spelling that was renamed: the words it started with then
/// (after `hide`) and the words they are now. A row whose `now` equals its
/// `old` keeps a longer spelling as it is (a mode such as `--hook` of a
/// command whose plain form moved), since the longest matching row wins.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub old: &'static [&'static str],
    pub now: &'static [&'static str],
}

/// An option of one command that was renamed.
#[derive(Clone, Copy, Debug)]
pub struct Flag {
    pub old: &'static str,
    pub now: &'static str,
    /// How many values the old option took.
    pub arity: usize,
    /// Read as `now` only when a value follows it: a bare `old` stays what
    /// it is today (`--json <plan>` became `--inline <plan>`, while a bare
    /// `--json` asks for the answer as JSON).
    pub only_with_value: bool,
}

/// The renamed options of one command, by the command's words now.
#[derive(Clone, Copy, Debug)]
pub struct Flags {
    pub command: &'static [&'static str],
    /// Every option the command takes a value for today, so a rewrite skips
    /// the value that follows one (`--body --machine` keeps `--machine` as
    /// the body). `hided`'s tests hold it to the contract table.
    pub valued: &'static [&'static str],
    pub renamed: &'static [Flag],
}

/// The table: what [`current`] reads.
#[derive(Clone, Copy, Debug)]
pub struct Table {
    pub commands: &'static [Command],
    pub options: &'static [Flags],
}

/// Every retired spelling of the `hide` command line.
pub const TABLE: Table = Table {
    commands: &[
        // `hide task` was `hide plan`; a session started under the old
        // `hide-plan` skill still types it, and its session file keeps it.
        Command {
            old: &["plan"],
            now: &["task"],
        },
        // `hide letter` was `hide request`: the command agents type most,
        // in every session file, edited skill and calling tool (sasu) since
        // letters began. `request` stays a letter's kind (`--kind request`)
        // and the verb of `device request` and `access request`, which this
        // row, keyed by the topic, never reads.
        Command {
            old: &["request"],
            now: &["letter"],
        },
    ],
    options: &[],
};

/// `words`, the words after `hide`, as the command they name today: the
/// command's words by the longest row of [`TABLE`] that starts them, then
/// its renamed options outside option values and before `--`. Words that
/// name no retired spelling come back as they are.
pub fn current<S: AsRef<str>>(words: &[S]) -> Vec<String> {
    TABLE.current(words)
}

impl Table {
    /// [`current`] over this table.
    pub fn current<S: AsRef<str>>(&self, words: &[S]) -> Vec<String> {
        let words: Vec<&str> = words.iter().map(AsRef::as_ref).collect();
        let mut line: Vec<String> = match self
            .commands
            .iter()
            .filter(|row| starts(&words, row.old))
            .max_by_key(|row| row.old.len())
        {
            Some(row) => row
                .now
                .iter()
                .copied()
                .chain(words[row.old.len()..].iter().copied())
                .map(str::to_owned)
                .collect(),
            None => words.iter().map(|word| (*word).to_owned()).collect(),
        };
        let Some(options) = self
            .options
            .iter()
            .filter(|row| starts(&line, row.command))
            .max_by_key(|row| row.command.len())
        else {
            return line;
        };
        let mut at = options.command.len();
        while at < line.len() {
            let word = line[at].as_str();
            if word == "--" {
                break;
            }
            let followed = line.get(at + 1).is_some_and(|next| !next.starts_with("--"));
            if let Some(renamed) = options
                .renamed
                .iter()
                .find(|renamed| renamed.old == word && (followed || !renamed.only_with_value))
            {
                line[at] = renamed.now.to_owned();
                at += 1 + renamed.arity;
                continue;
            }
            if let Some((name, value)) = word.split_once('=')
                && let Some(renamed) = options
                    .renamed
                    .iter()
                    .find(|renamed| renamed.old == name && renamed.arity == 1)
            {
                line[at] = format!("{}={value}", renamed.now);
                at += 1;
                continue;
            }
            at += if options.valued.contains(&word) { 2 } else { 1 };
        }
        line
    }
}

fn starts<S: AsRef<str>>(words: &[S], prefix: &[&str]) -> bool {
    words.len() >= prefix.len()
        && prefix
            .iter()
            .zip(words)
            .all(|(want, word)| *want == word.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(table: &Table, line: &str) -> String {
        let words: Vec<&str> = line.split(' ').collect();
        table.current(&words).join(" ")
    }

    /// A table with every kind of row, as later renames write them.
    const FIXTURE: Table = Table {
        commands: &[
            Command {
                old: &["plan"],
                now: &["task"],
            },
            Command {
                old: &["request"],
                now: &["letter"],
            },
            Command {
                old: &["workspace", "info"],
                now: &["checkout", "show"],
            },
            Command {
                old: &["workspace", "info", "--hook"],
                now: &["workspace", "info", "--hook"],
            },
        ],
        options: &[
            Flags {
                command: &["letter", "send"],
                valued: &["--body", "--intent", "--kind", "--device"],
                renamed: &[Flag {
                    old: "--machine",
                    now: "--device",
                    arity: 1,
                    only_with_value: false,
                }],
            },
            Flags {
                command: &["task", "set"],
                valued: &["--file", "--inline", "--reason"],
                renamed: &[Flag {
                    old: "--json",
                    now: "--inline",
                    arity: 1,
                    only_with_value: true,
                }],
            },
        ],
    };

    #[test]
    fn the_table_reads_hide_plan_as_hide_task() {
        assert_eq!(
            current(&["plan", "set", "--json", "{}"]),
            ["task", "set", "--json", "{}"]
        );
        assert_eq!(current(&["plan"]), ["task"]);
        assert_eq!(current(&["task", "show"]), ["task", "show"]);
        // A retired word is read only where it was the command's.
        assert_eq!(current(&["agent", "plan"]), ["agent", "plan"]);
        assert!(current::<&str>(&[]).is_empty());
    }

    #[test]
    fn the_table_reads_hide_request_as_hide_letter() {
        for verb in ["send", "reply", "show", "cancel", "ack"] {
            assert_eq!(
                current(&["request", verb, "x"]),
                ["letter", verb, "x"],
                "request {verb}"
            );
        }
        assert_eq!(
            current(&[
                "request", "send", "parent", "--kind", "request", "--body", "request"
            ]),
            [
                "letter", "send", "parent", "--kind", "request", "--body", "request"
            ]
        );
        // `request` as another topic's verb is never the letter topic.
        assert_eq!(
            current(&["device", "request", "pc"]),
            ["device", "request", "pc"]
        );
        assert_eq!(
            current(&["access", "request", "--name", "bot"]),
            ["access", "request", "--name", "bot"]
        );
        assert_eq!(current(&["letter", "show", "l1"]), ["letter", "show", "l1"]);
    }

    #[test]
    fn a_row_is_keyed_by_command_and_the_longest_row_wins() {
        assert_eq!(
            read(&FIXTURE, "request send x --body hi"),
            "letter send x --body hi"
        );
        // `request` as a letter's kind or another topic's verb stays.
        assert_eq!(read(&FIXTURE, "device request k1"), "device request k1");
        assert_eq!(
            read(&FIXTURE, "letter send x --kind request --body hi"),
            "letter send x --kind request --body hi"
        );
        assert_eq!(read(&FIXTURE, "workspace info"), "checkout show");
        assert_eq!(
            read(&FIXTURE, "workspace info --hook"),
            "workspace info --hook"
        );
        assert_eq!(read(&FIXTURE, "workspace purpose x"), "workspace purpose x");
    }

    #[test]
    fn an_option_is_renamed_outside_values_and_before_the_rest() {
        assert_eq!(
            read(&FIXTURE, "request send x --machine m1 --body hi"),
            "letter send x --device m1 --body hi"
        );
        assert_eq!(
            read(&FIXTURE, "letter send x --machine=m1"),
            "letter send x --device=m1"
        );
        // A value that looks like an option is a value.
        assert_eq!(
            read(&FIXTURE, "request send x --body --machine"),
            "letter send x --body --machine"
        );
        // After `--` every word passes through.
        assert_eq!(
            read(&FIXTURE, "letter send x -- --machine m1"),
            "letter send x -- --machine m1"
        );
        // The old option's value is skipped, even when it looks like one.
        assert_eq!(
            read(&FIXTURE, "letter send x --machine m1 --machine m2"),
            "letter send x --device m1 --device m2"
        );
        // Another command's option of the same name stays.
        assert_eq!(
            read(&FIXTURE, "agent spawn --machine m1"),
            "agent spawn --machine m1"
        );
    }

    #[test]
    fn an_option_renamed_only_with_a_value_keeps_its_bare_spelling() {
        assert_eq!(read(&FIXTURE, "plan set --json {}"), "task set --inline {}");
        assert_eq!(
            read(&FIXTURE, "task set --file p.json --json"),
            "task set --file p.json --json"
        );
        assert_eq!(
            read(&FIXTURE, "task set --json --reason why"),
            "task set --json --reason why"
        );
        assert_eq!(
            read(&FIXTURE, "task set --reason --json --inline {}"),
            "task set --reason --json --inline {}"
        );
    }
}
