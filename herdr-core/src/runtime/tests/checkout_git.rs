use super::*;
use crate::changes::ChannelRef;
use crate::checkout_git::{Ci, CiState, GitRequest, GitState, Header};
use hide_node_link::worktrees::CheckoutGit;

fn target(runtime: &Runtime, generation: u64) -> GitRequest {
    GitRequest {
        root: crate::files::DocumentRoot {
            device_id: crate::node::TEST_NODE.into(),
            path: "/fixture".into(),
            identity: None,
        },
        channel: Ok(ChannelRef(runtime.own_node.clone())),
        generation,
    }
}

#[test]
fn header_fences_late_git_ci_and_reconnect_results_and_ages_without_false_clean() {
    let mut runtime = runtime();
    let now = Instant::now();
    let first = target(&runtime, 1);
    runtime.checkout_header.target = Some(first.clone());
    runtime.checkout_header.header = Some(Header {
        state: GitState::Loading,
        facts: None,
        reason: None,
        ci: None,
        recent_ci: Vec::new(),
        remote: None,
        commit_links: Vec::new(),
        commit_pages: false,
        observed: None,
    });
    let facts = CheckoutGit {
        head_sha: Some("a".repeat(40)),
        origin_url: Some("https://github.com/example/fixture.git".into()),
        upstream_state: "none".into(),
        changed_files: 3,
        ..Default::default()
    };
    assert!(runtime.ingest_checkout_git((first.clone(), Ok(facts.clone())), now));
    let ci_target = runtime.checkout_ci_request().unwrap();
    let ci = Ci {
        sha: "a".repeat(40),
        state: CiState::Passing,
        observed: Some(crate::observed::Observation::fresh(100)),
        url: Some("https://example.invalid/check".into()),
    };
    assert!(runtime.ingest_checkout_ci((ci_target.clone(), Ok(vec![ci.clone()])), now));
    assert!(
        !runtime.ingest_checkout_git((first.clone(), Ok(facts.clone())), now),
        "identical status publishes no frame"
    );
    let mut next = facts.clone();
    next.head_sha = Some("b".repeat(40));
    assert!(runtime.ingest_checkout_git((first.clone(), Ok(next.clone())), now));
    assert!(
        !runtime.ingest_checkout_ci((ci_target, Ok(vec![ci])), now),
        "old SHA success cannot overwrite a new HEAD"
    );
    let header = runtime.checkout_header.header.as_ref().unwrap();
    assert_eq!(header.ci.as_ref().unwrap().state, CiState::Loading);
    assert!(header.ci.as_ref().unwrap().url.is_none());
    assert!(!runtime.age_checkout_header(now + std::time::Duration::from_secs(5)));
    assert!(runtime.age_checkout_header(now + std::time::Duration::from_millis(5001)));
    assert_eq!(
        runtime.checkout_header.header.as_ref().unwrap().state,
        GitState::Stale
    );
    assert!(
        !runtime.age_checkout_header(now + std::time::Duration::from_secs(6)),
        "stale publishes once"
    );
    let second = target(&runtime, 2);
    runtime.checkout_header.target = Some(second.clone());
    assert!(
        !runtime.ingest_checkout_git((first, Ok(facts)), now),
        "returning to a path does not accept its previous generation"
    );
    assert!(runtime.ingest_checkout_git((second.clone(), Err("offline".into())), now));
    assert_eq!(
        runtime
            .checkout_header
            .header
            .as_ref()
            .unwrap()
            .facts
            .as_ref()
            .unwrap()
            .changed_files,
        3
    );
    assert!(runtime.ingest_checkout_git((second, Ok(next)), now));
    assert_eq!(
        runtime.checkout_header.header.as_ref().unwrap().state,
        GitState::Fresh
    );
}

#[test]
fn header_keeps_provider_identity_without_exposing_origin_credentials_to_the_renderer() {
    let mut runtime = runtime();
    let request = target(&runtime, 1);
    runtime.checkout_header.target = Some(request.clone());
    runtime.checkout_header.header = Some(Header {
        state: GitState::Loading,
        facts: None,
        reason: None,
        ci: None,
        recent_ci: Vec::new(),
        remote: None,
        commit_links: Vec::new(),
        commit_pages: false,
        observed: None,
    });
    let facts = CheckoutGit {
        head_sha: Some("a".repeat(40)),
        origin_url: Some(
            "https://fixture-user:fixture-secret@github.com/example/fixture.git".into(),
        ),
        ..Default::default()
    };
    let node_wire = serde_json::to_string(&facts).unwrap();
    assert!(
        node_wire.contains("fixture-secret"),
        "node answers retain the configured URL until core ingestion"
    );
    assert!(runtime.ingest_checkout_git((request.clone(), Ok(facts.clone())), Instant::now()));
    assert_eq!(
        runtime.checkout_ci_request().unwrap().repository,
        "example/fixture"
    );
    let public = serde_json::to_value(
        runtime
            .checkout_header_snapshot(&(request.root.device_id.clone(), request.root.path.clone()))
            .unwrap(),
    )
    .unwrap();
    assert!(public["facts"].get("origin_url").is_none());
    assert!(!public.to_string().contains("fixture-secret"));
    assert!(
        !runtime.ingest_checkout_git((request, Ok(facts)), Instant::now()),
        "repeated node answers still publish no duplicate frame"
    );
}

#[test]
fn history_ci_is_fenced_by_every_ordered_sha_and_all_links_expire() {
    let mut runtime = runtime();
    let now = Instant::now();
    let request = target(&runtime, 1);
    runtime.checkout_header.target = Some(request.clone());
    runtime.checkout_header.header = Some(Header {
        state: GitState::Loading,
        facts: None,
        reason: None,
        ci: None,
        recent_ci: Vec::new(),
        remote: None,
        commit_links: Vec::new(),
        commit_pages: false,
        observed: None,
    });
    let mut facts = CheckoutGit {
        head_sha: Some("a".repeat(40)),
        origin_url: Some("https://github.com/example/fixture.git".into()),
        ..Default::default()
    };
    facts.recent_commits = ['a', 'b', 'c', 'd', 'e']
        .into_iter()
        .map(|name| hide_node_link::worktrees::CheckoutCommit {
            sha: name.to_string().repeat(40),
            subject: name.to_string(),
            commit_unix_seconds: 1,
        })
        .collect();
    assert!(runtime.ingest_checkout_git((request.clone(), Ok(facts.clone())), now));
    let ci_request = runtime.checkout_ci_request().unwrap();
    assert_eq!(ci_request.commits.len(), 5);
    let checks: Vec<_> = ci_request
        .commits
        .iter()
        .map(|sha| Ci {
            sha: sha.clone(),
            state: CiState::Failed,
            observed: Some(crate::observed::Observation::fresh(1)),
            url: Some(format!("https://github.com/example/fixture/commit/{sha}")),
        })
        .collect();
    let mut wrong_order = checks.clone();
    wrong_order.swap(1, 2);
    assert!(!runtime.ingest_checkout_ci((ci_request.clone(), Ok(wrong_order)), now));
    assert!(runtime.ingest_checkout_ci((ci_request.clone(), Ok(checks.clone())), now));
    facts.recent_commits[4].sha = "f".repeat(40);
    assert!(runtime.ingest_checkout_git((request.clone(), Ok(facts.clone())), now));
    assert!(
        !runtime.ingest_checkout_ci((ci_request, Ok(checks)), now),
        "same HEAD with changed ancestors rejects old history checks"
    );
    let next_request = runtime.checkout_ci_request().unwrap();
    let next: Vec<_> = next_request
        .commits
        .iter()
        .map(|sha| Ci {
            sha: sha.clone(),
            state: CiState::Passing,
            observed: Some(crate::observed::Observation::fresh(1)),
            url: Some(format!("https://github.com/example/fixture/commit/{sha}")),
        })
        .collect();
    assert!(runtime.ingest_checkout_ci((next_request.clone(), Ok(next)), now));
    // GitHub could not be read: every result stays, marked as a failed read
    // of the value from when it was read (B22).
    assert!(runtime.ingest_checkout_ci((next_request, Err("offline".into())), now));
    assert!(
        runtime
            .checkout_header
            .header
            .as_ref()
            .unwrap()
            .recent_ci
            .iter()
            .all(|ci| ci.state == CiState::Passing
                && ci.url.is_some()
                && ci.observed
                    == Some(crate::observed::Observation {
                        freshness: crate::observed::Freshness::Failed,
                        read_at_unix_ms: Some(1),
                    }))
    );
    // Aging keeps a failure a failure.
    runtime.age_checkout_header(now + std::time::Duration::from_secs(91));
    assert!(
        runtime
            .checkout_header
            .header
            .as_ref()
            .unwrap()
            .recent_ci
            .iter()
            .all(|ci| ci
                .observed
                .is_some_and(|observed| observed.freshness == crate::observed::Freshness::Failed))
    );
    facts
        .recent_commits
        .push(hide_node_link::worktrees::CheckoutCommit {
            sha: "0".repeat(40),
            subject: "sixth".into(),
            commit_unix_seconds: 1,
        });
    runtime.ingest_checkout_git((request, Ok(facts)), now);
    assert_eq!(
        runtime.checkout_header.header.as_ref().unwrap().state,
        GitState::Stale
    );
    assert_eq!(
        runtime
            .checkout_header
            .header
            .as_ref()
            .unwrap()
            .facts
            .as_ref()
            .unwrap()
            .recent_commits
            .len(),
        5
    );
}
