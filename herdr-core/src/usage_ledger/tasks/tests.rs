use super::*;

const HOUR: u64 = 3_600_000;

fn spend(session: &str, micro_usd: u64, first_ms: u64, last_ms: u64) -> SessionSpend {
    SessionSpend {
        agent: "claude".into(),
        session_id: session.into(),
        micro_usd,
        unpriced: false,
        tokens: micro_usd / 10,
        first_ms,
        last_ms,
        worked_ms: (last_ms - first_ms) / 2,
    }
}

fn recorded(parent: Option<&str>, made: &[u32], cwd: &str) -> Recorded {
    Recorded {
        agent: "claude".into(),
        parent: parent.map(str::to_owned),
        made: made.to_vec(),
        worked_on: Vec::new(),
        cwd: Some(cwd.into()),
        line: None,
        committed_at_unix_ms: None,
    }
}

fn ids(tasks: &[TaskCost]) -> Vec<(&str, u64)> {
    tasks
        .iter()
        .map(|task| (task.id.as_str(), task.micro_usd))
        .collect()
}

/// D-09, B28: a task is its top session with children and grandchildren; a
/// child that made its own pull request stands alone; every session counts
/// once, so the tasks add up to the spends.
#[test]
fn a_task_holds_its_descendants_and_a_child_with_its_own_pull_request_stands_alone() {
    let spends = [
        spend("root", 4, 0, HOUR),
        spend("child", 2, HOUR, 2 * HOUR),
        spend("grandchild", 1, HOUR, 2 * HOUR),
        spend("maker", 8, 0, HOUR),
        spend("maker-child", 3, 0, HOUR),
    ];
    let facts = Facts {
        recorded: HashMap::from([
            ("root".into(), recorded(None, &[], "/p")),
            ("child".into(), recorded(Some("root"), &[], "/p")),
            ("grandchild".into(), recorded(Some("child"), &[], "/p")),
            ("maker".into(), recorded(Some("root"), &[7], "/p/w")),
            ("maker-child".into(), recorded(Some("maker"), &[], "/p/w")),
        ]),
        ..Facts::default()
    };
    let Built { tasks, total, .. } = build(&spends, &facts);
    assert_eq!(ids(&tasks), [("task:maker", 11), ("task:root", 7)]);
    assert_eq!(total, 2);
    assert_eq!(
        tasks[1]
            .sessions
            .iter()
            .map(|s| (s.session_id.as_str(), s.depth))
            .collect::<Vec<_>>(),
        [("root", 0), ("child", 1), ("grandchild", 2)]
    );
    assert_eq!(
        tasks.iter().map(|t| t.micro_usd).sum::<u64>(),
        spends.iter().map(|s| s.micro_usd).sum::<u64>()
    );
    assert_eq!(
        (
            tasks[0].result.kind,
            tasks[0].result.number,
            tasks[0].result.state
        ),
        ("pr", Some(7), None)
    );
}

/// D-13, B31: a mission is one task, its workers' own pull requests
/// included, each node with what its worker spent and how often it ran.
#[test]
fn a_mission_is_one_task_with_each_node_s_worker_cost() {
    let spends = [
        spend("conductor", 1, 0, HOUR),
        spend("worker-a", 5, 0, HOUR),
        spend("worker-a-check", 2, 0, HOUR),
        spend("worker-b", 3, 0, HOUR),
    ];
    let facts = Facts {
        recorded: HashMap::from([
            ("conductor".into(), recorded(None, &[], "/p")),
            (
                "worker-a".into(),
                recorded(Some("conductor"), &[11], "/p/a"),
            ),
            (
                "worker-a-check".into(),
                recorded(Some("worker-a"), &[], "/p/a"),
            ),
            (
                "worker-b".into(),
                recorded(Some("conductor"), &[12], "/p/b"),
            ),
        ]),
        missions: vec![Mission {
            id: "m1".into(),
            title: "Ship export".into(),
            conductor: Some("conductor".into()),
            nodes: vec![
                MissionNode {
                    key: "a".into(),
                    title: "A".into(),
                    pr: Some(11),
                    attempts: 2,
                    reopened: 1,
                },
                MissionNode {
                    key: "b".into(),
                    title: "B".into(),
                    pr: Some(12),
                    attempts: 1,
                    reopened: 0,
                },
            ],
            started_at_unix_ms: Some(0),
        }],
        pulls: HashMap::from([
            (
                11,
                Pull {
                    state: "merged",
                    merged_at_unix_ms: Some(HOUR),
                    merged_day: Some("d1".into()),
                },
            ),
            (
                12,
                Pull {
                    state: "open",
                    merged_at_unix_ms: None,
                    merged_day: None,
                },
            ),
        ]),
        ..Facts::default()
    };
    let Built { tasks, week, .. } = build(&spends, &facts);
    assert_eq!(ids(&tasks), [("mission:m1", 11)]);
    let mission = tasks[0].mission.as_ref().unwrap();
    assert_eq!(
        (tasks[0].kind, mission.prs, mission.merged, mission.reopened),
        ("mission", 2, 1, 1)
    );
    assert_eq!(
        mission
            .nodes
            .iter()
            .map(|n| (n.key.as_str(), n.micro_usd, n.attempts))
            .collect::<Vec<_>>(),
        [("a", 7, 2), ("b", 3, 1)]
    );
    assert_eq!((week.missions, week.mission_reopened), (1, 1));
}

/// B30, D-20, D-21: working before a pull request, a pull request before
/// changes left, changes (the last task in that checkout only) before a
/// recorded `git commit` that landed, and none otherwise, with what was said
/// last: the record's line before the live row's.
#[test]
fn a_result_follows_the_order_working_pr_changes_commit_none() {
    let spends = [
        spend("busy", 7, 0, HOUR),
        spend("pr", 6, 0, HOUR),
        spend("dirty", 5, HOUR, 2 * HOUR),
        spend("before-dirty", 4, 0, HOUR / 2),
        spend("committed", 3, 0, HOUR),
        spend("idle", 2, 0, HOUR),
        spend("quiet", 1, 0, HOUR),
    ];
    let committed = |cwd: &str| Recorded {
        committed_at_unix_ms: Some(HOUR / 2),
        ..recorded(None, &[], cwd)
    };
    let facts = Facts {
        recorded: HashMap::from([
            ("busy".into(), recorded(None, &[9], "/p/busy")),
            ("pr".into(), recorded(None, &[9], "/p/pr")),
            ("dirty".into(), recorded(None, &[], "/p/dirty")),
            ("before-dirty".into(), recorded(None, &[], "/p/dirty")),
            ("committed".into(), committed("/p")),
            ("idle".into(), committed("/p/unmerged")),
            (
                "quiet".into(),
                Recorded {
                    line: Some("Stopped before the migration".into()),
                    ..recorded(None, &[], "/elsewhere")
                },
            ),
        ]),
        working: HashSet::from(["busy".into()]),
        last_lines: HashMap::from([
            ("idle".into(), "Gave up on the flaky test".into()),
            ("quiet".into(), "A live row line the record outranks".into()),
        ]),
        pulls: HashMap::from([(
            9,
            Pull {
                state: "merged",
                merged_at_unix_ms: Some(3 * HOUR),
                merged_day: Some("d2".into()),
            },
        )]),
        places: vec![
            Place {
                path: "/p".into(),
                left: false,
                landed: true,
            },
            Place {
                path: "/p/dirty".into(),
                left: true,
                landed: false,
            },
            Place {
                path: "/p/unmerged".into(),
                left: false,
                landed: false,
            },
        ],
        ..Facts::default()
    };
    let Built { tasks, week, .. } = build(&spends, &facts);
    let kinds: Vec<_> = tasks
        .iter()
        .map(|t| (t.root.session_id.as_str(), t.result.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("busy", "working"),
            ("pr", "pr"),
            ("dirty", "changes"),
            ("before-dirty", "none"),
            ("committed", "commit"),
            ("idle", "none"),
            ("quiet", "none")
        ]
    );
    assert_eq!(
        tasks[1].elapsed_ms,
        3 * HOUR,
        "a merged task runs to its merge"
    );
    assert_eq!(
        tasks[5].last_line.as_deref(),
        Some("Gave up on the flaky test")
    );
    assert_eq!(
        tasks[6].last_line.as_deref(),
        Some("Stopped before the migration")
    );
    assert_eq!((week.no_result_tasks, week.no_result_micro_usd), (3, 7));
}

/// B21: merged pull requests by day, and the averages over the tasks whose
/// pull request merged in the seven days.
#[test]
fn the_week_counts_merges_by_day_and_averages_over_merged_tasks() {
    let spends = [
        spend("a", 6, 0, HOUR),
        spend("b", 2, 0, HOUR),
        spend("c", 100, 0, HOUR),
    ];
    let facts = Facts {
        recorded: HashMap::from([
            ("a".into(), recorded(None, &[1], "/p")),
            ("b".into(), recorded(None, &[2], "/p")),
            ("c".into(), recorded(None, &[], "/p")),
        ]),
        pulls: HashMap::from([
            (
                1,
                Pull {
                    state: "merged",
                    merged_at_unix_ms: Some(HOUR),
                    merged_day: Some("d1".into()),
                },
            ),
            (
                2,
                Pull {
                    state: "merged",
                    merged_at_unix_ms: Some(11 * HOUR),
                    merged_day: Some("d2".into()),
                },
            ),
            (
                3,
                Pull {
                    state: "merged",
                    merged_at_unix_ms: Some(12 * HOUR),
                    merged_day: Some("d2".into()),
                },
            ),
        ]),
        days: vec!["d1".into(), "d2".into()],
        open_prs: 4,
        ..Facts::default()
    };
    let Built { week, .. } = build(&spends, &facts);
    assert_eq!(
        (week.merged_prs, week.open_prs, week.merged_by_day.clone()),
        (3, 4, vec![1, 2])
    );
    assert_eq!(
        week.per_pr.map(|p| p.micro_usd),
        Some(4),
        "the task with no pull request is in no average"
    );
}

/// B34: past the cap the cheapest tasks go, and the total keeps them.
#[test]
fn tasks_past_the_cap_are_counted_but_not_listed() {
    let spends: Vec<_> = (0..TASK_LIMIT as u64 + 3)
        .map(|index| spend(&format!("s{index}"), index + 1, 0, HOUR))
        .collect();
    let built = build(&spends, &Facts::default());
    assert_eq!(
        (built.tasks.len(), built.total, built.capped),
        (TASK_LIMIT, TASK_LIMIT + 3, true)
    );
    assert_eq!(built.tasks.last().unwrap().micro_usd, 4);
}

/// D-09: a pull request a task only worked on stands for it when it made
/// none, and never when it made its own.
#[test]
fn a_pull_request_worked_on_counts_only_for_a_task_that_made_none() {
    let spends = [spend("maker", 2, 0, HOUR), spend("reviewer", 1, 0, HOUR)];
    let facts = Facts {
        recorded: HashMap::from([
            (
                "maker".into(),
                Recorded {
                    worked_on: vec![1900],
                    ..recorded(None, &[50], "/p")
                },
            ),
            (
                "reviewer".into(),
                Recorded {
                    worked_on: vec![1900],
                    ..recorded(None, &[], "/p")
                },
            ),
        ]),
        ..Facts::default()
    };
    let Built { tasks, capped, .. } = build(&spends, &facts);
    assert_eq!(
        tasks
            .iter()
            .map(|t| (t.root.session_id.as_str(), t.result.number))
            .collect::<Vec<_>>(),
        [("maker", Some(50)), ("reviewer", Some(1900))]
    );
    assert!(!capped);
}
