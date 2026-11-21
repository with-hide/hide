//! A node's undecided rules must not clear a surviving holder's screen.
use super::*;

fn rules(region: &str) -> ScreenRules {
    ScreenRules {
        agent: "test".into(),
        sets: vec![RuleSet {
            source: "built-in".into(),
            version: "1".into(),
            min_engine_version: 1,
            manifest: serde_json::json!({
                "version": "1", "min_engine_version": 1,
                "rules": [{"id": "retained", "state": "working", "region": region,
                    "contains": [if region == "osc_title" { "retained-title" } else { "4;3;" }]}]
            })
            .to_string(),
        }],
    }
}

fn working(facts: &PaneFacts) -> bool {
    facts.screen
        == ScreenRead::Read {
            state: ScreenState::Working,
            rule: Some("retained".into()),
        }
}

#[test]
fn a_recovered_node_preserves_rules_until_an_explicit_decision_and_can_remove_them() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    node.holders().watch_facts(told.sink());
    let script = Script::new(
        &folder.home,
        vec![b"\x1b]0;retained-title\x07\x1b]9;4;3;\x07".to_vec()],
    );
    let panes = [("title", "osc_title"), ("progress", "osc_progress")];
    for (pane, region) in panes {
        node.start_pane(&request(
            folder.home.path(),
            pane,
            role_program("script"),
            vec![
                (ROLE.into(), "script".into()),
                (SCRIPT.into(), script.folder.clone().into()),
            ],
        ));
        told.wait(pane, "the retained title", |facts| {
            facts.title.as_deref() == Some("retained-title")
        });
        node.holders().set_rules(pane, Some(rules(region))).unwrap();
        told.wait(pane, "the retained screen state", working);
    }
    let before: Vec<_> = panes.iter().map(|(pane, _)| folder.record(pane)).collect();
    drop(node);
    let next = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    next.holders().watch_facts(told.sink());
    let recovered = next.holders().recover(next.owner(), &next.service);
    assert!(recovered.complete);
    assert_eq!(recovered.attached, ["progress", "title"]);
    for ((pane, region), before) in panes.into_iter().zip(before) {
        told.wait(
            pane,
            "the surviving rules before any core decision",
            working,
        );
        assert_eq!(
            folder.record(pane),
            before,
            "recovery keeps the holder and program identity"
        );
        // Require a fresh response to the same decision, not cached facts.
        told.facts
            .lock()
            .unwrap()
            .retain(|(named, _)| named != pane);
        next.holders().set_rules(pane, Some(rules(region))).unwrap();
        told.wait(
            pane,
            "the same rules without clearing retained state",
            working,
        );
        next.holders().set_rules(pane, None).unwrap();
        told.wait(pane, "explicit removal", |facts| {
            facts.screen == ScreenRead::NoRules
        });
        // Restoring the rules without new PTY output cannot restore the
        // title/progress a deliberate agent removal cleared.
        next.holders().set_rules(pane, Some(rules(region))).unwrap();
        told.wait(pane, "the retained state was cleared", |facts| {
            facts.screen
                == ScreenRead::Read {
                    state: ScreenState::Unknown,
                    rule: None,
                }
        });
        next.holders()
            .terminate(next.owner(), &next.service, pane)
            .unwrap();
    }
}
