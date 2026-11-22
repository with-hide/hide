//! `hide phone`: the phones paired with this computer (PRD device-connect-iroh
//! D-70, B61), for a computer with no window to open Settings > Mobile in.
//!
//! `pair` draws the pairing QR in this terminal and waits until a phone
//! scans it. It draws only on a terminal a person reads, never prints the
//! link the QR holds, and asks for a bigger window when the QR would not fit.
//! `list` and `remove` read and change the paired phones; a program in a Hide
//! pane may run them, and only a terminal outside the panes pairs (D-53).
//! This machine's own daemon answers each ask (`mobile::command`).

use std::io::{IsTerminal, Write as _};
use std::time::Duration;

use serde_json::{Value, json};

use crate::env::Env;
use crate::mobile::command::next_action;

pub const USAGE: &str =
    "hide phone pair\nhide phone list [--json]\nhide phone remove <phone> [--json]";

/// How often `pair` asks whether a phone has scanned the code.
const POLL: Duration = Duration::from_secs(1);
/// Light modules around the code. The standard asks for four; two keep the
/// picture inside a smaller window, and a phone's camera reads it, since the
/// light comes from the picture itself and not from the terminal's colours.
const QUIET: i32 = 2;
/// Lines `pair` prints around the code.
const AROUND: usize = 3;

#[derive(Debug, Eq, PartialEq)]
pub enum Verb {
    Pair,
    List,
    /// A phone by its id or its name.
    Remove(String),
}

#[derive(Debug, Eq, PartialEq)]
pub struct PhoneCli {
    pub verb: Verb,
    pub json: bool,
}

pub fn parse<'a>(args: impl Iterator<Item = &'a String>) -> Result<PhoneCli, String> {
    let mut args = args;
    let verb = args.next().map(String::as_str);
    let mut json = false;
    let mut phone = None;
    for arg in args {
        match arg.as_str() {
            // `pair` answers a person at a terminal only.
            "--json" if !json && verb != Some("pair") => json = true,
            value
                if verb == Some("remove")
                    && phone.is_none()
                    && !value.is_empty()
                    && value.len() <= 256
                    && !value.starts_with('-')
                    && !value.chars().any(char::is_control) =>
            {
                phone = Some(value.to_owned());
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    let verb = match (verb, phone) {
        (Some("pair"), None) => Verb::Pair,
        (Some("list"), None) => Verb::List,
        (Some("remove"), Some(phone)) => Verb::Remove(phone),
        _ => return Err(USAGE.to_owned()),
    };
    Ok(PhoneCli { verb, json })
}

/// A refusal, with what the person does about it.
struct Refused {
    code: String,
    message: String,
}

impl Refused {
    fn code(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: next_action(code).to_owned(),
        }
    }
}

pub fn run(env: &Env, cli: &PhoneCli) -> Result<(), String> {
    match answer(env, cli) {
        Ok((value, lines)) => {
            if cli.json {
                println!("{}", json!({"ok": true, "value": value}));
            } else {
                print!("{lines}");
            }
            Ok(())
        }
        Err(Refused { code, message }) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": code, "message": message}})
            );
            Err(code)
        }
    }
}

/// The answer and its lines for a person.
fn answer(env: &Env, cli: &PhoneCli) -> Result<(Value, String), Refused> {
    match &cli.verb {
        Verb::Pair => pair(env),
        Verb::List => {
            let value = ask(env, json!({"verb": "list"}))?;
            let lines = list_lines(&value, crate::mobile::now_ms());
            Ok((value, lines))
        }
        Verb::Remove(phone) => {
            let value = ask(env, json!({"verb": "remove", "phone": phone}))?;
            let name = value["name"].as_str().unwrap_or(phone);
            let lines = format!("Removed {name}: it can no longer connect to this computer.\n");
            Ok((value, lines))
        }
    }
}

/// One ask of this machine's daemon.
fn ask(env: &Env, ask: Value) -> Result<Value, Refused> {
    let mut answer = crate::workspace_cli::phone_ask(env, ask).map_err(|code| Refused {
        message: crate::cli::request_next_action(&code).to_owned(),
        code,
    })?;
    if answer["ok"] == true {
        if let Some(fields) = answer.as_object_mut() {
            fields.remove("ok");
        }
        return Ok(answer);
    }
    let code = answer["reason"].as_str().unwrap_or("phone_unavailable");
    Err(Refused {
        message: answer["next_action"]
            .as_str()
            .unwrap_or_else(|| next_action(code))
            .to_owned(),
        code: code.to_owned(),
    })
}

/// Shows the pairing QR and waits for the phone that scans it, at most as
/// long as the code lives.
#[allow(clippy::disallowed_methods)] // polls a code the daemon spends or ends on its own, bounded by the code's life
fn pair(env: &Env) -> Result<(Value, String), Refused> {
    // The daemon refuses a pane again by what the kernel says of the caller.
    if env.pane_id.is_some() || env.access.is_some() {
        return Err(Refused::code("phone_pair_from_pane"));
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Refused::code("phone_pair_needs_terminal"));
    }
    let window = hide_platform::stdio::stdout_window()
        .and_then(|window| hide_platform::stdio::stdout_escapes().map(|()| window))
        .map_err(|_| Refused::code("phone_pair_needs_terminal"))?;
    let shown = ask(env, json!({"verb": "pair"}))?;
    let (Some(link), Some(code_id)) = (shown["qr"].as_str(), shown["code_id"].as_str()) else {
        return Err(Refused::code("phone_unavailable"));
    };
    let end = || {
        let _ = ask(env, json!({"verb": "pair_end", "code_id": code_id}));
    };
    // The relay keeps a link within `hide_relay_protocol::qr::MAX_QR_URL`,
    // which a QR always holds.
    let Some(picture) = qr_lines(link) else {
        end();
        return Err(Refused::code("phone_unavailable"));
    };
    let (columns, rows) = (picture.width, picture.lines.len() + AROUND);
    if usize::from(window.columns) < columns || usize::from(window.rows) < rows {
        end();
        return Err(Refused {
            code: "phone_window_small".to_owned(),
            message: format!(
                "The QR needs a window at least {columns} columns wide and {rows} rows tall: make this window bigger and run hide phone pair again"
            ),
        });
    }
    let name = shown["name"].as_str().unwrap_or("this computer");
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "Scan this with your phone's camera, or with Scan QR code in the Hide app, to connect it to {name}."
    );
    for line in &picture.lines {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(
        out,
        "The code works once, for 5 minutes. Waiting for the phone..."
    );
    let _ = out.flush();
    drop(out);
    let deadline = std::time::Instant::now()
        + Duration::from_millis(crate::mobile::phones::CODE_TTL_MS)
        + POLL;
    loop {
        if std::time::Instant::now() >= deadline {
            end();
            return Err(Refused::code("phone_pair_ended"));
        }
        std::thread::sleep(POLL);
        let status = ask(env, json!({"verb": "pair_status", "code_id": code_id}))?;
        match status["state"].as_str() {
            Some("waiting") => {}
            Some("paired") => {
                let phone = status["name"].as_str().unwrap_or("the phone").to_owned();
                let lines = format!("Connected {phone}. hide phone list shows it.\n");
                return Ok((json!({"name": phone}), lines));
            }
            _ => return Err(Refused::code("phone_pair_ended")),
        }
    }
}

/// A QR drawn in a terminal: its lines and how many columns each takes.
struct Picture {
    lines: Vec<String>,
    width: usize,
}

/// The QR of `link` as terminal lines, two module rows to a line: each cell
/// is the upper half block, its foreground the upper module and its
/// background the lower one, in black and white of the picture's own, so it
/// scans the same on a dark terminal as on a light one.
fn qr_lines(link: &str) -> Option<Picture> {
    let qr = qrcodegen::QrCode::encode_text(link, qrcodegen::QrCodeEcc::Low).ok()?;
    let side = qr.size() + 2 * QUIET;
    // Out of the code's own bounds a module is light, which is the quiet zone.
    let dark = |x: i32, y: i32| qr.get_module(x - QUIET, y - QUIET);
    let mut lines = Vec::new();
    for y in (0..side).step_by(2) {
        let mut line = String::new();
        let mut last = None;
        for x in 0..side {
            let cell = (dark(x, y), dark(x, y + 1));
            if last != Some(cell) {
                let (upper, lower) = cell;
                let foreground = if upper { 30 } else { 97 };
                let background = if lower { 40 } else { 107 };
                line.push_str(&format!("\x1b[{foreground};{background}m"));
                last = Some(cell);
            }
            line.push('▀');
        }
        line.push_str("\x1b[0m");
        lines.push(line);
    }
    Some(Picture {
        lines,
        width: usize::try_from(side).ok()?,
    })
}

/// The paired phones, one line each, for a person.
fn list_lines(value: &Value, now_ms: u64) -> String {
    let mut lines = String::new();
    if value["enabled"] != true {
        lines.push_str(
            "Mobile is off on this computer, so no phone can connect: hide phone pair turns it on.\n",
        );
    }
    let phones = value["phones"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if phones.is_empty() {
        lines.push_str("No phone is connected to this computer. hide phone pair adds one.\n");
        return lines;
    }
    for phone in phones {
        let seen = if phone["connected"] == true {
            "connected now".to_owned()
        } else {
            let last = phone["last_seen_ms"].as_u64().unwrap_or(now_ms);
            format!("last seen {}", ago(now_ms.saturating_sub(last)))
        };
        let notifications = match phone["notifications"].as_str() {
            Some("on") => "notifications on",
            Some("off") => "notifications off",
            _ => "notifications not asked",
        };
        lines.push_str(&format!(
            "{}  {seen}  {notifications}  id {}\n",
            phone["name"].as_str().unwrap_or_default(),
            phone["id"].as_str().unwrap_or_default(),
        ));
    }
    lines
}

fn ago(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(words: &[&str]) -> Result<PhoneCli, String> {
        let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        parse(words.iter())
    }

    #[test]
    fn reads_each_verb_and_refuses_what_it_does_not_take() {
        assert_eq!(
            parsed(&["remove", "iPhone 2", "--json"]),
            Ok(PhoneCli {
                verb: Verb::Remove("iPhone 2".into()),
                json: true
            })
        );
        assert_eq!(
            parsed(&["list"]),
            Ok(PhoneCli {
                verb: Verb::List,
                json: false
            })
        );
        // The link is never an answer line, so pairing takes no --json.
        assert!(parsed(&["pair", "--json"]).is_err());
        assert!(parsed(&["remove"]).is_err());
        assert!(parsed(&["remove", "-x"]).is_err());
        assert!(parsed(&["remove", "a", "b"]).is_err());
        assert!(parsed(&["list", "extra"]).is_err());
    }

    /// Reads the drawing back into modules: each cell's foreground is the
    /// upper module and its background the lower one.
    fn modules(picture: &Picture) -> Vec<Vec<bool>> {
        let mut rows = Vec::new();
        for line in &picture.lines {
            let (mut upper, mut lower) = (Vec::new(), Vec::new());
            let mut colours = (false, false);
            let mut rest = line.as_str();
            while let Some(cell) = rest.chars().next() {
                if let Some(escape) = rest.strip_prefix("\x1b[") {
                    let end = escape.find('m').unwrap();
                    let codes: Vec<&str> = escape[..end].split(';').collect();
                    if codes != ["0"] {
                        colours = (codes[0] == "30", codes[1] == "40");
                    }
                    rest = &escape[end + 1..];
                    continue;
                }
                assert_eq!(cell, '▀');
                upper.push(colours.0);
                lower.push(colours.1);
                rest = &rest[cell.len_utf8()..];
            }
            rows.push(upper);
            rows.push(lower);
        }
        rows
    }

    #[test]
    fn the_drawing_holds_every_module_of_the_code_inside_a_light_border() {
        let link = format!("https://app.withhide.dev/#pair={}", "A".repeat(300));
        let picture = qr_lines(&link).unwrap();
        let qr = qrcodegen::QrCode::encode_text(&link, qrcodegen::QrCodeEcc::Low).unwrap();
        let side = usize::try_from(qr.size() + 2 * QUIET).unwrap();
        assert_eq!(picture.width, side);
        assert_eq!(picture.lines.len(), side.div_ceil(2));
        let drawn = modules(&picture);
        for (y, row) in drawn.iter().take(side).enumerate() {
            assert_eq!(row.len(), side);
            for (x, &dark) in row.iter().enumerate() {
                let (x, y) = (x as i32 - QUIET, y as i32 - QUIET);
                assert_eq!(dark, qr.get_module(x, y), "module {x},{y}");
            }
        }
    }

    #[test]
    fn a_list_says_where_each_phone_stands() {
        let value = json!({
            "enabled": true,
            "phones": [
                {"id": "a1", "name": "iPhone", "connected": true, "last_seen_ms": 0, "notifications": "on"},
                {"id": "b2", "name": "Pixel", "connected": false, "last_seen_ms": 1_000, "notifications": "unasked"},
            ],
        });
        assert_eq!(
            list_lines(&value, 1_000 + 3 * 3_600_000),
            "iPhone  connected now  notifications on  id a1\nPixel  last seen 3 h ago  notifications not asked  id b2\n"
        );
        assert_eq!(
            list_lines(&json!({"enabled": false, "phones": []}), 0),
            "Mobile is off on this computer, so no phone can connect: hide phone pair turns it on.\nNo phone is connected to this computer. hide phone pair adds one.\n"
        );
    }
}
