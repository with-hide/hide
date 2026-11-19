use super::paste::{Pasted, read};
use super::*;

fn one(text: &str) -> Pasted {
    let mut read = read(text);
    assert_eq!(read.len(), 1, "{text}: {read:?}");
    read.remove(0)
}

fn local(name: &str, command: &str, args: &[&str], env: &[&str]) -> Server {
    Server {
        name: name.to_owned(),
        reach: Reach::Local {
            command: command.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: env.iter().map(|name| (*name).to_owned()).collect(),
        },
    }
}

fn remote(name: &str, transport: Remote, url: &str, headers: &[&str]) -> Server {
    Server {
        name: name.to_owned(),
        reach: Reach::Remote {
            transport,
            url: url.to_owned(),
            headers: headers.iter().map(|name| (*name).to_owned()).collect(),
        },
    }
}

fn key(pasted: &Pasted, name: &str) -> Option<String> {
    pasted.keys.get(name).map(|value| value.expose().to_owned())
}

#[test]
fn a_runner_command_reads_as_a_local_server_named_after_its_package() {
    // The forms MCP servers' READMEs give for a local server.
    let cases = [
        (
            "npx -y @modelcontextprotocol/server-github",
            local(
                "github",
                "npx",
                &["-y", "@modelcontextprotocol/server-github"],
                &[],
            ),
        ),
        (
            "uvx mcp-server-fetch",
            local("fetch", "uvx", &["mcp-server-fetch"], &[]),
        ),
        (
            "docker run -i --rm ghcr.io/github/github-mcp-server",
            local(
                "github",
                "docker",
                &["run", "-i", "--rm", "ghcr.io/github/github-mcp-server"],
                &[],
            ),
        ),
        (
            "npx @playwright/mcp@latest",
            local("playwright", "npx", &["@playwright/mcp@latest"], &[]),
        ),
        (
            "/usr/local/bin/my-tool --stdio",
            local("my-tool", "/usr/local/bin/my-tool", &["--stdio"], &[]),
        ),
    ];
    for (text, server) in cases {
        let pasted = one(text);
        assert_eq!(pasted.server, Some(server), "{text}");
        assert!(pasted.keys.is_empty(), "{text}");
    }
}

#[test]
fn variables_before_the_command_become_its_keys() {
    let pasted =
        one("GITHUB_PERSONAL_ACCESS_TOKEN=ghp_abc npx -y @modelcontextprotocol/server-github");
    assert_eq!(
        pasted.server,
        Some(local(
            "github",
            "npx",
            &["-y", "@modelcontextprotocol/server-github"],
            &["GITHUB_PERSONAL_ACCESS_TOKEN"]
        ))
    );
    assert_eq!(
        key(&pasted, "GITHUB_PERSONAL_ACCESS_TOKEN").as_deref(),
        Some("ghp_abc")
    );
}

#[test]
fn an_agents_own_add_command_is_read_by_its_options() {
    let pasted = one(
        "claude mcp add github -s user -e GITHUB_TOKEN=ghp_abc -- npx -y @modelcontextprotocol/server-github",
    );
    assert_eq!(
        pasted.server,
        Some(local(
            "github",
            "npx",
            &["-y", "@modelcontextprotocol/server-github"],
            &["GITHUB_TOKEN"]
        ))
    );
    assert_eq!(key(&pasted, "GITHUB_TOKEN").as_deref(), Some("ghp_abc"));

    let pasted = one(
        "claude mcp add --transport http linear https://mcp.linear.app/mcp --header \"Authorization: Bearer lin_123\"",
    );
    assert_eq!(
        pasted.server,
        Some(remote(
            "linear",
            Remote::Http,
            "https://mcp.linear.app/mcp",
            &["Authorization"]
        ))
    );
    assert_eq!(
        key(&pasted, "Authorization").as_deref(),
        Some("Bearer lin_123")
    );

    let pasted = one("claude mcp add --transport sse asana https://mcp.asana.com/sse");
    assert_eq!(
        pasted.server,
        Some(remote(
            "asana",
            Remote::Sse,
            "https://mcp.asana.com/sse",
            &[]
        ))
    );

    let pasted = one("codex mcp add context7 --env API_KEY=c7 -- npx -y @upstash/context7-mcp");
    assert_eq!(
        pasted.server,
        Some(local(
            "context7",
            "npx",
            &["-y", "@upstash/context7-mcp"],
            &["API_KEY"]
        ))
    );
    assert_eq!(key(&pasted, "API_KEY").as_deref(), Some("c7"));

    let pasted = one("codex mcp add figma --url https://mcp.figma.com/mcp");
    assert_eq!(
        pasted.server,
        Some(remote(
            "figma",
            Remote::Http,
            "https://mcp.figma.com/mcp",
            &[]
        ))
    );
}

#[test]
fn an_address_reads_as_a_remote_server_and_an_sse_path_as_sse() {
    let pasted = one("https://mcp.github.com/mcp");
    assert_eq!(
        pasted.server,
        Some(remote(
            "github",
            Remote::Http,
            "https://mcp.github.com/mcp",
            &[]
        ))
    );
    let pasted = one("https://api.githubcopilot.com/mcp/");
    assert_eq!(
        pasted.server,
        Some(remote(
            "githubcopilot",
            Remote::Http,
            "https://api.githubcopilot.com/mcp/",
            &[]
        ))
    );
    let pasted = one("https://mcp.asana.com/sse");
    assert_eq!(
        pasted.server,
        Some(remote(
            "asana",
            Remote::Sse,
            "https://mcp.asana.com/sse",
            &[]
        ))
    );
    let pasted = one("http://localhost:3845/sse");
    assert_eq!(
        pasted.server,
        Some(remote(
            "localhost",
            Remote::Sse,
            "http://localhost:3845/sse",
            &[]
        ))
    );
}

#[test]
fn a_json_snippet_in_any_agents_shape_reads_its_servers_and_keys() {
    let pasted = one(r#"{
          "mcpServers": {
            "github": {
              "command": "npx",
              "args": ["-y", "@modelcontextprotocol/server-github"],
              "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "<YOUR_TOKEN>" }
            }
          }
        }"#);
    assert_eq!(
        pasted.server,
        Some(local(
            "github",
            "npx",
            &["-y", "@modelcontextprotocol/server-github"],
            &["GITHUB_PERSONAL_ACCESS_TOKEN"]
        ))
    );
    assert_eq!(
        key(&pasted, "GITHUB_PERSONAL_ACCESS_TOKEN").as_deref(),
        Some("<YOUR_TOKEN>")
    );

    // VS Code's `servers` with a remote server and its header.
    let pasted = one(
        r#"{"servers": {"Linear": {"type": "http", "url": "https://mcp.linear.app/mcp",
            "headers": {"Authorization": "Bearer x"}}}}"#,
    );
    assert_eq!(
        pasted.server,
        Some(remote(
            "linear",
            Remote::Http,
            "https://mcp.linear.app/mcp",
            &["Authorization"]
        ))
    );

    // OpenCode's command as one list, with `environment`.
    let pasted = one(
        r#"{"mcp": {"fetch": {"type": "local", "command": ["uvx", "mcp-server-fetch"],
            "environment": {"PROXY": "p"}}}}"#,
    );
    assert_eq!(
        pasted.server,
        Some(local("fetch", "uvx", &["mcp-server-fetch"], &["PROXY"]))
    );

    // agy's `serverUrl`, and a bare server with its name taken from the URL.
    let pasted = one(r#"{"notion": {"serverUrl": "https://mcp.notion.com/sse"}}"#);
    assert_eq!(
        pasted.server,
        Some(remote(
            "notion",
            Remote::Sse,
            "https://mcp.notion.com/sse",
            &[]
        ))
    );
    let pasted = one(r#"{"command": "uvx", "args": ["mcp-server-time"]}"#);
    assert_eq!(
        pasted.server,
        Some(local("time", "uvx", &["mcp-server-time"], &[]))
    );

    // Two servers stay two.
    let read =
        read(r#"{"mcpServers": {"a": {"command": "a"}, "b": {"url": "https://b.example/mcp"}}}"#);
    assert_eq!(
        read.iter()
            .map(|pasted| pasted.server.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn a_paste_hide_cannot_read_gives_empty_fields() {
    for text in [
        "",
        "   ",
        "{not json",
        "{\"x\": 1}",
        "ftp://example.com",
        "'unterminated",
    ] {
        let pasted = one(text);
        assert!(pasted.server.is_none(), "{text}: {pasted:?}");
        assert!(pasted.keys.is_empty(), "{text}");
    }
}

#[test]
fn a_name_is_normalized_to_what_every_agents_file_can_hold() {
    assert_eq!(normal_name("My Server!"), "my-server");
    assert_eq!(normal_name("  GitHub__x  "), "github__x");
    assert_eq!(normal_name("---"), "");
    assert_eq!(normal_name(&"a".repeat(80)).len(), MAX_NAME_BYTES);
    assert!(valid_name("context7_x-1"));
    assert!(!valid_name("has space"));
    assert!(!valid_name(""));
    assert!(!valid_name(&"a".repeat(MAX_NAME_BYTES + 1)));
}

#[test]
fn a_key_prints_masked_and_a_server_carries_none() {
    let pasted = one("TOKEN=s3cr3t-value npx -y some-mcp");
    let printed = format!("{pasted:?}");
    assert!(!printed.contains("s3cr3t-value"), "{printed}");
    assert!(printed.contains("••••"), "{printed}");
    let server = serde_json::to_string(&pasted.server).unwrap();
    assert!(!server.contains("s3cr3t-value"), "{server}");
    assert!(server.contains("TOKEN"), "{server}");
}
