//! Input and answers through a real PTY (PRD engine-bundle-2-pane-holder
//! D-8, D-9, B3, B5, B6): bytes go to the program as they are, a paste, a
//! pointer event and a wheel are encoded for the program's modes at that
//! moment, and the program's questions are answered once each, in order,
//! whether or not a node is connected. The program records everything it is
//! sent; its terminal is raw, so nothing is echoed or changed on the way.

#[cfg(unix)]
use hide_holder_protocol::{
    Appearance, ClientMessage, Modifiers, Mouse, MouseAction, MouseButton, Rgb, Scroll, Theme,
};

use crate::support::{Home, Script, wait_until};

#[cfg(unix)]
fn press(action: MouseAction) -> ClientMessage {
    ClientMessage::Mouse(Mouse {
        action,
        button: Some(MouseButton::Left),
        col: 3,
        row: 4,
        modifiers: Modifiers::default(),
    })
}

#[cfg(unix)]
fn wheel(lines: i16) -> ClientMessage {
    ClientMessage::Scroll(Scroll {
        lines,
        cell: Some((1, 2)),
        modifiers: Modifiers::default(),
    })
}

/// On Windows ConPTY sits between the holder and the program and turns
/// input into console events of its own; what reaches the program is
/// ConPTY's, so the bytes are checked where the PTY carries them as they
/// are.
#[cfg(unix)]
#[test]
fn input_reaches_the_program_as_sent_or_encoded_for_its_modes() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"\x1b[H\x1b[2J\x1b[?2004hmode-paste".to_vec(),
            b"\x1b[?2004lmode-plain".to_vec(),
            b"\x1b[?1000h\x1b[?1006hmode-mouse".to_vec(),
            b"\x1b[?1000l\x1b[?1006l\x1b[?1049h\x1b[?1007hmode-alternate".to_vec(),
        ],
    );
    let holder = script.start(&home);
    let mut connection = holder.open();
    let mut expected = Vec::new();

    connection.wait_for_drawn("mode-paste");
    connection
        .send(ClientMessage::Input(b"typed\x1b[A".to_vec()))
        .unwrap();
    connection
        .send(ClientMessage::Paste(b"a\nb\x1bc".to_vec()))
        .unwrap();
    expected.extend_from_slice(b"typed\x1b[A\x1b[200~a\nb c\x1b[201~");
    script.wait_heard(&expected);

    script.go(1);
    connection.wait_for_drawn("mode-plain");
    connection
        .send(ClientMessage::Paste(b"a\nb\x1bc".to_vec()))
        .unwrap();
    // The program tracks no mouse: the event is dropped, and the paste
    // after it shows nothing came in between.
    connection.send(press(MouseAction::Press)).unwrap();
    connection
        .send(ClientMessage::Input(b"|".to_vec()))
        .unwrap();
    expected.extend_from_slice(b"a\rb c|");
    script.wait_heard(&expected);

    script.go(2);
    connection.wait_for_drawn("mode-mouse");
    connection.send(press(MouseAction::Press)).unwrap();
    connection.send(press(MouseAction::Release)).unwrap();
    connection.send(wheel(3)).unwrap();
    expected.extend_from_slice(b"\x1b[<0;4;5M\x1b[<0;4;5m\x1b[<64;2;3M");
    script.wait_heard(&expected);

    script.go(3);
    connection.wait_for_drawn("mode-alternate");
    connection.send(wheel(-1)).unwrap();
    connection.send(wheel(1)).unwrap();
    expected.extend_from_slice(b"\x1b[B\x1b[A");
    script.wait_heard(&expected);
}

#[cfg(unix)]
fn theme() -> ClientMessage {
    ClientMessage::Theme(Theme {
        foreground: Some(Rgb {
            r: 0x10,
            g: 0x20,
            b: 0x30,
        }),
        background: None,
        cursor: None,
        palette: Vec::new(),
        appearance: Appearance::Dark,
    })
}

/// No node is connected while the program asks: a question split across
/// two writes, then several at once, are each answered once, in the order
/// asked, by libghostty or the holder as `answers.rs` divides them, and what
/// the path does not carry is not advertised. Started with no theme, the
/// holder answers a color question with libghostty's own default rather
/// than leaving the program to wait for an answer.
#[cfg(unix)]
#[test]
fn questions_are_answered_once_each_in_order_with_no_node_connected() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"\x1b[H\x1b[2Jfirst-half\x1b[".to_vec(),
            b"6n\x1b[c\x1b[18t\x1b[14t\x1b]10;?\x07\x1bP+q544E;4D73\x1b\\asked".to_vec(),
            // After the node's theme, the program's own color, then reset.
            b"\x1b]10;?\x07\x1b]10;#abcdef\x07\x1b]10;?\x07\x1b]110\x07\x1b]10;?\x07".to_vec(),
        ],
    );
    let holder = script.start(&home);
    wait_until("the first half of the question to be parsed", || {
        holder.read(5).0.contains("first-half")
    });
    script.go(1);
    wait_until("the questions to be parsed", || {
        holder.read(5).0.contains("asked")
    });
    script.wait_heard(
        b"\x1b[1;11R\x1b[?62;22c\x1b[8;24;80t\x1b]10;rgb:ffff/ffff/ffff\x07\
          \x1bP1+r544E=787465726D2D323536636F6C6F72\x1b\\",
    );

    let mut connection = holder.open();
    connection.send(theme()).unwrap();
    connection.send(ClientMessage::Release).unwrap();
    connection.wait_closed();
    script.go(2);
    script.wait_heard(
        b"\x1b[1;11R\x1b[?62;22c\x1b[8;24;80t\x1b]10;rgb:ffff/ffff/ffff\x07\
          \x1bP1+r544E=787465726D2D323536636F6C6F72\x1b\\\
          \x1b]10;rgb:1010/2020/3030\x07\x1b]10;rgb:abab/cdcd/efef\x07\
          \x1b]10;rgb:1010/2020/3030\x07",
    );
}

/// The node's theme comes with the holder's start, so the program's first
/// color question, asked before any node connects, is answered with it; a
/// color the theme leaves out is answered with libghostty's default.
#[cfg(unix)]
#[test]
fn a_color_question_asked_before_any_node_connects_is_answered_with_the_starting_theme() {
    let home = Home::new();
    let ClientMessage::Theme(theme) = theme() else {
        unreachable!()
    };
    let script = Script::new(&home, vec![b"\x1b]10;?\x07\x1b]11;?\x07".to_vec()]).with_theme(theme);
    let _holder = script.start(&home);
    script.wait_heard(b"\x1b]10;rgb:1010/2020/3030\x07\x1b]11;rgb:0000/0000/0000\x07");
}

/// On Windows the program's questions go to ConPTY first, which answers
/// the ones it keeps; whoever answers, each is answered once.
#[cfg(windows)]
#[test]
fn questions_are_answered_once_each_with_no_node_connected() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"\x1b[H\x1b[2J\x1b[6n\x1b[casked".to_vec(),
            b"\x1b[6nagain".to_vec(),
        ],
    );
    let holder = script.start(&home);
    wait_until("the questions to be parsed", || {
        holder.read(5).0.contains("asked")
    });
    wait_until("the answers", || {
        let heard = String::from_utf8_lossy(&script.heard()).into_owned();
        heard.contains('R') && heard.contains('c')
    });
    // One more question: its answer comes after anything still owed for
    // the first ones, since the program's input keeps its order.
    script.go(1);
    wait_until("the last answer", || {
        String::from_utf8_lossy(&script.heard())
            .matches('R')
            .count()
            >= 2
    });
    let heard = String::from_utf8_lossy(&script.heard()).into_owned();
    assert_eq!(heard.matches('R').count(), 2, "{heard:?}");
    assert_eq!(heard.matches('c').count(), 1, "{heard:?}");
}

/// On Windows a color question passes through ConPTY to the holder, which
/// answers it from the theme; ConPTY must not answer it too, or the second
/// answer reaches a program no longer waiting for one and is read as typed
/// keys (Antigravity took its closing BEL for Ctrl+G and opened its prompt
/// editor on the holder restart lane).
#[cfg(windows)]
#[test]
fn a_color_question_is_answered_once_through_conpty() {
    let home = Home::new();
    let script = Script::new(&home, vec![b"\x1b]11;?\x07asked".to_vec()]);
    let holder = script.start(&home);
    wait_until("the question to be parsed", || {
        holder.read(5).0.contains("asked")
    });
    wait_until("the answer", || {
        String::from_utf8_lossy(&script.heard()).contains("11;rgb:")
    });
    // Anything owed after the first answer has had its time.
    std::thread::park_timeout(std::time::Duration::from_secs(1));
    let heard = String::from_utf8_lossy(&script.heard()).into_owned();
    eprintln!("a color question heard back {heard:?}");
    assert_eq!(heard.matches("11;rgb:").count(), 1, "{heard:?}");
}
