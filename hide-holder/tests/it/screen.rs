//! The screen a holder sends (PRD engine-bundle-2-pane-holder D-7, B4):
//! fragments drawn on an xterm of their own, whatever it drew before, show
//! what the program's own output shows on a fresh one (the expected screen
//! comes from xterm, not from the holder), and drawing them answers and
//! reports nothing.

use hide_holder_protocol::{ClientMessage, Size};

use crate::support::{Draw, Home, Script, Shown, draw, fragments_after};

/// Less than the holder's one second bound on synchronized output, with room
/// for the open and the program's write: a takeover sooner than this must
/// still find the screen held.
const HELD_WITHIN: std::time::Duration = std::time::Duration::from_millis(700);

/// What a receiver drew before: the alternate screen, a scroll region,
/// origin mode, line drawing characters, insert mode, colors, a hidden
/// cursor, reverse video and text, at a size of its own. Display state
/// only: the web shell's xterm is never put in an input mode.
const PRIOR: &[u8] = b"\x1b[?1049h\x1b[3;6r\x1b[?6h\x1b(0\x1b[4h\x1b[1;31;44mqqqq\
    \x1b[2;2Hgarbage\x1b[?25l\x1b[?5h\x1b[7m";
const PRIOR_SIZE: (u16, u16) = (40, 12);

/// A screen with styles, wide characters, a hyperlink, a moved cursor and
/// a cursor shape, which also turns on the input modes and asks questions
/// a fragment must carry neither of.
fn first_screen() -> Vec<u8> {
    [
        "\x1b[H\x1b[2J",
        "\x1b[1;32mgreen bold\x1b[0m plain \x1b[38;2;10;20;30mrgb\x1b[0m\r\n",
        "wide: \u{4e2d}\u{6587} done\r\n",
        "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ after\r\n",
        "\x1b[4:3mcurly\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7minverse\x1b[0m\r\n",
        "\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1004h\x1b[>1u\x1b[6n\x1b[c",
        "\x1b[5 q",
        "\x1b[10;5Hmarker-one",
    ]
    .concat()
    .into_bytes()
}

fn reference(steps: Vec<Draw>) -> (u16, u16, Vec<Draw>) {
    (80, 24, steps)
}

fn assert_same(candidate: &Shown, expected: &Shown, what: &str) {
    assert_eq!(candidate.lines, expected.lines, "{what}: the text differs");
    assert_eq!(
        candidate.cursor, expected.cursor,
        "{what}: the cursor differs"
    );
    assert_eq!(
        candidate.cursor_visible, expected.cursor_visible,
        "{what}: the cursor's visibility differs"
    );
    assert!(
        candidate.data.is_empty(),
        "{what}: drawing the screen answered {:?}",
        String::from_utf8_lossy(&candidate.data)
    );
    assert_eq!(
        candidate.mouse, "none",
        "{what}: drawing the screen turned a mouse mode on"
    );
}

#[test]
fn fragments_give_the_programs_screen_whatever_the_receiver_drew_before() {
    let home = Home::new();
    let script = Script::new(&home, vec![first_screen()]);
    let holder = script.start(&home);
    let mut connection = holder.open();
    let drawn = connection.wait_for_drawn("marker-one");
    let fragments = &connection.fragments[..drawn];

    assert!(
        fragments[0].full && fragments[0].seq == 0,
        "{:?}",
        fragments[0]
    );
    for pair in fragments.windows(2) {
        assert!(!pair[1].full, "a second full fragment without a redraw");
        assert_eq!(
            (pair[1].epoch, pair[1].seq),
            (pair[0].epoch, pair[0].seq + 1)
        );
    }
    let shown = draw(&[
        reference(vec![Draw::Bytes(first_screen())]),
        fragments_after(b"", (80, 24), fragments),
        fragments_after(PRIOR, PRIOR_SIZE, fragments),
    ]);
    assert_same(&shown[1], &shown[0], "on a fresh xterm");
    assert_same(&shown[2], &shown[0], "on an xterm that drew before");
    assert_eq!(
        shown[2].cursor_shape, shown[0].cursor_shape,
        "the program's cursor shape"
    );
    // The program turned the input modes on for itself, not for the receiver.
    assert_eq!(shown[0].mouse, "vt200");
}

#[test]
fn the_alternate_screen_and_back_and_a_redraw_after_lost_fragments_give_the_same_screen() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"\x1b[H\x1b[2Jprimary text\r\nsecond line marker-a".to_vec(),
            b"\x1b[?1049h\x1b[H\x1b[2J\x1b[3;3Halternate text\x1b[5;1Hmarker-b".to_vec(),
            b"\x1b[?1049l\x1b[?25lmarker-c".to_vec(),
        ],
    );
    let holder = script.start(&home);
    let mut connection = holder.open();
    let mut sent = Vec::new();
    for (step, marker) in ["marker-a", "marker-b", "marker-c"].into_iter().enumerate() {
        if step > 0 {
            script.go(step);
        }
        sent.extend_from_slice(&script.steps[step]);
        let drawn = connection.wait_for_drawn(marker);
        let fragments = connection.fragments[..drawn].to_vec();
        let shown = draw(&[
            reference(vec![Draw::Bytes(sent.clone())]),
            fragments_after(PRIOR, PRIOR_SIZE, &fragments),
        ]);
        assert_same(&shown[1], &shown[0], marker);
    }

    // A receiver that lost every fragment after the first asks for a redraw:
    // the full fragment alone puts it right, in an epoch of its own.
    let first = connection.fragments[0].clone();
    let answered = connection.redraw();
    let full = &connection.fragments[answered - 1];
    assert!(
        full.full && full.seq == 0 && full.epoch > first.epoch,
        "{full:?}"
    );
    let shown = draw(&[
        reference(vec![Draw::Bytes(sent.clone())]),
        fragments_after(PRIOR, PRIOR_SIZE, &[first, full.clone()]),
    ]);
    assert_same(&shown[1], &shown[0], "after lost fragments and a redraw");
}

#[test]
fn a_resize_and_a_takeover_during_synchronized_output_give_the_same_screen() {
    let home = Home::new();
    let script = Script::new(
        &home,
        vec![
            b"\x1b[H\x1b[2Jbefore the resize marker-r0".to_vec(),
            // A program redraws after a resize; this one inside synchronized
            // output it ends in a later step.
            b"\x1b[?2026h\x1b[H\x1b[2Jhalf drawn".to_vec(),
            b"\x1b[3;90Hfar right\x1b[?2026l\x1b[30;1Hmarker-r1".to_vec(),
        ],
    );
    let holder = script.start(&home);
    let mut first = holder.open();
    first.wait_for_drawn("marker-r0");
    first
        .send(ClientMessage::Resize(Size {
            rows: 30,
            cols: 100,
        }))
        .unwrap();
    let started = std::time::Instant::now();
    while !first.fragments.iter().any(|fragment| {
        fragment.size
            == Size {
                rows: 30,
                cols: 100,
            }
    }) {
        assert!(
            started.elapsed() < crate::support::HANG_LIMIT,
            "no fragment of the new size"
        );
        first.drain();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    script.go(1);
    // Taken over while the program holds its output: the new connection's
    // first screen is full and, once the program ends its synchronized
    // output, the same as the program's.
    crate::support::wait_until("the half-drawn screen to be parsed", || {
        holder.read(10).0.contains("half drawn")
    });
    let parsed = std::time::Instant::now();
    let mut second = holder.open();
    let released = parsed.elapsed();
    script.go(2);
    let drawn = second.wait_for_drawn("marker-r1");
    let fragments = second.fragments[..drawn].to_vec();
    assert!(fragments[0].full, "{:?}", fragments[0]);
    // Held while the program held its output: the first screen came once
    // it ended it, unless the holder's bound passed first.
    if released < HELD_WITHIN {
        assert!(
            crate::support::contains(&fragments[0].bytes, b"far right"),
            "a screen was drawn while the program held its output"
        );
    }
    let mut sent = vec![Draw::Bytes(script.steps[0].clone())];
    sent.push(Draw::Resize {
        cols: 100,
        rows: 30,
    });
    sent.push(Draw::Bytes(script.steps[1].clone()));
    sent.push(Draw::Bytes(script.steps[2].clone()));
    let shown = draw(&[
        reference(sent),
        fragments_after(PRIOR, PRIOR_SIZE, &fragments),
    ]);
    assert_same(&shown[1], &shown[0], "after a resize and a takeover");
}
