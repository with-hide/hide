//! A local node, started the way hided starts it
//! (`hide_node::terminal::holder::Holders::start_local`), driven over its
//! standard input by the pane holder's restart lane
//! (`scripts/holder-restart-check.py`) and its measurements
//! (`scripts/holder-perf.py`); docs/TESTING.md, The holder restart lane, and
//! docs/PERFORMANCE_TESTING.md, The pane holder's path.
//!
//! It is not a product program: it is built by those lanes only
//! (`test = false` in `Cargo.toml`), and it must be copied beside the built
//! `hide-holder`, where the node finds its holder as hided does.
//!
//! ```text
//! holder_node node --home <dir> --state <dir> [--herdr-bin <herdr>] [--herdr-socket <socket>]
//! holder_node echo <log>
//! ```
//!
//! `node` reads one JSON request a line and writes one JSON answer a line,
//! `{"ok": true, ...}` or `{"ok": false, "error": "..."}`, in order. The
//! development switch is read from `HIDE_DEV_PANE_HOLDER`, as hided reads it;
//! with `--herdr-socket` the node's Herdr sessions go to that server through
//! the pinned client, as hided's do. `echo` is a pane program for the
//! measurements: on a raw terminal it writes each byte it reads to `<log>`
//! and draws it on the first row (`echo.rs`).

mod echo;
mod perf;
mod screens;

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hide_holder_protocol::record;
use hide_holder_protocol::{Answer, Appearance, Rgb, Size, Theme};
use hide_kit::layout;
use hide_kit::panes::{StartRequest, Started};
use hide_node::terminal::holder::{Holders, LocalTerminals, Switch};
use hide_node::terminal::{Attacher, LocalAttacher, Mode, OutputSink, ReportSink, SessionParts};
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalNode};
use serde_json::{Value, json};

use crate::screens::{Reports, Screens};

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let code = match args.first().and_then(|word| word.to_str()) {
        Some("node") => match Options::parse(&args[1..]).and_then(|options| node(&options)) {
            Ok(()) => 0,
            Err(why) => {
                eprintln!("holder_node: {why}");
                1
            }
        },
        Some("echo") => match args.get(1) {
            Some(log) => echo::run(Path::new(log)),
            None => usage(),
        },
        _ => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!(
        "usage: holder_node node --home <dir> --state <dir> [--herdr-bin <herdr>] [--herdr-socket <socket>]\n       holder_node echo <log>"
    );
    2
}

struct Options {
    home: PathBuf,
    state: PathBuf,
    herdr_bin: Option<PathBuf>,
    herdr_socket: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Self, String> {
        let (mut home, mut state, mut herdr_bin, mut herdr_socket) = (None, None, None, None);
        let mut words = args.iter();
        while let Some(word) = words.next() {
            let value = words
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{} takes a value", word.to_string_lossy()))?;
            match word.to_str() {
                Some("--home") => home = Some(value),
                Some("--state") => state = Some(value),
                Some("--herdr-bin") => herdr_bin = Some(value),
                Some("--herdr-socket") => herdr_socket = Some(value),
                _ => return Err(format!("unknown option {}", word.to_string_lossy())),
            }
        }
        Ok(Self {
            home: home.ok_or("--home is required")?,
            state: state.ok_or("--state is required")?,
            herdr_bin,
            herdr_socket,
        })
    }
}

/// Herdr's sessions when no Herdr server was named: none.
struct NoHerdr;

impl Attacher for NoHerdr {
    fn open(&self, _: &str, _: Mode, _: u16, _: u16) -> Result<SessionParts, String> {
        Err("this node was started without a Herdr server".to_owned())
    }
}

/// The node and what it handed its screen and its core.
pub struct Node {
    pub terminals: LocalTerminals,
    pub screens: Arc<Screens>,
    pub reports: Arc<Reports>,
    pub state: PathBuf,
}

impl Node {
    fn holders(&self) -> Result<&Holders, String> {
        self.terminals
            .holders
            .as_deref()
            .ok_or_else(|| "the development switch is off: this node runs no holder".to_owned())
    }
}

fn node(options: &Options) -> Result<(), String> {
    let switch = Switch::from_env()?;
    let screens = Arc::new(Screens::default());
    let reports = Arc::new(Reports::default());
    let herdr: Box<dyn Attacher> = match &options.herdr_socket {
        Some(socket) => Box::new(LocalAttacher::new(
            options.herdr_bin.clone(),
            socket.clone(),
        )),
        None => Box::new(NoHerdr),
    };
    let terminals = Holders::start_local(
        switch,
        &options.home,
        &options.state,
        options.herdr_bin.as_deref(),
        herdr,
        Arc::clone(&screens) as Arc<dyn OutputSink>,
        Arc::clone(&reports) as Arc<dyn ReportSink>,
    )?;
    let node = Node {
        terminals,
        screens,
        reports,
        state: options.state.clone(),
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    writeln!(
        stdout,
        "{}",
        json!({"ok": true, "ready": std::process::id()})
    )
    .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())?;
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(request) => match answer(&node, &request) {
                Ok(Some(mut value)) => {
                    value["ok"] = json!(true);
                    value
                }
                Ok(None) => break,
                Err(why) => json!({"ok": false, "error": why}),
            },
            Err(error) => json!({"ok": false, "error": format!("not JSON: {error}")}),
        };
        writeln!(stdout, "{answer}").map_err(|error| error.to_string())?;
        stdout.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn text(request: &Value, key: &str) -> Result<String, String> {
    request[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("`{key}` is required"))
}

pub fn number(request: &Value, key: &str, fallback: u64) -> u64 {
    request[key].as_u64().unwrap_or(fallback)
}

pub fn wait_limit(request: &Value) -> Duration {
    Duration::from_millis(number(request, "timeout_ms", 30_000))
}

/// The colors the node draws its panes with; the measurements and the lane
/// use one theme so the panes' color answers are alike on both paths.
pub fn theme() -> Theme {
    Theme {
        foreground: Some(Rgb {
            r: 0xd0,
            g: 0xd0,
            b: 0xd0,
        }),
        background: Some(Rgb {
            r: 0x10,
            g: 0x10,
            b: 0x10,
        }),
        cursor: None,
        palette: Vec::new(),
        appearance: Appearance::Dark,
    }
}

/// The answer to one request; `None` ends the node.
fn answer(node: &Node, request: &Value) -> Result<Option<Value>, String> {
    let op = text(request, "op")?;
    let value = match op.as_str() {
        "quit" => return Ok(None),
        "start" => start(node, request)?,
        "key" => {
            let pane = text(request, "pane")?;
            let bytes = unhex(&text(request, "hex")?)?;
            node.terminals.service.key(KeyTarget::Pane(pane), bytes, 0);
            json!({})
        }
        "release" => {
            let pane = text(request, "pane")?;
            node.terminals.service.control(TerminalControl::Release {
                pane,
                message: "released by the measurement".to_owned(),
            });
            json!({})
        }
        "attach" => {
            let pane = text(request, "pane")?;
            node.terminals.service.control(TerminalControl::Attach {
                pane,
                size: None,
                manual: true,
            });
            json!({})
        }
        "read" => {
            let pane = text(request, "pane")?;
            let lines = u16::try_from(number(request, "lines", 200)).unwrap_or(u16::MAX);
            match node.holders()?.read(&pane, lines) {
                Ok(Answer::Text { text, truncated }) => {
                    json!({"text": text, "truncated": truncated})
                }
                Ok(other) => return Err(format!("a read answered {other:?}")),
                Err(error) => return Err(format!("the read failed: {error}")),
            }
        }
        "wait_text" => wait_text(node, request)?,
        "record" => {
            let pane = text(request, "pane")?;
            let found = record::read(&layout::pane_record(&node.state, &pane))
                .map_err(|why| why.to_string())?
                .ok_or_else(|| format!("pane {pane} has no record"))?;
            json!({"record": found})
        }
        "tree" => tree(node, request)?,
        "frames" => {
            let pane = text(request, "pane")?;
            node.screens.summary(&pane)
        }
        "wait_full" => {
            let pane = text(request, "pane")?;
            let since = usize::try_from(number(request, "since", 0)).unwrap_or(usize::MAX);
            let (index, bytes) = node
                .screens
                .wait_full(&pane, since, wait_limit(request))
                .ok_or_else(|| format!("pane {pane} drew no full frame from frame {since} on"))?;
            json!({"index": index, "hex": hex(&bytes), "count": node.screens.count(&pane)})
        }
        "redraw" => {
            let pane = text(request, "pane")?;
            node.terminals.service.redraw(&pane);
            json!({})
        }
        "states" => {
            let pane = text(request, "pane")?;
            json!({"states": node.reports.states(&pane)})
        }
        "terminate" => {
            let pane = text(request, "pane")?;
            let ended = node
                .holders()?
                .terminate(&node.terminals.service, &pane)
                .map_err(|why| format!("{why:?}"))?;
            json!({"record": ended})
        }
        _ => perf::answer(node, &op, request)?,
    };
    Ok(Some(value))
}

fn start(node: &Node, request: &Value) -> Result<Value, String> {
    let pane = text(request, "pane")?;
    let program: Vec<OsString> = request["program"]
        .as_array()
        .ok_or("`program` is required")?
        .iter()
        .map(|word| {
            word.as_str()
                .map(OsString::from)
                .ok_or("`program` takes strings")
        })
        .collect::<Result<_, _>>()?;
    let env: Vec<(OsString, OsString)> = request["env"]
        .as_object()
        .map(|vars| {
            vars.iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (OsString::from(key), OsString::from(value)))
                })
                .collect()
        })
        .unwrap_or_default();
    let size = Size {
        rows: u16::try_from(number(request, "rows", 40)).map_err(|_| "rows is too large")?,
        cols: u16::try_from(number(request, "cols", 120)).map_err(|_| "cols is too large")?,
    };
    let start = StartRequest {
        pane: pane.clone(),
        program,
        cwd: PathBuf::from(text(request, "cwd")?),
        env,
        size,
        theme: theme(),
    };
    let started = node
        .holders()?
        .start(&node.terminals.service, &start)
        .map_err(|refused| format!("{pane} did not start: {refused}"))?;
    let (kind, found) = match started {
        Started::Started(found) => ("started", found),
        Started::Existing(found) => ("existing", found),
    };
    Ok(json!({"started": kind, "record": found}))
}

/// Polls the pane's read, a connection of its own, until its text holds
/// `text`. A poll only looks.
fn wait_text(node: &Node, request: &Value) -> Result<Value, String> {
    let pane = text(request, "pane")?;
    let wanted = text(request, "text")?;
    let absent = request["absent"].as_str().map(str::to_owned);
    let lines = u16::try_from(number(request, "lines", 200)).unwrap_or(u16::MAX);
    let every = Duration::from_millis(number(request, "every_ms", 100));
    let deadline = Instant::now() + wait_limit(request);
    let mut last = String::new();
    loop {
        if let Ok(Answer::Text { text, .. }) = node.holders()?.read(&pane, lines) {
            let found = text.lines().any(|line| {
                line.contains(&wanted)
                    && absent.as_ref().is_none_or(|absent| !line.contains(absent))
            });
            if found {
                return Ok(json!({"text": text}));
            }
            last = text;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "pane {pane} never showed {wanted:?}; it shows:\n{last}"
            ));
        }
        std::thread::park_timeout(every);
    }
}

/// The pane's program and every process under it, each with its start
/// time and name, from one look at the process table.
fn tree(node: &Node, request: &Value) -> Result<Value, String> {
    let pane = text(request, "pane")?;
    let found = record::read(&layout::pane_record(&node.state, &pane))
        .map_err(|why| why.to_string())?
        .ok_or_else(|| format!("pane {pane} has no record"))?;
    let program = found.program.ok_or("the pane's program has not started")?;
    let mut members = vec![program.pid];
    members.extend(
        hide_platform::process::descendants(program.pid).map_err(|error| error.to_string())?,
    );
    let processes: Vec<Value> = members
        .into_iter()
        .filter_map(|pid| {
            let started = hide_platform::process::start_time(pid).ok()?;
            let name = hide_platform::process::name_of(pid).unwrap_or_default();
            let parent = hide_platform::process::parent_of(pid).ok();
            Some(json!({"pid": pid, "started": started, "name": name, "parent": parent}))
        })
        .collect();
    Ok(json!({"holder": found.holder, "program": program, "processes": processes}))
}

pub fn grid(request: &Value) -> GridSize {
    GridSize {
        rows: u16::try_from(number(request, "rows", 40)).unwrap_or(40),
        cols: u16::try_from(number(request, "cols", 120)).unwrap_or(120),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex of odd length".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).map_err(|error| error.to_string()))
        .collect()
}
