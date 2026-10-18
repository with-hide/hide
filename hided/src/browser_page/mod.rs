//! `hide browser` page commands: read and act on a browser display through
//! hided's relay to the desktop's scoped CDP gateway. Each command connects,
//! attaches, does its work and disconnects; refs, baselines and the overlay
//! live in the page, so nothing outlives the command.

mod actions;
mod cdp;
pub mod comment;
#[cfg(test)]
mod fake_gateway;
mod page;

use std::time::Duration;

use serde_json::{Value, json};

use crate::env::Env;

pub const HELP: &str = include_str!("../../assets/browser/help.md");

/// One element a command points at.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// `@N` in the top document (or its same-origin frames), or `@<tag>:N` in
    /// the cross-origin frame whose document carries that tag.
    Ref { tag: Option<String>, number: u64 },
    /// A visible label (`click --text`).
    Text(String),
    /// A point in CSS pixels of the viewport, or of the current screenshot.
    Point { x: f64, y: f64, image: bool },
}

// Parsed coordinates are always finite, so equality is total.
impl Eq for Target {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verify {
    /// Wait this long after the action, then report what changed.
    After(Duration),
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragMode {
    Auto,
    Pointer,
    Html5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    Text(String),
    Selector { selector: String, gone: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Snapshot {
        display: String,
        interactive: bool,
        diff: bool,
        grep: Option<String>,
    },
    Click {
        display: String,
        target: Target,
        verify: Verify,
    },
    Fill {
        display: String,
        target: Target,
        text: String,
        verify: Verify,
    },
    Type {
        display: String,
        text: String,
        verify: Verify,
    },
    Press {
        display: String,
        key: String,
        verify: Verify,
    },
    Hover {
        display: String,
        target: Target,
        verify: Verify,
    },
    Drag {
        display: String,
        from: Target,
        to: Target,
        mode: DragMode,
        verify: Verify,
    },
    Scroll {
        display: String,
        /// `up`, `down`, or an element to bring into view.
        direction: Option<bool>,
        target: Option<Target>,
        verify: Verify,
    },
    Wait {
        display: String,
        wait: Wait,
        timeout: Duration,
    },
    Screenshot {
        display: String,
        path: String,
        target: Option<Target>,
    },
    Eval {
        display: String,
        expression: String,
    },
    Console {
        display: String,
    },
    Network {
        display: String,
    },
}

impl Eq for Command {}

pub const VERBS: &[&str] = &[
    "snapshot",
    "click",
    "fill",
    "type",
    "press",
    "hover",
    "drag",
    "scroll",
    "wait",
    "screenshot",
    "eval",
    "console",
    "network",
];

const DEFAULT_VERIFY: Duration = Duration::from_millis(300);
const MAX_VERIFY_MS: u64 = 10_000;
const DEFAULT_WAIT: Duration = Duration::from_secs(5);
const MAX_WAIT_MS: u64 = 60_000;

fn usage(verb: &str) -> String {
    let line = match verb {
        "snapshot" => "hide browser snapshot <display> [--interactive] [--diff | --grep <pattern>]",
        "click" => {
            "hide browser click <display> (@ref | --text <label> | --xy <x> <y> [--space css|image]) [--verify <ms> | --no-verify]"
        }
        "fill" => "hide browser fill <display> @ref <text> [--verify <ms> | --no-verify]",
        "type" => "hide browser type <display> <text> [--verify <ms> | --no-verify]",
        "press" => "hide browser press <display> <key> [--verify <ms> | --no-verify]",
        "hover" => {
            "hide browser hover <display> (@ref | --xy <x> <y> [--space css|image]) [--verify <ms> | --no-verify]"
        }
        "drag" => {
            "hide browser drag <display> (@ref | <x>,<y>) (@ref | <x>,<y>) [--mode auto|pointer|html5] [--space css|image] [--verify <ms> | --no-verify]"
        }
        "scroll" => {
            "hide browser scroll <display> (up | down | @ref) [--verify <ms> | --no-verify]"
        }
        "wait" => {
            "hide browser wait <display> (--text <text> | --selector <selector|@ref> [--gone]) [--timeout <ms>]"
        }
        "screenshot" => "hide browser screenshot <display> <path> [--ref @ref]",
        "eval" => "hide browser eval <display> <expression>",
        "console" => "hide browser console <display>",
        "network" => "hide browser network <display>",
        _ => "hide browser help",
    };
    format!("usage: {line}")
}

/// `@N` or `@<tag>:N`; anything else is not a ref.
pub fn parse_ref(text: &str) -> Option<Target> {
    let body = text.strip_prefix('@')?;
    let (tag, number) = match body.split_once(':') {
        Some((tag, number)) => {
            if tag.is_empty()
                || tag.len() > 16
                || !tag
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            {
                return None;
            }
            (Some(tag.to_owned()), number)
        }
        None => (None, body),
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Target::Ref {
        tag,
        number: number.parse().ok().filter(|n| *n > 0)?,
    })
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Parses the words after `hide browser <verb>`.
pub fn parse<'a>(verb: &str, args: impl Iterator<Item = &'a String>) -> Result<Command, String> {
    let words: Vec<&str> = args.map(String::as_str).collect();
    let fail = || usage(verb);
    let (display, rest) = words.split_first().ok_or_else(fail)?;
    if display.is_empty() || display.starts_with('-') {
        return Err(fail());
    }
    let display = (*display).to_owned();
    // Options may come in any order after the display; positional words keep
    // their order.
    let mut positional = Vec::new();
    let mut verify = None;
    let mut interactive = false;
    let mut diff = false;
    let mut grep = None;
    let mut text = None;
    let mut xy = None;
    let mut image = None;
    let mut mode = None;
    let mut selector = None;
    let mut gone = false;
    let mut timeout = None;
    let mut reference = None;
    let mut index = 0;
    while index < rest.len() {
        let word = rest[index];
        let value = |index: &mut usize| -> Result<&str, String> {
            *index += 1;
            rest.get(*index).copied().ok_or_else(fail)
        };
        match word {
            "--interactive" if verb == "snapshot" && !interactive => interactive = true,
            "--diff" if verb == "snapshot" && !diff => diff = true,
            "--grep" if verb == "snapshot" && grep.is_none() => {
                grep = Some(value(&mut index)?.to_owned())
            }
            "--no-verify" if verify.is_none() => verify = Some(Verify::Off),
            "--verify" if verify.is_none() => {
                let ms: u64 = value(&mut index)?.parse().map_err(|_| fail())?;
                verify = Some(Verify::After(Duration::from_millis(ms.min(MAX_VERIFY_MS))));
            }
            "--text" if matches!(verb, "click" | "wait") && text.is_none() => {
                text = Some(value(&mut index)?.to_owned())
            }
            "--xy" if matches!(verb, "click" | "hover") && xy.is_none() => {
                let x = number(value(&mut index)?).ok_or_else(fail)?;
                let y = number(value(&mut index)?).ok_or_else(fail)?;
                xy = Some((x, y));
            }
            "--space" if matches!(verb, "click" | "hover" | "drag") && image.is_none() => {
                image = Some(match value(&mut index)? {
                    "css" => false,
                    "image" => true,
                    _ => return Err(fail()),
                })
            }
            "--mode" if verb == "drag" && mode.is_none() => {
                mode = Some(match value(&mut index)? {
                    "auto" => DragMode::Auto,
                    "pointer" => DragMode::Pointer,
                    "html5" => DragMode::Html5,
                    _ => return Err(fail()),
                })
            }
            "--selector" if verb == "wait" && selector.is_none() => {
                selector = Some(value(&mut index)?.to_owned())
            }
            "--gone" if verb == "wait" && !gone => gone = true,
            "--timeout" if verb == "wait" && timeout.is_none() => {
                let ms: u64 = value(&mut index)?.parse().map_err(|_| fail())?;
                if ms == 0 {
                    return Err(fail());
                }
                timeout = Some(Duration::from_millis(ms.min(MAX_WAIT_MS)));
            }
            "--ref" if verb == "screenshot" && reference.is_none() => {
                reference = Some(parse_ref(value(&mut index)?).ok_or_else(fail)?)
            }
            "--" => {
                positional.extend(rest[index + 1..].iter().copied());
                break;
            }
            word if word.starts_with("--") => return Err(fail()),
            word => positional.push(word),
        }
        index += 1;
    }
    let verify = verify.unwrap_or(Verify::After(DEFAULT_VERIFY));
    let acting = matches!(
        verb,
        "click" | "fill" | "type" | "press" | "hover" | "drag" | "scroll"
    );
    if !acting && verify != Verify::After(DEFAULT_VERIFY) {
        return Err(fail());
    }
    let point = |(x, y): (f64, f64)| Target::Point {
        x,
        y,
        image: image.unwrap_or(false),
    };
    let end = |word: &str| -> Option<Target> {
        if word.starts_with('@') {
            return parse_ref(word);
        }
        let (x, y) = word.split_once(',')?;
        Some(Target::Point {
            x: number(x)?,
            y: number(y)?,
            image: image.unwrap_or(false),
        })
    };
    let command = match (verb, positional.as_slice()) {
        ("snapshot", []) if !(diff && grep.is_some()) => Command::Snapshot {
            display,
            interactive,
            diff,
            grep,
        },
        ("click", [word]) if text.is_none() && xy.is_none() && image.is_none() => Command::Click {
            display,
            target: parse_ref(word).ok_or_else(fail)?,
            verify,
        },
        ("click", []) => match (text, xy) {
            (Some(label), None) if image.is_none() && !label.trim().is_empty() => Command::Click {
                display,
                target: Target::Text(label),
                verify,
            },
            (None, Some(xy)) => Command::Click {
                display,
                target: point(xy),
                verify,
            },
            _ => return Err(fail()),
        },
        ("fill", [word, value]) => Command::Fill {
            display,
            target: parse_ref(word).ok_or_else(fail)?,
            text: (*value).to_owned(),
            verify,
        },
        ("type", [value]) if !value.is_empty() => Command::Type {
            display,
            text: (*value).to_owned(),
            verify,
        },
        ("press", [key]) => Command::Press {
            display,
            key: (*key).to_owned(),
            verify,
        },
        ("hover", [word]) if xy.is_none() && image.is_none() => Command::Hover {
            display,
            target: parse_ref(word).ok_or_else(fail)?,
            verify,
        },
        ("hover", []) => Command::Hover {
            display,
            target: point(xy.ok_or_else(fail)?),
            verify,
        },
        ("drag", [from, to]) => Command::Drag {
            display,
            from: end(from).ok_or_else(fail)?,
            to: end(to).ok_or_else(fail)?,
            mode: mode.unwrap_or(DragMode::Auto),
            verify,
        },
        ("scroll", [word]) => match *word {
            "up" | "down" => Command::Scroll {
                display,
                direction: Some(*word == "up"),
                target: None,
                verify,
            },
            word => Command::Scroll {
                display,
                direction: None,
                target: Some(parse_ref(word).ok_or_else(fail)?),
                verify,
            },
        },
        ("wait", []) => {
            let wait = match (text, selector) {
                (Some(text), None) if !gone && !text.is_empty() => Wait::Text(text),
                (None, Some(selector)) if !selector.is_empty() => Wait::Selector { selector, gone },
                _ => return Err(fail()),
            };
            Command::Wait {
                display,
                wait,
                timeout: timeout.unwrap_or(DEFAULT_WAIT),
            }
        }
        ("screenshot", [path]) if !path.is_empty() => Command::Screenshot {
            display,
            path: (*path).to_owned(),
            target: reference,
        },
        ("eval", [expression]) if !expression.trim().is_empty() => Command::Eval {
            display,
            expression: (*expression).to_owned(),
        },
        ("console", []) => Command::Console { display },
        ("network", []) => Command::Network { display },
        _ => return Err(fail()),
    };
    Ok(command)
}

/// A refusal a caller can branch on: a stable reason, what to do next, and
/// the page's own words where they help (a covering element, the options).
#[derive(Debug)]
pub struct Failure {
    pub reason: String,
    pub detail: Option<String>,
    pub next_action: Option<String>,
    /// The page was asked and gave no answer in time, as against a closed
    /// connection or a protocol error; only silence says a frame is held.
    pub silent: bool,
}

impl Failure {
    pub fn new(reason: &str, detail: Option<String>) -> Self {
        Self {
            reason: reason.to_owned(),
            detail,
            next_action: None,
            silent: false,
        }
    }

    /// A refusal from one document (a protocol error, a script that threw as
    /// its frame navigated), not from the connection or a held page.
    pub fn page_side(&self) -> bool {
        matches!(self.reason.as_str(), "cdp_error" | "page_script_failed")
    }
}

fn next_action(reason: &str, display: &str) -> String {
    let snapshot = format!("hide browser snapshot {display}");
    match reason {
        "ref_stale" => format!("Take a fresh snapshot ({snapshot}) and use its refs"),
        "target_hidden" => format!("The element is not visible; run {snapshot} to find a visible one"),
        "target_covered" => format!(
            "Another element covers it; dismiss or act on the covering element first, then run {snapshot}"
        ),
        "target_outside_viewport" | "outside_viewport" => format!(
            "Bring it into view with hide browser scroll {display} @ref, or use a point inside the viewport"
        ),
        "text_not_found" => format!("No visible control has that label; run {snapshot} --grep <text>"),
        "text_ambiguous" => "Several controls match; click by @ref or a longer label".to_owned(),
        "target_not_fillable" => format!("Fill an input, select, textarea or contenteditable ref from {snapshot}"),
        "option_missing" => "Fill the select with one of the listed option values or labels".to_owned(),
        "fill_rejected" => format!(
            "The text was inserted but the page shows something else; look with {snapshot} --diff before inserting it again"
        ),
        "key_unsupported" => "Press one of the listed keys".to_owned(),
        "invalid_selector" => "Use a valid CSS selector or an @ref".to_owned(),
        "drag_across_frames" => "Drag between two points of the same frame".to_owned(),
        "drag_carries_files" => "The drag carries local files, which hide browser never drops on a page; do not retry it (--mode pointer would start a native drag the gateway cannot see), and ask the operator if the page needs them".to_owned(),
        "display_busy" => "Another CDP client or hide browser command holds this display; close it (agent-browser, Playwright) or let it finish, then retry".to_owned(),
        "display_hidden" => {
            "The display is not in front; ask the operator to show it, then retry".to_owned()
        }
        "display_unsupported" | "browser_address_unsupported" => {
            "hide browser works on http(s) and blank displays; open the page with hide browser open <url>".to_owned()
        }
        "display_missing" | "browser_display_missing" => {
            "Run hide view list and choose a current browser display".to_owned()
        }
        "display_closed" => "The display closed during the command; run hide view list".to_owned(),
        "dialog_open" => "A JavaScript dialog, or a script that never yields, holds the page; the operator must answer the dialog in hide before the next command".to_owned(),
        "page_unresponsive" => "The page did not answer in time; retry, or ask the operator to check the page".to_owned(),
        "timeout" => format!("The condition did not hold in time; run {snapshot} to see the page"),
        "eval_error" => "Fix the expression and retry".to_owned(),
        "screenshot_unwritable" => "Choose a path in a writable folder".to_owned(),
        "browser_relay_message_limit" | "browser_limit" => {
            "A message crossed a size or rate limit; capture a region with screenshot --ref, narrow the request, or wait a minute, then retry".to_owned()
        }
        "browser_relay_limit" | "browser_control_busy" => {
            "Other hide browser commands or CDP clients are running; let one finish, then retry".to_owned()
        }
        "browser_control_unavailable" => "Reconnect the Hide desktop app and retry".to_owned(),
        "browser_window_unreachable" => {
            "The machine whose window shows this display is not reachable; retry once it is back".to_owned()
        }
        "drag_not_started" => "The source did not start a native drag; retry with --mode pointer".to_owned(),
        "screenshot_failed" => "Retry the screenshot; if it keeps failing, check the display with hide view status".to_owned(),
        "cdp_error" | "page_script_failed" => {
            format!("The page changed under the command; run {snapshot} and retry against what it shows")
        }
        _ => "Check Hide status and the display (hide view list), then retry".to_owned(),
    }
}

/// The relay's failures are named for this command surface (B31).
fn relay_reason(reason: &str) -> &str {
    match reason {
        "browser_address_unsupported" => "display_unsupported",
        "browser_display_missing" => "display_missing",
        other => other,
    }
}

/// No command runs longer: `wait` is at most a minute, and a page that stays
/// silent past this is not going to answer.
const COMMAND_DEADLINE: Duration = Duration::from_secs(120);

/// Page text reaches the caller's terminal: control characters (except
/// newlines and tabs) and the bidirectional controls would let a page
/// rewrite or reorder what is printed, so each becomes U+FFFD. Joiners stay,
/// since emoji and several scripts need them.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            let bidi = matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
            if (c.is_control() && c != '\n' && c != '\t') || bidi {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}

pub fn run(env: &Env, command: Command) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "runtime_unavailable".to_owned())?;
    let display = display_of(&command).to_owned();
    let outcome = runtime.block_on(async {
        tokio::time::timeout(COMMAND_DEADLINE, execute(env, command))
            .await
            .unwrap_or_else(|_| Err(Failure::new("page_unresponsive", None)))
    });
    match outcome {
        Ok(Output::Text(text)) => {
            let text = printable(&text);
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            Ok(())
        }
        Ok(Output::Json(value)) => {
            println!("{}", printable(&value.to_string()));
            Ok(())
        }
        Err(failure) => {
            let reason = relay_reason(&failure.reason).to_owned();
            let next = failure
                .next_action
                .clone()
                .unwrap_or_else(|| next_action(&reason, &display));
            let mut answer =
                json!({"ok": false, "reason": reason, "display": display, "next_action": next});
            if let Some(detail) = failure.detail {
                answer["detail"] = json!(detail);
            }
            println!("{}", printable(&answer.to_string()));
            Err(printable(&reason))
        }
    }
}

fn display_of(command: &Command) -> &str {
    match command {
        Command::Snapshot { display, .. }
        | Command::Click { display, .. }
        | Command::Fill { display, .. }
        | Command::Type { display, .. }
        | Command::Press { display, .. }
        | Command::Hover { display, .. }
        | Command::Drag { display, .. }
        | Command::Scroll { display, .. }
        | Command::Wait { display, .. }
        | Command::Screenshot { display, .. }
        | Command::Eval { display, .. }
        | Command::Console { display }
        | Command::Network { display } => display,
    }
}

pub enum Output {
    Text(String),
    Json(Value),
}

async fn execute(env: &Env, command: Command) -> Result<Output, Failure> {
    let display = display_of(&command).to_owned();
    let mut credential = crate::workspace_cli::Credential::acquire(env).map_err(|reason| {
        let mut failure = Failure::new(&reason, None);
        failure.next_action = Some(crate::cli::bootstrap_next_action(&reason).to_owned());
        failure
    })?;
    let (socket, selected, source) = crate::workspace_cli::browser_relay(&mut credential, &display)
        .await
        .map_err(|(reason, next_action)| Failure {
            reason,
            detail: None,
            next_action,
            silent: false,
        })?;
    let mut page = page::Page::attach(cdp::Cdp::new(socket), &display, selected).await?;
    page.source = source;
    let outcome = actions::run(&mut page, command).await;
    page.close().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(words: &[&str]) -> Result<Command, String> {
        let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        parse(&words[0], words[1..].iter())
    }

    #[test]
    fn page_text_cannot_steer_the_terminal() {
        assert_eq!(
            printable("@1 button \"Go\u{1b}]52;c;x\u{7}\"\n\u{202e}gnp.exe\tok\u{9b}2J"),
            "@1 button \"Go\u{fffd}]52;c;x\u{fffd}\"\n\u{fffd}gnp.exe\tok\u{fffd}2J"
        );
        assert_eq!(
            printable("한글 값 -> /x 👩\u{200d}💻"),
            "한글 값 -> /x 👩\u{200d}💻"
        );
    }

    #[test]
    fn refs_name_a_top_document_element_or_a_tagged_frame_element() {
        assert_eq!(
            parse_ref("@12"),
            Some(Target::Ref {
                tag: None,
                number: 12
            })
        );
        assert_eq!(
            parse_ref("@k7q2:3"),
            Some(Target::Ref {
                tag: Some("k7q2".into()),
                number: 3
            })
        );
        for text in [
            "12", "@", "@0", "@-1", "@k7q2:", "@:3", "@K7Q2:3", "@a b:3", "@3x",
        ] {
            assert_eq!(parse_ref(text), None, "{text}");
        }
    }

    #[test]
    fn each_command_reads_its_documented_forms() {
        assert_eq!(
            parsed(&["snapshot", "browser-1", "--interactive", "--diff"]).unwrap(),
            Command::Snapshot {
                display: "browser-1".into(),
                interactive: true,
                diff: true,
                grep: None
            }
        );
        assert_eq!(
            parsed(&["click", "browser-1", "@k7q2:3", "--no-verify"]).unwrap(),
            Command::Click {
                display: "browser-1".into(),
                target: Target::Ref {
                    tag: Some("k7q2".into()),
                    number: 3
                },
                verify: Verify::Off,
            }
        );
        assert_eq!(
            parsed(&[
                "click",
                "browser-1",
                "--xy",
                "10",
                "20.5",
                "--space",
                "image"
            ])
            .unwrap(),
            Command::Click {
                display: "browser-1".into(),
                target: Target::Point {
                    x: 10.0,
                    y: 20.5,
                    image: true
                },
                verify: Verify::After(DEFAULT_VERIFY),
            }
        );
        assert_eq!(
            parsed(&["fill", "browser-1", "@2", "한글 값", "--verify", "50000"]).unwrap(),
            Command::Fill {
                display: "browser-1".into(),
                target: Target::Ref {
                    tag: None,
                    number: 2
                },
                text: "한글 값".into(),
                verify: Verify::After(Duration::from_millis(MAX_VERIFY_MS)),
            }
        );
        assert_eq!(
            parsed(&["drag", "browser-1", "@1", "300,40.5", "--mode", "pointer"]).unwrap(),
            Command::Drag {
                display: "browser-1".into(),
                from: Target::Ref {
                    tag: None,
                    number: 1
                },
                to: Target::Point {
                    x: 300.0,
                    y: 40.5,
                    image: false
                },
                mode: DragMode::Pointer,
                verify: Verify::After(DEFAULT_VERIFY),
            }
        );
        assert_eq!(
            parsed(&[
                "wait",
                "browser-1",
                "--selector",
                "@4",
                "--gone",
                "--timeout",
                "999999"
            ])
            .unwrap(),
            Command::Wait {
                display: "browser-1".into(),
                wait: Wait::Selector {
                    selector: "@4".into(),
                    gone: true
                },
                timeout: Duration::from_millis(MAX_WAIT_MS),
            }
        );
        assert_eq!(
            parsed(&["eval", "browser-1", "--", "--x"]).unwrap(),
            Command::Eval {
                display: "browser-1".into(),
                expression: "--x".into()
            }
        );
    }

    #[test]
    fn a_malformed_command_is_refused_with_its_usage() {
        for words in [
            &["snapshot"][..],
            &["snapshot", "--diff"],
            &["snapshot", "b", "--diff", "--grep", "x"],
            &["snapshot", "b", "--no-verify"],
            &["click", "b"],
            &["click", "b", "3"],
            &["click", "b", "@3", "--text", "Go"],
            &["click", "b", "--xy", "1"],
            &["fill", "b", "@3"],
            &["type", "b", ""],
            &["wait", "b", "--text", "x", "--gone"],
            &["wait", "b", "--timeout", "0", "--text", "x"],
            &["scroll", "b", "left"],
            &["screenshot", "b"],
            &["drag", "b", "@1", "1;2"],
            &["eval", "b", "1", "--verify", "10"],
        ] {
            let error = parsed(words).unwrap_err();
            assert!(
                error.starts_with("usage: hide browser"),
                "{words:?}: {error}"
            );
        }
    }
}
