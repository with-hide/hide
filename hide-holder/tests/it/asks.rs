//! Questions a node asks on a connection of their own (PRD
//! engine-bundle-2-pane-holder D-11, D-18, B9, B16): the pane's recent text,
//! its processes and its scroll position, answered without taking the screen
//! connection over or moving the screen's scroll position, except a scroll
//! to, which moves the screen the holding connection is shown, and refused
//! with a reason past their bounds.

use std::time::Instant;

use hide_holder_protocol::{
    Answer, Ask, ClientMessage, CloseReason, Foreground, MAX_READ_LINES, Modifiers, OpenError,
    PROTOCOL_VERSION, Scroll, ask,
};

use crate::support::{
    HANG_LIMIT, Holder, Home, Script, current_version, draw, fragments_after, shell, source,
    start_ready, wait_until,
};

fn refusal(holder: &Holder, question: Ask) -> CloseReason {
    match ask(
        &holder.socket,
        question,
        PROTOCOL_VERSION,
        Instant::now() + HANG_LIMIT,
    ) {
        Err(OpenError::Refused(closed)) => closed.reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_read_leaves_the_screen_connection_and_its_scroll_position_as_they_were() {
    let home = Home::new();
    let lines: String = (0..300).map(|line| format!("line-{line:04}\r\n")).collect();
    let script = Script::new(&home, vec![format!("{lines}marker-end").into_bytes()]);
    let holder = script.start(&home);
    let mut connection = holder.open();
    connection.wait_for_drawn("marker-end");

    // The recent rows, joined, newest last; more rows exist than asked for.
    let (text, truncated) = holder.read(3);
    assert!(truncated);
    assert_eq!(text, "line-0298\nline-0299\nmarker-end");
    let (text, _) = holder.read(MAX_READ_LINES);
    assert!(
        text.contains("line-0000\n") && text.ends_with("marker-end"),
        "{text}"
    );
    for lines in [0, MAX_READ_LINES + 1] {
        assert_eq!(
            refusal(&holder, Ask::Read { lines }),
            CloseReason::AskInvalid
        );
    }

    // Scrolled back by the node, the screen stays where it was through a
    // read, and the connection that holds it goes on.
    let before = connection.fragments.len();
    connection
        .send(ClientMessage::Scroll(Scroll {
            lines: 10,
            cell: None,
            modifiers: Modifiers::default(),
        }))
        .unwrap();
    wait_until("the scrolled screen", || {
        connection.drain();
        connection.fragments.len() > before
    });
    let scrolled = connection.fragments.clone();
    let _ = holder.read(MAX_READ_LINES);
    let _ = ask(
        &holder.socket,
        Ask::Processes,
        PROTOCOL_VERSION,
        Instant::now() + HANG_LIMIT,
    )
    .unwrap();
    let answered = connection.redraw();
    let full = connection.fragments[answered - 1].clone();
    let shown = draw(&[
        fragments_after(b"", (80, 24), &scrolled),
        fragments_after(b"", (80, 24), &[full]),
    ]);
    assert_eq!(shown[1].lines, shown[0].lines, "the read moved the screen");
    assert!(
        !shown[0]
            .lines
            .iter()
            .any(|line| line.contains("marker-end")),
        "the screen never scrolled back: {:?}",
        shown[0].lines
    );
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
    assert_eq!(
        holder.open().welcome.generation,
        2,
        "an ask took the pane over"
    );
}

#[test]
fn processes_name_the_shell_and_the_program_that_holds_the_terminal() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    let processes = || match ask(
        &holder.socket,
        Ask::Processes,
        PROTOCOL_VERSION,
        Instant::now() + HANG_LIMIT,
    ) {
        Ok(Answer::Processes { shell, foreground }) => (shell, foreground),
        other => panic!("{other:?}"),
    };
    let (shell_pid, foreground) = processes();
    assert_eq!(shell_pid, holder.program.pid);
    // The idle shell holds the terminal: on Unix its own group, on Windows
    // the shell standing in for the console's foreground, which it runs no
    // program under.
    let Foreground::Known { group, members } = foreground else {
        panic!("{foreground:?}");
    };
    assert_eq!(group, holder.program.pid);
    let shell = members
        .iter()
        .find(|member| member.pid == holder.program.pid)
        .unwrap_or_else(|| panic!("{members:?}"));
    assert!(
        shell.name.as_deref().is_some_and(|name| !name.is_empty()),
        "{shell:?}"
    );

    // A program the shell runs holds the terminal, named as the system
    // names it.
    let (line, program): (&[u8], &str) = if cfg!(windows) {
        (b"ping -n 30 127.0.0.1\r", "ping.exe")
    } else {
        (b"sleep 30\r", "sleep")
    };
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(line.to_vec()))
        .unwrap();
    // Until the shell's child runs the program it is a copy of the shell,
    // which may take the terminal first: the program holds it once a member
    // has its name.
    let names_the_program = |members: &[hide_holder_protocol::Member]| {
        members.iter().any(|member| {
            member
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(program))
        })
    };
    wait_until("the program to hold the terminal", || {
        matches!(processes().1, Foreground::Known { group, members }
            if group != holder.program.pid && names_the_program(&members))
    });
    let Foreground::Known { members, .. } = processes().1 else {
        unreachable!()
    };
    assert!(
        members
            .iter()
            .all(|member| member.pid != holder.program.pid)
            && members.iter().any(|member| member
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(program))),
        "{members:?}"
    );
    connection
        .send(ClientMessage::Input(b"\x03".to_vec()))
        .unwrap();
}

/// A styled read (Herdr's `ansi` format, of the rows on screen and of the
/// last rows) keeps the color the program wrote as an escape sequence, and
/// the plain read of the same rows does not; a styled read past its rows is
/// refused as a plain one is.
#[test]
fn a_styled_read_keeps_the_color_the_program_wrote_and_a_plain_read_does_not() {
    let home = Home::new();
    let script = Script::new(&home, vec![b"\x1b[31mred-word\x1b[0m plain\r\n".to_vec()]);
    let holder = script.start(&home);
    wait_until("the colored line", || {
        holder.read(5).0.contains("red-word plain")
    });
    let (plain, _) = holder.read(5);
    assert!(!plain.contains('\x1b'), "{plain:?}");
    for question in [Ask::Rows { lines: 5 }, Ask::Screen] {
        let text = match ask(
            &holder.socket,
            question.clone(),
            PROTOCOL_VERSION,
            Instant::now() + HANG_LIMIT,
        ) {
            Ok(Answer::Text { text, .. }) => text,
            other => panic!("{question:?} was answered {other:?}"),
        };
        let word = text
            .find("red-word")
            .unwrap_or_else(|| panic!("{question:?} read {text:?}"));
        let before = &text[..word];
        assert!(
            before.contains("\x1b[31m") || before.contains("\x1b[38;5;1m"),
            "{question:?} lost the color: {text:?}"
        );
    }
    for lines in [0, MAX_READ_LINES + 1] {
        assert_eq!(
            refusal(&holder, Ask::Rows { lines }),
            CloseReason::AskInvalid
        );
    }
}

fn scroll(holder: &Holder, question: Ask) -> Answer {
    ask(
        &holder.socket,
        question,
        PROTOCOL_VERSION,
        Instant::now() + HANG_LIMIT,
    )
    .unwrap()
}

#[test]
fn a_scroll_ask_says_where_the_screen_is_and_a_scroll_to_moves_the_screen_shown() {
    let home = Home::new();
    let lines: String = (0..300).map(|line| format!("line-{line:04}\r\n")).collect();
    let script = Script::new(&home, vec![format!("{lines}marker-end").into_bytes()]);
    let holder = script.start(&home);
    let mut connection = holder.open();
    connection.wait_for_drawn("marker-end");

    // At the bottom, with the 300 rows and the script's own above it.
    let Answer::Scroll {
        offset_from_bottom: 0,
        max_offset_from_bottom: max,
    } = scroll(&holder, Ask::Scroll)
    else {
        panic!("the screen is not at the bottom");
    };
    assert!(max >= 277, "{max}");
    let at = |offset_from_bottom| Answer::Scroll {
        offset_from_bottom,
        max_offset_from_bottom: max,
    };

    let before = connection.fragments.len();
    assert_eq!(
        scroll(
            &holder,
            Ask::ScrollTo {
                offset_from_bottom: 10
            }
        ),
        at(10)
    );
    wait_until("the scrolled screen", || {
        connection.drain();
        connection.fragments.len() > before
    });
    let shown = draw(&[fragments_after(b"", (80, 24), &connection.fragments)]);
    let lines = &shown[0].lines;
    assert!(
        lines.first().is_some_and(|line| line.contains("line-0267")),
        "{lines:?}"
    );
    assert!(
        lines.last().is_some_and(|line| line.contains("line-0290")),
        "{lines:?}"
    );
    assert_eq!(scroll(&holder, Ask::Scroll), at(10));
    // Past the top is the top.
    assert_eq!(
        scroll(
            &holder,
            Ask::ScrollTo {
                offset_from_bottom: u64::MAX
            }
        ),
        at(max)
    );
    assert!(connection.closed.is_none(), "{:?}", connection.closed);
    assert_eq!(
        holder.open().welcome.generation,
        2,
        "an ask took the pane over"
    );
}
