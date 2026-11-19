//! Hide's MCP servers in each agent's own MCP file (PRD skill-mcp-manager
//! B11, B14, B22, B27, D-27, D-28), written into a home of the test's own.
//! The expected entries are each agent's own `mcp add` output and
//! documentation (D-27), written here by hand, never read from the
//! converter; the files around them are what an operator keeps there.

use std::path::{Path, PathBuf};

use hide_agent_adapter::{AccountRef, AgentAdapter, NodeEnv, Profile, adapter};
use hide_agent_hooks::account::AccountAt;
use hide_agent_hooks::mcp::{self, Keys, McpFile, Put, Reach, Remote, Secret, Server, Transport};
use serde_json::{Value, json};

const KEY: &str = "ghp_k3y-under-test";

struct Agent {
    kind: &'static str,
    /// The file below the home, and what the operator keeps in it: an
    /// entry of their own, `mine`, and settings that are not MCP's.
    file: &'static str,
    original: &'static str,
    toml: bool,
    /// Hide's local `github` entry as the agent writes it.
    local: Value,
    /// Hide's Streamable HTTP `linear` entry.
    http: Value,
    /// Hide's SSE `asana` entry, `None` where the agent takes no SSE.
    sse: Option<Value>,
    /// Where the agent's MCP servers are, below the file's root.
    table: &'static [&'static str],
}

fn agents() -> Vec<Agent> {
    let typed_local = json!({
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-github"],
        "env": {"GITHUB_TOKEN": KEY}
    });
    let plain_local = json!({
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-github"],
        "env": {"GITHUB_TOKEN": KEY}
    });
    vec![
        Agent {
            kind: "claude-code",
            file: ".claude.json",
            original: "{\n  \"numStartups\": 3,\n  \"mcpServers\": {\n    \"mine\": {\n      \"type\": \"stdio\",\n      \"command\": \"my-tool\"\n    }\n  },\n  \"projects\": {}\n}\n",
            toml: false,
            local: typed_local.clone(),
            http: json!({"type": "http", "url": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: Some(json!({"type": "sse", "url": "https://mcp.asana.com/sse"})),
            table: &["mcpServers"],
        },
        Agent {
            kind: "omp",
            file: ".omp/agent/mcp.json",
            original: "{\n  \"mcpServers\": {\n    \"mine\": { \"type\": \"stdio\", \"command\": \"my-tool\" }\n  }\n}\n",
            toml: false,
            local: typed_local,
            http: json!({"type": "http", "url": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: Some(json!({"type": "sse", "url": "https://mcp.asana.com/sse"})),
            table: &["mcpServers"],
        },
        Agent {
            kind: "pi",
            file: ".pi/agent/mcp.json",
            original: "{\"mcpServers\": {\"mine\": {\"command\": \"my-tool\"}}, \"other\": [1, 2]}",
            toml: false,
            local: plain_local.clone(),
            http: json!({"url": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: None,
            table: &["mcpServers"],
        },
        Agent {
            kind: "antigravity",
            file: ".gemini/config/mcp_config.json",
            original: "{\n  \"mcpServers\": {\n    \"mine\": {\"command\": \"my-tool\", \"disabled\": false}\n  }\n}\n",
            toml: false,
            local: plain_local,
            http: json!({"serverUrl": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: None,
            table: &["mcpServers"],
        },
        Agent {
            kind: "opencode",
            file: ".config/opencode/opencode.json",
            original: "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"model\": \"x/y\",\n  \"mcp\": {\n    \"mine\": {\"type\": \"local\", \"command\": [\"my-tool\"]}\n  }\n}\n",
            toml: false,
            local: json!({
                "type": "local",
                "command": ["npx", "-y", "@modelcontextprotocol/server-github"],
                "environment": {"GITHUB_TOKEN": KEY}
            }),
            http: json!({"type": "remote", "url": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: None,
            table: &["mcp"],
        },
        Agent {
            kind: "codex",
            file: ".codex/config.toml",
            original: "# my settings\nmodel = \"gpt-5\"\n\n[mcp_servers.mine]\ncommand = \"my-tool\" # mine\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n",
            toml: true,
            local: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": {"GITHUB_TOKEN": KEY}
            }),
            http: json!({"url": "https://mcp.linear.app/mcp", "http_headers": {"Authorization": "Bearer lin"}}),
            sse: None,
            table: &["mcp_servers"],
        },
        Agent {
            kind: "grok",
            file: ".grok/config.toml",
            original: "[ui]\ntheme = \"dark\"\n\n[mcp_servers.mine]\ncommand = \"my-tool\"\nenabled = true\n",
            toml: true,
            local: json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": {"GITHUB_TOKEN": KEY}
            }),
            http: json!({"url": "https://mcp.linear.app/mcp", "headers": {"Authorization": "Bearer lin"}}),
            sse: Some(json!({"type": "sse", "url": "https://mcp.asana.com/sse"})),
            table: &["mcp_servers"],
        },
    ]
}

fn github() -> (Server, Keys) {
    (
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
        },
        Keys::from([("GITHUB_TOKEN".to_owned(), Secret::new(KEY.to_owned()))]),
    )
}

fn linear() -> (Server, Keys) {
    (
        Server {
            name: "linear".to_owned(),
            reach: Reach::Remote {
                transport: Remote::Http,
                url: "https://mcp.linear.app/mcp".to_owned(),
                headers: vec!["Authorization".to_owned()],
            },
        },
        Keys::from([(
            "Authorization".to_owned(),
            Secret::new("Bearer lin".to_owned()),
        )]),
    )
}

fn asana() -> Server {
    Server {
        name: "asana".to_owned(),
        reach: Reach::Remote {
            transport: Remote::Sse,
            url: "https://mcp.asana.com/sse".to_owned(),
            headers: Vec::new(),
        },
    }
}

fn the(kind: &str) -> &'static AgentAdapter {
    adapter(kind).unwrap()
}

fn default_file(agent: &Agent, home: &Path) -> McpFile {
    let file = McpFile::of(the(agent.kind), AccountAt::Home(home)).unwrap();
    assert_eq!(file.path(), home.join(agent.file), "{}", agent.kind);
    file
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A TOML item as JSON: strings, lists and tables, which is all an MCP
/// entry holds.
fn toml_json(item: &toml_edit::Item) -> Value {
    fn plain(item: &toml_edit::Value) -> Value {
        match item {
            toml_edit::Value::String(text) => Value::String(text.value().clone()),
            toml_edit::Value::Boolean(flag) => Value::Bool(*flag.value()),
            toml_edit::Value::Integer(number) => json!(*number.value()),
            toml_edit::Value::Array(list) => Value::Array(list.iter().map(plain).collect()),
            toml_edit::Value::InlineTable(table) => Value::Object(
                table
                    .iter()
                    .map(|(key, each)| (key.to_owned(), plain(each)))
                    .collect(),
            ),
            other => panic!("unexpected TOML value {other}"),
        }
    }
    match item {
        toml_edit::Item::Value(each) => plain(each),
        other => Value::Object(
            other
                .as_table_like()
                .unwrap()
                .iter()
                .map(|(key, each)| (key.to_owned(), toml_json(each)))
                .collect(),
        ),
    }
}

/// The file's servers, by name, as JSON.
fn servers(agent: &Agent, path: &Path) -> serde_json::Map<String, Value> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut at = if agent.toml {
        toml_json(text.parse::<toml_edit::DocumentMut>().unwrap().as_item())
    } else {
        serde_json::from_str(&text).unwrap()
    };
    for key in agent.table {
        at = at[*key].clone();
    }
    at.as_object().cloned().unwrap_or_default()
}

#[test]
fn each_agent_gets_hides_server_in_its_own_shape_and_keeps_the_rest_of_its_file() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        write(&path, agent.original);
        let mine = servers(&agent, &path)["mine"].clone();

        let (server, keys) = github();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Written,
            "{}",
            agent.kind
        );
        let after = servers(&agent, &path);
        assert_eq!(after["github"], agent.local, "{}", agent.kind);
        assert_eq!(after["mine"], mine, "{}", agent.kind);

        let (server, keys) = linear();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Written,
            "{}",
            agent.kind
        );
        assert_eq!(
            servers(&agent, &path)["linear"],
            agent.http,
            "{}",
            agent.kind
        );

        let sse = mcp::put(&file, &asana(), &Keys::new()).unwrap();
        match &agent.sse {
            Some(entry) => {
                assert_eq!(sse, Put::Written, "{}", agent.kind);
                assert_eq!(&servers(&agent, &path)["asana"], entry, "{}", agent.kind);
                assert!(mcp::accepts(file.format(), Transport::Sse));
            }
            None => {
                assert_eq!(sse, Put::Unsupported, "{}", agent.kind);
                assert!(
                    !servers(&agent, &path).contains_key("asana"),
                    "{}",
                    agent.kind
                );
                assert!(!mcp::accepts(file.format(), Transport::Sse));
            }
        }

        // Every server Hide wrote comes out, and the file is byte for byte
        // the operator's again.
        for name in ["github", "linear", "asana"] {
            mcp::take(&file, name).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            agent.original,
            "{}",
            agent.kind
        );
        assert!(
            mcp::recorded(home.path()).unwrap().is_empty(),
            "{}",
            agent.kind
        );
    }
}

#[test]
fn writing_the_same_server_again_changes_no_byte_and_a_new_key_rewrites_it() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        write(&path, agent.original);
        let (server, keys) = github();
        assert_eq!(mcp::put(&file, &server, &keys).unwrap(), Put::Written);
        let written = std::fs::read(&path).unwrap();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Unchanged,
            "{}",
            agent.kind
        );
        assert_eq!(std::fs::read(&path).unwrap(), written, "{}", agent.kind);

        // A key edited in Hide reaches the agent's file (B12).
        let edited = Keys::from([("GITHUB_TOKEN".to_owned(), Secret::new("ghp_new".to_owned()))]);
        assert_eq!(
            mcp::put(&file, &server, &edited).unwrap(),
            Put::Written,
            "{}",
            agent.kind
        );
        let entry = &servers(&agent, &path)["github"];
        let env = if agent.kind == "opencode" {
            "environment"
        } else {
            "env"
        };
        assert_eq!(entry[env]["GITHUB_TOKEN"], "ghp_new", "{}", agent.kind);
    }
}

#[test]
fn a_file_that_is_not_there_is_made_with_hides_server_alone() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        let (server, keys) = github();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Written,
            "{}",
            agent.kind
        );
        let after = servers(&agent, &path);
        assert_eq!(after.len(), 1, "{}", agent.kind);
        assert_eq!(after["github"], agent.local, "{}", agent.kind);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "{} holds keys, so it is the account's alone",
                agent.kind
            );
        }
    }
}

#[test]
fn an_entry_of_the_same_name_hide_did_not_write_stays_the_operators_until_taken_in() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        write(&path, agent.original);
        // `mine` is the operator's: Hide neither overwrites nor removes it.
        let (mut server, keys) = github();
        server.name = "mine".to_owned();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Theirs,
            "{}",
            agent.kind
        );
        assert!(!mcp::take(&file, "mine").unwrap(), "{}", agent.kind);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), agent.original);

        let found = mcp::read(&file).unwrap();
        let mine = found.iter().find(|found| found.name == "mine").unwrap();
        assert!(!mine.hide, "{}", agent.kind);
        let (read, read_keys) = mine.server.as_ref().unwrap();
        assert_eq!(
            read.reach,
            Reach::Local {
                command: "my-tool".to_owned(),
                args: Vec::new(),
                env: Vec::new()
            },
            "{}",
            agent.kind
        );
        assert!(read_keys.is_empty());

        // Taken in (B20): Hide's record names it, its next write is Hide's,
        // and taking it out removes it.
        assert!(mcp::adopt(&file, "mine").unwrap(), "{}", agent.kind);
        assert!(
            mcp::read(&file)
                .unwrap()
                .iter()
                .any(|found| found.name == "mine" && found.hide)
        );
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Written,
            "{}",
            agent.kind
        );
        assert_eq!(
            servers(&agent, &path)["mine"],
            agent.local,
            "{}",
            agent.kind
        );
        assert!(mcp::take(&file, "mine").unwrap(), "{}", agent.kind);
        assert!(
            !servers(&agent, &path).contains_key("mine"),
            "{}",
            agent.kind
        );
        assert!(!mcp::adopt(&file, "absent").unwrap(), "{}", agent.kind);
    }
}

#[test]
fn a_profile_account_gets_its_own_file_and_the_default_stays_untouched() {
    let home = tempfile::tempdir().unwrap();
    let node = NodeEnv::new(Some(home.path().to_path_buf()));
    let work = home.path().join("accounts").join("claude-work");
    std::fs::create_dir_all(&work).unwrap();
    let cases: [(&str, AccountRef, PathBuf); 3] = [
        (
            "claude-code",
            AccountRef::Folder(work.to_string_lossy().into_owned()),
            work.join(".claude.json"),
        ),
        (
            "omp",
            AccountRef::Name("work".to_owned()),
            home.path().join(".omp/profiles/work/agent/mcp.json"),
        ),
        (
            "codex",
            AccountRef::Folder(work.join("codex").to_string_lossy().into_owned()),
            work.join("codex").join("config.toml"),
        ),
    ];
    for (kind, account, expected) in cases {
        let profile = Profile {
            account: Some(account),
            ..Profile::builtin_default(kind)
        };
        let resolved = the(kind).resolve(&profile, &node).unwrap();
        let file = McpFile::of(
            the(kind),
            AccountAt::Account {
                home: home.path(),
                account: &resolved,
            },
        )
        .unwrap();
        assert_eq!(file.path(), expected, "{kind}");
        let (server, keys) = github();
        assert_eq!(
            mcp::put(&file, &server, &keys).unwrap(),
            Put::Written,
            "{kind}"
        );
        assert!(expected.exists(), "{kind}");
        let default = McpFile::of(the(kind), AccountAt::Home(home.path())).unwrap();
        assert!(!default.path().exists(), "{kind}");
        assert_eq!(
            mcp::take_all(&file).unwrap(),
            vec!["github".to_owned()],
            "{kind}"
        );
    }
}

#[test]
fn a_broken_file_is_left_as_it_is_and_the_failure_quotes_no_key() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        let broken = if agent.toml {
            format!("[mcp_servers.x]\nenv = {{ T = \"{KEY}\" \n")
        } else {
            format!("{{\"mcpServers\": {{\"x\": {{\"env\": {{\"T\": \"{KEY}\"}}}}")
        };
        write(&path, &broken);
        let (server, keys) = github();
        let failure = mcp::put(&file, &server, &keys).unwrap_err();
        assert!(
            !failure.message().contains(KEY),
            "{}: {}",
            agent.kind,
            failure.message()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            broken,
            "{}",
            agent.kind
        );
        assert!(
            mcp::recorded(home.path()).unwrap().is_empty(),
            "{}",
            agent.kind
        );
    }
}

#[test]
fn a_key_that_is_missing_or_runs_a_command_writes_nothing() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        write(&path, agent.original);
        let (server, _) = github();
        let missing = mcp::put(&file, &server, &Keys::new()).unwrap_err();
        assert!(
            missing.message().contains("GITHUB_TOKEN"),
            "{}",
            missing.message()
        );
        let runs = Keys::from([(
            "GITHUB_TOKEN".to_owned(),
            Secret::new("!cat ~/.ssh/id_rsa".to_owned()),
        )]);
        let refused = mcp::put(&file, &server, &runs).unwrap_err();
        assert!(
            !refused.message().contains("id_rsa"),
            "{}",
            refused.message()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            agent.original,
            "{}",
            agent.kind
        );
        assert!(
            mcp::recorded(home.path()).unwrap().is_empty(),
            "{}",
            agent.kind
        );
    }
}

#[test]
fn opencodes_second_shape_counts_and_a_server_taken_in_from_it_moves_to_the_first() {
    let agent = agents()
        .into_iter()
        .find(|agent| agent.kind == "opencode")
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    let file = default_file(&agent, home.path());
    let path = home.path().join(agent.file);
    let original =
        "{\"mcp\": {\"servers\": {\"github\": {\"type\": \"local\", \"command\": [\"gh-mcp\"]}}}}";
    write(&path, original);
    let (server, keys) = github();
    assert_eq!(mcp::put(&file, &server, &keys).unwrap(), Put::Theirs);
    // A server named like OpenCode 2's table cannot be written beside it.
    let mut named_servers = server.clone();
    named_servers.name = "servers".to_owned();
    assert_eq!(
        mcp::put(&file, &named_servers, &keys).unwrap(),
        Put::Unsupported
    );
    assert!(mcp::adopt(&file, "github").unwrap());
    assert_eq!(mcp::put(&file, &server, &keys).unwrap(), Put::Written);
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["mcp"]["github"], agent.local);
    assert_eq!(written["mcp"]["servers"], json!({}));
}

#[test]
fn taking_every_entry_out_of_a_file_leaves_the_operators_alone() {
    for agent in agents() {
        let home = tempfile::tempdir().unwrap();
        let file = default_file(&agent, home.path());
        let path = home.path().join(agent.file);
        write(&path, agent.original);
        let (local, local_keys) = github();
        let (remote, remote_keys) = linear();
        mcp::put(&file, &local, &local_keys).unwrap();
        mcp::put(&file, &remote, &remote_keys).unwrap();
        let recorded = mcp::recorded(home.path()).unwrap();
        assert_eq!(recorded.len(), 1, "{}", agent.kind);
        assert_eq!(recorded[0].0, file);
        assert_eq!(recorded[0].1, ["github", "linear"]);
        let mut gone = mcp::take_all(&file).unwrap();
        gone.sort();
        assert_eq!(gone, ["github", "linear"], "{}", agent.kind);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            agent.original,
            "{}",
            agent.kind
        );
        assert!(mcp::recorded(home.path()).unwrap().is_empty());
    }
}

#[test]
fn what_hide_reads_back_prints_no_key() {
    let agent = agents().remove(0);
    let home = tempfile::tempdir().unwrap();
    let file = default_file(&agent, home.path());
    let (server, keys) = github();
    mcp::put(&file, &server, &keys).unwrap();
    let found = mcp::read(&file).unwrap();
    let printed = format!("{found:?} {keys:?} {file:?}");
    assert!(!printed.contains(KEY), "{printed}");
    let (_, read_keys) = found[0].server.as_ref().unwrap();
    assert_eq!(read_keys["GITHUB_TOKEN"].expose(), KEY);
}
