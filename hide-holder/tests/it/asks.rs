//! Questions a node asks on a connection of their own (PRD
//! engine-bundle-2-pane-holder D-11, D-18, B9, B16): the pane's recent text
//! and its processes, answered without taking the screen connection over or
//! moving the screen's scroll position, and refused with a reason past their
//! bounds.

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
fn processes_name_the_shell_and_the_foreground_or_say_it_is_not_known() {
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
    #[cfg(windows)]
    assert_eq!(
        foreground,
        Foreground::Unknown(hide_holder_protocol::Unknown::NotOnThisSystem)
    );
    #[cfg(unix)]
    {
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

        let mut connection = holder.open();
        connection
            .send(ClientMessage::Input(b"sleep 30\r".to_vec()))
            .unwrap();
        wait_until(
            "the sleep to hold the terminal",
            || matches!(processes().1, Foreground::Known { group, .. } if group != holder.program.pid),
        );
        let Foreground::Known { members, .. } = processes().1 else {
            unreachable!()
        };
        assert!(
            members
                .iter()
                .any(|member| member.name.as_deref() == Some("sleep")),
            "{members:?}"
        );
        connection
            .send(ClientMessage::Input(b"\x03".to_vec()))
            .unwrap();
    }
}
