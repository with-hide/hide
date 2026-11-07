//! A stand-in for hided's bootstrap service, which a hook's requests reach
//! (`hide_agent_hooks::hook_request`). Each request is answered by the test's
//! stand-in `hide` program, run with the arguments of the `hide` command that
//! sends the same request and with the request's input on its stdin, so a
//! test keeps one table of answers, keyed by the command an operator would
//! type, for everything the hook asks the daemon.
//!
//! The service lives in a state folder of its own, which [`route`] hands the
//! hook through `HIDE_STATE_DIR`, so no test's hook can reach the account's
//! own daemon; a test of how the hook finds its state folder names the
//! folder ([`serve_at`](StandInDaemon::serve_at)).
//!
//! Its answers come from the `sh` stand-ins the Unix tests keep; on Windows
//! the hook's requests are proved against the real daemon by the agent
//! journey.

#![cfg(unix)]
//!
//! [`route`]: StandInDaemon::route

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use hide_platform::fs::private;
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream};
use serde_json::Value;

pub struct StandInDaemon {
    /// The folder the socket sits in, and the state folder when the test
    /// named none.
    _sockets: tempfile::TempDir,
    state: PathBuf,
    closer: ListenerCloser,
    running: Arc<Mutex<Running>>,
    accepting: Option<JoinHandle<()>>,
    answering: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl StandInDaemon {
    /// Serves every request with `program`, run with `envs` besides the test's
    /// own environment, the request's pane as `HIDE_PANE_ID` and this
    /// service's state folder as `HIDE_STATE_DIR`.
    pub fn serve(program: &Path, envs: &[(&'static str, OsString)]) -> Self {
        Self::serve_at(None, program, envs)
    }

    /// [`serve`](Self::serve) for the daemon of the state folder `state`.
    pub fn serve_at(
        state: Option<&Path>,
        program: &Path,
        envs: &[(&'static str, OsString)],
    ) -> Self {
        let sockets = short_dir();
        let state = state.map_or_else(|| sockets.path().join("state"), Path::to_path_buf);
        let socket = sockets.path().join("s").join("b.sock");
        private::create_dir(socket.parent().unwrap()).unwrap();
        let listener = LocalListener::bind(&socket).unwrap();
        let record = hide_agent_hooks::hook_request::bootstrap_socket_record(&state);
        private::create_dir_all(record.parent().unwrap()).unwrap();
        private::create_new_file(&record)
            .unwrap()
            .write_all(socket.to_str().unwrap().as_bytes())
            .unwrap();
        let closer = listener.closer();
        let answering = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(Mutex::new(Running::default()));
        let served = Served {
            program: program.to_path_buf(),
            envs: envs.to_vec(),
            state: state.clone(),
            running: Arc::clone(&running),
        };
        let threads = Arc::clone(&answering);
        let accepting = std::thread::spawn(move || {
            while let Ok(stream) = listener.accept() {
                let served = served.clone();
                let thread = std::thread::spawn(move || served.answer(stream));
                threads.lock().unwrap().push(thread);
            }
        });
        Self {
            _sockets: sockets,
            state,
            closer,
            running,
            accepting: Some(accepting),
            answering,
        }
    }

    /// Withdraws the socket's record, as a daemon that is not running has
    /// published none: no request reaches this service from here on.
    pub fn withdraw(&self) {
        std::fs::remove_file(hide_agent_hooks::hook_request::bootstrap_socket_record(
            &self.state,
        ))
        .unwrap();
    }

    /// Points `command`'s hook at this service.
    pub fn route<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command.env("HIDE_STATE_DIR", &self.state)
    }
}

impl Drop for StandInDaemon {
    fn drop(&mut self) {
        // No request is taken after this, and no answer thread is started.
        self.closer.close();
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        // A stand-in still answering (one that never does, as a stuck
        // daemon) is ended with the service, and one started from here on
        // is ended as it registers; every one is reaped.
        let children = {
            let mut running = self.running.lock().unwrap();
            running.closed = true;
            std::mem::take(&mut running.children)
        };
        for child in children {
            end(child);
        }
        for thread in self.answering.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
struct Served {
    program: PathBuf,
    envs: Vec<(&'static str, OsString)>,
    state: PathBuf,
    running: Arc<Mutex<Running>>,
}

/// The stand-ins answering now, and whether the service has ended, under one
/// lock, so none registers after the service ended them.
#[derive(Default)]
struct Running {
    closed: bool,
    children: Vec<Child>,
}

fn end(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

impl Served {
    fn answer(&self, mut stream: LocalStream) {
        let mut line = String::new();
        if BufReader::new(&mut stream).read_line(&mut line).is_err() {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let hook = &request["hook"];
        let (arguments, input) = as_hide_command(&hook["command"]);
        let mut command = Command::new(&self.program);
        command
            .args(&arguments)
            .envs(self.envs.iter().map(|(key, value)| (key, value)))
            .env("HIDE_STATE_DIR", &self.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        match request["pane_id"].as_str() {
            Some(pane) if !pane.is_empty() => command.env("HIDE_PANE_ID", pane),
            _ => command.env_remove("HIDE_PANE_ID"),
        };
        let Ok(mut child) = command.spawn() else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let stdout = child.stdout.take();
        {
            let mut running = self.running.lock().unwrap();
            if running.closed {
                drop(running);
                end(child);
                return;
            }
            running.children.push(child);
        }
        // Its output ends when it does, or when the service ends it.
        let mut output = Vec::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut output);
        }
        // What the stand-in printed is the daemon's answer; one that printed
        // no JSON line ends the stream without an answer.
        let Some(Ok(mut answer)) = String::from_utf8_lossy(&output)
            .lines()
            .next()
            .map(serde_json::from_str::<Value>)
        else {
            return;
        };
        if answer["ok"] == true && answer.get("request_id").is_none() {
            answer["request_id"] = hook["request_id"].clone();
        }
        // It stands in for the daemon of the installation the pane names, if
        // any, which its answer names back (`hook_request::install`).
        if answer["ok"] == true && answer.get("install").is_none() {
            answer["install"] = request["install"].clone();
        }
        let _ = writeln!(stream, "{answer}");
    }
}

/// The `hide` command line and stdin that send `command`.
fn as_hide_command(command: &Value) -> (Vec<String>, String) {
    let text = |key: &str| command[key].as_str().unwrap_or_default().to_owned();
    match command["kind"].as_str() {
        Some("memory") => (
            [
                "workspace",
                "memory",
                "--event",
                &text("event"),
                "--runtime",
                &text("runtime"),
                "--session",
                &text("session"),
            ]
            .map(str::to_owned)
            .to_vec(),
            text("prompt"),
        ),
        Some("workspace_info") => (
            ["workspace", "info", "--hook"].map(str::to_owned).to_vec(),
            String::new(),
        ),
        Some("status") => (
            ["workspace", "status", "--hook"]
                .map(str::to_owned)
                .to_vec(),
            command["report"].to_string(),
        ),
        Some("letter_pull") => {
            let mut arguments = vec!["inbox".to_owned(), "--hook".to_owned()];
            if let Some(digest) = command["prompt_digest"].as_str() {
                arguments.extend(["--prompt-digest".to_owned(), digest.to_owned()]);
            }
            if let Some(session) = command["session"].as_str() {
                arguments.extend(["--session".to_owned(), session.to_owned()]);
            }
            match command["point"].as_str() {
                Some("tool") => arguments.push("--tool".to_owned()),
                Some("turn_end") => arguments.push("--turn-end".to_owned()),
                _ => {}
            }
            (arguments, String::new())
        }
        Some("letter_confirm") => {
            let mut arguments = vec!["inbox".to_owned(), "--confirm".to_owned()];
            arguments.extend(
                command["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned)),
            );
            (arguments, String::new())
        }
        Some("registration") => (
            ["workspace", "bootstrap"].map(str::to_owned).to_vec(),
            String::new(),
        ),
        _ => (Vec::new(), String::new()),
    }
}

/// A folder short enough for a socket path on every system.
fn short_dir() -> tempfile::TempDir {
    if cfg!(unix) {
        tempfile::Builder::new().prefix("hd").tempdir_in("/tmp")
    } else {
        tempfile::Builder::new().prefix("hd").tempdir()
    }
    .unwrap()
}
