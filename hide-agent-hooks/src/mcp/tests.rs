use super::paste::Pasted;
use super::*;

/// A paste read on a machine that finds every program.
fn read(text: &str) -> Vec<Pasted> {
    super::paste::read(text, &|_| true)
}

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

#[test]
fn a_local_servers_version_is_what_its_package_or_image_pins() {
    let pin_of = |line: &str| {
        let words: Vec<String> = line.split(' ').map(str::to_owned).collect();
        pin(&words[0], &words[1..])
    };
    let cases = [
        ("npx -y @modelcontextprotocol/server-github", Pin::NpmLatest),
        ("npx @playwright/mcp@latest", Pin::NpmLatest),
        (
            "npx -y @upstash/context7-mcp@1.0.14",
            Pin::Version("1.0.14".to_owned()),
        ),
        ("npx some-mcp@2.1.0", Pin::Version("2.1.0".to_owned())),
        ("pnpm dlx some-mcp", Pin::NpmLatest),
        ("uvx mcp-server-fetch", Pin::Unpinned),
        (
            "uvx mcp-server-fetch==2025.4.7",
            Pin::Version("2025.4.7".to_owned()),
        ),
        (
            "docker run -i --rm ghcr.io/github/github-mcp-server",
            Pin::Unpinned,
        ),
        (
            "docker run -i --rm ghcr.io/github/github-mcp-server:v0.5.0",
            Pin::Version("v0.5.0".to_owned()),
        ),
        ("docker run -i localhost:5000/tool", Pin::Unpinned),
        ("/usr/local/bin/my-tool --stdio", Pin::Unpinned),
    ];
    for (line, expected) in cases {
        assert_eq!(pin_of(line), expected, "{line}");
    }
}

#[test]
fn a_server_hide_keeps_has_a_name_a_command_or_address_and_bounded_fields() {
    for text in [
        "GITHUB_TOKEN=x npx -y @modelcontextprotocol/server-github",
        "claude mcp add --transport http linear https://mcp.linear.app/mcp --header \"Authorization: Bearer x\"",
        "https://mcp.asana.com/sse",
        r#"{"mcp": {"fetch": {"type": "local", "command": ["uvx", "mcp-server-fetch"]}}}"#,
    ] {
        let server = one(text).server.unwrap();
        assert_eq!(server.check(), Ok(()), "{text}");
    }
    let refused = [
        local("has space", "npx", &[], &[]),
        local("ok", "  ", &[], &[]),
        local("ok", "npx", &[], &["A", "A"]),
        local("ok", "npx", &[], &["A=B"]),
        local("ok", "npx", &[], &[""]),
        local("ok", &"x".repeat(MAX_ARG_BYTES + 1), &[], &[]),
        remote("ok", Remote::Http, "ftp://example.com/mcp", &[]),
        remote("ok", Remote::Http, "https://example.com/a b", &[]),
        remote(
            "ok",
            Remote::Http,
            "https://example.com/mcp",
            &["Bad Header"],
        ),
    ];
    for server in refused {
        assert!(server.check().is_err(), "{server:?}");
    }
    let many: Vec<String> = (0..=MAX_ARGS).map(|index| index.to_string()).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(local("ok", "npx", &many, &[]).check().is_err());
}

#[test]
fn every_named_key_needs_a_value_an_agents_file_can_carry() {
    let server = local("github", "npx", &["x"], &["TOKEN", "OTHER"]);
    let keys = |pairs: &[(&str, &str)]| -> Keys {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Secret::new((*value).to_owned())))
            .collect()
    };
    assert_eq!(
        server.check_keys(&keys(&[("TOKEN", "s3cr3t"), ("OTHER", "o")])),
        Ok(())
    );
    for (given, refusal) in [
        (keys(&[("TOKEN", "s3cr3t")]), "OTHER has no value"),
        (
            keys(&[("TOKEN", "s3cr3t"), ("OTHER", "")]),
            "OTHER has no value",
        ),
        (
            keys(&[("TOKEN", "!cat ~/.ssh/id_rsa"), ("OTHER", "o")]),
            "TOKEN starts with !, which Pi and omp run as a command",
        ),
    ] {
        let reason = server.check_keys(&given).unwrap_err();
        assert_eq!(reason, refusal);
        assert!(!reason.contains("s3cr3t") && !reason.contains("ssh"));
    }
}

#[test]
fn a_command_line_whose_program_this_machine_lacks_is_no_server_and_a_json_snippet_still_is() {
    let found = |program: &str| program == "npx";
    let read = |text: &str| super::paste::read(text, &found);
    let servers = |text: &str| -> Vec<Option<Server>> {
        read(text).into_iter().map(|pasted| pasted.server).collect()
    };
    assert_eq!(servers("please add the github server for me"), [None]);
    assert_eq!(servers("uvx mcp-server-fetch"), [None]);
    assert!(servers("npx -y @modelcontextprotocol/server-github")[0].is_some());
    // A path is a program wherever it is.
    assert!(servers("/opt/fetch/bin/fetch-mcp --port 8080")[0].is_some());
    // A README's snippet is a server whatever it runs (B8).
    assert_eq!(
        servers(r#"{"mcpServers": {"fetch": {"command": "uvx", "args": ["mcp-server-fetch"]}}}"#),
        [Some(local("fetch", "uvx", &["mcp-server-fetch"], &[]))]
    );
}
