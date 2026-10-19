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
        checked_at_unix_ms: Some(100),
        url: Some("https://example.invalid/check".into()),
    };
    assert!(runtime.ingest_checkout_ci((ci_target.clone(), ci.clone()), now));
    assert!(
        !runtime.ingest_checkout_git((first.clone(), Ok(facts.clone())), now),
        "identical status publishes no frame"
    );
    let mut next = facts.clone();
    next.head_sha = Some("b".repeat(40));
    assert!(runtime.ingest_checkout_git((first.clone(), Ok(next.clone())), now));
    assert!(
        !runtime.ingest_checkout_ci((ci_target, ci), now),
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
