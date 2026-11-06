//! A phone's pull-request requests (PRD mobile-companion-ui B38-B44, D-12,
//! D-13): read a listed pull request's card, merge it, read the feedback a
//! hand-over's prompt names, and hand it to an agent. Each is the one core
//! event the desktop's PR card sends (`pr_card_read`, `pr_merge`,
//! `pr_feedback_read`, `pr_delegate`), built only for a pull request the
//! phone's own `prs` frame lists, so a phone never names a project, a path or
//! a repository: it names the list's key.
//!
//! Which reason keeps Merge off, and what a hand-over's prompt says, are the
//! web's own rules, run on the phone (`prCardModel.ts`, `prDelegate.ts`).
//! What stays here is the scope: a listed, open pull request, the head
//! commit the card showed, a method the card offered, and a checkout that
//! already holds the branch.

use herdr_core::agent_state::phone::{PhonePr, PhonePrs};
use serde_json::{Value, json};

use super::start;

/// The longest list key a phone may name.
const KEY_CHARS: usize = 256;

/// The listed pull request a message names.
fn listed<'a>(message: &Value, prs: &'a PhonePrs) -> Result<&'a PhonePr, &'static str> {
    message
        .get("pr")
        .and_then(Value::as_str)
        .filter(|key| key.chars().count() <= KEY_CHARS)
        .and_then(|key| prs.find(key))
        .ok_or("gone")
}

fn open(pr: &PhonePr) -> Result<(), &'static str> {
    if pr.pr.badge.is_settled() {
        Err("settled")
    } else {
        Ok(())
    }
}

fn event(kind: &str, payload: Value) -> Value {
    json!({"schema_version": herdr_core::SCHEMA_VERSION, "kind": kind, "payload": payload})
}

fn bytes(event: Value) -> Vec<u8> {
    event.to_string().into_bytes()
}

/// `pr_card_read`: what the desktop's card asks when it opens. `look_again`
/// is the desktop's own `Look again` after a refused write.
pub fn card_read(message: &Value, prs: &PhonePrs) -> Result<Vec<u8>, &'static str> {
    let pr = listed(message, prs)?;
    let look_again = match message.get("look_again") {
        None => false,
        Some(Value::Bool(look_again)) => *look_again,
        Some(_) => return Err("invalid_request"),
    };
    Ok(bytes(event(
        "pr_card_read",
        json!({"workspace_id": pr.project, "pr_number": pr.pr.number, "look_again": look_again}),
    )))
}

/// `pr_merge`: the head commit and the method are the card's own, so a merge
/// goes out only for what the phone was shown. The core refuses a head the
/// list moved past and GitHub refuses a head it moved past, as for the
/// desktop's card.
pub fn merge(message: &Value, prs: &PhonePrs) -> Result<Vec<u8>, &'static str> {
    let pr = listed(message, prs)?;
    open(pr)?;
    let head = message
        .get("head")
        .and_then(Value::as_str)
        .ok_or("invalid_request")?;
    let Some(card) = pr.card.as_ref() else {
        return Err("unread");
    };
    let Some(shown) = card.head.as_deref() else {
        return Err("unread");
    };
    if !shown.eq_ignore_ascii_case(head) {
        return Err("head_moved");
    }
    let Some(merge) = card.merge.as_ref() else {
        return Err("unread");
    };
    let method = match message.get("method") {
        None | Some(Value::Null) => None,
        Some(Value::String(method)) => Some(method.as_str()),
        Some(_) => return Err("invalid_request"),
    };
    // A repository with a merge queue takes no method; any other takes one the
    // card offered.
    match (merge.queue, method) {
        (true, None) => {}
        (false, Some(method)) if merge.methods.iter().any(|offered| offered == method) => {}
        _ => return Err("invalid_request"),
    }
    let mut payload = json!({"workspace_id": pr.project, "pr_number": pr.pr.number, "head": head});
    if let Some(method) = method {
        payload["method"] = json!(method);
    }
    Ok(bytes(event("pr_merge", payload)))
}

/// `pr_feedback_read`: the failed checks and change requests a hand-over's
/// first prompt names, answered in the `prs` frame under this request id.
pub fn feedback(
    message: &Value,
    prs: &PhonePrs,
    request_id: &str,
) -> Result<Vec<u8>, &'static str> {
    let pr = listed(message, prs)?;
    open(pr)?;
    Ok(bytes(event(
        "pr_feedback_read",
        json!({"request_id": request_id, "workspace_id": pr.project, "pr_number": pr.pr.number}),
    )))
}

/// `pr_delegate` with `stay_behind`: the desktop card's hand-over, started
/// where it would start (the checkout that holds the branch, else a new
/// worktree of it), with the desktop's screen, tab and keyboard where they are.
pub fn delegate(message: &Value, prs: &PhonePrs, request_id: &str) -> Result<Value, &'static str> {
    let pr = listed(message, prs)?;
    open(pr)?;
    let text = |field: &str| message.get(field).and_then(Value::as_str);
    let (Some(prompt), Some(kind)) = (text("prompt"), text("kind")) else {
        return Err("invalid_request");
    };
    if let Some(problem) = start::prompt_problem(prompt) {
        return Err(problem);
    }
    let kind = hide_agent_adapter::start_kind(kind).ok_or("unknown_kind")?;
    let model = match message.get("model") {
        None | Some(Value::Null) => None,
        Some(Value::String(model)) if start::model_valid(model) => Some(model.as_str()),
        Some(Value::String(_)) => return Err("unknown_model"),
        Some(_) => return Err("invalid_request"),
    };
    let mut payload = json!({
        "workspace_id": pr.project, "pr_number": pr.pr.number, "provider": kind,
        "prompt": prompt, "request_id": request_id, "stay_behind": true,
    });
    if let Some(model) = model {
        payload["model"] = json!(model);
    }
    Ok(event("pr_delegate", payload))
}

#[cfg(test)]
pub(super) mod fixture {
    use herdr_core::agent_state::phone::{PhonePrs, project};
    use serde_json::{Value, json};

    pub const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn pull_request(number: u32, badge: &str) -> Value {
        json!({
            "closing_issues": [], "title": format!("pull {number}"), "checks": "passing", "number": number,
            "head_branch": format!("b{number}"), "base_branch": "main",
            "url": format!("https://github.com/with-hide/hide/pull/{number}"), "badge": badge,
            "review": null, "is_draft": false, "merged_at_unix_ms": null, "created_at_unix_ms": null,
            "updated_at_unix_ms": null,
            "work": {
                "author": "me", "author_is_bot": false, "mine": true, "merge_state": "clean",
                "review_requests": [], "asked_of_me": "none", "failed_checks": [], "pending_checks": 0,
                "queue": {"state": "not_queued"}, "review_requested_at_unix_ms": null,
            },
        })
    }

    fn row(number: u32, group: &str, kind: &str, checkout: bool) -> Value {
        json!({
            "number": number, "group": group, "fold": null, "reason": {"kind": kind},
            "agents": [], "agent_pane": null,
            "checkout_id": checkout.then_some("checkout-1"), "checkout_ids": [], "issue": null,
        })
    }

    pub fn card(head: &str, queue: bool, methods: &[&str]) -> Value {
        json!({
            "workspace_id": "w1", "number": 1, "reading": false, "head": head, "body": "private body",
            "files": [{"path": "src/a.rs", "additions": 3, "deletions": 1, "generated": false}], "files_total": 1,
            "threads": [{"reviewer": "x", "path": "p", "line": 1, "text": "private thread"}],
            "merge": {"methods": methods, "queue": queue, "can_merge": true}, "failed": [], "write": null,
        })
    }

    fn rest(extra_card: Value) -> Value {
        json!({
            "navigator": {
                "agents": [], "devices": [{"id": "local", "label": null}],
                "workspaces": [{
                    "id": "w1", "label": "hide", "path": "/private/never-send/repo", "device_id": "local",
                    "is_git": true, "is_home": false, "checkouts": [{"github": {
                        "failure_category": null, "available": true, "loading": false, "stale": false,
                        "last_success_at_unix_ms": 5, "unread_queries": [],
                    }, "path": "/private/never-send/repo"}],
                    "pull_requests": [pull_request(1, "open"), pull_request(2, "open"), pull_request(3, "merged")],
                    "agent_scope": {"prs": {
                        "rows": [row(1, "needs_you", "merge", true), row(2, "waiting", "ci", false), row(3, "merged", "merged", false)],
                        "groups": [{"group": "needs_you", "numbers": [1]}, {"group": "waiting", "numbers": [2]}, {"group": "merged", "numbers": [3]}],
                        "needs_you": 1,
                    }},
                }],
            },
            "status": {"remote": []},
            "pr_work": {"cards": [extra_card]},
        })
    }

    pub fn prs(card: Value) -> PhonePrs {
        let rest = rest(card);
        PhonePrs::project(&rest, "local", &project(&rest, "local"))
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{HEAD, card, prs};
    use super::*;

    fn message(body: Value) -> Value {
        let mut message = json!({"pr": "w1#1", "request_id": "r1"});
        message
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        message
    }

    fn payload(bytes: Vec<u8>) -> Value {
        serde_json::from_slice::<Value>(&bytes).unwrap()
    }

    #[test]
    fn a_merge_is_the_card_s_own_event_for_the_head_and_method_it_showed() {
        let prs = prs(card(HEAD, false, &["squash", "merge"]));
        let sent =
            payload(merge(&message(json!({"head": HEAD, "method": "squash"})), &prs).unwrap());
        assert_eq!(sent["kind"], "pr_merge");
        assert_eq!(
            sent["payload"],
            json!({"workspace_id": "w1", "pr_number": 1, "head": HEAD, "method": "squash"})
        );
        let upper = merge(
            &message(json!({"head": HEAD.to_uppercase(), "method": "merge"})),
            &prs,
        );
        assert!(upper.is_ok(), "a commit name is not case sensitive");
    }

    #[test]
    fn a_repository_with_a_merge_queue_takes_no_method_and_one_without_takes_an_offered_one() {
        let queued = prs(card(HEAD, true, &[]));
        assert!(merge(&message(json!({"head": HEAD})), &queued).is_ok());
        assert_eq!(
            merge(&message(json!({"head": HEAD, "method": "squash"})), &queued).unwrap_err(),
            "invalid_request"
        );
        let plain = prs(card(HEAD, false, &["squash"]));
        for body in [
            json!({"head": HEAD}),
            json!({"head": HEAD, "method": "rebase"}),
            json!({"head": HEAD, "method": 7}),
        ] {
            assert_eq!(
                merge(&message(body), &plain).unwrap_err(),
                "invalid_request"
            );
        }
    }

    #[test]
    fn a_merge_for_another_head_an_unread_card_a_settled_or_an_unlisted_pull_request_is_refused() {
        let prs = prs(card(HEAD, false, &["squash"]));
        let other = "f".repeat(40);
        assert_eq!(
            merge(&message(json!({"head": other, "method": "squash"})), &prs).unwrap_err(),
            "head_moved"
        );
        assert_eq!(
            merge(&message(json!({"method": "squash"})), &prs).unwrap_err(),
            "invalid_request"
        );
        let mut unread = message(json!({"head": HEAD, "method": "squash"}));
        unread["pr"] = json!("w1#2");
        assert_eq!(
            merge(&unread, &prs).unwrap_err(),
            "unread",
            "pull request 2 has no card"
        );
        let mut settled = message(json!({"head": HEAD, "method": "squash"}));
        settled["pr"] = json!("w1#3");
        assert_eq!(merge(&settled, &prs).unwrap_err(), "settled");
        for key in ["w1#9", "../../etc", ""] {
            let mut gone = message(json!({"head": HEAD, "method": "squash"}));
            gone["pr"] = json!(key);
            assert_eq!(merge(&gone, &prs).unwrap_err(), "gone", "{key}");
        }
        let mut no_key = message(json!({"head": HEAD}));
        no_key.as_object_mut().unwrap().remove("pr");
        assert_eq!(merge(&no_key, &prs).unwrap_err(), "gone");
    }

    #[test]
    fn reading_a_card_or_its_feedback_names_the_listed_pull_request_and_nothing_else() {
        let prs = prs(card(HEAD, false, &["squash"]));
        let read = payload(card_read(&message(json!({"look_again": true})), &prs).unwrap());
        assert_eq!(read["kind"], "pr_card_read");
        assert_eq!(
            read["payload"],
            json!({"workspace_id": "w1", "pr_number": 1, "look_again": true})
        );
        assert_eq!(
            card_read(&message(json!({"look_again": "yes"})), &prs).unwrap_err(),
            "invalid_request"
        );
        let feedback = payload(feedback(&message(json!({})), &prs, "r1").unwrap());
        assert_eq!(feedback["kind"], "pr_feedback_read");
        assert_eq!(
            feedback["payload"],
            json!({"request_id": "r1", "workspace_id": "w1", "pr_number": 1})
        );
    }

    #[test]
    fn a_hand_over_stays_behind_whether_or_not_a_checkout_holds_the_branch() {
        let prs = prs(card(HEAD, false, &["squash"]));
        let event = delegate(
            &message(json!({"kind": "claude", "model": "opus", "prompt": "Fix PR #1"})),
            &prs,
            "r1",
        )
        .unwrap();
        assert_eq!(event["kind"], "pr_delegate");
        assert_eq!(
            event["payload"],
            json!({
                "workspace_id": "w1", "pr_number": 1, "provider": "claude", "prompt": "Fix PR #1",
                "request_id": "r1", "stay_behind": true, "model": "opus",
            })
        );
        // A pull request no checkout holds is started in a new worktree of its
        // branch, which the core makes without taking the desktop's screen.
        let mut no_checkout = message(json!({"kind": "claude", "prompt": "Fix PR #2"}));
        no_checkout["pr"] = json!("w1#2");
        let event = delegate(&no_checkout, &prs, "r1").unwrap();
        assert_eq!(event["payload"]["pr_number"], 2);
        assert_eq!(event["payload"]["stay_behind"], true);
    }

    #[test]
    fn a_hand_over_with_an_empty_long_or_unknown_part_is_refused() {
        let prs = prs(card(HEAD, false, &["squash"]));
        let refused = |body: Value| delegate(&message(body), &prs, "r1").unwrap_err();
        assert_eq!(refused(json!({"kind": "claude", "prompt": "  "})), "empty");
        assert_eq!(
            refused(json!({"kind": "claude", "prompt": "x".repeat(5000)})),
            "too_long"
        );
        assert_eq!(
            refused(json!({"kind": "terminal", "prompt": "go"})),
            "unknown_kind"
        );
        assert_eq!(
            refused(json!({"kind": "claude", "prompt": "go", "model": ""})),
            "unknown_model"
        );
        assert_eq!(refused(json!({"prompt": "go"})), "invalid_request");
        let mut settled = message(json!({"kind": "claude", "prompt": "go"}));
        settled["pr"] = json!("w1#3");
        assert_eq!(delegate(&settled, &prs, "r1").unwrap_err(), "settled");
    }

    #[test]
    fn the_list_carries_no_path_body_or_thread_and_keeps_the_boards_order() {
        let prs = prs(card(HEAD, false, &["squash"]));
        let sent = serde_json::to_string(&prs).unwrap();
        for private in ["never-send", "private body", "private thread", "checkout-1"] {
            assert!(!sent.contains(private), "reached the phone: {private}");
        }
        let keys: Vec<&str> = prs.items.iter().map(|pr| pr.key.as_str()).collect();
        assert_eq!(
            keys,
            ["w1#1", "w1#2", "w1#3"],
            "open ones by the board's order, then the merged"
        );
        assert!(prs.find("w1#1").unwrap().checkout);
        assert!(!prs.find("w1#2").unwrap().checkout);
        assert_eq!(
            prs.find("w1#1").unwrap().card.as_ref().unwrap().files_total,
            1
        );
    }
}
