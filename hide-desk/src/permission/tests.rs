//! How Hide reads what Desk's agent asks: the agents' own request shapes,
//! the card they become, and the `hide` line Hide answers itself under
//! 에이전트대로 (PRD Desk D-92).

use super::*;
use serde_json::json;

const HIDE: &str = "/Applications/hide.app/Contents/Resources/hide";
const WORK: &str = "/state/desk/work";
const PROJECT: &str = "/r/herdr-ide";

fn place() -> Place<'static> {
    Place {
        hide: Path::new(HIDE),
        cwd: WORK,
        home: Some("/home/u"),
    }
}

fn is_hide_line(line: &str) -> bool {
    hide_line(line, &place())
}

const POSIX: &[Syntax] = &[Syntax::Posix];
/// What Windows reads a line in, checked on every system.
const WINDOWS: &[Syntax] = &[Syntax::Posix, Syntax::PowerShell];

#[test]
fn a_line_of_hide_commands_only_is_hide_s_to_answer() {
    // `hide` commands joined, piped into each other and redirected nowhere,
    // in both shells Windows may run.
    for line in [
        "hide task show agent-18399",
        "hide agent list; hide ps",
        "hide agent list && hide ps || hide inbox",
        "hide agent read agent-18238 --lines 80 2>&1",
        "hide mission show mission-1 >/dev/null",
        "hide agent archive agent-3",
        &format!("{HIDE} inbox"),
        "hide request send a --intent i --body 'use `hide task` here'",
    ] {
        assert!(hide_line_in(line, &place(), POSIX), "{line}");
        assert!(hide_line_in(line, &place(), WINDOWS), "{line}");
    }
    // A letter's body from a heredoc that substitutes nothing, which only
    // a POSIX shell reads: PowerShell has no heredoc, so on Windows the
    // agent asks.
    let heredoc =
        "hide request send a --intent i --body-file - <<'EOF'\nrun `cargo test` first\nEOF";
    assert!(hide_line_in(heredoc, &place(), POSIX));
    assert!(!hide_line_in(heredoc, &place(), WINDOWS));
}

#[test]
fn a_line_that_runs_anything_but_hide_or_touches_a_file_is_the_agent_s_to_ask() {
    for line in [
        // Another program beside `hide`, a reader included.
        "hide ps | head -c 3000",
        "hide agent list | jq -r '.value.items[]'",
        "hide agent list; echo ---; hide ps",
        "hide mission show mission-1 >/dev/null || true",
        "hide agent list; touch x",
        "hide agent list | sh",
        "hide agent list | xargs rm",
        "env hide agent list",
        // A file written or read.
        "hide agent list > agents.json",
        "hide ps >& out.txt",
        "hide ps; hide inbox >& ~/.zshenv",
        "hide ps &> out.txt",
        "hide ps; hide inbox >>& tr",
        "hide ps >>&! out.txt",
        "hide ps 3>& out.txt",
        "hide ps >&2-",
        "hide ps >& $x",
        "hide ps >&$x",
        "hide ps &>! /dev/null",
        "hide ps &>| out.txt",
        // The file can be Desk's own `hide`, which the line would overwrite.
        "hide ps >>& hide",
        "hide ps >>&| hide",
        "hide ps &>>| hide",
        "hide ps >>| hide",
        "hide ps >! hide",
        &format!("hide ps &>| {HIDE}"),
        &format!("hide ps >>& {HIDE}"),
        "hide ps 2>&1-",
        // A redirection with no word after it, which the shell refuses.
        "hide ps >",
        "hide ps >> &",
        "hide ps > ; hide inbox",
        "hide request send a --body-file - < notes.md",
        // A substitution, a backquote, a heredoc that substitutes.
        "hide request send a --body \"$(cat notes.md)\"",
        "hide request send a --intent i --body \"$(cat <<'EOF'\nhi\nEOF\n)\"",
        "hide request send a --body \"`touch y`\"",
        "hide request send a --body-file - <<EOF\n$(touch y)\nEOF",
        // Another `hide`, a variable before it, a function, a compound.
        "PATH=/tmp hide agent list",
        "/tmp/hide agent list",
        "./hide agent list",
        "hide() { touch x; }; hide agent list",
        "if true; then hide ps; fi",
        // Nothing of Hide's runs.
        "echo hi",
        "",
    ] {
        assert!(!is_hide_line(line), "{line}");
    }
    // `hide.exe` and another case are `hide` only where the system finds a
    // program so.
    assert_eq!(is_hide_line("hide.exe ps"), cfg!(windows));
    assert_eq!(is_hide_line("HIDE ps"), cfg!(windows));
}

/// Every retired spelling of the one table (`hide_shell::retired`) counts
/// as the command it is now: a session file keeps the spelling a call ran
/// with for good.
#[test]
fn a_retired_spelling_counts_as_the_command_it_is_now() {
    let tails = [
        "show",
        "list",
        "send agent-3 --intent k --body hi",
        "spawn --name a --intent k",
    ];
    let mut counted = 0;
    for row in hide_shell::retired::TABLE.commands {
        for tail in tails {
            let now = hide_step(&format!("hide {} {tail}", row.now.join(" ")));
            counted += usize::from(now != Step::Other);
            let old = format!("hide {} {tail}", row.old.join(" "));
            assert_eq!(hide_step(&old), now, "{old}");
        }
    }
    assert!(counted > 0, "no retired spelling names a step Desk counts");
    assert_eq!(hide_step("hide plan show"), Step::Sessions);
}

#[test]
fn a_turns_tool_calls_count_as_sessions_read_letters_and_starts() {
    let run = |command: &str| step(&json!({"kind": "execute", "rawInput": {"command": command}}));
    assert_eq!(run("hide agent show agent-3"), Step::Sessions);
    assert_eq!(run("hide ps && hide agent list"), Step::Sessions);
    assert_eq!(
        run("hide request send agent-3 --intent k --body hi"),
        Step::Letter
    );
    assert_eq!(run("hide agent spawn --name a --intent k"), Step::Start);
    assert_eq!(run("hide agent archive agent-3"), Step::Other);
    assert_eq!(run("ls -la"), Step::Other);
    assert_eq!(
        step(&json!({"kind": "read", "locations": [{"path": "/a"}]})),
        Step::Files
    );
}

/// claude-agent-acp 0.99.1's options for a Bash call (`withOptionalUpdate`
/// in its `permissions/options/shared.js`), in its order.
fn claude_options() -> Value {
    json!([
        {"optionId": "allow-once", "name": "Yes", "kind": "allow_once"},
        {"optionId": "allow-with-updates", "name": "Yes, and don't ask again for npm run commands", "kind": "allow_always"},
        {"optionId": "reject", "name": "No", "kind": "reject_once"}
    ])
}

#[test]
fn a_request_keeps_the_agent_s_options_as_it_names_them_in_its_order() {
    let request = read_request(
        &json!({"toolCall": {"toolCallId": "t1", "kind": "execute", "title": "npm run build",
            "rawInput": {"command": "npm run build", "description": "Build the web"}},
            "options": claude_options()}),
        None,
    );
    assert_eq!(
        request.choices,
        [
            Choice {
                id: "allow-once".into(),
                kind: ChoiceKind::AllowOnce,
                name: "Yes".into()
            },
            Choice {
                id: "allow-with-updates".into(),
                kind: ChoiceKind::AllowAlways,
                name: "Yes, and don't ask again for npm run commands".into()
            },
            Choice {
                id: "reject".into(),
                kind: ChoiceKind::RejectOnce,
                name: "No".into()
            },
        ]
    );
    assert_eq!(allow_once(&request.choices), Some("allow-once"));
    let card = card(&request.action, &place());
    assert_eq!(
        (
            card.kind,
            card.command.as_str(),
            card.folder.as_deref(),
            card.reason.as_deref()
        ),
        (
            CardKind::Command,
            "npm run build",
            Some(WORK),
            Some("Build the web")
        )
    );
    // On the wire, the kind a later agent may add reads as `other`.
    let unknown: Choice =
        serde_json::from_value(json!({"id": "x", "kind": "allow_session", "name": "Session"}))
            .unwrap();
    assert_eq!(unknown.kind, ChoiceKind::Other);
    assert_eq!(
        serde_json::to_value(&request.choices[1]).unwrap(),
        json!({"id": "allow-with-updates", "kind": "allow_always",
            "name": "Yes, and don't ask again for npm run commands"})
    );
}

#[test]
fn claude_s_reads_and_edits_are_read_with_their_files() {
    let read = read_request(
        &json!({"toolCall": {"toolCallId": "t2", "kind": "read", "title": "Read",
            "locations": [{"path": "/r/herdr-ide/README.md"}]}, "options": claude_options()}),
        None,
    );
    assert_eq!(
        read.action,
        Action::Read {
            what: "/r/herdr-ide/README.md".into()
        }
    );
    let edit = read_request(
        &json!({"toolCall": {"toolCallId": "t3", "kind": "edit", "title": "Edit",
            "rawInput": {"file_path": "/r/web/src/PaneHeader.tsx"}},
            "options": claude_options()}),
        None,
    );
    let card = card(&edit.action, &place());
    assert_eq!(card.kind, CardKind::Edit);
    assert_eq!(card.command, "/r/web/src/PaneHeader.tsx");
}

#[test]
fn codex_s_request_is_read_with_the_call_it_announced() {
    // codex-acp 2.3 sent the request with only the id and the kind.
    let announced = json!({"sessionUpdate": "tool_call", "toolCallId": "exec-1",
        "kind": "execute", "title": "touch x",
        "rawInput": {"command": ["/bin/zsh", "-lc", "touch x"], "cwd": "/r"}});
    let request = read_request(
        &json!({"toolCall": {"toolCallId": "exec-1", "kind": "execute", "status": "pending"},
            "options": [{"optionId": "allow_once", "kind": "allow_once"},
                        {"optionId": "cancel", "kind": "reject_once"}]}),
        Some(&announced),
    );
    let card = card(&request.action, &place());
    assert_eq!(card.command, "touch x");
    assert_eq!(card.folder.as_deref(), Some("/r"));
    assert_eq!(allow_once(&request.choices), Some("allow_once"));
}

#[test]
fn a_request_hide_can_tell_little_of_is_still_a_card_named_as_the_agent_named_it() {
    let bare = read_request(
        &json!({"toolCall": {"toolCallId": "exec-2", "kind": "execute", "title": "Run command"},
            "options": claude_options()}),
        None,
    );
    assert_eq!(
        bare.action,
        Action::Other {
            title: "Run command".into(),
            reason: None
        }
    );
    let untitled = read_request(
        &json!({"toolCall": {"toolCallId": "t", "kind": "fetch"},
            "options": [{"optionId": "always", "kind": "allow_always"}]}),
        None,
    );
    assert_eq!(card(&untitled.action, &place()).command, "fetch");
    // No allowance for this one call: under 다 허용 the operator chooses.
    assert_eq!(allow_once(&untitled.choices), None);
}

#[test]
fn each_agent_s_own_request_for_a_command_or_an_edit_becomes_a_card() {
    // The requests OpenCode 2.0.29, omp 18.7.0 and agy 1.4.12 sent for
    // `touch x` and for writing notes.txt, measured on 2026-10-31.
    let options = json!([{"optionId": "once", "kind": "allow_once"},
                         {"optionId": "reject", "kind": "reject_once"}]);
    let card_of = |call: Value| {
        let request = read_request(&json!({"toolCall": call, "options": options.clone()}), None);
        card(&request.action, &place())
    };
    for raw in [
        json!({"command": "touch x", "cwd": "/r"}),
        json!({"command": "touch x"}),
        json!({"CommandLine": "touch x", "IsDaemon": false}),
    ] {
        let card = card_of(
            json!({"toolCallId": "c1", "kind": "execute", "title": "touch x", "rawInput": raw}),
        );
        assert_eq!(
            (card.kind, card.command.as_str()),
            (CardKind::Command, "touch x")
        );
    }
    let card = card_of(
        json!({"toolCallId": "e1", "kind": "edit", "title": "notes.txt",
        "locations": [{"path": "/r/notes.txt"}], "rawInput": {"Path": "/r/notes.txt"}}),
    );
    assert_eq!(
        (card.kind, card.command.as_str()),
        (CardKind::Edit, "/r/notes.txt")
    );
}

#[test]
fn every_card_names_its_folder() {
    let named = card(
        &Action::Command {
            command: "rm -rf /tmp/x".into(),
            folder: Some(PROJECT.into()),
            reason: None,
        },
        &place(),
    );
    assert_eq!(named.folder.as_deref(), Some(PROJECT));

    let edit = |paths: &[&str]| {
        card(
            &Action::Edit {
                paths: paths.iter().map(|path| (*path).to_owned()).collect(),
                reason: None,
            },
            &place(),
        )
        .folder
    };
    assert_eq!(
        edit(&["/r/herdr-ide/web/a.ts", "/r/herdr-ide/web/src/b.ts"]).as_deref(),
        Some("/r/herdr-ide/web")
    );
    assert_eq!(edit(&["notes.md"]).as_deref(), Some(WORK));
    assert_eq!(edit(&["/a/x", "/b/y"]).as_deref(), Some("/"));
    // A Windows drive's root, read by the wire's spelling on every system.
    assert_eq!(edit(&["C:/a/x", "C:/b/y"]).as_deref(), Some("C:/"));
    assert_eq!(edit(&["C:/a/x", "D:/a/y"]), None);
    // A path Hide cannot name leaves the folder out rather than guess.
    assert_eq!(edit(&["$HOME/x"]), None);

    let read = card(
        &Action::Read {
            what: "src/lib.rs".into(),
        },
        &place(),
    );
    assert_eq!(read.folder.as_deref(), Some(WORK));
}

#[test]
fn a_request_keeps_at_most_its_first_options_each_named_within_its_cap() {
    let mut options: Vec<Value> = (0..12)
        .map(
            |n| json!({"optionId": format!("o{n}"), "kind": "allow_once", "name": "é".repeat(400)}),
        )
        .collect();
    // An id too long to send back whole is left out, not cut.
    options.insert(
        1,
        json!({"optionId": "x".repeat(MAX_OPTION_ID_BYTES + 1), "kind": "reject_once"}),
    );
    let request = read_request(
        &json!({"toolCall": {"toolCallId": "t", "kind": "execute", "title": "ls"}, "options": options}),
        None,
    );
    assert_eq!(request.choices.len(), MAX_OPTIONS);
    assert_eq!(request.choices[0].id, "o0");
    assert_eq!(request.choices[1].id, "o1");
    assert!(
        request
            .choices
            .iter()
            .all(|choice| choice.name.len() <= MAX_OPTION_NAME_BYTES)
    );
    assert!(request.choices[0].name.chars().all(|c| c == 'é'));
}
