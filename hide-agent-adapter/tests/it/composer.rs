//! Each agent's composer declaration against screens captured from the real
//! agent under Herdr 0.9.5 at 60 columns on 2026-10-21: type `hello
//! 안녕하세요`, Backspace it all, paste 12 lines, Backspace twice, clear
//! (`docs/delivery.md`, Reading the composer). The expected verdict is what
//! the operator saw in the composer at each step.

use std::path::Path;

use hide_agent_adapter::composer::{Miss, PANE_FLOOR_COLUMNS, Verdict};
use hide_agent_adapter::{ADAPTERS, AgentId};

fn screen(name: &str) -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/composer/{name}.ansi"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn every_supported_agent_reads_typed_pasted_and_erased_composers() {
    use Verdict::{Empty, Holds};
    let cases = [
        (
            AgentId::ClaudeCode,
            "claude",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
        // Two Backspaces leave eleven and a half pasted lines in Codex.
        (
            AgentId::Codex,
            "codex",
            [Empty, Holds, Empty, Holds, Holds, Empty],
        ),
        (
            AgentId::Grok,
            "grok",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
        (
            AgentId::OpenCode,
            "opencode",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
        (
            AgentId::Pi,
            "pi",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
        (
            AgentId::Omp,
            "omp",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
        (
            AgentId::Antigravity,
            "agy",
            [Empty, Holds, Empty, Holds, Empty, Empty],
        ),
    ];
    assert_eq!(
        cases.len(),
        ADAPTERS.len(),
        "every supported agent has captures"
    );
    let steps = [
        "0-empty",
        "1-typed",
        "2-erased",
        "3-pasted",
        "4-pasted-bs2",
        "5-cleared",
    ];
    for (agent, prefix, expected) in cases {
        let composer = agent
            .adapter()
            .composer
            .supported()
            .unwrap_or_else(|| panic!("{prefix}: no composer declaration"));
        for (step, want) in steps.iter().zip(expected) {
            assert_eq!(
                composer.judge(&screen(&format!("{prefix}-{step}"))),
                want,
                "{prefix}-{step}"
            );
        }
    }
}

/// Every declared composer at the pane floor's width, captured on 2026-11-27
/// from the installed agents (Claude Code 2.1.332, Codex 0.172.0, Grok
/// 1.0.92, OpenCode 2.0.29, Pi 1.0.4, omp 18.7.0 and Antigravity 1.5.18)
/// in a private 40-row terminal with a Hide pane's `TERM` and `COLORTERM`:
/// the composer at rest, then `hello 안녕하세요` typed into it. The account
/// and folder names are replaced.
#[test]
fn every_declared_composer_reads_empty_and_typed_at_the_pane_floor() {
    let cases = [
        (AgentId::ClaudeCode, "claude"),
        (AgentId::Codex, "codex"),
        (AgentId::Grok, "grok"),
        (AgentId::OpenCode, "opencode"),
        (AgentId::Pi, "pi"),
        (AgentId::Omp, "omp"),
        (AgentId::Antigravity, "agy"),
    ];
    assert_eq!(
        cases.len(),
        ADAPTERS.len(),
        "every supported agent has captures"
    );
    for (agent, prefix) in cases {
        let composer = agent.adapter().composer.supported().unwrap();
        let name = |step: &str| format!("{prefix}-{PANE_FLOOR_COLUMNS}-columns-{step}");
        assert_eq!(
            composer.judge(&screen(&name("0-empty"))),
            Verdict::Empty,
            "{}",
            name("0-empty")
        );
        assert_eq!(
            composer.judge(&screen(&name("1-typed"))),
            Verdict::Holds,
            "{}",
            name("1-typed")
        );
    }
}

/// Under 27 columns Codex 0.172.0 cuts its dim placeholder at the pane's
/// edge, at a word at 18 columns (`Ask Codex to do`) and inside one at 24
/// and 26 (`… anyth`, `… anythin`): each reads empty. The same words typed
/// are drawn in the default pen and hold text, and so does typed text
/// followed by a dim rest of the placeholder.
#[test]
fn codex_reads_its_cut_placeholder_as_empty_and_typed_words_as_text() {
    let codex = AgentId::Codex.adapter().composer.supported().unwrap();
    for name in [
        "codex-18-columns-0-empty",
        "codex-24-columns-0-empty",
        "codex-26-columns-0-empty",
    ] {
        assert_eq!(codex.explain(&screen(name)), Ok(()), "{name}");
    }
    assert_eq!(
        codex.explain(&screen("codex-18-columns-2-typed-placeholder")),
        Err(Miss::PlaceholderStyle)
    );
    let typed_then_dim = "\x1b[1m›\x1b[0m Ask\x1b[2m Codex to do\x1b[0m\n";
    assert_eq!(codex.explain(typed_then_dim), Err(Miss::PlaceholderStyle));
    let other_then_dim = "\x1b[1m›\x1b[0m hello\x1b[2m Ask Codex\x1b[0m\n";
    assert_eq!(codex.explain(other_then_dim), Err(Miss::NotPlaceholder));
}

/// Grok 1.0.52 under Herdr 0.9.5 at 119 columns on 2026-10-22, after a turn
/// ended: its predicted next prompt stands in the empty composer as italic
/// ghost text (`suggest`), and a console Grok takes for Windows' legacy one
/// (`GROK_FORCE_LEGACY_CONSOLE=1`, as a pane on Windows is) draws `>`
/// for `❯` (`legacy`). Each step as above, then typing the start of the
/// suggestion, which leaves the rest of it as ghost text after the typed
/// letters.
#[test]
fn grok_reads_its_suggestion_as_empty_on_either_console() {
    use Verdict::{Empty, Holds};
    let grok = AgentId::Grok.adapter().composer.supported().unwrap();
    let steps = [
        ("0-suggested", Empty),
        ("1-typed", Holds),
        ("2-erased", Empty),
        ("3-pasted", Holds),
        ("4-pasted-bs2", Empty),
        ("5-cleared", Empty),
        ("6-typed-prefix", Holds),
    ];
    for console in ["suggest", "legacy"] {
        for (step, want) in steps {
            let name = format!("grok-{console}-{step}");
            assert_eq!(grok.judge(&screen(&name)), want, "{name}");
        }
    }
}

/// omp 19.2.1 in the agent journey's pane at 80 columns on Linux on
/// 2026-11-28 (hand run 46094104848), after its last turn ended: the next
/// prompt it predicts stands in the empty composer as ghost text in its
/// theme's `dim` colour, (107,114,128) in the default theme, with no SGR
/// attribute. A theme may paint typed text that colour (`colors.text`), so
/// the same cells can be a draft: both hold text, never empty.
#[test]
fn omp_reads_its_predicted_prompt_and_a_draft_in_the_same_colour_as_text() {
    let omp = AgentId::Omp.adapter().composer.supported().unwrap();
    let predicted = screen("omp-19.2.1-predicted");
    assert_eq!(omp.judge(&predicted), Verdict::Holds);
    let ghost = "\x1b[38;2;107;114;128mJOURNEY-FB3D0CB2";
    assert!(predicted.contains(ghost));
    let themed_draft = predicted.replace(ghost, "\x1b[38;2;107;114;128mhello there");
    assert_eq!(omp.judge(&themed_draft), Verdict::Holds);
}

/// Claude Code 2.1.314 under Herdr 0.9.5 at 60 columns on 2026-10-30. A
/// session named with `--name` or `/rename` draws its name in the composer's
/// top rule (`labelled`), and the same steps run beside a session without a
/// name (`plain`), then after a turn (`6-answered`). `/color green` draws the
/// rule green and the name black on it (`coloured`), and a name too long for
/// the pane loses the rule's run and ends in `…` (`cut-label`).
#[test]
fn claude_reads_its_composer_under_a_labelled_top_rule() {
    use Verdict::{Empty, Holds};
    let claude = AgentId::ClaudeCode.adapter().composer.supported().unwrap();
    let steps = [
        ("0-empty", Empty),
        ("1-typed", Holds),
        ("2-erased", Empty),
        ("3-pasted", Holds),
        ("4-pasted-bs2", Empty),
        ("5-cleared", Empty),
        ("6-answered", Empty),
    ];
    for session in ["labelled", "plain"] {
        for (step, want) in steps {
            let name = format!("claude-2.1.314-{session}-{step}");
            assert_eq!(claude.judge(&screen(&name)), want, "{name}");
        }
    }
    for (name, want) in [
        ("coloured-0-answered", Empty),
        ("coloured-1-typed", Holds),
        ("cut-label-0-empty", Empty),
        ("cut-label-1-typed", Holds),
    ] {
        let name = format!("claude-2.1.314-{name}");
        assert_eq!(claude.judge(&screen(&name)), want, "{name}");
    }
}

/// Antigravity 1.5.11 in an isolated Hide at 80 columns on 2026-11-19
/// (with-hide/hide#2580): the mode hint shift+tab cycles stands in its
/// empty composer in palette grey, so its prompt reads empty with no mode,
/// in accept-edits, in plan and after an interrupted turn, while typed text
/// is drawn in the default colour; and every list or question agy opens
/// under its composer (`/`, `?`, `/resume`, `/model`, a permission or ask
/// question) takes the keys, so a screen showing one has no composer to
/// type into. The trust capture's folder path is replaced with
/// `~/projects/fixture`.
#[test]
fn antigravity_reads_its_mode_hint_as_empty_and_a_list_under_its_composer_as_none() {
    let agy = AgentId::Antigravity.adapter().composer.supported().unwrap();
    for (step, want) in [
        ("0-empty", Ok(())),
        ("accept-edits-hint", Ok(())),
        ("plan-hint", Ok(())),
        // At 50 and 40 columns agy cuts its hint with `...`, and at 40 drops
        // the model from its status line, which stays one row.
        ("50-columns-plan-hint", Ok(())),
        ("40-columns-plan-hint", Ok(())),
        ("interrupted", Ok(())),
        ("typed", Err(Miss::PlaceholderStyle)),
        ("slash-popup", Err(Miss::OpenUnder)),
        ("shortcuts", Err(Miss::OpenUnder)),
        ("session-picker", Err(Miss::OpenUnder)),
        ("model-picker", Err(Miss::OpenUnder)),
        // Its questions are drawn under the composer too, and the
        // folder-trust question at start draws no rules at all.
        ("permission", Err(Miss::OpenUnder)),
        ("question", Err(Miss::OpenUnder)),
        ("trust", Err(Miss::NoRule)),
    ] {
        let name = format!("agy-1.5.11-{step}");
        assert_eq!(agy.explain(&screen(&name)), want, "{name}");
    }
    assert!(agy.holds(&screen("agy-1.5.11-typed"), "hello"));
}

/// What an agent draws under its composer is read only where it is
/// declared. An empty composer reads empty only above exactly the declared
/// rows, each beginning with its words; blank rows never count, and any
/// other row, alone or beside them, is something open under it, such as a
/// notice or a plan approval a capture never showed. A composer holding
/// text may change those rows' words, so it only has no more of them.
#[test]
fn an_empty_composer_reads_empty_only_above_the_rows_its_agent_declares_under_it() {
    let agy = AgentId::Antigravity.adapter().composer.supported().unwrap();
    let grey = |text: &str| format!("\x1b[38;5;8m{text}\x1b[0m");
    let rule = grey(&"─".repeat(40));
    let prompt = format!("\x1b[38;5;12m> \x1b[0m{}", grey("Plan mode: research"));
    let screen = |under: &str| format!("{rule}\n{prompt}\n{rule}\n{under}");
    let status = "? for shortcuts            plan · Gemini 3.1 Pro · low";
    assert_eq!(agy.explain(&screen(status)), Ok(()));
    assert_eq!(agy.explain(&screen("? for shortcuts\n\n\n")), Ok(()));
    for under in [
        "",
        "                           plan · Gemini 3.1 Pro · low",
        "esc to cancel              plan · Gemini 3.1 Pro · low",
        "Update available: run agy update",
        "  Conversations\n? for shortcuts",
        "? for shortcuts\n  Approve this plan?",
    ] {
        assert_eq!(
            agy.explain(&screen(under)),
            Err(Miss::OpenUnder),
            "{under:?}"
        );
    }
    assert_eq!(Miss::OpenUnder.verdict(), Verdict::NotFound);
    // Typed text, which drops `? for shortcuts`, still reads as held.
    let typed = |under: &str| format!("{rule}\n> hello\n{rule}\n{under}");
    let model = "                           plan · Gemini 3.1 Pro · low";
    assert!(agy.holds(&typed(model), "hello"));
    assert!(!agy.holds(&typed(&format!("  Approve this plan?\n{model}")), "hello"));
    // A grey line is the hint; the same words in the default colour were typed.
    assert_eq!(
        agy.explain(&format!("{rule}\n> Plan mode: research\n{rule}\n")),
        Err(Miss::PlaceholderStyle)
    );
    // Claude Code declares nothing under its composer: its lines there are not read.
    let claude = AgentId::ClaudeCode.adapter().composer.supported().unwrap();
    let rule = "─".repeat(40);
    assert_eq!(
        claude.judge(&format!("{rule}\n❯ \n{rule}\none\ntwo\nthree\n")),
        Verdict::Empty
    );
}

/// A label is text between a space and a closing run of `─` drawn in the
/// bottom rule's own pen; typed text that ends in `─` is the operator's.
#[test]
fn only_a_rule_in_the_bottom_rules_pen_carries_a_label() {
    let claude = AgentId::ClaudeCode.adapter().composer.supported().unwrap();
    let grey = |text: &str| format!("\x1b[38;2;136;136;136m{text}\x1b[0m");
    let bottom = grey(&"─".repeat(40));
    let screen = |top: &str, composer: &str| format!("{top}\r\n{composer}\r\n{bottom}\r\n");
    let labelled = grey(&format!("{} name ─", "─".repeat(33)));
    assert_eq!(claude.judge(&screen(&labelled, "❯ ")), Verdict::Empty);
    assert_eq!(claude.judge(&screen(&labelled, "❯ hello")), Verdict::Holds);
    assert_eq!(
        claude.judge(&screen(&grey(" a long na… ─"), "❯ ")),
        Verdict::Empty
    );
    // The same line in another pen, as typed text above an empty first line
    // of a draft would be, is no rule: no composer is found below a rule.
    let typed = format!("{} name ─", "─".repeat(33));
    assert_eq!(claude.judge(&screen(&typed, "❯ ")), Verdict::NotFound);
    assert_eq!(
        claude.judge(&screen(&format!("\x1b[38;5;8m{typed}\x1b[0m"), "❯ ")),
        Verdict::NotFound
    );
    // A label needs its spaces and a closing `─`.
    for top in [
        format!("{}name─", "─".repeat(33)),
        format!("{} name", "─".repeat(33)),
        format!("{}   ─", "─".repeat(33)),
    ] {
        assert_eq!(
            claude.judge(&screen(&grey(&top), "❯ ")),
            Verdict::NotFound,
            "{top}"
        );
    }
}

#[test]
fn a_screen_without_the_declared_composer_is_not_found() {
    for row in ADAPTERS {
        let composer = row.composer.supported().unwrap();
        for screen in ["", "\r\n\r\n", "$ ls\r\nREADME.md\r\n$ "] {
            assert_eq!(
                composer.judge(screen),
                Verdict::NotFound,
                "{}: {screen:?}",
                row.id
            );
        }
    }
}

#[test]
fn only_the_declared_signature_reads_empty() {
    let claude = AgentId::ClaudeCode.adapter().composer.supported().unwrap();
    let rule = "─".repeat(40);
    let composer = |line: &str| format!("{rule}\r\n{line}\r\n{rule}\r\n");
    assert_eq!(claude.judge(&composer("❯ ")), Verdict::Empty);
    assert_eq!(
        claude.judge(&composer("❯ \x1b[2mTry \"fix it\"\x1b[0m")),
        Verdict::Empty
    );
    // A faint colour is not the dim attribute, and a dim line ended by a
    // reset leaves the plain text after it.
    assert_eq!(
        claude.judge(&composer("❯ \x1b[38;2;80;80;80mhello\x1b[0m")),
        Verdict::Holds
    );
    assert_eq!(
        claude.judge(&composer("❯ \x1b[2mTry\x1b[0m hello")),
        Verdict::Holds
    );
    // A second line of a multi-line draft is content even when the first is empty.
    assert_eq!(
        claude.judge(&format!("{rule}\r\n❯ \r\n  more\r\n{rule}\r\n")),
        Verdict::Holds
    );
    // Another program's screen with two rules is not Claude's composer.
    assert_eq!(claude.judge(&composer("$ ")), Verdict::NotFound);

    let codex = AgentId::Codex.adapter().composer.supported().unwrap();
    assert_eq!(
        codex.judge("\x1b[1m› \x1b[0m\x1b[2mAsk Codex to do anything\x1b[0m\r\n\r\n"),
        Verdict::Empty
    );
    // Another dim text in the same place is not the known placeholder.
    assert_eq!(
        codex.judge("\x1b[1m› \x1b[0m\x1b[2mExplain this codebase\x1b[0m\r\n\r\n"),
        Verdict::Holds
    );
    assert_eq!(
        codex.judge("› Ask Codex to do anything\r\n\r\n"),
        Verdict::Holds
    );

    let grok = AgentId::Grok.adapter().composer.supported().unwrap();
    let boxed = |line: &str| format!("╭────╮\r\n│ {line} │\r\n╰────╯\r\n");
    // A faint colour is not italic, and italic switched off leaves plain text.
    assert_eq!(
        grok.judge(&boxed("❯ \x1b[38;2;88;88;88mhello\x1b[0m")),
        Verdict::Holds
    );
    assert_eq!(
        grok.judge(&boxed("> \x1b[3mrun the tests\x1b[23m hello")),
        Verdict::Holds
    );
    assert_eq!(
        grok.judge(&boxed("$ \x1b[3mhello\x1b[0m")),
        Verdict::NotFound
    );
}

/// A screen that does not read empty names the step that stopped it, so a
/// diagnostic can say why without the screen's text.
#[test]
fn a_screen_not_read_empty_names_the_step_that_stopped_it() {
    let claude = AgentId::ClaudeCode.adapter().composer.supported().unwrap();
    let rule = "─".repeat(40);
    let composer = |line: &str| format!("{rule}\r\n{line}\r\n{rule}\r\n");
    for (screen, miss) in [
        (String::new(), Miss::NoRule),
        (format!("{rule}\r\n❯ \r\n"), Miss::NoTopRule),
        (format!("─── name\r\n❯ \r\n{rule}\r\n"), Miss::NoTopRule),
        (composer("$ "), Miss::NoMarker),
        (composer("❯ hello"), Miss::PlaceholderStyle),
    ] {
        assert_eq!(claude.explain(&screen), Err(miss), "{screen:?}");
        assert_eq!(claude.judge(&screen), miss.verdict());
    }
    assert_eq!(
        claude.explain(&screen("claude-2.1.314-labelled-0-empty")),
        Ok(())
    );

    let codex = AgentId::Codex.adapter().composer.supported().unwrap();
    assert_eq!(
        codex.explain("\x1b[1m› \x1b[0m\x1b[2mExplain this codebase\x1b[0m\r\n\r\n"),
        Err(Miss::NotPlaceholder)
    );
    assert_eq!(
        codex.explain("› Ask Codex to do anything\r\n\r\n"),
        Err(Miss::PlaceholderStyle)
    );
    let grok = AgentId::Grok.adapter().composer.supported().unwrap();
    assert_eq!(grok.explain("$ ls\r\n"), Err(Miss::NoRegion));

    for miss in [
        Miss::NoRule,
        Miss::NoMarker,
        Miss::OpenUnder,
        Miss::NotPlaceholder,
    ] {
        assert_eq!(Miss::from_code(miss.code()), Some(miss));
    }
    assert_eq!(Miss::from_code("hello"), None);
}

/// A composer still showing what was typed into it, and only that, is told
/// from one showing anything else (`docs/delivery.md`, Input an agent did not
/// take): typed input that was never submitted gets its Enter.
#[test]
fn a_composer_holds_typed_text_only_when_it_shows_that_text_and_nothing_else() {
    for (agent, name) in [
        (AgentId::ClaudeCode, "claude"),
        (AgentId::Codex, "codex"),
        (AgentId::Grok, "grok"),
        (AgentId::OpenCode, "opencode"),
        (AgentId::Pi, "pi"),
        (AgentId::Omp, "omp"),
        (AgentId::Antigravity, "agy"),
    ] {
        let composer = agent.adapter().composer.supported().expect(name);
        let typed = screen(&format!("{name}-1-typed"));
        assert!(composer.holds(&typed, "hello 안녕하세요"), "{name}");
        // omp's composer is one marked line, which shows only a text's
        // first line; the others show the text however it wraps.
        if agent != AgentId::Omp {
            assert!(
                composer.holds(&typed, "hello\n안녕하세요\n"),
                "{name}: wrapped the same"
            );
        }
        assert!(!composer.holds(&typed, "hello"), "{name}: more than that");
        assert!(
            !composer.holds(&typed, "hello 안녕하세요 again"),
            "{name}: less than that"
        );
        assert!(
            !composer.holds(&screen(&format!("{name}-0-empty")), "hello 안녕하세요"),
            "{name}: empty"
        );
    }
}

/// omp 18.7.0 left a three-line brief in its composer, its Enter lost while
/// its session start ran (captured 2026-10-31 under Herdr 0.9.5): the one
/// line omp's composer is read from is the brief's first line.
#[test]
fn omp_holds_a_brief_left_unsubmitted_by_its_first_line() {
    let composer = AgentId::Omp.adapter().composer.supported().unwrap();
    let held = screen("omp-18.7.0-brief-held");
    let brief =
        "Line one of the brief.\nReply with exactly the token on the next line.\nJOURNEY-31FD51C1";
    assert!(composer.holds(&held, brief));
    assert!(!composer.holds(&held, "Another brief.\nJOURNEY-31FD51C1"));
}

/// A paste the agent keeps as one mark shows in its composer as that mark
/// alone, with the paste's line count in the agent's own unit: twelve lines
/// pasted (the captures above) are held, any other count is another paste,
/// and an agent whose mark has no count, or that shows the text, holds it
/// only as text.
#[test]
fn a_composer_holds_a_paste_by_the_agents_mark_with_its_line_count() {
    let twelve: String = (1..=12)
        .map(|n| format!("composer check line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let thirteen = format!("{twelve}\ncomposer check line 13");
    for (agent, name) in [
        (AgentId::ClaudeCode, "claude"),
        (AgentId::ClaudeCode, "claude-2.1.314-labelled"),
        (AgentId::Grok, "grok"),
        (AgentId::Grok, "grok-legacy"),
        (AgentId::OpenCode, "opencode"),
        (AgentId::Pi, "pi"),
        (AgentId::Antigravity, "agy"),
    ] {
        let composer = agent.adapter().composer.supported().expect(name);
        let pasted = screen(&format!("{name}-3-pasted"));
        assert!(composer.holds(&pasted, &twelve), "{name}");
        assert!(!composer.holds(&pasted, &thirteen), "{name}: another count");
        assert!(
            !composer.holds(&screen(&format!("{name}-5-cleared")), &twelve),
            "{name}: cleared"
        );
    }
    let omp = AgentId::Omp.adapter().composer.supported().unwrap();
    assert!(
        !omp.holds(&screen("omp-3-pasted"), &twelve),
        "no count to match"
    );
}

/// OpenCode 2.0.29 on Windows kept a spawn's eighteen-line brief as its
/// paste mark, the Enter that would submit it lost while its model loaded
/// (agent journey run 41951563729): the mark is that brief, so it gets its
/// Enter, and a brief of another length is not it.
#[test]
fn opencode_holds_a_brief_left_as_its_paste_mark() {
    let composer = AgentId::OpenCode.adapter().composer.supported().unwrap();
    let held = screen("opencode-2.0.29-windows-brief-pasted");
    let notes: Vec<String> = (1..=12)
        .map(|n| format!("- Note {n}: keep `hide agent show` and `--brief-file` as they are."))
        .collect();
    let brief = format!(
        "# Journey brief\n\n## Notes\n{}\n\n## Request\nReply with exactly JOURNEY-1\n",
        notes.join("\n")
    );
    assert!(composer.holds(&held, &brief));
    assert!(!composer.holds(&held, &format!("{brief}One more line.\n")));
}

/// `hide pane send --paste` goes only to an agent the composer cell
/// measured taking a paste whole: every measured agent on every system but
/// Grok on Windows, which sends a paste partway (docs/INSTALL.md).
#[test]
fn every_measured_agent_takes_one_paste_except_grok_on_windows() {
    for agent in [
        AgentId::ClaudeCode,
        AgentId::Codex,
        AgentId::OpenCode,
        AgentId::Pi,
        AgentId::Omp,
        AgentId::Antigravity,
    ] {
        assert!(
            agent.takes_one_paste(false) && agent.takes_one_paste(true),
            "{agent:?}"
        );
    }
    assert!(AgentId::Grok.takes_one_paste(false));
    assert!(!AgentId::Grok.takes_one_paste(true));
    assert!(!AgentId::Cursor.takes_one_paste(false));
}
