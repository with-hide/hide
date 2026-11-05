//! The guardrails of a shell call: the `git` and `gh` commands each
//! protection names, and the files protection over every word.
//!
//! The call is read as the shell that runs it reads it
//! (`hide_shell::shell_line`, in the call's syntax): one simple command at a
//! time, with quotes, heredoc bodies and comments set aside. A `cd` or
//! `pushd` earlier in the line (in PowerShell also `Set-Location` and its
//! aliases) moves where the commands after it run, and `git -C <dir>` moves
//! one command. A PowerShell word's `\` separates names, as PowerShell reads
//! it on every system (`.\.env`, `C:\repo`). A command hidden a layer
//! deeper (`bash -c`, `$(...)`, a script, an alias) is not looked for (D-18).

use std::path::{Path, PathBuf};

use super::facts::{Checkout, Located, Place};
use super::{Dirty, Facts, Refusal, Rule, absolute, protected};
use hide_shell::{Syntax, command_words, shell_line};

/// The most words of one call checked against the protected paths; past it
/// the words are left unchecked and the log says so, while every later
/// `git` and `gh` command is still judged.
pub(super) const WORD_LIMIT: usize = 512;

pub(super) fn shell(
    facts: &Facts<'_>,
    cwd: &Path,
    command: &str,
    syntax: Syntax,
    dirty: Dirty<'_>,
) -> Option<Refusal> {
    let line = shell_line(command, syntax);
    let mut dir = cwd.to_path_buf();
    let mut checked = 0usize;
    for (index, words) in line.commands.iter().enumerate() {
        let words = command_words(words, true);
        let Some((first, args)) = words.split_first() else {
            continue;
        };
        let program = syntax.program(first);
        if syntax.changes_folder(&program) {
            // A bare `cd` goes home; `cd -` goes where this line cannot see.
            match args.iter().find(|arg| !arg.starts_with('-') || *arg == "-") {
                Some(target) if target != "-" => {
                    let target = syntax.path_word(target);
                    dir = absolute(&facts.home, &dir, Path::new(target.as_ref()));
                }
                Some(_) => {}
                None if !matches!(program.as_str(), "pushd" | "push-location") => {
                    dir = facts.home.clone();
                }
                None => {}
            }
            continue;
        }
        let refusal = match program.as_str() {
            "git" => git(facts, &dir, args, syntax, dirty),
            "gh" => gh(facts, &dir, args),
            _ => None,
        };
        if refusal.is_some() {
            return refusal;
        }
        let targets = line
            .targets
            .iter()
            .filter(|(command, _)| *command == index)
            .map(|(_, target)| target);
        for (at, word, target) in words
            .iter()
            .enumerate()
            .map(|(at, word)| (Some(at), word, false))
            .chain(targets.map(|target| (None, target, true)))
        {
            for candidate in path_words(word) {
                if checked >= WORD_LIMIT {
                    facts.word_limit.set(true);
                    break;
                }
                let path = absolute(
                    &facts.home,
                    &dir,
                    Path::new(syntax.path_word(candidate).as_ref()),
                );
                // A word with a space in it that is text (a commit message,
                // a title, a pattern) is prose unless it names something
                // that is there; anywhere else it is a path, which `cp` or
                // `touch` may be about to make.
                if !target
                    && candidate.chars().any(char::is_whitespace)
                    && at.is_some_and(|at| text_word(&program, words, at, candidate != word))
                    && std::fs::symlink_metadata(&path).is_err()
                {
                    continue;
                }
                checked += 1;
                // The agent's own project lists the home paths, wherever a
                // `cd` in the line went.
                if let Some(refusal) = protected(facts, cwd, &path) {
                    return Some(refusal);
                }
            }
        }
    }
    None
}

/// The programs whose text options below take prose, a pattern or a script:
/// Git and the forges' CLIs, the interpreters' `-c` and `-e`, and a search's
/// or a stream editor's `-e`. Another program's `-m` or `-c` may make a file
/// (`touch -m`, `curl -c`).
const TEXT_PROGRAMS: &[&str] = &[
    "git", "gh", "glab", "python", "python3", "node", "deno", "bun", "perl", "ruby", "sh", "bash",
    "zsh", "fish", "grep", "egrep", "fgrep", "rg", "sed", "awk", "jq",
];
/// Options whose value is text: a message, a title, a body, a script.
const TEXT_OPTIONS: &[&str] = &[
    "-m",
    "--message",
    "-t",
    "--title",
    "-b",
    "--body",
    "--subject",
    "--notes",
    "--description",
    "--comment",
    "-c",
    "-e",
    "--eval",
    "--expression",
    "--regexp",
];
/// The letters that end a short-option cluster taking a text value (`-am`).
const TEXT_LETTERS: &[char] = &['m', 'b', 'c', 'e'];
/// The options of a search or a stream editor that take a value, which is
/// not its pattern or script slot.
const VALUE_OPTIONS: &[&str] = &[
    "-A",
    "-B",
    "-C",
    "-m",
    "-g",
    "-t",
    "-T",
    "-f",
    "-e",
    "-j",
    "--glob",
    "--type",
    "--type-not",
    "--max-count",
    "--context",
    "--before-context",
    "--after-context",
    "--file",
    "--regexp",
    "--threads",
];

/// Whether word `at` of a command, or its `--name=value` value when `value`,
/// is text rather than a path: the value of an option that takes text
/// (`-m`, `--title`), what `echo` and `printf` print, or the pattern or script
/// a search or a stream editor takes first. Any other word may name a file
/// about to be made, so it is read as a path.
fn text_word(program: &str, words: &[String], at: usize, value: bool) -> bool {
    // The program word itself is a path, spaces and all.
    if at == 0 {
        return false;
    }
    let text_option = |option: &str| {
        TEXT_PROGRAMS.contains(&program)
            && (TEXT_OPTIONS.contains(&option)
                || short(option).is_some_and(|letters| letters.ends_with(TEXT_LETTERS)))
    };
    if value {
        return words[at]
            .split_once('=')
            .is_some_and(|(option, _)| option.starts_with('-') && text_option(option));
    }
    if at
        .checked_sub(1)
        .and_then(|before| words.get(before))
        .is_some_and(|before| text_option(before))
    {
        return true;
    }
    match program {
        "echo" | "printf" => true,
        "grep" | "egrep" | "fgrep" | "rg" | "sed" | "awk" | "jq" => {
            // A pattern given as `-e` or `-f` leaves no pattern slot.
            if words[1..at]
                .iter()
                .any(|word| matches!(word.as_str(), "-e" | "-f" | "--regexp" | "--file"))
            {
                return false;
            }
            let mut index = 1;
            while let Some(word) = words.get(index) {
                if word == "--" {
                    return index + 1 == at;
                }
                if !word.starts_with('-') || word == "-" {
                    return index == at;
                }
                // awk's `-v name=value` and `-F sep` take a value; grep's
                // `-v` and `-F` do not.
                let valued = VALUE_OPTIONS.contains(&word.as_str())
                    || (program == "awk" && matches!(word.as_str(), "-v" | "-F"));
                index += if valued { 2 } else { 1 };
            }
            false
        }
        _ => false,
    }
}

/// The parts of a word that may name a path: the word, and the value of a
/// `--name=value` or `NAME=value` word. A word still holding an expansion
/// (`$HOME`, `$(...)`, a backtick) is not read, since its path is unknown.
fn path_words(word: &str) -> Vec<&str> {
    let readable = |candidate: &&str| {
        !candidate.is_empty()
            && !candidate.contains('$')
            && !candidate.contains('`')
            && !candidate.starts_with('-')
    };
    std::iter::once(word)
        .chain(word.split_once('=').map(|(_, value)| value))
        .filter(readable)
        .collect()
}

/// Letters of a short-option cluster (`-fd` is `fd`); `None` for anything else.
fn short(word: &str) -> Option<&str> {
    let letters = word.strip_prefix('-')?;
    (!letters.is_empty()
        && !letters.starts_with('-')
        && letters.chars().all(|c| c.is_ascii_alphabetic()))
    .then_some(letters)
}

fn has(args: &[String], names: &[&str]) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| names.contains(&arg.as_str()))
}

fn has_short(args: &[String], letter: char) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .filter_map(|arg| short(arg))
        .any(|letters| letters.contains(letter))
}

/// The checkout and policy project a git command run in `dir` works on.
struct Target<'f> {
    checkout: Checkout,
    project: &'f super::ProjectPolicy,
}

enum Where<'f> {
    Outside,
    Unreadable(&'f super::ProjectPolicy),
    Git(Target<'f>),
}

fn place<'f>(facts: &Facts<'f>, dir: &Path) -> Where<'f> {
    let found: Place = facts.locate(dir);
    if found.nested {
        return Where::Outside;
    }
    match (found.project(facts), found.place) {
        (Some(project), Located::In(checkout)) if checkout.git() => Where::Git(Target {
            checkout,
            project: project.policy,
        }),
        (Some(project), Located::Unreadable) => Where::Unreadable(project.policy),
        _ => Where::Outside,
    }
}

fn git(
    facts: &Facts<'_>,
    dir: &Path,
    args: &[String],
    syntax: Syntax,
    dirty: Dirty<'_>,
) -> Option<Refusal> {
    let mut dir: PathBuf = dir.to_path_buf();
    let mut at = 0;
    // Git's own options before the subcommand.
    while let Some(arg) = args.get(at) {
        match arg.as_str() {
            "-C" => {
                let target = syntax.path_word(args.get(at + 1)?);
                dir = absolute(&facts.home, &dir, Path::new(target.as_ref()));
                at += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--config-env"
            | "--super-prefix" => at += 2,
            other if other.starts_with('-') => at += 1,
            _ => break,
        }
    }
    let subcommand = args.get(at)?.as_str();
    let rest = &args[at + 1..];
    let target = match place(facts, &dir) {
        Where::Outside => return None,
        // Only a command a protection that is on judges needs the checkout;
        // `git status` or `git log` runs.
        Where::Unreadable(project) => {
            let rule = judging_rule(project, subcommand)?;
            return Some(Refusal::unread(rule, "the checkout this command runs in"));
        }
        Where::Git(target) => target,
    };
    let rules = &target.project.rules;
    let checkout = &target.checkout;
    if rules.worktree_folder && checkout.original() {
        let switches = match subcommand {
            "switch" => !has(rest, &["-h", "--help"]),
            "checkout" => checkout_switches(rest, &dir, checkout),
            _ => false,
        };
        if switches {
            return Some(Refusal::new(
                Rule::WorktreeFolder,
                "This call changes the branch of the project's original checkout",
            ));
        }
    }
    if rules.base_branch {
        let commits = match subcommand {
            "commit" => true,
            // A fast-forward merge lands a branch unreviewed as surely as a
            // merge commit; only `pull --ff-only` takes the remote's own.
            "merge" => !has(rest, &["--abort", "--quit"]),
            "rebase" => !has(
                rest,
                &[
                    "--abort",
                    "--quit",
                    "--skip",
                    "--edit-todo",
                    "--show-current-patch",
                ],
            ),
            "cherry-pick" | "revert" => !has(rest, &["--abort", "--quit", "--skip"]),
            "am" => !has(
                rest,
                &["--abort", "--quit", "--skip", "--show-current-patch"],
            ),
            "pull" => !has(rest, &["--ff-only"]),
            _ => false,
        };
        let pushes = subcommand == "push";
        if commits || pushes {
            let Some(base) = facts.base(target.project, checkout) else {
                return Some(Refusal::base_unknown());
            };
            if commits && checkout.branch.as_deref() == Some(base.as_str()) {
                return Some(Refusal::new(
                    Rule::BaseBranch,
                    format!("This call makes a commit on the base branch {base}"),
                ));
            }
            if pushes && Push::read(rest).reaches(&base, checkout.branch.as_deref()) {
                return Some(Refusal::new(
                    Rule::BaseBranch,
                    format!("This call pushes to the base branch {base}"),
                ));
            }
        }
    }
    if rules.git_commands {
        if subcommand == "push" {
            let push = Push::read(rest);
            if push.force {
                return Some(Refusal::new(
                    Rule::GitCommands,
                    "This call force-pushes without a lease; use --force-with-lease",
                ));
            }
            if push.no_verify {
                return Some(Refusal::new(
                    Rule::GitCommands,
                    "This call skips Git's hooks with --no-verify; run it without and fix what they report",
                ));
            }
        }
        if subcommand == "commit" && commit_skips_hooks(rest) {
            return Some(Refusal::new(
                Rule::GitCommands,
                "This call skips Git's hooks with --no-verify; run it without and fix what they report",
            ));
        }
        let discards = match subcommand {
            "reset" => has(rest, &["--hard"]),
            "checkout" => checkout_discards(rest, &dir, checkout),
            "restore" => restore_discards(rest),
            "clean" => {
                (has(rest, &["--force"]) || has_short(rest, 'f'))
                    && !(has(rest, &["--dry-run"]) || has_short(rest, 'n'))
            }
            "switch" => has(rest, &["-f", "--force", "--discard-changes"]),
            _ => false,
        };
        if discards {
            match dirty(&checkout.root, subcommand == "clean") {
                Some(true) => {
                    return Some(Refusal::new(
                        Rule::GitCommands,
                        "This call discards uncommitted changes; commit or stash them first",
                    ));
                }
                Some(false) => {}
                None => {
                    return Some(Refusal::unread(
                        Rule::GitCommands,
                        "whether the checkout has uncommitted changes",
                    ));
                }
            }
        }
    }
    None
}

fn gh(facts: &Facts<'_>, dir: &Path, args: &[String]) -> Option<Refusal> {
    // The command's first two words, past `-R`/`--repo` and their value.
    let mut words: Vec<&str> = Vec::new();
    let mut rest = args.iter().map(String::as_str);
    while words.len() < 2
        && let Some(arg) = rest.next()
    {
        match arg {
            "-R" | "--repo" => {
                rest.next();
            }
            flag if flag.starts_with('-') => {}
            word => words.push(word),
        }
    }
    let (merge, checkout) = match words.as_slice() {
        ["pr", "merge"] => (true, false),
        ["pr", "checkout"] | ["pr", "co"] => (false, true),
        _ => return None,
    };
    let target = match place(facts, dir) {
        Where::Outside => return None,
        Where::Unreadable(project) => {
            let rules = &project.rules;
            if (merge && rules.base_branch && rules.agent_merge)
                || (checkout && rules.worktree_folder)
            {
                let rule = if merge {
                    Rule::AgentMerge
                } else {
                    Rule::WorktreeFolder
                };
                return Some(Refusal::unread(rule, "the checkout this command runs in"));
            }
            return None;
        }
        Where::Git(target) => target,
    };
    let rules = &target.project.rules;
    if merge && rules.base_branch && rules.agent_merge {
        return Some(Refusal::new(
            Rule::AgentMerge,
            "This call merges a pull request",
        ));
    }
    if checkout && rules.worktree_folder && target.checkout.original() {
        return Some(Refusal::new(
            Rule::WorktreeFolder,
            "This call changes the branch of the project's original checkout",
        ));
    }
    None
}

/// The protection that is on and judges `git <subcommand>`, for a refusal
/// whose checkout could not be read; `None` when none does.
fn judging_rule(project: &super::ProjectPolicy, subcommand: &str) -> Option<Rule> {
    let rules = &project.rules;
    let judged = |names: &[&str]| names.contains(&subcommand);
    if rules.worktree_folder && judged(&["switch", "checkout"]) {
        Some(Rule::WorktreeFolder)
    } else if rules.base_branch
        && judged(&[
            "commit",
            "merge",
            "rebase",
            "cherry-pick",
            "revert",
            "am",
            "pull",
            "push",
        ])
    {
        Some(Rule::BaseBranch)
    } else if rules.git_commands
        && judged(&[
            "push", "commit", "reset", "checkout", "restore", "clean", "switch",
        ])
    {
        Some(Rule::GitCommands)
    } else {
        None
    }
}

/// What `git checkout <args>` does, read as Git reads it.
#[derive(Debug, PartialEq, Eq)]
enum CheckoutForm {
    /// It moves `HEAD`: a branch, a commit, `-b`, `--orphan`, `--detach`, `-`.
    Switch,
    /// It writes paths from the index or a tree (`-- <paths>`, `<tree> --
    /// <paths>`, `.`, or a word that is a path and no branch).
    Paths,
    /// Nothing to judge (no operand).
    Neither,
}

fn checkout_form(args: &[String], dir: &Path, checkout: &Checkout) -> CheckoutForm {
    let mut operand = None;
    for arg in args {
        match arg.as_str() {
            // Whatever came before it names a tree, and what follows paths.
            "--" => return CheckoutForm::Paths,
            "-b" | "-B" | "--orphan" | "--detach" | "-" => return CheckoutForm::Switch,
            other if other.starts_with("--orphan=") => return CheckoutForm::Switch,
            other if other.starts_with('-') => {}
            other => {
                operand.get_or_insert(other);
            }
        }
    }
    match operand {
        None => CheckoutForm::Neither,
        // Git takes a local branch over a path of the same name.
        Some(word) if checkout.has_branch(word) => CheckoutForm::Switch,
        Some(word) if word == "." || std::fs::symlink_metadata(dir.join(word)).is_ok() => {
            CheckoutForm::Paths
        }
        Some(_) => CheckoutForm::Switch,
    }
}

/// Whether `git checkout <args>` changes the branch (D-14).
fn checkout_switches(args: &[String], dir: &Path, checkout: &Checkout) -> bool {
    checkout_form(args, dir, checkout) == CheckoutForm::Switch
}

/// Whether `git checkout <args>` overwrites the working tree's changes: a
/// path form or `-f`.
fn checkout_discards(args: &[String], dir: &Path, checkout: &Checkout) -> bool {
    if has(args, &["-f", "--force"]) {
        return true;
    }
    if let Some(at) = args.iter().position(|arg| arg == "--") {
        return args.len() > at + 1;
    }
    checkout_form(args, dir, checkout) == CheckoutForm::Paths
}

/// `git restore` touches the working tree unless it names only the index.
fn restore_discards(args: &[String]) -> bool {
    let staged = has(args, &["--staged"]) || has_short(args, 'S');
    let worktree = has(args, &["--worktree"]) || has_short(args, 'W');
    let operands = args.iter().any(|arg| !arg.starts_with('-'));
    operands && (!staged || worktree)
}

fn commit_skips_hooks(args: &[String]) -> bool {
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        match arg.as_str() {
            "--" => return false,
            "--no-verify" => return true,
            // Options whose value is the next word, which may itself look
            // like `-n`.
            "-m" | "-F" | "-c" | "-C" | "-t" | "--message" | "--file" | "--author" | "--date"
            | "--template" | "--reuse-message" | "--reedit-message" | "--fixup" | "--squash"
            | "--trailer" | "--cleanup" => {
                words.next();
            }
            other => {
                if let Some(letters) = short(other) {
                    for letter in letters.chars() {
                        if letter == 'n' {
                            return true;
                        }
                        // The rest of the cluster is this option's value.
                        if "mFcCt".contains(letter) {
                            break;
                        }
                    }
                }
            }
        }
    }
    false
}

/// What one `git push` asks.
struct Push {
    force: bool,
    no_verify: bool,
    every_branch: bool,
    delete: bool,
    refspecs: Vec<String>,
}

impl Push {
    fn read(args: &[String]) -> Self {
        let mut push = Self {
            force: false,
            no_verify: false,
            every_branch: false,
            delete: false,
            refspecs: Vec::new(),
        };
        let mut positional = Vec::new();
        let mut words = args.iter();
        while let Some(arg) = words.next() {
            match arg.as_str() {
                "--" => {
                    positional.extend(words.by_ref().cloned());
                    break;
                }
                "--force" => push.force = true,
                "--no-verify" => push.no_verify = true,
                // A mirror push force-updates every ref the remote has.
                "--mirror" => {
                    push.every_branch = true;
                    push.force = true;
                }
                "--all" | "--branches" => push.every_branch = true,
                "--delete" => push.delete = true,
                "--repo" | "-o" | "--push-option" | "--receive-pack" | "--exec" => {
                    words.next();
                }
                other if other.starts_with("--") => {}
                other if short(other).is_some() => {
                    let letters = short(other).unwrap_or_default();
                    for letter in letters.chars() {
                        match letter {
                            'f' => push.force = true,
                            'd' => push.delete = true,
                            // `-o` takes the rest, or the next word.
                            'o' => break,
                            _ => {}
                        }
                    }
                }
                other => positional.push(other.to_owned()),
            }
        }
        // The first operand is the remote; the rest are refspecs.
        push.refspecs = positional.into_iter().skip(1).collect();
        if push.refspecs.iter().any(|refspec| refspec.starts_with('+')) {
            push.force = true;
        }
        push
    }

    /// Whether this push updates `base` on the remote, from a checkout on
    /// `current` (B9).
    fn reaches(&self, base: &str, current: Option<&str>) -> bool {
        if self.every_branch {
            return true;
        }
        if self.refspecs.is_empty() {
            return current == Some(base);
        }
        self.refspecs.iter().any(|refspec| {
            let refspec = refspec.trim_start_matches('+');
            let destination = if self.delete {
                refspec
            } else {
                match refspec.split_once(':') {
                    Some((source, "")) => source,
                    Some((_, destination)) => destination,
                    None if refspec == "HEAD" || refspec == "@" => current.unwrap_or_default(),
                    None => refspec,
                }
            };
            let destination = destination
                .strip_prefix("refs/heads/")
                .unwrap_or(destination);
            // A pattern refspec (`refs/heads/*:refs/heads/*`) reaches every
            // branch its one `*` matches.
            match destination.split_once('*') {
                Some((before, after)) => {
                    base.len() >= before.len() + after.len()
                        && base.starts_with(before)
                        && base.ends_with(after)
                }
                None => destination == base,
            }
        })
    }
}
