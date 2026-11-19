//! What the operator pastes into the MCP add field (PRD skill-mcp-manager
//! B8, D-10): a command line (`npx -y @scope/server-x`, `uvx mcp-server-y`,
//! `claude mcp add ...`, `codex mcp add ...`), an `https` address, or a JSON
//! snippet from a README (`mcpServers`, `servers`, a named server or a bare
//! one). Each reading fills the fields the form shows (name, local or remote,
//! command or address, the variables and headers) and the values the paste
//! carried, which go to Hide's key file and nowhere else. A paste it cannot
//! read gives empty fields for the operator to fill in.

use serde_json::{Map, Value};

use super::{Keys, Reach, Remote, Secret, Server, normal_name};

/// One server read from a paste. `server` is `None` when nothing in the
/// paste could be read, and its name is empty when the paste named none.
#[derive(Debug, Default)]
pub struct Pasted {
    pub server: Option<Server>,
    /// The values the paste gave its variables or headers, by name.
    pub keys: Keys,
}

/// The servers a paste holds, in the order it lists them; one empty reading
/// when it holds none Hide can read. `found` answers whether a program name
/// is found on this machine: a command line whose program is neither a path
/// nor found is a sentence pasted into the field, not a server. A JSON
/// snippet's servers stay whatever they run, as a README gives them.
pub fn read(text: &str, found: &dyn Fn(&str) -> bool) -> Vec<Pasted> {
    let text = text.trim();
    let read = if text.starts_with('{') {
        json(text)
    } else if let Some(address) = address(text) {
        vec![address]
    } else {
        command_line(text, found).into_iter().collect()
    };
    if read.is_empty() {
        vec![Pasted::default()]
    } else {
        read
    }
}

/// An `http(s)` address on its own: a server Hide reaches remotely, through
/// Server-Sent Events when its path is an `/sse` endpoint.
fn address(text: &str) -> Option<Pasted> {
    if text.contains(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = text.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some(Pasted {
        server: Some(Server {
            name: name_of_host(host),
            reach: Reach::Remote {
                transport: transport_of_url(text),
                url: text.to_owned(),
                headers: Vec::new(),
            },
        }),
        keys: Keys::new(),
    })
}

fn transport_of_url(url: &str) -> Remote {
    let path = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    if path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .is_some_and(|last| last.eq_ignore_ascii_case("sse"))
    {
        Remote::Sse
    } else {
        Remote::Http
    }
}

/// The name a server reached at `host` goes by: the host's distinctive
/// label, `mcp.github.com` and `api.githubcopilot.com` read as `github` and
/// `githubcopilot`.
fn name_of_host(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let meaningful = labels
        .iter()
        .take(labels.len().saturating_sub(1).max(1))
        .rev()
        .find(|label| !matches!(**label, "mcp" | "api" | "www" | "server" | "co" | "com"))
        .or(labels.first());
    normal_name(meaningful.copied().unwrap_or_default())
}

/// A command line: `NAME=value` words before the program become variables;
/// `claude mcp add` and `codex mcp add` lines are read by their own options.
fn command_line(text: &str, found: &dyn Fn(&str) -> bool) -> Option<Pasted> {
    let words = hide_shell::simple_commands(text, hide_shell::Syntax::Posix)
        .into_iter()
        .find(|words| !words.is_empty())?;
    let mut keys = Keys::new();
    let mut env = Vec::new();
    let at = words
        .iter()
        .position(|word| !hide_shell::is_assignment(word))?;
    for word in &words[..at] {
        let (name, value) = word.split_once('=').expect("an assignment holds =");
        env.push(name.to_owned());
        keys.insert(name.to_owned(), Secret::new(value.to_owned()));
    }
    let words = &words[at..];
    // An address of another scheme is no program.
    if words[0].contains("://") {
        return None;
    }
    let program = hide_shell::Syntax::Posix.program(&words[0]);
    if words.get(1).map(String::as_str) == Some("mcp")
        && words.get(2).map(String::as_str) == Some("add")
    {
        return match program.as_str() {
            "claude" => claude_add(&words[3..], env, keys),
            "codex" => codex_add(&words[3..], env, keys),
            _ => None,
        };
    }
    if !words[0].contains(['/', '\\']) && !found(&words[0]) {
        return None;
    }
    Some(Pasted {
        server: Some(Server {
            name: name_of_command(words),
            reach: Reach::Local {
                command: words[0].clone(),
                args: words[1..].to_vec(),
                env,
            },
        }),
        keys,
    })
}

/// `claude mcp add [options] <name> <command-or-url> [args...]`, with
/// `-e/--env NAME=value`, `-t/--transport stdio|http|sse`, `-H/--header
/// "Name: value"` and `-s/--scope`, which Hide's servers do not take.
fn claude_add(words: &[String], mut env: Vec<String>, mut keys: Keys) -> Option<Pasted> {
    let mut transport = None;
    let mut headers = Vec::new();
    let mut rest = Vec::new();
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            rest.extend(words.by_ref().cloned());
            break;
        }
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| words.next().cloned());
        match flag {
            "-e" | "--env" => {
                let (name, given) = value()?
                    .split_once('=')
                    .map(|(n, v)| (n.to_owned(), v.to_owned()))?;
                env.push(name.clone());
                keys.insert(name, Secret::new(given));
            }
            "-t" | "--transport" => transport = Some(value()?),
            "-H" | "--header" => {
                let header = value()?;
                let (name, given) = header.split_once(':')?;
                let name = name.trim().to_owned();
                headers.push(name.clone());
                keys.insert(name, Secret::new(given.trim().to_owned()));
            }
            "-s" | "--scope" | "--client-id" | "--callback-port" => {
                value()?;
            }
            _ if flag.starts_with('-') && rest.is_empty() => {}
            _ => rest.push(word.clone()),
        }
    }
    let mut rest = rest.into_iter();
    let name = normal_name(&rest.next()?);
    let target = rest.next()?;
    let reach = match transport.as_deref() {
        Some("http") => Reach::Remote {
            transport: Remote::Http,
            url: target,
            headers,
        },
        Some("sse") => Reach::Remote {
            transport: Remote::Sse,
            url: target,
            headers,
        },
        Some("stdio") | None => Reach::Local {
            command: target,
            args: rest.collect(),
            env,
        },
        Some(_) => return None,
    };
    Some(Pasted {
        server: Some(Server { name, reach }),
        keys,
    })
}

/// `codex mcp add <name> [--env NAME=value] -- <command> [args...]` or
/// `codex mcp add <name> --url <url> [--bearer-token-env-var NAME]`.
fn codex_add(words: &[String], mut env: Vec<String>, mut keys: Keys) -> Option<Pasted> {
    let mut name = None;
    let mut url = None;
    let mut command = Vec::new();
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            command.extend(words.by_ref().cloned());
            break;
        }
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| words.next().cloned());
        match flag {
            "--env" => {
                let (variable, given) = value()?
                    .split_once('=')
                    .map(|(n, v)| (n.to_owned(), v.to_owned()))?;
                env.push(variable.clone());
                keys.insert(variable, Secret::new(given));
            }
            "--url" => url = Some(value()?),
            "--bearer-token-env-var" => {
                value()?;
            }
            _ if flag.starts_with('-') => {}
            _ if name.is_none() => name = Some(normal_name(word)),
            _ => command.push(word.clone()),
        }
    }
    let name = name?;
    let reach = match url {
        Some(url) => Reach::Remote {
            transport: Remote::Http,
            url,
            headers: Vec::new(),
        },
        None => {
            let mut command = command.into_iter();
            Reach::Local {
                command: command.next()?,
                args: command.collect(),
                env,
            }
        }
    };
    Some(Pasted {
        server: Some(Server { name, reach }),
        keys,
    })
}

/// A local server's program, and the package it runs when the program is
/// a runner (`npx`, `uvx`, `bunx`, `pnpm dlx`, `docker run`, ...).
pub(super) fn package_of(words: &[String]) -> (String, Option<&str>) {
    let program = hide_shell::Syntax::Posix.program(&words[0]);
    let runner = matches!(
        program.as_str(),
        "npx"
            | "uvx"
            | "bunx"
            | "pipx"
            | "pnpx"
            | "pnpm"
            | "yarn"
            | "bun"
            | "uv"
            | "docker"
            | "podman"
            | "deno"
            | "node"
            | "python"
            | "python3"
    );
    let package = if runner {
        words[1..]
            .iter()
            .find(|word| {
                !word.starts_with('-')
                    && !matches!(word.as_str(), "dlx" | "run" | "x" | "exec" | "tool")
            })
            .map(String::as_str)
    } else {
        None
    };
    (program, package)
}

/// The name a local server goes by: its package for a runner (`npx`,
/// `uvx`, `bunx`, `pnpm dlx`, `docker run`), the program's otherwise, with
/// the words every server package carries (`@scope/`, `server-`, `mcp-`,
/// `-mcp`) taken off.
fn name_of_command(words: &[String]) -> String {
    let (program, package) = package_of(words);
    let package = package.unwrap_or(&program);
    // `@scope/name@1.2.3`, `ghcr.io/org/name:tag`, `pkg==1.0`, `./server.js`;
    // a scoped package whose own name says only that it is a server
    // (`@playwright/mcp`) goes by its scope.
    let mut parts = package.trim_start_matches('@').rsplit('/');
    let base = parts.next().unwrap_or(package);
    let base = match parts.next() {
        Some(scope)
            if package.starts_with('@')
                && matches!(
                    base.split('@').next().unwrap_or(base),
                    "mcp" | "server" | "mcp-server"
                ) =>
        {
            scope
        }
        _ => base,
    };
    let base = base.split(['@', ':']).next().unwrap_or(base);
    let base = base.split("==").next().unwrap_or(base);
    let base = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".py"))
        .or_else(|| base.strip_suffix(".ts"))
        .unwrap_or(base);
    let mut name = base.to_ascii_lowercase();
    for prefix in ["mcp-server-", "server-", "mcp-"] {
        if let Some(rest) = name.strip_prefix(prefix)
            && !rest.is_empty()
        {
            name = rest.to_owned();
        }
    }
    for suffix in ["-mcp-server", "-server", "-mcp"] {
        if let Some(rest) = name.strip_suffix(suffix)
            && !rest.is_empty()
        {
            name = rest.to_owned();
        }
    }
    normal_name(&name)
}

/// A JSON snippet: `{"mcpServers": {...}}` and its relatives (`servers`,
/// `mcp_servers`, OpenCode's `mcp`), `{"<name>": {<server>}}`, or a bare
/// server object.
fn json(text: &str) -> Vec<Pasted> {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    for list in [
        "mcpServers",
        "servers",
        "mcp_servers",
        "mcp",
        "context_servers",
    ] {
        if let Some(Value::Object(servers)) = root.get(list) {
            let servers = match servers.get("servers") {
                Some(Value::Object(inner)) => inner,
                _ => servers,
            };
            return named(servers);
        }
    }
    if let Some(pasted) = server(String::new(), &root) {
        return vec![pasted];
    }
    named(&root)
}

fn named(servers: &Map<String, Value>) -> Vec<Pasted> {
    servers
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_object()
                .and_then(|object| server(normal_name(name), object))
        })
        .collect()
}

/// One server object in any of the agents' shapes: `command` as a string
/// with `args` or as one list (OpenCode's), `env` or `environment`, `url`,
/// `serverUrl` or `httpUrl`, and `type` or `transport`.
fn server(name: String, object: &Map<String, Value>) -> Option<Pasted> {
    let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
    let kind = text("type")
        .or_else(|| text("transport"))
        .map(|kind| kind.to_ascii_lowercase());
    let mut keys = Keys::new();
    let mut take = |values: Option<&Value>| -> Vec<String> {
        let Some(Value::Object(values)) = values else {
            return Vec::new();
        };
        values
            .iter()
            .map(|(name, value)| {
                let given = match value {
                    Value::String(given) => given.clone(),
                    other => other.to_string(),
                };
                keys.insert(name.clone(), Secret::new(given));
                name.clone()
            })
            .collect()
    };
    let url = text("url")
        .or_else(|| text("serverUrl"))
        .or_else(|| text("httpUrl"));
    let reach = if let Some(url) = url {
        let transport = match kind.as_deref() {
            Some("sse") => Remote::Sse,
            Some("http" | "streamable-http" | "streamablehttp" | "remote") => Remote::Http,
            _ if object.contains_key("httpUrl") => Remote::Http,
            _ => transport_of_url(&url),
        };
        Reach::Remote {
            transport,
            url,
            headers: take(object.get("headers").or_else(|| object.get("http_headers"))),
        }
    } else {
        let (command, mut args) = match object.get("command")? {
            Value::String(command) => (command.clone(), Vec::new()),
            Value::Array(words) => {
                let mut words = words.iter().map(|word| word.as_str().map(str::to_owned));
                let command = words.next()??;
                (command, words.collect::<Option<Vec<_>>>()?)
            }
            _ => return None,
        };
        if let Some(Value::Array(more)) = object.get("args") {
            args.extend(more.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        Reach::Local {
            command,
            args,
            env: take(object.get("env").or_else(|| object.get("environment"))),
        }
    };
    let name = if name.is_empty() {
        match &reach {
            Reach::Local { command, args, .. } => {
                let mut words = vec![command.clone()];
                words.extend(args.iter().cloned());
                name_of_command(&words)
            }
            Reach::Remote { url, .. } => address(url)
                .and_then(|pasted| pasted.server)
                .map(|server| server.name)
                .unwrap_or_default(),
        }
    } else {
        name
    };
    Some(Pasted {
        server: Some(Server { name, reach }),
        keys,
    })
}
