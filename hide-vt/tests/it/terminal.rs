use std::sync::{Arc, Mutex};

use hide_vt::Terminal;

fn row_text(terminal: &Terminal, row: usize) -> String {
    let rows = terminal.screen_text_rows().unwrap();
    rows[row]
        .cells
        .iter()
        .flat_map(|cell| cell.graphemes.iter().copied())
        .filter_map(char::from_u32)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn written_bytes_are_read_back_as_the_screens_text() {
    let mut terminal = Terminal::new(20, 4, 1_000).unwrap();

    terminal.write(b"first line\r\n\x1b[1msecond\x1b[0m line");

    assert_eq!(row_text(&terminal, 0), "first line");
    assert_eq!(row_text(&terminal, 1), "second line");
    assert_eq!(row_text(&terminal, 2), "");
}

#[test]
fn a_cursor_position_query_is_answered_through_the_pty_callback() {
    let mut terminal = Terminal::new(20, 4, 1_000).unwrap();
    let answers = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = answers.clone();
    terminal
        .set_write_pty_callback(move |bytes| sink.lock().unwrap().extend_from_slice(bytes))
        .unwrap();

    // The cursor stands on row 2, column 4 (1-based) when the child asks.
    terminal.write(b"x\r\nabc\x1b[6n");

    assert_eq!(answers.lock().unwrap().as_slice(), b"\x1b[2;4R");
}
