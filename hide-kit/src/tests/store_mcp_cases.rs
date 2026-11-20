//! The store's MCP servers against a private HOME (PRD skill-mcp-manager
//! B8-B22, B25-B28, B30, D-11, D-39): a server goes into the seven agents'
//! own files and a profile's, in each agent's shape, with its keys kept per
//! item in an owner-only file, and the switches, the key edit, the undo
//! list, adoption, a same-name entry of the operator's, a copy to another
//! machine and the machine's removal do what the operator sees.

use super::*;
use crate::store::{CellFailure, McpPreview, PastedPick, Sealed, SentKeys};
use hide_agent_hooks::mcp::{self as files, Keys, McpFile, Reach, Remote, Server};

const KEY: &str = "ghp_s3cr3t-key-value";

/// The seven agents' MCP files in a default account, by column.
const FILES: [(&str, &str); 7] = [
    ("claude-code", ".claude.json"),
    ("codex", ".codex/config.toml"),
    ("opencode", ".config/opencode/opencode.json"),
    ("pi", ".pi/agent/mcp.json"),
    ("omp", ".omp/agent/mcp.json"),
    ("grok", ".grok/config.toml"),
    ("antigravity", ".gemini/config/mcp_config.json"),
];

/// Every agent installed and on, each with its own folder made, as the
/// agents make them when they first run.
fn mcp_agents() -> Fixture {
    let fixture = every_agent();
    for folder in [
        ".claude",
        ".codex",
        ".config/opencode",
        ".pi/agent",
        ".omp/agent",
        ".grok",
        ".gemini/config",
    ] {
        std::fs::create_dir_all(fixture.home().join(folder)).unwrap();
    }
    with_program(&fixture, "npx");
    fixture
}

/// A stand-in program `name` where the fixture's account finds its
/// programs, so a pasted command that runs it reads as a server.
fn with_program(fixture: &Fixture, name: &str) {
    let bin = fixture.home().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let path = bin.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    std::fs::write(bin.join(format!("{name}.cmd")), "@exit /b 0\r\n").unwrap();
}

/// Every byte under the store's staging folder, as text.
fn staged_bytes(fixture: &Fixture) -> String {
    let dir = crate::layout::store_dir(fixture.home()).join("staging");
    if !dir.exists() {
        return String::new();
    }
    tree(&dir)
        .into_values()
        .filter_map(|(bytes, _)| bytes)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect()
}

fn github() -> Server {
    Server {
        name: "github".to_owned(),
        reach: Reach::Local {
            command: "npx".to_owned(),
            args: vec![
                "-y".to_owned(),
                "@modelcontextprotocol/server-github".to_owned(),
            ],
            env: vec!["GITHUB_TOKEN".to_owned()],
        },
    }
}

fn sent(pairs: &[(&str, &str)]) -> SentKeys {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Sealed::new(*value)))
        .collect()
}

fn add(fixture: &Fixture, server: Server, keys: &[(&str, &str)]) -> StoreReport {
    store(
        fixture,
        vec![StoreChange::AddMcp {
            server,
            keys: sent(keys),
            from: None,
        }],
    )
}

fn file(agent: &str, path: &Path) -> McpFile {
    McpFile::at(
        crate::agents::adapter(agent).unwrap().declaration(),
        path.to_path_buf(),
    )
    .unwrap()
}

/// The agent's MCP file, its names joined one at a time below the home as
/// an agent definition's place is, so it is spelled as Hide spells it on
/// every system (a backslash between names on Windows).
fn path_of(fixture: &Fixture, agent: &str) -> PathBuf {
    let (_, relative) = FILES.iter().find(|(id, _)| *id == agent).unwrap();
    relative
        .split('/')
        .fold(fixture.home().to_path_buf(), |path, name| path.join(name))
}

/// The place the item's `column` cell speaks of.
fn cell_place(report: &StoreReport, id: &str, column: &str) -> String {
    report
        .items
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.cells.iter().find(|cell| cell.column == column))
        .and_then(|cell| cell.place.clone())
        .unwrap_or_else(|| panic!("no place on {id}'s {column} cell"))
}

/// The place a report lists `agent`'s copy of `name` at, which the screen
/// sends back to take that copy over.
fn listed_place(report: &StoreReport, name: &str, agent: &str) -> String {
    report
        .unmanaged
        .iter()
        .find(|found| found.name == name)
        .and_then(|found| {
            found
                .copies
                .iter()
                .find(|copy| copy.agents.iter().any(|listed| listed == agent))
        })
        .unwrap_or_else(|| panic!("no listed copy of {name} for {agent}"))
        .place
        .clone()
}

/// What `agent`'s default file holds under `name`, read in its shape.
fn held(fixture: &Fixture, agent: &str, name: &str) -> Option<(Server, Keys)> {
    held_at(agent, &path_of(fixture, agent), name)
}

fn held_at(agent: &str, path: &Path, name: &str) -> Option<(Server, Keys)> {
    files::entry(&file(agent, path), name)
        .unwrap()
        .map(|found| found.server.expect("an entry Hide reads"))
}

fn key_of(held: &(Server, Keys), name: &str) -> String {
    held.1[name].expose().to_owned()
}

fn every(state: CellState) -> BTreeMap<String, CellState> {
    FILES
        .iter()
        .map(|(agent, _)| ((*agent).to_owned(), state))
        .collect()
}

/// Every byte under the store's folder, as text.
fn store_bytes(fixture: &Fixture) -> String {
    let dir = crate::layout::store_dir(fixture.home());
    if !dir.exists() {
        return String::new();
    }
    tree(&dir)
        .into_values()
        .filter_map(|(bytes, _)| bytes)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect()
}

fn the_mcp_files(fixture: &Fixture) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    FILES
        .iter()
        .map(|(_, relative)| {
            let path = fixture.home().join(relative);
            let bytes = std::fs::read(&path).ok();
            (path, bytes)
        })
        .collect()
}

#[test]
fn a_server_reaches_every_agents_file_and_a_profiles_with_its_key_and_a_second_pass_changes_nothing()
 {
    let mut fixture = mcp_agents();
    let profile = claude_profile(&mut fixture);
    let profile_file = profile.parent().unwrap().join(".claude.json");
    // The operator's own settings in two of the files stay as they are.
    write(
        &path_of(&fixture, "codex"),
        "model = \"o3\"\n\n[mcp_servers.theirs]\ncommand = \"theirs\"\n",
    );
    write(
        &path_of(&fixture, "claude-code"),
        "{\n  \"theme\": \"dark\",\n  \"numStartups\": 3\n}\n",
    );

    let report = add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);

    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    for (agent, _) in FILES {
        let entry = held(&fixture, agent, "github").unwrap_or_else(|| panic!("{agent}"));
        assert_eq!(entry.0, github(), "{agent}");
        assert_eq!(key_of(&entry, "GITHUB_TOKEN"), KEY, "{agent}");
    }
    let entry = held_at("claude-code", &profile_file, "github").unwrap();
    assert_eq!(key_of(&entry, "GITHUB_TOKEN"), KEY);
    let codex = std::fs::read_to_string(path_of(&fixture, "codex")).unwrap();
    assert!(
        codex.starts_with("model = \"o3\"\n\n[mcp_servers.theirs]\ncommand = \"theirs\"\n"),
        "{codex}"
    );
    let claude: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path_of(&fixture, "claude-code")).unwrap())
            .unwrap();
    assert_eq!(claude["theme"], "dark");
    assert_eq!(claude["numStartups"], 3);

    // The row shows the server and the names of its keys, never a value; the
    // key is in one owner-only file of the item's folder.
    let row = report
        .items
        .iter()
        .find(|item| item.id == "mcp:github")
        .unwrap();
    assert_eq!(row.server.as_ref(), Some(&github()));
    assert_eq!(row.version, Version::NpmLatest);
    assert!(!serde_json::to_string(&report).unwrap().contains(KEY));
    let keys = crate::layout::store_dir(fixture.home()).join("items/mcp-github/keys.json");
    assert!(hide_platform::fs::private::is_private(&keys).unwrap());
    assert!(std::fs::read_to_string(&keys).unwrap().contains(KEY));
    let record =
        std::fs::read_to_string(crate::layout::store_dir(fixture.home()).join("store.json"))
            .unwrap();
    assert!(!record.contains(KEY), "the record holds no key");

    // The same pass again writes no byte (B27).
    let before = the_mcp_files(&fixture);
    let profile_before = std::fs::read(&profile_file).unwrap();
    store(&fixture, Vec::new());
    assert_eq!(the_mcp_files(&fixture), before);
    assert_eq!(std::fs::read(&profile_file).unwrap(), profile_before);
}

#[test]
fn an_agent_that_takes_no_sse_server_reads_unsupported_and_gets_nothing() {
    let fixture = mcp_agents();
    let asana = Server {
        name: "asana".to_owned(),
        reach: Reach::Remote {
            transport: Remote::Sse,
            url: "https://mcp.asana.com/sse".to_owned(),
            headers: Vec::new(),
        },
    };
    let report = add(&fixture, asana, &[]);
    let cells = cells(&report, "mcp:asana");
    for (agent, _) in FILES {
        let takes = matches!(agent, "claude-code" | "omp" | "grok");
        assert_eq!(
            cells[agent],
            if takes {
                CellState::On
            } else {
                CellState::Unsupported
            },
            "{agent}"
        );
        assert_eq!(held(&fixture, agent, "asana").is_some(), takes, "{agent}");
    }
    let row = report
        .items
        .iter()
        .find(|item| item.id == "mcp:asana")
        .unwrap();
    assert_eq!(row.version, Version::Remote);
    let reason = row
        .cells
        .iter()
        .find(|cell| cell.column == "codex")
        .and_then(|cell| cell.reason.clone())
        .unwrap();
    assert!(reason.contains("SSE"), "{reason}");
}

#[test]
fn a_switch_takes_out_only_hides_entry_and_turning_it_on_puts_it_back() {
    let fixture = mcp_agents();
    add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);
    write(
        &path_of(&fixture, "pi"),
        "{\"mcpServers\": {\"theirs\": {\"command\": \"theirs\"}}}",
    );
    store(&fixture, Vec::new());

    let report = store(
        &fixture,
        vec![StoreChange::Set {
            item: "mcp:github".to_owned(),
            column: Some("codex".to_owned()),
            on: false,
        }],
    );
    let mut expected = every(CellState::On);
    expected.insert("codex".to_owned(), CellState::Off);
    assert_eq!(cells(&report, "mcp:github"), expected);
    assert!(held(&fixture, "codex", "github").is_none());
    assert!(held(&fixture, "grok", "github").is_some());

    let report = store(
        &fixture,
        vec![StoreChange::Set {
            item: "mcp:github".to_owned(),
            column: None,
            on: false,
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::Off));
    for (agent, _) in FILES {
        assert!(held(&fixture, agent, "github").is_none(), "{agent}");
    }
    assert!(
        held(&fixture, "pi", "theirs").is_some(),
        "the operator's own server stays"
    );

    let report = store(
        &fixture,
        vec![StoreChange::Set {
            item: "mcp:github".to_owned(),
            column: None,
            on: true,
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    assert_eq!(
        key_of(&held(&fixture, "codex", "github").unwrap(), "GITHUB_TOKEN"),
        KEY
    );
}

#[test]
fn a_key_edit_rewrites_every_file_the_server_is_in_and_a_file_that_failed_gets_it_later() {
    let mut fixture = mcp_agents();
    let profile = claude_profile(&mut fixture);
    let profile_file = profile.parent().unwrap().join(".claude.json");
    add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);
    let grok = path_of(&fixture, "grok");
    let written = std::fs::read_to_string(&grok).unwrap();
    // Grok's file is broken while the key changes.
    write(&grok, "[mcp_servers.github\n");

    let report = store(
        &fixture,
        vec![StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: github(),
            keys: sent(&[("GITHUB_TOKEN", "ghp_new")]),
        }],
    );
    for (agent, _) in FILES.iter().filter(|(agent, _)| *agent != "grok") {
        assert_eq!(
            key_of(&held(&fixture, agent, "github").unwrap(), "GITHUB_TOKEN"),
            "ghp_new",
            "{agent}"
        );
    }
    assert_eq!(
        key_of(
            &held_at("claude-code", &profile_file, "github").unwrap(),
            "GITHUB_TOKEN"
        ),
        "ghp_new"
    );
    assert_eq!(cells(&report, "mcp:github")["grok"], CellState::Failed);
    // The screen words the kind of failure and names the file; the sentence is the log's.
    let cell = report
        .items
        .iter()
        .find(|item| item.id == "mcp:github")
        .and_then(|item| item.cells.iter().find(|cell| cell.column == "grok"))
        .unwrap();
    assert_eq!(cell.failure, Some(CellFailure::Unreadable));
    // The same file, however this system spells it (`/private/var` on
    // macOS, no `\\?\` on Windows).
    assert_eq!(
        cell.path
            .as_deref()
            .map(|path| std::fs::canonicalize(path).unwrap()),
        Some(std::fs::canonicalize(&grok).unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(&grok).unwrap(),
        "[mcp_servers.github\n",
        "a broken file is left as it is"
    );

    // Once the file reads again, the next pass gives it the new key.
    write(&grok, &written);
    let report = store(&fixture, Vec::new());
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    assert_eq!(
        key_of(&held(&fixture, "grok", "github").unwrap(), "GITHUB_TOKEN"),
        "ghp_new"
    );

    // An edit that types no value keeps the key, and a server's name stays.
    store(
        &fixture,
        vec![StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: github(),
            keys: SentKeys::new(),
        }],
    );
    assert_eq!(
        key_of(&held(&fixture, "pi", "github").unwrap(), "GITHUB_TOKEN"),
        "ghp_new"
    );
    let renamed = crate::apply_store(
        &fixture.target,
        vec![StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: Server {
                name: "gh".to_owned(),
                ..github()
            },
            keys: SentKeys::new(),
        }],
    );
    assert!(renamed.outcomes[0].refused.is_some());
}

#[test]
fn a_delete_takes_the_server_out_at_once_and_its_key_stays_only_while_it_can_be_undone() {
    let fixture = mcp_agents();
    add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);

    let report = store(
        &fixture,
        vec![StoreChange::Delete {
            item: "mcp:github".to_owned(),
        }],
    );
    for (agent, _) in FILES {
        assert!(held(&fixture, agent, "github").is_none(), "{agent}");
    }
    assert!(
        store_bytes(&fixture).contains(KEY),
        "the key stays in Hide while the delete can be undone"
    );
    let entry = report.trash[0].id.clone();

    let report = store(
        &fixture,
        vec![StoreChange::Restore {
            entry: entry.clone(),
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    assert_eq!(
        key_of(&held(&fixture, "omp", "github").unwrap(), "GITHUB_TOKEN"),
        KEY
    );

    let report = store(
        &fixture,
        vec![StoreChange::Delete {
            item: "mcp:github".to_owned(),
        }],
    );
    let deleted = report.trash[0].id.clone();

    // A server added again under the same name keeps its own key when the
    // older one's undo entry goes for good.
    add(&fixture, github(), &[("GITHUB_TOKEN", "ghp_again")]);
    store(&fixture, vec![StoreChange::Forget { entry: deleted }]);
    let bytes = store_bytes(&fixture);
    assert!(!bytes.contains(KEY), "a permanent delete wipes the key");
    assert!(bytes.contains("ghp_again"));
    assert_eq!(
        key_of(&held(&fixture, "omp", "github").unwrap(), "GITHUB_TOKEN"),
        "ghp_again"
    );
}

#[test]
fn a_change_that_carries_a_key_prints_and_echoes_none() {
    let changes = [
        StoreChange::ReadMcp {
            text: Sealed::new(format!("GITHUB_TOKEN={KEY} npx some-mcp")),
        },
        StoreChange::AddMcp {
            server: github(),
            keys: sent(&[("GITHUB_TOKEN", KEY)]),
            from: None,
        },
        StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: github(),
            keys: sent(&[("GITHUB_TOKEN", KEY)]),
        },
    ];
    for change in changes {
        // The wire to a machine carries the key; a log line and an outcome
        // do not.
        assert!(serde_json::to_string(&change).unwrap().contains(KEY));
        assert!(!format!("{change:?}").contains(KEY), "{change:?}");
        let redacted = change.redacted();
        assert!(!serde_json::to_string(&redacted).unwrap().contains(KEY));
    }
}

#[test]
fn a_server_of_the_same_name_the_operator_put_in_is_left_alone_until_taken_over() {
    let fixture = mcp_agents();
    let theirs = "{\n  \"mcpServers\": {\n    \"github\": {\n      \"command\": \"docker\",\n      \"args\": [\"run\", \"-i\", \"ghcr.io/github/github-mcp-server\"],\n      \"env\": {\"GITHUB_TOKEN\": \"ghp_theirs\"}\n    }\n  }\n}\n";
    write(&path_of(&fixture, "omp"), theirs);

    let report = add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);
    let mut expected = every(CellState::On);
    expected.insert("omp".to_owned(), CellState::HandWritten);
    assert_eq!(cells(&report, "mcp:github"), expected);
    assert_eq!(
        std::fs::read_to_string(path_of(&fixture, "omp")).unwrap(),
        theirs
    );

    // Taking omp's copy over makes it the item for every agent, its key
    // with it, and the store's own copy goes to the undo list.
    // The cell names the place holding omp's own server, which a take-over
    // sends back.
    let place = cell_place(&report, "mcp:github", "omp");
    let report = store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "github".to_owned(),
            from: Some(place),
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    for (agent, _) in FILES {
        let entry = held(&fixture, agent, "github").unwrap();
        assert_eq!(key_of(&entry, "GITHUB_TOKEN"), "ghp_theirs", "{agent}");
        assert!(
            matches!(&entry.0.reach, Reach::Local { command, .. } if command == "docker"),
            "{agent}"
        );
    }
    assert_eq!(report.trash.len(), 1);
}

#[test]
fn a_server_only_one_agent_had_is_listed_and_adopted_into_every_agent_with_its_key() {
    let fixture = mcp_agents();
    write(
        &path_of(&fixture, "codex"),
        "[mcp_servers.fetch]\ncommand = \"uvx\"\nargs = [\"mcp-server-fetch\"]\nenv = { PROXY = \"http://proxy\" }\n",
    );

    let report = store(&fixture, Vec::new());
    let found = report
        .unmanaged
        .iter()
        .find(|unmanaged| unmanaged.kind == ItemKind::Mcp && unmanaged.name == "fetch")
        .unwrap_or_else(|| panic!("{report:#?}"));
    assert_eq!(found.copies.len(), 1);
    assert_eq!(found.copies[0].agents, ["codex"]);
    assert_eq!(
        found.copies[0].path,
        path_of(&fixture, "codex").display().to_string()
    );

    let report = store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "fetch".to_owned(),
            from: None,
        }],
    );
    assert_eq!(cells(&report, "mcp:fetch"), every(CellState::On));
    for (agent, _) in FILES {
        assert_eq!(
            key_of(&held(&fixture, agent, "fetch").unwrap(), "PROXY"),
            "http://proxy",
            "{agent}"
        );
    }
    assert!(report.unmanaged.iter().all(|found| found.name != "fetch"));
    let row = report
        .items
        .iter()
        .find(|item| item.id == "mcp:fetch")
        .unwrap();
    assert_eq!(row.version, Version::Unpinned);
}

#[test]
fn a_paste_fills_the_fields_and_its_values_stay_in_the_store() {
    let fixture = mcp_agents();
    let pasted = format!("GITHUB_TOKEN={KEY} npx -y @modelcontextprotocol/server-github");
    let report = store(
        &fixture,
        vec![StoreChange::ReadMcp {
            text: Sealed::new(pasted.clone()),
        }],
    );
    let McpPreview { id, servers, .. } = &report.mcp_previews[0];
    assert_eq!(servers[0].server, github());
    assert_eq!(servers[0].given, ["GITHUB_TOKEN"]);
    assert!(staged_bytes(&fixture).contains(KEY));
    assert_eq!(servers[0].agents.len(), FILES.len());
    let wire = serde_json::to_string(&report).unwrap();
    assert!(!wire.contains(KEY), "{wire}");
    assert!(!format!("{report:?}").contains(KEY));

    // Adding the pasted server without typing the key takes the paste's.
    let report = store(
        &fixture,
        vec![StoreChange::AddMcp {
            server: github(),
            keys: SentKeys::new(),
            from: Some(PastedPick {
                preview: id.clone(),
                index: 0,
            }),
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    assert_eq!(
        key_of(
            &held(&fixture, "antigravity", "github").unwrap(),
            "GITHUB_TOKEN"
        ),
        KEY
    );
    // The key lives in one place once the add ends: the item's, and the
    // paste's answer, all of it added, goes with its staged copy.
    assert!(report.mcp_previews.iter().all(|preview| &preview.id != id));
    assert!(!staged_bytes(&fixture).contains(KEY));

    // Closing a paste's answer drops what it staged at once.
    let report = store(
        &fixture,
        vec![StoreChange::ReadMcp {
            text: Sealed::new("LINEAR_KEY=lin_s3cr3t npx -y linear-mcp"),
        }],
    );
    let closed = report.mcp_previews.last().unwrap().id.clone();
    assert!(staged_bytes(&fixture).contains("lin_s3cr3t"));
    let report = store(
        &fixture,
        vec![StoreChange::CancelMcp {
            preview: closed.clone(),
        }],
    );
    assert!(
        report
            .mcp_previews
            .iter()
            .all(|preview| preview.id != closed)
    );
    assert!(!staged_bytes(&fixture).contains("lin_s3cr3t"));

    // A sentence, or a command whose program this machine does not have,
    // is no server; the same command once the program is there is one.
    for text in [
        "please add the github server for me",
        "fetch-mcp --port 8080",
    ] {
        let report = store(
            &fixture,
            vec![StoreChange::ReadMcp {
                text: Sealed::new(text),
            }],
        );
        assert!(
            report.mcp_previews.last().unwrap().servers.is_empty(),
            "{text}"
        );
    }
    with_program(&fixture, "fetch-mcp");
    let report = store(
        &fixture,
        vec![StoreChange::ReadMcp {
            text: Sealed::new("fetch-mcp --port 8080"),
        }],
    );
    assert_eq!(report.mcp_previews.last().unwrap().servers.len(), 1);

    // A paste Hide cannot read leaves the fields empty.
    let report = store(
        &fixture,
        vec![StoreChange::ReadMcp {
            text: Sealed::new("ftp://example.com/mcp"),
        }],
    );
    assert!(report.mcp_previews.last().unwrap().servers.is_empty());

    // A key the server names with no value adds nothing.
    let refused = crate::apply_store(
        &fixture.target,
        vec![StoreChange::AddMcp {
            server: Server {
                name: "linear".to_owned(),
                ..github()
            },
            keys: SentKeys::new(),
            from: None,
        }],
    );
    let refusal = refused.outcomes[0].refused.as_ref().unwrap();
    assert_eq!(refusal.reason, "GITHUB_TOKEN has no value");
    assert!(refused.items.iter().all(|item| item.id != "mcp:linear"));
}

#[test]
fn a_copy_carries_the_server_and_its_key_and_a_new_key_alone_reaches_the_other_machine() {
    let here = mcp_agents();
    add(&here, github(), &[("GITHUB_TOKEN", KEY)]);
    let export = crate::export_store(&here.target);
    assert_eq!(export.refused, None);

    let there = mcp_agents();
    let received = StoreChange::Receive {
        item: Box::new(export.items[0].clone()),
    };
    let report = store(&there, vec![received]);
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    assert_eq!(
        key_of(&held(&there, "codex", "github").unwrap(), "GITHUB_TOKEN"),
        KEY
    );
    assert!(
        !serde_json::to_string(&report).unwrap().contains(KEY),
        "the outcome holds the carried change with no key"
    );

    // The key changes here, and the next copy gives the other machine's
    // agents the new one.
    store(
        &here,
        vec![StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: github(),
            keys: sent(&[("GITHUB_TOKEN", "ghp_new")]),
        }],
    );
    let export = crate::export_store(&here.target);
    store(
        &there,
        vec![StoreChange::Receive {
            item: Box::new(export.items[0].clone()),
        }],
    );
    for (agent, _) in FILES {
        assert_eq!(
            key_of(&held(&there, agent, "github").unwrap(), "GITHUB_TOKEN"),
            "ghp_new",
            "{agent}"
        );
    }
}

#[test]
fn a_take_over_sets_each_other_copy_aside_whole_and_a_machine_leaving_puts_it_back() {
    let fixture = mcp_agents();
    let chosen = "{\n  \"mcpServers\": {\n    \"github\": {\n      \"command\": \"docker\",\n      \"args\": [\"run\", \"-i\", \"ghcr.io/github/github-mcp-server\"],\n      \"env\": {\"GITHUB_TOKEN\": \"ghp_theirs\"}\n    }\n  }\n}\n";
    write(&path_of(&fixture, "claude-code"), chosen);
    // Another server of the name, with a field Hide's copy leaves out.
    write(
        &path_of(&fixture, "codex"),
        "model = \"gpt-5\"\n\n[mcp_servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"@modelcontextprotocol/server-github\"]\nenv = { GITHUB_TOKEN = \"ghp_codex\" }\nstartup_timeout_sec = 30\n",
    );
    // The chosen server itself, with a switch Hide's copy leaves out.
    write(
        &path_of(&fixture, "grok"),
        "[mcp_servers.github]\ncommand = \"docker\"\nargs = [\"run\", \"-i\", \"ghcr.io/github/github-mcp-server\"]\nenv = { GITHUB_TOKEN = \"ghp_theirs\" }\nenabled = false\n",
    );

    let place = listed_place(&store(&fixture, Vec::new()), "github", "claude-code");
    let report = store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "github".to_owned(),
            from: Some(place),
        }],
    );
    assert_eq!(cells(&report, "mcp:github"), every(CellState::On));
    for (agent, _) in FILES {
        let entry = held(&fixture, agent, "github").unwrap();
        assert_eq!(key_of(&entry, "GITHUB_TOKEN"), "ghp_theirs", "{agent}");
    }
    let codex = std::fs::read_to_string(path_of(&fixture, "codex")).unwrap();
    assert!(codex.contains("model = \"gpt-5\""));
    assert!(!codex.contains("startup_timeout_sec"));
    assert!(
        !std::fs::read_to_string(path_of(&fixture, "grok"))
            .unwrap()
            .contains("enabled")
    );
    // Both set-aside copies wait in the undo list, keys and all.
    assert_eq!(report.trash.len(), 2, "{:#?}", report.trash);
    assert!(
        report
            .trash
            .iter()
            .all(|kept| matches!(&kept.cause, TrashCause::Replaced { .. }))
    );

    remove(&fixture.target);

    let codex = std::fs::read_to_string(path_of(&fixture, "codex")).unwrap();
    assert!(codex.contains("startup_timeout_sec = 30"), "{codex}");
    assert_eq!(
        key_of(&held(&fixture, "codex", "github").unwrap(), "GITHUB_TOKEN"),
        "ghp_codex"
    );
    assert!(
        std::fs::read_to_string(path_of(&fixture, "grok"))
            .unwrap()
            .contains("enabled = false")
    );
    // The copy taken over where it was is the operator's and stays; the
    // copies Hide wrote go.
    assert_eq!(
        std::fs::read_to_string(path_of(&fixture, "claude-code")).unwrap(),
        chosen
    );
    for agent in ["opencode", "pi", "omp", "antigravity"] {
        assert!(held(&fixture, agent, "github").is_none(), "{agent}");
    }
    assert!(!crate::layout::store_dir(fixture.home()).exists());
}

#[test]
fn a_server_taken_over_and_switched_off_where_it_was_goes_back_there_when_the_machine_leaves() {
    let fixture = mcp_agents();
    write(
        &path_of(&fixture, "codex"),
        "[mcp_servers.fetch]\ncommand = \"uvx\"\nargs = [\"mcp-server-fetch\"]\nenv = { PROXY = \"http://proxy\" }\n",
    );
    store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "fetch".to_owned(),
            from: None,
        }],
    );
    // Switched off for Codex, the operator's own entry leaves its file: the
    // store holds the only original now.
    store(
        &fixture,
        vec![StoreChange::Set {
            item: "mcp:fetch".to_owned(),
            column: Some("codex".to_owned()),
            on: false,
        }],
    );
    assert!(held(&fixture, "codex", "fetch").is_none());

    let report = remove(&fixture.target);

    assert!(
        report
            .agents
            .iter()
            .all(|(_, outcome)| !matches!(outcome, crate::RemoveOutcome::Failed { .. })),
        "{:?}",
        report.agents
    );
    assert_eq!(
        key_of(&held(&fixture, "codex", "fetch").unwrap(), "PROXY"),
        "http://proxy"
    );
    for (agent, _) in FILES.iter().filter(|(agent, _)| *agent != "codex") {
        assert!(held(&fixture, agent, "fetch").is_none(), "{agent}");
    }
    assert!(!crate::layout::store_dir(fixture.home()).exists());
}

/// `github` as Pi and omp keep it, with `key`.
fn github_with(key: &str) -> String {
    format!(
        "{{\"mcpServers\": {{\"github\": {{\"command\": \"npx\", \"args\": [\"-y\", \"@modelcontextprotocol/server-github\"], \"env\": {{\"GITHUB_TOKEN\": \"{key}\"}}}}}}}}"
    )
}

#[test]
fn the_same_server_with_two_keys_is_two_servers_and_a_take_over_keeps_both_keys() {
    let fixture = mcp_agents();
    write(&path_of(&fixture, "omp"), &github_with("ghp_omp_work"));
    write(&path_of(&fixture, "pi"), &github_with("ghp_pi_personal"));

    // Listed as copies that differ, and taken over only from the one picked.
    let report = store(&fixture, Vec::new());
    let found = report
        .unmanaged
        .iter()
        .find(|unmanaged| unmanaged.name == "github")
        .unwrap();
    assert!(!found.same);
    let refused = crate::apply_store(
        &fixture.target,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "github".to_owned(),
            from: None,
        }],
    );
    assert_eq!(
        refused.outcomes[0]
            .refused
            .as_ref()
            .map(|refusal| refusal.cause),
        Some(StoreRefusalCause::Differs)
    );
    assert_eq!(
        key_of(&held(&fixture, "omp", "github").unwrap(), "GITHUB_TOKEN"),
        "ghp_omp_work"
    );

    let place = listed_place(&refused, "github", "pi");
    let report = store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "github".to_owned(),
            from: Some(place),
        }],
    );
    // omp's server, its key with it, waits in the undo list; every agent
    // gets the one picked.
    assert_eq!(report.trash.len(), 1, "{:#?}", report.trash);
    assert!(store_bytes(&fixture).contains("ghp_omp_work"));
    for (agent, _) in FILES {
        assert_eq!(
            key_of(&held(&fixture, agent, "github").unwrap(), "GITHUB_TOKEN"),
            "ghp_pi_personal",
            "{agent}"
        );
    }
    // Switching omp off and on writes Hide's server and never loses omp's
    // own key, which stays in the undo list.
    for on in [false, true] {
        store(
            &fixture,
            vec![StoreChange::Set {
                item: "mcp:github".to_owned(),
                column: Some("omp".to_owned()),
                on,
            }],
        );
    }
    assert!(store_bytes(&fixture).contains("ghp_omp_work"));

    // An item already in the store meets an agent's own entry of another
    // key, where Hide's copy is not (switched off there): that entry goes
    // to the undo list whole too.
    store(
        &fixture,
        vec![StoreChange::Set {
            item: "mcp:github".to_owned(),
            column: Some("grok".to_owned()),
            on: false,
        }],
    );
    write(
        &path_of(&fixture, "grok"),
        "[mcp_servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"@modelcontextprotocol/server-github\"]\nenv = { GITHUB_TOKEN = \"ghp_grok_other\" }\n",
    );
    let report = store(
        &fixture,
        vec![StoreChange::Adopt {
            kind: ItemKind::Mcp,
            name: "github".to_owned(),
            from: None,
        }],
    );
    assert_eq!(report.trash.len(), 2, "{:#?}", report.trash);
    assert!(store_bytes(&fixture).contains("ghp_grok_other"));
    assert!(held(&fixture, "grok", "github").is_none());
}

#[test]
fn a_key_typed_into_hides_entry_by_hand_makes_it_the_operators_and_is_never_lost() {
    let fixture = mcp_agents();
    add(&fixture, github(), &[("GITHUB_TOKEN", "ghp_hide_a")]);
    let codex = path_of(&fixture, "codex");
    let typed = std::fs::read_to_string(&codex)
        .unwrap()
        .replace("ghp_hide_a", "ghp_typed_b");
    write(&codex, &typed);
    let typed_key = || key_of(&held(&fixture, "codex", "github").unwrap(), "GITHUB_TOKEN");

    // The record keeps a fingerprint of the key Hide wrote, never a value.
    let record =
        std::fs::read_to_string(crate::layout::store_dir(fixture.home()).join("store.json"))
            .unwrap();
    assert!(!record.contains("ghp_hide_a") && !record.contains("ghp_typed_b"));
    let report = store(&fixture, Vec::new());
    assert_eq!(
        cells(&report, "mcp:github")["codex"],
        CellState::HandWritten
    );

    // (1) Switched off and on again for Codex: the typed key stays.
    for on in [false, true] {
        store(
            &fixture,
            vec![StoreChange::Set {
                item: "mcp:github".to_owned(),
                column: Some("codex".to_owned()),
                on,
            }],
        );
        assert_eq!(typed_key(), "ghp_typed_b", "on: {on}");
    }

    // (2) A key edit in Hide reaches every other agent and leaves Codex's.
    store(
        &fixture,
        vec![StoreChange::SetMcp {
            item: "mcp:github".to_owned(),
            server: github(),
            keys: sent(&[("GITHUB_TOKEN", "ghp_hide_c")]),
        }],
    );
    assert_eq!(typed_key(), "ghp_typed_b");
    for (agent, _) in FILES.iter().filter(|(agent, _)| *agent != "codex") {
        assert_eq!(
            key_of(&held(&fixture, agent, "github").unwrap(), "GITHUB_TOKEN"),
            "ghp_hide_c",
            "{agent}"
        );
    }

    // (3) Deleting the item takes Hide's copies out and leaves Codex's.
    store(
        &fixture,
        vec![StoreChange::Delete {
            item: "mcp:github".to_owned(),
        }],
    );
    assert_eq!(typed_key(), "ghp_typed_b");
    for (agent, _) in FILES.iter().filter(|(agent, _)| *agent != "codex") {
        assert!(held(&fixture, agent, "github").is_none(), "{agent}");
    }
}

#[test]
fn removing_the_machine_takes_hides_servers_and_their_keys_out() {
    let fixture = mcp_agents();
    add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);
    write(
        &path_of(&fixture, "opencode"),
        "{\"mcp\": {\"theirs\": {\"type\": \"local\", \"command\": [\"theirs\"]}}}",
    );
    store(&fixture, Vec::new());

    remove(&fixture.target);

    for (agent, _) in FILES {
        assert!(held(&fixture, agent, "github").is_none(), "{agent}");
    }
    assert!(held(&fixture, "opencode", "theirs").is_some());
    assert!(!crate::layout::store_dir(fixture.home()).exists());
}

#[test]
fn a_store_whose_secret_is_gone_or_unreadable_is_kept_and_no_agent_file_changes() {
    for damage in ["gone", "garbage"] {
        let fixture = mcp_agents();
        add(&fixture, github(), &[("GITHUB_TOKEN", KEY)]);
        let dir = crate::layout::store_dir(fixture.home());
        let secret = dir.join(crate::mcp_keys::SECRET_FILE);
        if damage == "gone" {
            std::fs::remove_file(&secret).unwrap();
        } else {
            hide_platform::fs::atomic::write_file(
                &secret,
                b"not a secret",
                hide_platform::fs::Access::Private,
            )
            .unwrap();
        }
        let files = || -> Vec<String> {
            FILES
                .iter()
                .map(|(agent, _)| std::fs::read_to_string(path_of(&fixture, agent)).unwrap())
                .collect()
        };
        let before = files();

        // A pass refuses rather than make a new secret, which would read
        // every server Hide wrote as the operator's.
        let refused = crate::apply_store(&fixture.target, Vec::new());
        assert!(refused.refused.is_some(), "{damage}");
        // The screen says so on the section, with the manual start-over (decisions 2026-11-19).
        assert_eq!(
            refused.refusal,
            Some(crate::store::StorePassRefusal::KeysSecretGone),
            "{damage}"
        );
        assert_eq!(secret.exists(), damage == "garbage");

        let report = remove(&fixture.target);
        assert!(
            report
                .agents
                .iter()
                .any(|(_, outcome)| matches!(outcome, crate::RemoveOutcome::Failed { .. })),
            "{damage}: {:?}",
            report.agents
        );
        assert!(dir.join("store.json").exists(), "{damage}");
        assert_eq!(files(), before, "{damage}");
    }
}
