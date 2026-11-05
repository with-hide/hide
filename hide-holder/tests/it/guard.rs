//! A guarded submission (`hide_holder_protocol::Guard`,
//! with-hide/hide-private#545): the holder writes the submit only once the
//! typed text shows on the screen, judged on the text itself, types it
//! again while it does not, and writes no submit when it never shows. The
//! stand-in drops its first keys while a spinner redraws, as a starting
//! agent did on a Windows runner; these run on every system.

use hide_holder_protocol::{
    Capability, ClientMessage, Guard, Key, NoticeReason, Submit, SubmitPart,
};

use crate::support::{Home, start_role, wait_until};

const TEXT: &str = "Reply with exactly JOURNEY-3EC1681D";

fn guarded() -> ClientMessage {
    ClientMessage::Submit(Submit {
        text: vec![SubmitPart::Paste(TEXT.as_bytes().to_vec())],
        gap: std::time::Duration::from_millis(300),
        enter: SubmitPart::Keys(vec![Key::Enter]),
        guard: Guard {
            shown: vec!["JOURNEY-3EC1681D".to_owned(), "[Pasted ~".to_owned()],
            at_rest: false,
        },
    })
}

fn read(folder: &std::path::Path, file: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(folder.join(file)).unwrap_or_default()).into_owned()
}

/// The stand-in drops the first typing while its spinner redraws: the
/// redraw is not taken for the text, the text is typed again once the first
/// typing's bound passed, and the program then reads it once, whole, with
/// its Enter after it.
#[test]
fn text_a_starting_program_dropped_is_typed_again_and_submitted_once_it_shows() {
    let home = Home::new();
    let folder = home.path().join("composer");
    std::fs::create_dir_all(&folder).unwrap();
    let holder = start_role(&home, "composer", &folder);
    let mut connection = holder.open_accepting(vec![
        Capability::Screen,
        Capability::Keys,
        Capability::Notices,
    ]);
    connection.wait_for_text("composer starting");
    connection.send(guarded()).unwrap();
    wait_until("the stand-in to drop the first typing", || {
        read(&folder, "discarded").contains("JOURNEY-3EC1681D")
    });
    std::fs::write(folder.join("go"), b"").unwrap();
    connection.wait_for_text("SUBMITTED");
    let heard = read(&folder, "heard");
    assert_eq!(
        heard.matches("JOURNEY-3EC1681D").count(),
        1,
        "the text was read once after the drop: {heard:?}"
    );
    assert!(
        heard
            .trim_end_matches(['\r', '\n'])
            .ends_with("JOURNEY-3EC1681D\x1b[201~")
            || heard
                .trim_end_matches(['\r', '\n'])
                .ends_with("JOURNEY-3EC1681D"),
        "the Enter came after the text: {heard:?}"
    );
    assert!(
        !read(&folder, "discarded").contains('\r'),
        "no Enter followed the dropped text"
    );
    holder.wait_logged("holder.submitted", 1, &mut connection);
    let line = holder
        .log_text()
        .lines()
        .find(|line| line.contains("\"event\":\"holder.submitted\""))
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .unwrap();
    assert_eq!(line["typings"], 2, "{line}");
    assert_eq!(line["written"], true, "{line}");
}

/// Text that never shows, however often it is typed, gets no Enter, and the
/// node is told: an Enter on a screen that did not take the text would
/// answer whatever that screen asks (a picker, a dialog).
#[test]
fn text_that_never_shows_gets_no_submit_and_a_notice() {
    let home = Home::new();
    let folder = home.path().join("composer");
    std::fs::create_dir_all(&folder).unwrap();
    let holder = start_role(&home, "composer", &folder);
    let mut connection = holder.open_accepting(vec![
        Capability::Screen,
        Capability::Keys,
        Capability::Notices,
    ]);
    connection.wait_for_text("composer starting");
    connection.send(guarded()).unwrap();
    let notice = connection.wait_notice(|notice| notice.reason == NoticeReason::SubmitNotShown);
    assert_eq!(
        notice.count,
        hide_holder_protocol::MAX_SUBMIT_TYPINGS as u64
    );
    let dropped = read(&folder, "discarded");
    assert_eq!(
        dropped.matches("JOURNEY-3EC1681D").count(),
        hide_holder_protocol::MAX_SUBMIT_TYPINGS,
        "{dropped:?}"
    );
    assert!(!dropped.contains('\r'), "no Enter was written: {dropped:?}");
    // Input sent after the refused submission still reaches the program.
    std::fs::write(folder.join("go"), b"").unwrap();
    connection
        .send(ClientMessage::Input(b"after".to_vec()))
        .unwrap();
    connection.wait_for_text("after");
}
