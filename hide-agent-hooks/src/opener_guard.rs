//! The opener guard: a `PreToolUse` refusal of an agent's shell call that
//! hands a result to the system opener (`open`, `xdg-open`, `start` and the
//! like), answered with the `hide browser open` or `hide file open` command
//! that shows it in Hide's View instead.
//!
//! A page, image or document the system opener shows lands in an outside app:
//! nothing reaches Hide's View, and the operator's Needs You never says which
//! agent opened something to look at. The session guidance names the Hide
//! commands, but a skill's or a habit's `open` wins over it, so the call is
//! refused where it is made. Like the spawn guard it is a redirect, not a
//! security boundary: it reads one shell call at one layer, and acts only
//! inside a Herdr pane whose checkout Hide has registered, asked through the
//! spawn guard's [`registration`](crate::spawn_guard::registration)
//! (`docs/agent-hooks.md`, The opener guard).
//!
//! The call is read as the spawn guard reads it (`hide_shell`): each simple
//! command of the line, its environment assignments and wrappers set aside. An
//! opener hidden one layer deeper (`bash -c`, `$(...)`, a script) is not looked
//! for, except the one line `cmd /c` and `powershell -Command` take as text,
//! which is how a Windows agent reaches `start`. Every function here is pure
//! except [`record_refusal`].

use std::path::Path;

use hide_shell::simple_commands;

use crate::spawn_guard::{append_line, command_words, log_path, now_ms};

/// What Hide's View shows a target as, and so which `hide` command shows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// An `http:`, `https:` or `file:` URL.
    Url,
    Html,
    Image,
    Pdf,
    Markdown,
    Text,
    Video,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::Html => "html",
            Self::Image => "image",
            Self::Pdf => "pdf",
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Video => "video",
        }
    }

    /// A page is shown in a browser display; everything else in a file view.
    fn is_page(self) -> bool {
        matches!(self, Self::Url | Self::Html)
    }
}

/// The file extensions Hide's View shows, by kind. A source file, a folder, an
/// app bundle and a project (`.xcodeproj`) are not results and stay the
/// system opener's.
const EXTENSIONS: &[(Kind, &[&str])] = &[
    (Kind::Html, &["html", "htm"]),
    (
        Kind::Image,
        &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "avif", "svg", "ico",
        ],
    ),
    (Kind::Pdf, &["pdf"]),
    (Kind::Markdown, &["md", "markdown", "mdx"]),
    (Kind::Text, &["txt", "log", "csv", "json", "yaml", "yml"]),
    (Kind::Video, &["mp4", "webm", "mov"]),
];

/// One target an opener was handed, as the shell read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shown {
    pub target: String,
    pub kind: Kind,
}

/// One opener command of a call and the viewable targets it names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opening {
    /// The opener, as the refusal log names it.
    pub opener: &'static str,
    pub targets: Vec<Shown>,
}

/// Whether the payload can hold an opening at all. Every shell call reaches
/// this hook, so this byte search decides every call that names no opener or
/// no viewable target before anything is parsed, read or started. Both halves
/// are ASCII and matched without case, since PowerShell's and cmd's names are.
pub fn may_hold_opening(payload: &[u8]) -> bool {
    names_opener(payload) && names_target(payload)
}

fn names_opener(bytes: &[u8]) -> bool {
    const WORDS: [&[u8]; 7] = [
        b"open",
        b"start",
        b"saps",
        b"invoke-item",
        b"explorer",
        b"browser",
        b"wslview",
    ];
    WORDS.iter().any(|word| contains(bytes, word))
        // `ii`, Invoke-Item's alias, only as a word of its own.
        || (0..bytes.len().saturating_sub(1)).any(|at| {
            bytes[at..at + 2].eq_ignore_ascii_case(b"ii")
                && (at == 0 || !bytes[at - 1].is_ascii_alphanumeric())
                && !bytes.get(at + 2).is_some_and(u8::is_ascii_alphanumeric)
        })
}

fn names_target(bytes: &[u8]) -> bool {
    if [&b"http:"[..], b"https:", b"file:"]
        .iter()
        .any(|scheme| contains(bytes, scheme))
    {
        return true;
    }
    bytes.iter().enumerate().any(|(at, &byte)| {
        byte == b'.'
            && EXTENSIONS
                .iter()
                .flat_map(|(_, extensions)| extensions.iter())
                .any(|extension| {
                    let end = at + 1 + extension.len();
                    bytes
                        .get(at + 1..end)
                        .is_some_and(|found| found.eq_ignore_ascii_case(extension.as_bytes()))
                        && !bytes.get(end).is_some_and(u8::is_ascii_alphanumeric)
                })
    })
}

fn contains(bytes: &[u8], word: &[u8]) -> bool {
    bytes
        .windows(word.len())
        .any(|window| window.eq_ignore_ascii_case(word))
}

/// What a target is, if Hide's View shows it: a URL by its scheme, a path by
/// its file name's extension. A folder (`.`), a path with no extension and
/// every other kind of file is not a viewable target.
pub fn kind_of(word: &str) -> Option<Kind> {
    let scheme = |prefix: &str| {
        word.get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    if scheme("http://") || scheme("https://") || scheme("file:") {
        return Some(Kind::Url);
    }
    let name = word.rsplit(['/', '\\']).next()?;
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    EXTENSIONS
        .iter()
        .find(|(_, extensions)| {
            extensions
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
        .map(|(kind, _)| *kind)
}

/// Every opening in `command`, in the order the line runs them.
pub fn find_openings(command: &str) -> Vec<Opening> {
    let mut openings = Vec::new();
    for words in simple_commands(command) {
        opening_of(&words, 0, &mut openings);
    }
    openings
}

/// How deep `cmd /c` and `powershell -Command` are read: one line of text in.
const NESTING: usize = 1;

fn opening_of(words: &[String], depth: usize, openings: &mut Vec<Opening>) {
    let (words, _) = command_words(words, &|_| None, true);
    let Some((first, arguments)) = words.split_first() else {
        return;
    };
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let lower = name.to_ascii_lowercase();
    let windows_name = lower.strip_suffix(".exe").unwrap_or(&lower);
    let (opener, operands): (&'static str, Option<Vec<&String>>) = match name {
        "open" => ("open", macos_open(arguments)),
        "xdg-open" | "sensible-browser" | "x-www-browser" | "www-browser" | "wslview"
        | "gnome-open" | "kde-open" | "kde-open5" | "exo-open" | "gvfs-open" => {
            ("xdg-open", Some(plain_operands(arguments)))
        }
        "gio" if arguments.first().is_some_and(|verb| verb == "open") => {
            ("gio-open", Some(plain_operands(&arguments[1..])))
        }
        _ => match windows_name {
            "start" | "start-process" | "saps" => ("start", Some(windows_operands(arguments))),
            "invoke-item" | "ii" => ("invoke-item", Some(windows_operands(arguments))),
            "explorer" => ("explorer", explorer(arguments)),
            "cmd" if depth < NESTING => {
                if let Some(line) = after_switch(arguments, &["/c", "/k", "//c", "//k"]) {
                    nested(line, depth, openings);
                }
                return;
            }
            "powershell" | "pwsh" if depth < NESTING => {
                if let Some(line) = after_switch(arguments, &["-command", "-c", "/command", "/c"]) {
                    nested(line, depth, openings);
                }
                return;
            }
            _ if windows_name.starts_with("python") || windows_name == "py" => {
                ("webbrowser", python_webbrowser(arguments))
            }
            _ => return,
        },
    };
    let targets: Vec<Shown> = operands
        .unwrap_or_default()
        .into_iter()
        .filter_map(|word| {
            Some(Shown {
                kind: kind_of(word)?,
                target: word.clone(),
            })
        })
        .collect();
    if !targets.is_empty() {
        openings.push(Opening { opener, targets });
    }
}

/// The text after `cmd`'s `/c` or PowerShell's `-Command`, read as a line of
/// its own: one word is the line, several are its words.
fn after_switch<'a>(arguments: &'a [String], switches: &[&str]) -> Option<&'a [String]> {
    let at = arguments.iter().position(|word| {
        switches
            .iter()
            .any(|switch| word.eq_ignore_ascii_case(switch))
    })?;
    Some(&arguments[at + 1..])
}

fn nested(line: &[String], depth: usize, openings: &mut Vec<Opening>) {
    let text = line.join(" ");
    for words in simple_commands(&text) {
        opening_of(&words, depth + 1, openings);
    }
}

/// macOS `open`'s operands (`man open`). A reveal (`-R`), a header search
/// (`-h`) and text from stdin (`-f`) open no target the call names; `-a`,
/// `-b`, `-s` and the long options take a value that is not one; `-u` takes a
/// URL that is; every word after `--args` is the app's own.
fn macos_open(arguments: &[String]) -> Option<Vec<&String>> {
    let mut operands = Vec::new();
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--args" => break,
            "--help" => return None,
            "--env" | "--stdin" | "--stdout" | "--stderr" | "--arch" => {
                words.next();
            }
            "--" => {
                operands.extend(words.by_ref());
                break;
            }
            long if long.starts_with("--") => {}
            short if short.starts_with('-') && short.len() > 1 => {
                for (at, flag) in short.char_indices().skip(1) {
                    match flag {
                        'R' | 'h' | 'f' => return None,
                        'a' | 'b' | 's' | 'u' => {
                            // The value is the rest of the cluster, or the next word.
                            let value = if at + 1 < short.len() {
                                None
                            } else {
                                words.next()
                            };
                            if flag == 'u' {
                                operands.extend(value);
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ => operands.push(word),
        }
    }
    Some(operands)
}

/// The operands of an opener that takes options and then targets.
fn plain_operands(arguments: &[String]) -> Vec<&String> {
    arguments
        .iter()
        .filter(|word| !word.starts_with('-'))
        .collect()
}

/// The operands of cmd's `start` and PowerShell's `Start-Process` and
/// `Invoke-Item`: a `/` switch of cmd and a `-` parameter of PowerShell are
/// skipped with the value the ones below take; `-FilePath`, `-ArgumentList`,
/// `-Path` and `-LiteralPath` name targets. Any viewable operand is a target,
/// since `start chrome x.html` shows `x.html` as much as `start x.html` does.
fn windows_operands(arguments: &[String]) -> Vec<&String> {
    const VALUED: [&str; 14] = [
        "/d",
        "/node",
        "/affinity",
        "-workingdirectory",
        "-verb",
        "-windowstyle",
        "-credential",
        "-redirectstandardinput",
        "-redirectstandardoutput",
        "-redirectstandarderror",
        "-environment",
        "-filter",
        "-include",
        "-exclude",
    ];
    let mut operands = Vec::new();
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        let lower = word.to_ascii_lowercase();
        if VALUED.contains(&lower.as_str()) {
            words.next();
        } else if !(lower.starts_with('-') || (lower.starts_with('/') && !lower[1..].contains('/')))
        {
            // A word of cmd's switch form (`/MIN`) is not a path; `/c/x.html`
            // in Git Bash is.
            operands.push(word);
        }
    }
    operands
}

/// `explorer <target>`: a folder or a `/select,` reveal is the operator's
/// file manager, and only a document it is handed opens in an app.
fn explorer(arguments: &[String]) -> Option<Vec<&String>> {
    if arguments
        .iter()
        .any(|word| word.to_ascii_lowercase().starts_with("/select"))
    {
        return None;
    }
    Some(
        arguments
            .iter()
            .filter(|word| !(word.starts_with('/') && word.contains(',')))
            .collect(),
    )
}

/// `python -m webbrowser [-n|-t] <url>`, the system opener through Python.
fn python_webbrowser(arguments: &[String]) -> Option<Vec<&String>> {
    let at = arguments
        .iter()
        .position(|word| word == "-mwebbrowser" || word == "webbrowser")?;
    if at > 0 && arguments[at] == "webbrowser" && arguments[at - 1] != "-m" {
        return None;
    }
    Some(plain_operands(&arguments[at + 1..]))
}

// --- What the agent is told ------------------------------------------------------

/// A word as a POSIX shell reads it back: bare when it holds nothing a shell
/// treats specially, a home path's `~/` kept outside the quotes, a variable
/// inside double quotes, otherwise in single quotes.
fn shell_word(word: &str) -> String {
    let plain = |text: &str| {
        !text.is_empty()
            && text.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | ',' | '@' | '%' | '+')
            })
    };
    if plain(word) {
        return word.to_owned();
    }
    if let Some(rest) = word.strip_prefix("~/") {
        return format!("~/{}", shell_word(rest));
    }
    if word.contains('$') {
        let escaped: String = word
            .chars()
            .flat_map(|c| {
                matches!(c, '"' | '\\' | '`')
                    .then_some('\\')
                    .into_iter()
                    .chain(Some(c))
            })
            .collect();
        return format!("\"{escaped}\"");
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The command that shows `shown` in Hide's View, through `hide`, the CLI
/// beside the helper as the session guidance names it.
pub fn show_command(hide: &str, shown: &Shown) -> String {
    let verb = if shown.kind.is_page() {
        "browser open"
    } else {
        "file open"
    };
    let program = shell_word(hide);
    if shown.kind != Kind::Url && shown.target.contains(['*', '?', '[']) {
        // A pattern names many files; each takes its own command.
        return format!(
            "{program} {verb} <path>   (once for each file {} matches)",
            shell_word(&shown.target)
        );
    }
    format!("{program} {verb} {}", shell_word(&shown.target))
}

/// The reason a refused call reads, for the agent: one command per target.
pub fn refusal_reason(hide: &str, openings: &[Opening]) -> String {
    let mut commands: Vec<String> = Vec::new();
    for shown in openings.iter().flat_map(|opening| &opening.targets) {
        let command = show_command(hide, shown);
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    format!(
        "Not run: this call shows its result in an app outside Hide, where the operator cannot see that you opened it. \
         Show it in Hide's View instead:\n\n{}\n\n\
         It opens as a tab of the View beside you, and the operator's Needs You tells them to look at it. \
         This holds for a file outside the checkout too. If Hide refuses the path, say so in your reply with the path; \
         do not fall back to open, xdg-open or start. If this call held other commands, run them separately.",
        commands.join("\n")
    )
}

// --- What is recorded ------------------------------------------------------------

/// One refusal, as one structured line in the spawn guard's log: the pane,
/// the openers and the kinds of their targets, and the runtime. Neither the
/// command nor a path is written: either can hold a name the operator keeps
/// private. A line that cannot be written is a refusal that already
/// happened, so nothing is retried and the call is refused regardless.
pub fn record_refusal(home: &Path, runtime: &str, pane: &str, openings: &[Opening]) {
    let mut openers: Vec<&str> = openings.iter().map(|opening| opening.opener).collect();
    openers.dedup();
    let mut kinds: Vec<&str> = openings
        .iter()
        .flat_map(|opening| &opening.targets)
        .map(|shown| shown.kind.code())
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    let line = serde_json::json!({
        "component": "opener_guard",
        "kind": "opener.refused",
        "runtime": runtime,
        "pane": pane,
        "openers": openers,
        "targets": kinds,
        "at_unix_ms": now_ms(),
    })
    .to_string();
    eprintln!("{line}");
    let _ = append_line(&log_path(home), &line);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(command: &str) -> Vec<(&'static str, String, Kind)> {
        find_openings(command)
            .into_iter()
            .flat_map(|opening| {
                opening
                    .targets
                    .into_iter()
                    .map(move |shown| (opening.opener, shown.target, shown.kind))
            })
            .collect()
    }

    fn one(command: &str) -> (&'static str, String, Kind) {
        let found = targets(command);
        assert_eq!(found.len(), 1, "one target in: {command}: {found:?}");
        found.into_iter().next().unwrap()
    }

    fn none(command: &str) {
        assert_eq!(targets(command), [], "no opening in: {command}");
    }

    #[test]
    fn macos_open_of_a_page_image_document_or_url_is_found() {
        for (command, target, kind) in [
            ("open report.html", "report.html", Kind::Html),
            ("open /tmp/shot.PNG", "/tmp/shot.PNG", Kind::Image),
            ("open 'out dir/spec.pdf'", "out dir/spec.pdf", Kind::Pdf),
            ("open README.md", "README.md", Kind::Markdown),
            ("open notes.txt", "notes.txt", Kind::Text),
            ("open demo.mov", "demo.mov", Kind::Video),
            (
                "open http://localhost:3000",
                "http://localhost:3000",
                Kind::Url,
            ),
            (
                "open https://example.com/a",
                "https://example.com/a",
                Kind::Url,
            ),
            ("open file:///tmp/a.html", "file:///tmp/a.html", Kind::Url),
            ("/usr/bin/open x.html", "x.html", Kind::Html),
            ("open -a 'Google Chrome' x.html", "x.html", Kind::Html),
            ("open -a Preview shot.png", "shot.png", Kind::Image),
            ("open -b com.apple.Safari x.html", "x.html", Kind::Html),
            ("open -e notes.txt", "notes.txt", Kind::Text),
            ("open -t README.md", "README.md", Kind::Markdown),
            ("open -gna Safari x.html", "x.html", Kind::Html),
            (
                "open -u https://example.com",
                "https://example.com",
                Kind::Url,
            ),
            ("open -W --env A=1 x.pdf", "x.pdf", Kind::Pdf),
            ("open -- -odd.png", "-odd.png", Kind::Image),
            ("cd out && open index.html", "index.html", Kind::Html),
            (
                "npm run build; open dist/index.html",
                "dist/index.html",
                Kind::Html,
            ),
            ("FOO=1 nohup open x.html &", "x.html", Kind::Html),
            ("command open x.html", "x.html", Kind::Html),
            ("open \"$OUT/report.html\"", "$OUT/report.html", Kind::Html),
        ] {
            let (opener, found, found_kind) = one(command);
            assert_eq!(
                (opener, found.as_str(), found_kind),
                ("open", target, kind),
                "{command}"
            );
        }
        // Every target of one call, and of every opener in the line.
        assert_eq!(targets("open a.png b.pdf c.app").len(), 2);
        assert_eq!(targets("open a.html && xdg-open b.png").len(), 2);
    }

    #[test]
    fn what_the_system_opener_shows_that_is_not_a_result_is_left_alone() {
        for command in [
            "open .",
            "open ~/Downloads",
            "open -R out/shot.png",
            "open -Rg out/shot.png",
            "open -h stdio.h",
            "open -f",
            "open -a Preview",
            "open -a Simulator",
            "open /Applications/Safari.app",
            "open Hide.xcodeproj",
            "open --help",
            "open src/main.rs",
            "open x.html.app",
            "open .md",
            "open -a Safari --args x.html",
            "echo open x.html",
            "git commit -m 'open x.html'",
            "grep -rn 'open x.html' docs",
            "cat <<EOF\nopen x.html\nEOF",
            "bash -c 'open x.html'",
            "echo \"$(open x.html)\"",
            "# open x.html",
            "opener x.html",
            "xopen x.html",
            "openssl x509 -in cert.pem",
            "npm start",
            "gio info x.html",
            "google-chrome --headless --screenshot x.html",
            "firefox x.html",
            "python3 -m http.server",
            "python3 serve.py page.html",
            "explorer .",
            "explorer /select,C:/out/x.png",
            "ls x.html",
        ] {
            none(command);
        }
    }

    #[test]
    fn linux_openers_are_found() {
        for (command, opener) in [
            ("xdg-open out/index.html", "xdg-open"),
            ("sensible-browser https://example.com", "xdg-open"),
            ("x-www-browser report.html", "xdg-open"),
            ("www-browser report.html", "xdg-open"),
            ("wslview report.html", "xdg-open"),
            ("gnome-open shot.png", "xdg-open"),
            ("kde-open5 shot.png", "xdg-open"),
            (
                "exo-open --launch WebBrowser https://example.com",
                "xdg-open",
            ),
            ("gio open shot.png", "gio-open"),
            ("python3 -m webbrowser -t https://example.com", "webbrowser"),
            ("python -mwebbrowser report.html", "webbrowser"),
        ] {
            assert_eq!(one(command).0, opener, "{command}");
        }
        none("xdg-open --help");
        none("xdg-open .");
        none("gio open .");
    }

    #[test]
    fn windows_openers_are_found_in_any_case_and_through_cmd_and_powershell() {
        for (command, opener, target) in [
            ("start report.html", "start", "report.html"),
            ("start \"\" report.html", "start", "report.html"),
            ("start /MIN /D C:/out report.html", "start", "report.html"),
            (
                "start chrome https://example.com",
                "start",
                "https://example.com",
            ),
            ("START x.pdf", "start", "x.pdf"),
            ("cmd /c start x.html", "start", "x.html"),
            ("cmd.exe /C \"start \"\" x.html\"", "start", "x.html"),
            ("cmd //c start x.html", "start", "x.html"),
            ("Start-Process x.html", "start", "x.html"),
            ("start-process -FilePath out.png -Wait", "start", "out.png"),
            (
                "Start-Process msedge -ArgumentList https://example.com",
                "start",
                "https://example.com",
            ),
            ("saps x.pdf", "start", "x.pdf"),
            (
                "Invoke-Item 'C:/out/shot.png'",
                "invoke-item",
                "C:/out/shot.png",
            ),
            ("ii shot.png", "invoke-item", "shot.png"),
            ("explorer.exe report.html", "explorer", "report.html"),
            (
                "explorer https://example.com",
                "explorer",
                "https://example.com",
            ),
            (
                "powershell -Command \"Start-Process x.html\"",
                "start",
                "x.html",
            ),
            (
                "pwsh -NoProfile -c 'ii shot.png'",
                "invoke-item",
                "shot.png",
            ),
            ("& start x.html", "start", "x.html"),
            ("start /c/work/out/x.html", "start", "/c/work/out/x.html"),
        ] {
            let (found_opener, found, _) = one(command);
            assert_eq!(
                (found_opener, found.as_str()),
                (opener, target),
                "{command}"
            );
        }
        for command in [
            "start",
            "start /WAIT setup.exe",
            "Start-Process notepad",
            "Start-Process cargo -RedirectStandardOutput out.txt",
            "Invoke-Item .",
            "cmd /c dir",
            "cmd /c \"cmd /c start x.html\"",
            "powershell -EncodedCommand AAAA",
            "pwsh -File build.ps1 x.html",
        ] {
            none(command);
        }
    }

    #[test]
    fn the_payload_prefilter_needs_an_opener_word_and_a_viewable_target() {
        assert!(may_hold_opening(br#"{"command":"open out/index.html"}"#));
        assert!(may_hold_opening(br#"{"command":"Start-Process x.PDF"}"#));
        assert!(may_hold_opening(br#"{"command":"ii shot.png"}"#));
        assert!(may_hold_opening(b"ii shot.png"));
        assert!(may_hold_opening(br#"{"command":"xdg-open https://a"}"#));
        assert!(!may_hold_opening(br#"{"command":"cargo test"}"#));
        assert!(!may_hold_opening(br#"{"command":"git add docs/x.md"}"#));
        assert!(!may_hold_opening(br#"{"command":"open ."}"#));
        // `ii` inside a word and an extension inside a longer one are not marks.
        assert!(!may_hold_opening(br#"{"command":"ascii x.png"}"#));
        assert!(!may_hold_opening(br#"{"command":"open x.jsonl"}"#));
        assert!(!may_hold_opening(br#"{"command":"open x.htmlx"}"#));
    }

    #[test]
    fn the_offered_command_is_filled_per_target() {
        let hide = "/Applications/hide.app/Contents/Resources/hide";
        let shown = |target: &str| Shown {
            target: target.into(),
            kind: kind_of(target).unwrap(),
        };
        assert_eq!(
            show_command(hide, &shown("out/index.html")),
            format!("{hide} browser open out/index.html")
        );
        assert_eq!(
            show_command(hide, &shown("http://localhost:3000/a?b=1")),
            format!("{hide} browser open 'http://localhost:3000/a?b=1'")
        );
        assert_eq!(
            show_command("hide", &shown("shot.png")),
            "hide file open shot.png"
        );
        assert_eq!(
            show_command("/a b/hide", &shown("it's.pdf")),
            "'/a b/hide' file open 'it'\\''s.pdf'"
        );
        assert_eq!(
            show_command("hide", &shown("$OUT/a.md")),
            "hide file open \"$OUT/a.md\""
        );
        assert_eq!(
            show_command("hide", &shown("~/My Shots/a.png")),
            "hide file open ~/'My Shots/a.png'"
        );
        assert_eq!(
            show_command("hide", &shown("out/*.png")),
            "hide file open <path>   (once for each file 'out/*.png' matches)"
        );
    }

    #[test]
    fn the_refusal_names_each_command_once_and_what_the_operator_sees() {
        let reason = refusal_reason("hide", &find_openings("open a.html b.png; open a.html"));
        assert!(reason.contains("hide browser open a.html\nhide file open b.png\n"));
        assert_eq!(reason.matches("a.html").count(), 1);
        assert!(reason.contains("Needs You tells them to look at it"));
        assert!(reason.contains("outside the checkout too"));
        assert!(reason.contains("do not fall back to open, xdg-open or start"));
        assert!(!reason.contains('\u{2014}'), "no em dash");
    }

    #[test]
    fn a_refusal_is_logged_without_the_command_or_a_path() {
        let home = tempfile::tempdir().unwrap();
        let openings = find_openings("open /Users/someone/SECRET/report.html shot.png");
        record_refusal(home.path(), "codex", "w1:p2", &openings);
        let text = std::fs::read_to_string(log_path(home.path())).unwrap();
        let line: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(line["kind"], "opener.refused");
        assert_eq!(line["pane"], "w1:p2");
        assert_eq!(line["openers"], serde_json::json!(["open"]));
        assert_eq!(line["targets"], serde_json::json!(["html", "image"]));
        assert!(!text.contains("SECRET") && !text.contains("shot"), "{text}");
    }
}
