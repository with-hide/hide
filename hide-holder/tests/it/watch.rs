//! A watch follows a pane's facts (PRD engine-bundle-3-core-owns D-18,
//! D-21): its title, folder, prompts, bracketed paste and what its screen
//! rules read on its screen, each time any of them changed, beside the
//! connection that holds the screen and without taking it over; a newer
//! watch replaces an older one, which is told `taken_over`, and its first
//! facts are the pane's whole state. That an idle pane's watch sends nothing
//! and wakes nothing is `idle.rs`'s.

use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Capability, CloseReason, Closed, Facts, HolderMessage, PROTOCOL_VERSION, ScreenRead,
    ScreenState, WatchMessage, Watching, read_watch_holder, write_watch,
};
use hide_platform::ipc::LocalStream;

use crate::role::{CONSOLE_PIDS, END, GO, PRINT_FOR, QUIET};
#[cfg(unix)]
use crate::support::wait_until;
use crate::support::{
    HANG_LIMIT, Home, ROLE, Script, current_version, role_program, shell, source, start_ready,
};

/// A rule set that reads `busy` on the screen as working, as a node sends
/// one.
const BUSY: &str = r#"{"version":"1","min_engine_version":1,"rules":[{"id":"busy","state":"working","contains":["busy"]}]}"#;

/// A node's side of a watch, read on the test's own thread with a hang
/// guard.
struct Watch {
    stream: LocalStream,
    watching: Watching,
    facts: Vec<Facts>,
}

impl Watch {
    fn open(socket: &std::path::Path) -> Self {
        Self::accepting(socket, vec![Capability::Facts])
    }

    fn accepting(socket: &std::path::Path, accepts: Vec<Capability>) -> Self {
        let (stream, watching) =
            hide_holder_protocol::watch(socket, accepts, Instant::now() + HANG_LIMIT).unwrap();
        Self {
            stream,
            watching,
            facts: Vec::new(),
        }
    }

    fn rules(&mut self, manifest: Option<&str>) {
        write_watch(
            &mut self.stream,
            &WatchMessage::Rules(manifest.map(str::to_owned)),
        )
        .unwrap();
    }

    fn next(&mut self) -> HolderMessage {
        self.stream.set_deadline(Instant::now() + HANG_LIMIT);
        read_watch_holder(&mut self.stream, &self.watching.sends).unwrap_or_else(|error| {
            panic!(
                "the watch ended or never spoke ({error}); its facts were {:?}",
                self.facts
            )
        })
    }

    /// The pane's facts once `matches` holds for them: the last sent when
    /// it holds already, else the next for which it does.
    fn wait(&mut self, what: &str, matches: impl Fn(&Facts) -> bool) -> Facts {
        if let Some(last) = self.facts.last().filter(|facts| matches(facts)) {
            return last.clone();
        }
        loop {
            match self.next() {
                HolderMessage::Facts(facts) => {
                    self.facts.push(facts.clone());
                    if matches(&facts) {
                        return facts;
                    }
                }
                other => panic!("{what}: the watch was sent {other:?}"),
            }
        }
    }

    /// Reads until the holder closes the watch, and answers why.
    fn wait_closed(&mut self) -> Closed {
        loop {
            match self.next() {
                HolderMessage::Facts(facts) => self.facts.push(facts),
                HolderMessage::Closed(closed) => return closed,
                other => panic!("the watch was sent {other:?}"),
            }
        }
    }
}

fn read(state: ScreenState, rule: Option<&str>) -> ScreenRead {
    ScreenRead::Read {
        state,
        rule: rule.map(str::to_owned),
    }
}

#[test]
fn a_watch_follows_the_panes_facts_and_hands_them_to_a_newer_watch() {
    let home = Home::new();
    let folder = home.path().to_string_lossy().into_owned();
    // OSC 7 from a Unix shell, OSC 9;9 from Windows PowerShell's prompt.
    let report = if cfg!(windows) {
        format!("\x1b]9;9;{folder}\x1b\\")
    } else {
        format!("\x1b]7;file://{folder}\x07")
    };
    let script = Script::new(
        &home,
        vec![
            b"\x1b]2;first\x07".to_vec(),
            report.into_bytes(),
            b"\x1b]133;A\x07".to_vec(),
            b"busy\r\n".to_vec(),
            b"\x1b[?2004h".to_vec(),
        ],
    );
    let holder = script.start(&home);
    let mut connection = holder.open();

    let mut watch = Watch::open(&holder.socket);
    assert_eq!(watch.watching.version, PROTOCOL_VERSION);
    assert_eq!(
        watch.watching.rules_engine,
        hide_screen_rules::ENGINE_VERSION
    );
    assert_eq!(watch.watching.sends, vec![Capability::Facts]);
    // The first facts come whatever they are; the title the program set
    // as it started is in them or follows.
    watch.wait("the first facts", |_| true);
    let titled = watch.wait("the title", |facts| facts.title.as_deref() == Some("first"));
    assert_eq!(titled.screen, ScreenRead::NoRules);
    assert_eq!(titled.foreground, None, "the pane's own program holds it");

    script.go(1);
    watch.wait("the folder", |facts| {
        facts.cwd.as_deref() == Some(folder.as_str())
    });
    // ConPTY re-renders what the program writes, and whether it forwards a
    // prompt mark or a mode is not this test's claim: the unit tests read
    // both from the parser on every system.
    script.go(2);
    if cfg!(unix) {
        watch.wait("the prompt", |facts| facts.prompts == 1);
    }

    // Rules read the screen at once, before any output: nothing matches yet.
    watch.rules(Some(BUSY));
    watch.wait("the screen read by the rules", |facts| {
        facts.screen == read(ScreenState::Unknown, None)
    });
    script.go(3);
    let busy = watch.wait("the busy screen", |facts| {
        facts.screen == read(ScreenState::Working, Some("busy"))
    });
    assert_eq!(busy.cwd.as_deref(), Some(folder.as_str()));
    script.go(4);
    if cfg!(unix) {
        watch.wait("bracketed paste", |facts| facts.bracketed_paste);
    }

    // A newer watch is sent the whole state at once, rules included, and
    // the older one is told why it ends.
    let mut newer = Watch::open(&holder.socket);
    let whole = newer.wait("the newer watch's first facts", |_| true);
    assert_eq!(whole.title.as_deref(), Some("first"));
    assert_eq!(whole.cwd.as_deref(), Some(folder.as_str()));
    assert_eq!(whole.screen, read(ScreenState::Working, Some("busy")));
    if cfg!(unix) {
        assert_eq!((whole.prompts, whole.bracketed_paste), (1, true));
    }
    assert_eq!(watch.wait_closed().reason, CloseReason::TakenOver);

    // Rules taken away read no rules; the screen connection went on through
    // all of it.
    newer.rules(None);
    newer.wait("no rules", |facts| facts.screen == ScreenRead::NoRules);
    connection.drain();
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
}

/// The program in the foreground other than the pane's own is named with
/// its arguments once the terminal's output reads the facts again, and is
/// none once the shell holds the terminal again. Windows keeps no
/// foreground group for a terminal, so there it is always none (`asks.rs`).
#[cfg(unix)]
#[test]
fn a_watch_names_the_program_in_the_foreground_and_none_once_the_shell_holds_it() {
    use hide_holder_protocol::{Answer, Ask, ClientMessage, Foreground};

    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    let mut connection = holder.open();
    let mut watch = Watch::open(&holder.socket);
    watch.wait("the shell's facts", |facts| facts.foreground.is_none());

    connection
        .send(ClientMessage::Input(b"cat\r".to_vec()))
        .unwrap();
    wait_until("cat to hold the terminal", || {
        matches!(
            hide_holder_protocol::ask(
                &holder.socket,
                Ask::Processes,
                PROTOCOL_VERSION,
                Instant::now() + HANG_LIMIT,
            ),
            Ok(Answer::Processes { shell, foreground: Foreground::Known { group, .. } })
                if group != shell
        )
    });
    // cat writes nothing of its own; the terminal's echo of a line typed
    // into it is output, which reads the facts again.
    connection
        .send(ClientMessage::Input(b"typed\r".to_vec()))
        .unwrap();
    let held = watch.wait("cat in the foreground", |facts| facts.foreground.is_some());
    let program = held.foreground.unwrap();
    assert_eq!(program.name, "cat");
    assert_eq!(program.argv, vec!["cat".to_owned()]);
    assert_ne!(program.pid, holder.program.pid);

    connection
        .send(ClientMessage::Input(b"\x03".to_vec()))
        .unwrap();
    watch.wait("the shell in the foreground again", |facts| {
        facts.foreground.is_none()
    });
}

/// The line typed into a pane's shell that runs this test binary as the
/// role the shell's environment names.
fn role_line() -> Vec<u8> {
    let words: Vec<String> = role_program("")
        .iter()
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let program = if cfg!(windows) {
        format!("\"{}\"", words[0])
    } else {
        format!("'{}'", words[0])
    };
    format!("{program} {}\r", words[1..].join(" ")).into_bytes()
}

/// A pane whose shell starts `line_input_when_told` (`role.rs`) for each
/// [`role_line`] typed into it, told by the files `go`, `end` and `pids`.
struct LineInputPane {
    /// Kept for the pane's folder, which goes with it.
    _home: Home,
    holder: crate::support::Holder,
    go: std::path::PathBuf,
    end: std::path::PathBuf,
    quiet: std::path::PathBuf,
    /// Written on Windows only (`role.rs`).
    #[cfg_attr(unix, allow(dead_code))]
    pids: std::path::PathBuf,
}

impl LineInputPane {
    fn start() -> Self {
        Self::printing_first(None)
    }

    /// A pane whose program writes for `print` before it waits in its line.
    fn printing_first(print: Option<Duration>) -> Self {
        let home = Home::new();
        let folder = home.prepare(current_version(), &source());
        let go = home.path().join("go");
        let end = home.path().join("end");
        let quiet = home.path().join("quiet");
        let pids = home.path().join("console-pids");
        let role = std::ffi::OsString::from("line_input_when_told");
        let (go_value, end_value, quiet_value, pids_value) = (
            go.clone().into_os_string(),
            end.clone().into_os_string(),
            quiet.clone().into_os_string(),
            pids.clone().into_os_string(),
        );
        let millis = print.map(|print| std::ffi::OsString::from(print.as_millis().to_string()));
        let mut env = vec![
            (ROLE, &role),
            (GO, &go_value),
            (END, &end_value),
            (CONSOLE_PIDS, &pids_value),
        ];
        if let Some(millis) = &millis {
            env.extend([(PRINT_FOR, millis), (QUIET, &quiet_value)]);
        }
        let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &env);
        Self {
            _home: home,
            holder,
            go,
            end,
            quiet,
            pids,
        }
    }
}

/// A program reads keys only once it turns the terminal's line editing off,
/// which it does without writing anything (Pi 1.0.4 does it as its screen
/// starts): a watch whose node accepts the fact says so while the program
/// holds the foreground, on every system, and a watch that does not is sent
/// facts without it. A Windows pane's console keeps the mode in its console
/// host, and a helper attached to that console reads it, never the holder,
/// whom the console's close would end (with-hide/hide-private#545).
#[test]
fn a_watch_says_when_the_program_in_the_foreground_turns_line_editing_off() {
    use hide_holder_protocol::{ClientMessage, LineEditing};

    let pane = LineInputPane::start();
    let mut connection = pane.holder.open();
    let accepts = vec![
        Capability::Facts,
        Capability::ProgramPackage,
        Capability::LineEditing,
    ];
    let mut watch = Watch::accepting(&pane.holder.socket, accepts.clone());
    assert_eq!(watch.watching.sends, accepts);
    watch.wait("the shell's facts", |facts| facts.foreground.is_none());

    connection.send(ClientMessage::Input(role_line())).unwrap();
    let editing = watch.wait("the program in the terminal's line", |facts| {
        facts.foreground.is_some() && facts.line_editing == Some(LineEditing::Edits)
    });
    std::fs::write(&pane.go, b"").unwrap();
    let reading = watch.wait("the program reading keys", |facts| {
        facts.line_editing == Some(LineEditing::ReadsKeys)
    });
    assert_eq!(reading.foreground, editing.foreground);
    #[cfg(windows)]
    {
        let attached: Vec<u32> = std::fs::read_to_string(&pane.pids)
            .unwrap()
            .lines()
            .map(|pid| pid.parse().unwrap())
            .collect();
        let program = editing.foreground.as_ref().unwrap().pid;
        assert!(
            !attached.contains(&pane.holder.process.pid),
            "the holder attached to its pane's console: {attached:?}"
        );
        assert!(
            attached
                .iter()
                .any(|pid| ![pane.holder.program.pid, program].contains(pid)),
            "no helper was attached to the console as the program read keys: {attached:?}"
        );
    }

    let mut older = Watch::open(&pane.holder.socket);
    assert_eq!(older.watching.sends, vec![Capability::Facts]);
    let facts = older.wait("the facts", |_| true);
    assert_eq!(facts.line_editing, None);
    std::fs::write(&pane.end, b"").unwrap();
    older.wait("the shell again", |facts| facts.foreground.is_none());
}

/// A program that writes as it starts, past a start's settle, and then
/// waits in its line is read editing the line while it still writes: the
/// writer asks the terminal again only after a quiet spell, and a program
/// first read not known as it stopped would have had a start typed into its
/// line (Pi 1.0.4, nightly 41789009557). On Windows the facts themselves
/// ask the console once the program has held the foreground a recheck long.
#[test]
fn a_program_that_writes_before_it_waits_in_its_line_is_read_editing_it_while_it_writes() {
    use hide_holder_protocol::{ClientMessage, LineEditing};

    let pane = LineInputPane::printing_first(Some(Duration::from_secs(5)));
    let mut connection = pane.holder.open();
    let mut watch = Watch::accepting(
        &pane.holder.socket,
        vec![Capability::Facts, Capability::LineEditing],
    );
    watch.wait("the shell's facts", |facts| facts.foreground.is_none());
    connection.send(ClientMessage::Input(role_line())).unwrap();
    watch.wait("the program in the terminal's line", |facts| {
        facts.foreground.is_some() && facts.line_editing == Some(LineEditing::Edits)
    });
    assert!(
        !pane.quiet.exists(),
        "the program was read editing its line only once it stopped writing"
    );
    std::fs::write(&pane.end, b"").unwrap();
    watch.wait("the shell again", |facts| facts.foreground.is_none());
}

/// Programs that end in the console's line input, while the holder's helper
/// is attached to the console to read it, leave the holder holding its pane:
/// the console's events reach the helper, never the holder.
#[cfg(windows)]
#[test]
fn programs_that_end_while_their_console_is_read_leave_the_holder_holding_the_pane() {
    use hide_holder_protocol::{ClientMessage, LineEditing};

    let pane = LineInputPane::start();
    let mut connection = pane.holder.open();
    let mut watch = Watch::accepting(
        &pane.holder.socket,
        vec![Capability::Facts, Capability::LineEditing],
    );
    watch.wait("the shell's facts", |facts| facts.foreground.is_none());
    let mut seen = Vec::new();
    for round in 0..5 {
        let _ = std::fs::remove_file(&pane.end);
        connection.send(ClientMessage::Input(role_line())).unwrap();
        let editing = watch.wait("the program in the console's line input", |facts| {
            facts
                .foreground
                .as_ref()
                .is_some_and(|program| !seen.contains(&program.pid))
                && facts.line_editing == Some(LineEditing::Edits)
        });
        seen.push(editing.foreground.unwrap().pid);
        std::fs::write(&pane.end, b"").unwrap();
        watch.wait(&format!("the shell again after round {round}"), |facts| {
            facts.foreground.is_none()
        });
    }
    connection
        .send(ClientMessage::Input(crate::support::print_ok().to_vec()))
        .unwrap();
    connection.wait_for_text("hide-ok");
}
