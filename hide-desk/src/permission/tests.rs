//! The permission table and the irreversible set, read against the PRD's
//! own examples (Desk D-82, D-83, B46, B47) and the agents' request shapes.

use super::*;
use serde_json::json;

const HIDE: &str = "/Applications/hide.app/Contents/Resources/hide";
const WORK: &str = "/state/desk/work";
const PROJECT: &str = "/r/herdr-ide";

/// The core's facts: one project, and the sessions that work now.
struct Known {
    working: &'static [&'static str],
}

impl Facts for Known {
    fn working(&self, agent: &str) -> bool {
        self.working.contains(&agent)
    }

    fn in_project(&self, path: &str) -> bool {
        hide_platform::path::wire_relative(PROJECT, path).is_ok_and(|below| !below.is_root())
    }
}

const NONE_WORKING: Known = Known { working: &[] };

fn place() -> Place<'static> {
    Place {
        hide: Path::new(HIDE),
        cwd: WORK,
        home: Some("/home/u"),
    }
}

fn command(line: &str) -> Action {
    Action::Command {
        command: line.to_owned(),
        folder: None,
        reason: Some("Why it runs".to_owned()),
    }
}

fn reading(line: &str) -> Reading {
    classify(&command(line), &place())
}

fn asks_with(line: &str, table: &Table, facts: &Known, allowed: &BTreeSet<Ask>) -> Vec<Ask> {
    decide(&reading(line), table, facts, allowed).asks
}

/// What a line asks under the default table, nothing allowed yet.
fn asks(line: &str) -> Vec<Ask> {
    asks_with(line, &Table::default(), &NONE_WORKING, &BTreeSet::new())
}

const ALL_ASK: Table = Table {
    hide: Answer::Ask,
    read: Answer::Ask,
    shell: Answer::Ask,
    edit: Answer::Ask,
};

#[test]
fn every_row_runs_until_the_operator_changes_it_and_an_old_store_reads_as_that() {
    assert_eq!(Table::default(), serde_json::from_value(json!({})).unwrap());
    let table: Table = serde_json::from_value(json!({"shell": "ask"})).unwrap();
    assert_eq!(table.answer(Row::Shell), Answer::Ask);
    for row in [Row::Hide, Row::Read, Row::Edit] {
        assert_eq!(table.answer(row), Answer::Run, "{row:?}");
    }
    assert_eq!(
        serde_json::to_value(Table::default()).unwrap(),
        json!({"hide": "run", "read": "run", "shell": "run", "edit": "run"})
    );
}

#[test]
fn the_prd_s_hide_lines_run_without_a_card_by_default() {
    // B46's examples, and the ones the mockup's old card showed.
    for line in [
        "hide agent list | jq -r '.value.items[] | select(.runtime!=\"ended\") | [.id, .name] | @tsv'",
        "hide ps | head -c 3000",
        "hide agent read agent-18238 --lines 80 2>&1 | tail -60",
        "hide plan show agent-18399",
        "hide agent list; echo ---; hide ps",
        "hide agent list && hide ps | sort | grep -v ended",
        "hide mission show mission-1",
        "hide stop",
        &format!("{HIDE} inbox"),
        "hide request send a --intent i --body \"$(cat notes.md)\"",
        "hide request send a --intent i --body-file - <<'EOF'\nrun `cargo test` first\nEOF",
        "hide request send a --intent i --body 'use `hide plan` here'",
    ] {
        assert_eq!(reading(line).row, Row::Hide, "{line}");
        assert_eq!(asks(line), [], "{line}");
    }
}

#[test]
fn a_hide_line_that_writes_or_runs_more_than_reads_is_another_shell_command() {
    for line in [
        "hide agent list > agents.json",
        "hide agent list | tee agents.json",
        "hide agent list; touch x",
        "PATH=/tmp hide agent list",
        "/tmp/hide agent list",
        "hide() { touch x; }; hide agent list",
        "hide agent list | sort -o sorted.txt",
    ] {
        assert_eq!(reading(line).row, Row::Shell, "{line}");
    }
    let shell_asks = Table {
        shell: Answer::Ask,
        ..Table::default()
    };
    assert_eq!(
        asks_with(
            "hide agent list > agents.json",
            &shell_asks,
            &NONE_WORKING,
            &BTreeSet::new()
        ),
        [Ask::Row(Row::Shell)]
    );
    assert_eq!(
        asks_with(
            "hide agent list | jq .",
            &shell_asks,
            &NONE_WORKING,
            &BTreeSet::new()
        ),
        []
    );
}

#[test]
fn each_row_set_to_ask_asks_for_its_own_actions() {
    let read = Action::Read {
        what: "/r/herdr-ide/README.md".into(),
    };
    let edit = Action::Edit {
        paths: vec!["/r/herdr-ide/web/src/App.tsx".into()],
        delete: false,
        reason: None,
    };
    let other = Action::Other {
        title: "Fetch https://example.com".into(),
        reason: None,
    };
    for (action, row) in [
        (command("hide agent list"), Row::Hide),
        (command("rg -n desk_spawn /r/herdr-ide"), Row::Read),
        (command("cat README.md | wc -l"), Row::Read),
        (command("git -C /r/herdr-ide log --oneline -5"), Row::Read),
        (read, Row::Read),
        (command("cargo test"), Row::Shell),
        (command("git commit -m x"), Row::Shell),
        (other, Row::Shell),
        (edit, Row::Edit),
    ] {
        let reading = classify(&action, &place());
        assert_eq!(reading.row, row, "{action:?}");
        assert_eq!(
            decide(&reading, &Table::default(), &NONE_WORKING, &BTreeSet::new()).asks,
            [],
            "{action:?}"
        );
        assert_eq!(
            decide(&reading, &ALL_ASK, &NONE_WORKING, &BTreeSet::new()).asks,
            [Ask::Row(row)],
            "{action:?}"
        );
    }
}

#[test]
fn archiving_a_working_session_always_asks_and_an_ended_one_follows_the_hide_row() {
    let working = Known {
        working: &["agent-3"],
    };
    let line = "hide agent archive agent-3";
    let decision = decide(
        &reading(line),
        &Table::default(),
        &working,
        &BTreeSet::new(),
    );
    assert_eq!(
        decision.asks,
        [Ask::Irreversible(Irreversible::EndWorkingSession)]
    );
    assert_eq!(decision.sessions, ["agent-3"]);
    assert_eq!(
        asks_with(line, &Table::default(), &NONE_WORKING, &BTreeSet::new()),
        []
    );
    let hide_asks = Table {
        hide: Answer::Ask,
        ..Table::default()
    };
    assert_eq!(
        asks_with(line, &hide_asks, &NONE_WORKING, &BTreeSet::new()),
        [Ask::Row(Row::Hide)]
    );
    // Closing panes through Herdr names no session Hide can check.
    assert_eq!(
        asks("herdr pane close w1:p2"),
        [Ask::Irreversible(Irreversible::EndWorkingSession)]
    );
}

#[test]
fn a_push_that_overwrites_the_remote_always_asks() {
    for line in [
        "git push -f",
        "git push --force origin desk/x",
        "git push --force-with-lease=main origin main",
        "git push origin +main",
        "git -C /r/herdr-ide push -uf origin x",
        "bash -lc 'cd /r/herdr-ide && git push --mirror'",
        "sudo git push --force-if-includes",
        "echo $(git push -f)",
    ] {
        assert_eq!(
            asks(line),
            [Ask::Irreversible(Irreversible::ForcePush)],
            "{line}"
        );
    }
    for line in [
        "git push origin main",
        "git push -u origin desk/x",
        "git push --no-force-with-lease origin x",
    ] {
        assert_eq!(asks(line), [], "{line}");
    }
}

#[test]
fn deleting_outside_every_project_always_asks() {
    let outside = [Ask::Irreversible(Irreversible::DeleteOutside)];
    for line in [
        "rm -rf /tmp/build",
        "rm -rf build",
        "rm -rf /r/herdr-ide",
        "rm -rf /r/herdr-ide/../other",
        "rm -rf ~/notes",
        "rm -rf $TARGET",
        "rm -- -weird",
        "find /tmp -name '*.log' -delete",
        "xargs rm < list.txt",
        "git -C /tmp/x clean -fdx",
        "cd /tmp && rm -rf x",
        "unlink /r/herdr-ide-other/x",
    ] {
        assert_eq!(asks(line), outside, "{line}");
    }
    for line in [
        "rm -rf /r/herdr-ide/build",
        "cd /r/herdr-ide && rm -rf target web/dist",
        "find /r/herdr-ide -name '*.o' -exec rm {} +",
        "git -C /r/herdr-ide clean -fdx",
        "git clean -n",
    ] {
        assert_eq!(asks(line), [], "{line}");
    }
    let decision = decide(
        &reading("rm /r/herdr-ide/a /tmp/b"),
        &Table::default(),
        &NONE_WORKING,
        &BTreeSet::new(),
    );
    assert_eq!(decision.outside.as_deref(), Some("/tmp/b"));
    let removed = Action::Edit {
        paths: vec!["/etc/hosts".into()],
        delete: true,
        reason: None,
    };
    assert_eq!(
        decide(
            &classify(&removed, &place()),
            &Table::default(),
            &NONE_WORKING,
            &BTreeSet::new()
        )
        .asks,
        outside
    );
}

#[test]
fn a_folder_the_agent_names_is_where_relative_paths_are() {
    let in_project = Action::Command {
        command: "rm -rf build".into(),
        folder: Some(PROJECT.into()),
        reason: None,
    };
    assert_eq!(
        decide(
            &classify(&in_project, &place()),
            &Table::default(),
            &NONE_WORKING,
            &BTreeSet::new()
        )
        .asks,
        []
    );
}

#[test]
fn a_program_hide_cannot_read_always_asks() {
    for line in [
        "$CMD agent list",
        "`echo rm` -rf /",
        "eval \"$LINE\"",
        "env -S 'rm -rf /' x",
        "echo 'unclosed",
    ] {
        assert!(
            asks(line).contains(&Ask::Irreversible(Irreversible::Unreadable)),
            "{line}"
        );
    }
    // A command inside a command is read, to a depth.
    assert_eq!(
        asks("bash -c \"sh -c 'git push -f'\""),
        [Ask::Irreversible(Irreversible::ForcePush)]
    );
    let deep = (0..=MAX_DEPTH).fold("hide ps".to_owned(), |inner, _| {
        format!("echo \"$({inner})\"")
    });
    assert!(asks(&deep).contains(&Ask::Irreversible(Irreversible::Unreadable)));
}

#[test]
fn what_the_operator_allowed_in_this_conversation_stops_asking_that_kind_only() {
    let mut allowed = BTreeSet::new();
    allowed.insert(Ask::Row(Row::Shell));
    assert_eq!(
        asks_with("cargo test", &ALL_ASK, &NONE_WORKING, &allowed),
        []
    );
    assert_eq!(
        asks_with("hide agent list", &ALL_ASK, &NONE_WORKING, &allowed),
        [Ask::Row(Row::Hide)]
    );
    // A row allowed does not cover the irreversible kinds.
    assert_eq!(
        asks_with("git push -f", &ALL_ASK, &NONE_WORKING, &allowed),
        [Ask::Irreversible(Irreversible::ForcePush)]
    );
    allowed.insert(Ask::Irreversible(Irreversible::ForcePush));
    assert_eq!(
        asks_with("git push -f", &ALL_ASK, &NONE_WORKING, &allowed),
        []
    );
    assert_eq!(
        asks_with("rm -rf /tmp/x", &Table::default(), &NONE_WORKING, &allowed),
        [Ask::Irreversible(Irreversible::DeleteOutside)]
    );
}

#[test]
fn a_card_asks_irreversible_kinds_before_its_row() {
    assert_eq!(
        asks_with(
            "rm -rf /tmp/x; git push -f",
            &ALL_ASK,
            &NONE_WORKING,
            &BTreeSet::new()
        ),
        [
            Ask::Irreversible(Irreversible::ForcePush),
            Ask::Irreversible(Irreversible::DeleteOutside),
            Ask::Row(Row::Shell),
        ]
    );
    assert_eq!(
        serde_json::to_value(Ask::Irreversible(Irreversible::EndWorkingSession)).unwrap(),
        json!({"kind": "irreversible", "value": "end_working_session"})
    );
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

fn options() -> Value {
    json!([
        {"optionId": "allow-once", "name": "Yes", "kind": "allow_once"},
        {"optionId": "allow-always", "name": "Always", "kind": "allow_always"},
        {"optionId": "reject", "name": "No", "kind": "reject_once"}
    ])
}

#[test]
fn claude_s_requests_are_read_with_their_reason_files_and_reads() {
    let bash = read_request(
        AdapterId::Claude,
        &json!({"toolCall": {"toolCallId": "t1", "kind": "execute", "title": "ls -la",
            "rawInput": {"command": "ls -la", "description": "Why it runs"}},
            "options": options()}),
        None,
    );
    assert_eq!(bash.allow_once(), Some("allow-once"));
    let reading = classify(readable(&bash).unwrap(), &place());
    assert_eq!(reading.card.command, "ls -la");
    assert_eq!(reading.card.reason.as_deref(), Some("Why it runs"));
    let read = read_request(
        AdapterId::Claude,
        &json!({"toolCall": {"toolCallId": "t2", "kind": "read", "title": "Read",
            "locations": [{"path": "/r/herdr-ide/README.md"}]}, "options": options()}),
        None,
    );
    assert_eq!(
        read.action,
        Some(Action::Read {
            what: "/r/herdr-ide/README.md".into()
        })
    );
    let edit = read_request(
        AdapterId::Claude,
        &json!({"toolCall": {"toolCallId": "t3", "kind": "edit", "title": "Edit",
            "rawInput": {"file_path": "/r/web/src/PaneHeader.tsx"}},
            "options": options()}),
        None,
    );
    let reading = classify(readable(&edit).unwrap(), &place());
    assert_eq!(reading.row, Row::Edit);
    assert_eq!(reading.card.kind, CardKind::Edit);
    assert_eq!(reading.card.command, "/r/web/src/PaneHeader.tsx");
}

#[test]
fn codex_s_request_is_read_with_the_call_it_announced() {
    // codex-acp 2.3 sent the request with only the id and the kind.
    let announced = json!({"sessionUpdate": "tool_call", "toolCallId": "exec-1",
        "kind": "execute", "title": "touch x",
        "rawInput": {"command": ["/bin/zsh", "-lc", "touch x"], "cwd": "/r"}});
    let request = read_request(
        AdapterId::Codex,
        &json!({"toolCall": {"toolCallId": "exec-1", "kind": "execute", "status": "pending"},
            "options": [{"optionId": "allow_once", "kind": "allow_once"},
                        {"optionId": "cancel", "kind": "reject_once"}]}),
        Some(&announced),
    );
    let reading = classify(readable(&request).unwrap(), &place());
    assert_eq!(reading.card.command, "touch x");
    assert_eq!(reading.card.folder.as_deref(), Some("/r"));
    assert_eq!(reading.row, Row::Shell);
    assert_eq!(request.reject(), Some("cancel"));
}

#[test]
fn a_request_hide_cannot_read_does_not_run() {
    let bare = read_request(
        AdapterId::Codex,
        &json!({"toolCall": {"toolCallId": "exec-2", "kind": "execute"}, "options": options()}),
        None,
    );
    assert_eq!(bare.action, None);
    assert_eq!(readable(&bare), None);
    let no_once = read_request(
        AdapterId::Claude,
        &json!({"toolCall": {"toolCallId": "t", "kind": "read"},
            "options": [{"optionId": "always", "kind": "allow_always"}]}),
        None,
    );
    assert_eq!(readable(&no_once), None);
}

#[test]
fn pi_s_confirm_from_desk_s_extension_is_read_and_another_is_not() {
    let confirm = |title: &str, message: &str| {
        read_request(
            AdapterId::Pi,
            &json!({"toolCall": {"toolCallId": "pi-ui-1", "kind": "other", "title": title,
                "rawInput": {"method": "confirm", "title": title, "message": message}},
                "options": [{"optionId": "yes", "kind": "allow_once"},
                            {"optionId": "no", "kind": "reject_once"}]}),
            None,
        )
    };
    let bash = confirm(
        crate::adapter::PI_CONFIRM_TITLE,
        r#"{"hide_desk":1,"tool":"bash","input":{"command":"hide agent list | jq ."}}"#,
    );
    assert_eq!(classify(readable(&bash).unwrap(), &place()).row, Row::Hide);
    let read = confirm(
        crate::adapter::PI_CONFIRM_TITLE,
        r#"{"hide_desk":1,"tool":"read","input":{"path":"notes.md"}}"#,
    );
    assert_eq!(classify(readable(&read).unwrap(), &place()).row, Row::Read);
    let write = confirm(
        crate::adapter::PI_CONFIRM_TITLE,
        r#"{"hide_desk":1,"tool":"write","input":{"path":"notes.md","content":"x"}}"#,
    );
    assert_eq!(classify(readable(&write).unwrap(), &place()).row, Row::Edit);
    let other = confirm("Delete branch?", "yes or no");
    assert_eq!(readable(&other), None);
}

const UNREADABLE: Ask = Ask::Irreversible(Irreversible::Unreadable);
const OUTSIDE: Ask = Ask::Irreversible(Irreversible::DeleteOutside);

#[test]
fn code_a_shell_or_an_interpreter_runs_that_hide_does_not_read_always_asks() {
    for line in [
        "printf '%s' 'rm -f /fixture/outside/victim' | sh",
        "sh script.sh",
        "bash -s < steps.sh",
        "bash <<EOF\nrm -f /fixture/outside/victim\nEOF",
        "python3 -c 'import shutil; shutil.rmtree(\"/fixture/outside\")'",
        "python3.12 tool.py",
        "node -e 'require(\"fs\").rmSync(\"/fixture/outside\", {recursive: true})'",
        "hide ps | awk '{system(\"rm -f /fixture/outside/victim\")}'",
        "fish -c 'rm (pwd)/victim'",
        "xargs sh < list.txt",
        r"find . -name '*.sh' -exec sh {} \;",
        "source ./setup.sh",
        "git -c core.pager='rm -f /fixture/outside/victim' log",
        "git --exec-path=/fixture/bin status",
    ] {
        assert!(asks(line).contains(&UNREADABLE), "{line}");
    }
    // A shell line given to `-c` is read like any other.
    assert_eq!(asks("sh -c 'rm -f /fixture/outside/victim'"), [OUTSIDE]);
    assert_eq!(asks("bash -c 'hide ps | head'"), []);
}

#[test]
fn what_a_heredoc_body_substitutes_is_read_unless_its_end_word_is_quoted() {
    for line in [
        "cat <<EOF\n$(rm -f /fixture/outside/victim)\nEOF",
        "cat <<-EOF\n\t`rm -f /fixture/outside/victim`\n\tEOF",
        "hide request send a --intent i --body-file - <<EOF\nnote $(rm -rf /fixture/outside)\nEOF",
    ] {
        assert_eq!(asks(line), [OUTSIDE], "{line}");
    }
    for line in [
        "cat <<'EOF'\n$(rm -f /fixture/outside/victim)\nEOF",
        "cat <<\"EOF\"\n`rm -f /fixture/outside/victim`\nEOF",
        "cat <<\\EOF\n$(rm -f /fixture/outside/victim)\nEOF",
        "cat <<EOF\nonly \\$(text) here\nEOF",
    ] {
        assert_eq!(asks(line), [], "{line}");
    }
}

#[test]
fn a_program_named_by_path_or_run_with_a_changed_environment_always_asks() {
    for line in [
        "/fixture/bin/cat README.md",
        "./cat README.md",
        "hide ps | /fixture/bin/cat",
        "PATH=/fixture/bin cat README.md",
        "export PATH=/fixture/bin; cat README.md",
        "path=(/fixture/bin $path); cat README.md",
        "for PATH in /fixture/bin; do cat README.md; done",
        "env PATH=/fixture/bin hide ps",
        "LESSOPEN='|rm -f /fixture/outside/victim %s' less README.md",
        "hash -p /fixture/bin/cat cat; cat README.md",
        "{rm,-f,/fixture/outside/victim}",
        "alias cat='rm -f /fixture/outside/victim'; cat x",
    ] {
        assert!(
            asks(line).contains(&UNREADABLE) || asks(line).contains(&OUTSIDE),
            "{line}"
        );
    }
    // A loop's own lowercase variable changes no program.
    assert_eq!(
        asks("for id in agent-1 agent-2; do hide agent read $id --lines 20; done"),
        []
    );
    assert_eq!(
        reading("for id in agent-1 agent-2; do hide agent read $id; done").row,
        Row::Hide
    );
}

#[test]
fn a_relative_delete_is_judged_in_every_folder_the_shell_may_be_in_then() {
    for line in [
        // A subshell's `cd` ends with it.
        "cd /fixture/outside; (cd /r/herdr-ide); rm -f victim",
        "cd /fixture/outside && (cd /r/herdr-ide) && rm -f victim",
        // A `cd` may fail, and what follows `;` or `||` runs anyway.
        "cd /r/herdr-ide/missing; rm -f victim",
        "cd /r/herdr-ide || rm -f victim",
        "cd /fixture/outside || cd /r/herdr-ide && rm -f victim",
        // A function, a loop or `eval` may move the shell anywhere.
        "f() { cd /fixture/outside; }; cd /r/herdr-ide && f && rm -f victim",
        "cd /r/herdr-ide && for d in a b; do cd $d; done; rm -f victim",
        "cd /r/herdr-ide && eval cd /fixture/outside && rm -f victim",
        // `env` runs a program named `cd`, which leaves the shell's folder.
        "env cd /r/herdr-ide && rm -f victim",
        // A pattern in a folder's name may match a link.
        "rm -f /r/herdr-ide/*/victim",
        "rm -f /r/herdr-ide/{a,../../fixture/outside}/victim",
    ] {
        assert_eq!(asks(line), [OUTSIDE], "{line}");
    }
    for line in [
        "cd /r/herdr-ide && rm -rf target web/dist",
        "(cd /r/herdr-ide && rm -rf target)",
        "cd /r/herdr-ide && cd ./web && rm -rf dist",
        "cd /r/herdr-ide && command cd ./web/src && rm -f old.ts",
        "rm -f /r/herdr-ide/*.log",
    ] {
        assert_eq!(asks(line), [], "{line}");
    }
}

#[test]
fn a_delete_after_a_command_that_may_make_a_link_cannot_be_named() {
    for line in [
        "ln -s /fixture/outside /r/herdr-ide/link && rm -f /r/herdr-ide/link/victim",
        "rm -f /r/herdr-ide/link/victim | ln -s /fixture/outside /r/herdr-ide/link",
        "git checkout links && rm -rf /r/herdr-ide/link/",
        "echo $(ln -s /fixture/outside /r/herdr-ide/link); rm -f /r/herdr-ide/link/victim",
    ] {
        assert_eq!(asks(line), [OUTSIDE], "{line}");
    }
    // What only reads, deletes or makes folders and files makes no link.
    assert_eq!(
        asks(
            "ls /r/herdr-ide && rm -rf /r/herdr-ide/build && mkdir /r/herdr-ide/build && rm -f /r/herdr-ide/build/x"
        ),
        []
    );
}

/// The core's facts about one project on this machine's disk.
struct OnDisk {
    project: String,
}

impl Facts for OnDisk {
    fn working(&self, _: &str) -> bool {
        false
    }

    fn in_project(&self, path: &str) -> bool {
        hide_platform::path::wire_relative(&self.project, path).is_ok_and(|below| !below.is_root())
    }
}

#[test]
fn a_delete_through_a_link_is_judged_where_the_link_leads() {
    let root = tempfile::tempdir().unwrap();
    let wire = |path: &Path| hide_platform::path::to_wire_lossy(path);
    let project = root.path().join("project");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(project.join("real")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    hide_platform::fs::link::create_link(&outside, &project.join("link")).unwrap();
    let facts = OnDisk {
        project: wire(&hide_platform::fs::identity::canonical(&project).unwrap()),
    };
    // The paths are written as the operator's shell names them, which may
    // itself pass through a link (macOS's temporary folder does).
    let project = wire(&project);
    let asks = |line: &str| {
        let mut reading = reading(line);
        follow_links(&mut reading);
        decide(&reading, &Table::default(), &facts, &BTreeSet::new()).asks
    };
    for line in [
        format!("rm -f {project}/link/victim"),
        format!("rm -rf {project}/link/"),
        format!("cd {project}/link && rm -f victim"),
        format!("rm -rf {project}/link/../victim"),
        format!("find {project}/link -name '*.log' -delete"),
        format!("git -C {project}/link clean -fdx"),
    ] {
        assert_eq!(asks(&line), [OUTSIDE], "{line}");
    }
    for line in [
        // Deleting the link removes the link, inside the project.
        format!("rm -f {project}/link"),
        format!("rm -rf {project}/real/victim {project}/new/victim"),
        format!("cd {project}/real && rm -f ../victim"),
    ] {
        assert_eq!(asks(&line), [], "{line}");
    }
}
