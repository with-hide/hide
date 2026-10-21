//! Each agent's composer declaration against screens captured from the real
//! agent under Herdr 0.9.5 at 60 columns on 2026-10-21: type `hello
//! 안녕하세요`, Backspace it all, paste 12 lines, Backspace twice, clear
//! (`docs/delivery.md`, Reading the composer). The expected verdict is what
//! the operator saw in the composer at each step.

use std::path::Path;

use hide_agent_adapter::composer::Verdict;
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
}
