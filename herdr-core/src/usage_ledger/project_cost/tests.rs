use super::*;
use hide_session::usage::Tokens;

fn call(
    agent: &str,
    day: &str,
    session: &str,
    cwd: Option<&str>,
    model: &str,
    micro_usd: Option<u64>,
) -> Call {
    Call {
        agent: agent.into(),
        day: day.into(),
        event: Event {
            key: format!("{session}-{day}-{model}"),
            at_ms: 0,
            model: model.into(),
            tokens: Tokens {
                input: 1000,
                output: 100,
                ..Tokens::default()
            },
            recorded_cost_micro_usd: micro_usd,
            session: session.into(),
            cwd: cwd.map(str::to_owned),
            provider: None,
            incomplete: false,
        },
    }
}

fn projects() -> Vec<ProjectRoots> {
    vec![
        ProjectRoots {
            workspace_id: "hide".into(),
            roots: vec!["/work/hide".into(), "/work/hide.worktrees/a".into()],
        },
        ProjectRoots {
            workspace_id: "nested".into(),
            roots: vec!["/work/hide/site".into()],
        },
    ]
}

fn today() -> Date {
    "2026-10-23".parse().unwrap()
}

/// PRD lane-two-columns B31, D-19: the seven days end today, each day and
/// model is priced from the recorded calls of sessions inside the project's
/// checkouts, the deepest checkout decides, and a call outside every
/// checkout, older than the seven days or of an agent with no price counts
/// for no amount.
#[test]
fn a_project_s_cost_sums_its_checkouts_calls_of_the_last_seven_days() {
    let calls = vec![
        call(
            "claude",
            "2026-10-23",
            "s1",
            Some("/work/hide"),
            "opus",
            Some(3_000_000),
        ),
        call(
            "claude",
            "2026-10-22",
            "s1",
            Some("/work/hide/src"),
            "opus",
            Some(1_000_000),
        ),
        call(
            "codex",
            "2026-10-17",
            "s2",
            Some("/work/hide.worktrees/a"),
            "gpt",
            Some(2_000_000),
        ),
        call(
            "codex",
            "2026-10-16",
            "s2",
            Some("/work/hide.worktrees/a"),
            "gpt",
            Some(9_000_000),
        ),
        call(
            "claude",
            "2026-10-23",
            "s3",
            Some("/work/hide/site"),
            "opus",
            Some(500_000),
        ),
        call(
            "claude",
            "2026-10-23",
            "s4",
            Some("/elsewhere"),
            "opus",
            Some(7_000_000),
        ),
        call("claude", "2026-10-23", "s5", None, "opus", Some(7_000_000)),
        call(
            "opencode",
            "2026-10-21",
            "s6",
            Some("/work/hide"),
            "glm",
            None,
        ),
    ];
    let costs = prepare(&calls, false, &projects(), None, today(), 0).unwrap();
    let hide = costs.iter().find(|c| c.workspace_id == "hide").unwrap();
    assert_eq!(hide.micro_usd, 6_000_000);
    assert_eq!(
        hide.days
            .iter()
            .map(|d| (d.date.as_str(), d.micro_usd))
            .collect::<Vec<_>>(),
        [
            ("2026-10-17", 2_000_000),
            ("2026-10-18", 0),
            ("2026-10-19", 0),
            ("2026-10-20", 0),
            ("2026-10-21", 0),
            ("2026-10-22", 1_000_000),
            ("2026-10-23", 3_000_000),
        ]
    );
    assert_eq!(
        hide.sessions
            .iter()
            .map(|s| (s.session_id.as_str(), s.micro_usd))
            .collect::<Vec<_>>(),
        [("s1", 4_000_000), ("s2", 2_000_000)]
    );
    assert_eq!(
        hide.agents
            .iter()
            .map(|a| (a.agent.as_str(), a.micro_usd))
            .collect::<Vec<_>>(),
        [("claude", 4_000_000), ("codex", 2_000_000)]
    );
    assert_eq!(hide.models[0].model.as_deref(), Some("opus"));
    assert_eq!(
        hide.unpriced_agents,
        ["opencode"],
        "an agent with no price is named apart"
    );
    let nested = costs.iter().find(|c| c.workspace_id == "nested").unwrap();
    assert_eq!(nested.micro_usd, 500_000, "the deepest checkout decides");
}

/// A project with no call in the seven days has no cost to show.
#[test]
fn a_project_without_calls_has_no_cost() {
    let calls = vec![call(
        "claude",
        "2026-10-01",
        "s1",
        Some("/work/hide"),
        "opus",
        Some(1),
    )];
    assert!(
        prepare(&calls, false, &projects(), None, today(), 0)
            .unwrap()
            .is_empty()
    );
}
