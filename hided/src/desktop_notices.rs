//! The desktop app's agent notifications (PRD first-run-onboarding D-09,
//! B31-B38): the phone push's notification transitions
//! (`herdr_core::agent_state::push::Transitions`) run a second time for the
//! desktop windows, independent of the Mobile setting.
//!
//! They judge the projection the mobile follower already reads off the core's
//! lock (`mobile::Mobile::follow`), so a burst serializes one snapshot however
//! many consumers there are, and they run only while a desktop window has the
//! switch on, which each window says for itself like any observation hint
//! (`demand.rs`). The first projection after that seeds them, so nothing that
//! was already waiting when the app opened is announced (B33). A judgment
//! that announces or clears something is one `agent_notice` frame for the
//! desktop windows; whether a notification shows, and in what words, is the
//! desktop host's and the window's to decide (D-10, B39).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use herdr_core::agent_state::phone::{AgentKey, Projection};
use herdr_core::agent_state::push::{Notice, Transitions};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::demand::ObservationDemand;

/// Frames a window may fall behind by before it misses one; a missed frame
/// costs one notification, never a wrong one.
const FRAME_BACKLOG: usize = 16;

pub struct DesktopNotices {
    /// The desktop windows whose switch is on.
    demand: ObservationDemand,
    wanted: AtomicBool,
    transitions: Mutex<Transitions>,
    frames: broadcast::Sender<Arc<str>>,
}

impl Default for DesktopNotices {
    fn default() -> Self {
        Self {
            demand: ObservationDemand::default(),
            wanted: AtomicBool::new(false),
            transitions: Mutex::default(),
            frames: broadcast::channel(FRAME_BACKLOG).0,
        }
    }
}

impl DesktopNotices {
    fn lock(&self) -> std::sync::MutexGuard<'_, Transitions> {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One desktop window says whether its switch is on. True when the
    /// windows as a whole just turned it on, so the follower must wake.
    pub fn set(&self, connection: u64, wanted: bool) -> bool {
        self.demand.set(connection, wanted, |on| self.turned(on)) == Some(true)
    }

    /// A closed window takes its demand with it.
    pub fn release(&self, connection: u64) {
        self.demand.release(connection, |on| self.turned(on));
    }

    fn turned(&self, on: bool) {
        self.wanted.store(on, Ordering::SeqCst);
        self.lock().reset();
        herdr_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notices.demand", "on": on,
        }));
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// Whether the switch is on and no projection has seeded it since.
    pub fn seed_pending(&self) -> bool {
        self.wanted() && !self.lock().seeded()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.frames.subscribe()
    }

    /// Judges one projection; a change worth a notification, or one that
    /// clears a notification shown before (B35), goes out as one frame.
    pub fn observe(&self, projection: &Projection) {
        let (notices, seen) = self.lock().observe(projection);
        if notices.is_empty() && seen.is_empty() {
            return;
        }
        herdr_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notice.judged",
            "agents": notices.iter().map(|notice| notice.key.tag()).collect::<Vec<_>>(),
            "cleared": seen.iter().map(AgentKey::tag).collect::<Vec<_>>(),
        }));
        let frame = frame(&notices, seen.iter());
        // No window listening is no notification, which is the answer.
        let _ = self.frames.send(Arc::from(frame.to_string()));
    }
}

/// The `agent_notice` frame: data only, as the phone's push payload is; the
/// window words the state in the interface language.
fn frame<'a>(notices: &[Notice], clear: impl Iterator<Item = &'a AgentKey>) -> Value {
    json!({
        "type": "agent_notice",
        "payload": {
            "notices": notices.iter().map(|notice| json!({
                "tag": notice.key.tag(),
                "device_id": notice.key.device_id,
                "pane_id": notice.key.pane_id,
                "title": notice.title,
                "state": notice.state.as_str(),
                "status_code": notice.status_code,
                "detail": notice.detail,
                "subject": notice.subject,
                "place": notice.place,
            })).collect::<Vec<_>>(),
            "clear": clear.map(AgentKey::tag).collect::<Vec<_>>(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_core::agent_state::phone::project;

    fn rest(group: &str, demand: &str) -> Value {
        json!({
            "navigator": {
                "agents": [{
                    "pane_id": "w1:p1", "group": group, "mark": "going",
                    "identity_label": "Add a dark mode switch", "agent_kind": "claude",
                    "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle",
                    "demand": demand,
                }],
                "workspaces": [{"label": "my-app", "is_git": true, "checkouts": [
                    {"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "w1:p1"}]}]},
                ]}],
                "devices": [{"id": "node", "label": null}],
            },
        })
    }

    fn judged(notices: &DesktopNotices, rest: &Value) -> Option<Value> {
        let mut frames = notices.subscribe();
        notices.observe(&project(rest, "node"));
        frames
            .try_recv()
            .ok()
            .map(|frame| serde_json::from_str(&frame).unwrap())
    }

    /// B33: the projection after the switch turns on only seeds; a root that
    /// newly needs the operator is one notice with the task and the project,
    /// the same state again is none, and the root turning Seen clears it.
    #[test]
    fn the_first_projection_seeds_and_a_new_wait_is_one_notice_cleared_when_seen() {
        let notices = DesktopNotices::default();
        assert!(notices.set(7, true));
        assert!(notices.wanted() && notices.seed_pending());
        assert_eq!(judged(&notices, &rest("needs_you", "approval")), None);
        assert!(!notices.seed_pending());
        assert_eq!(judged(&notices, &rest("working", "none")), None);

        let frame = judged(&notices, &rest("needs_you", "approval")).expect("a notice");
        assert_eq!(frame["type"], "agent_notice");
        let notice = &frame["payload"]["notices"][0];
        assert_eq!(notice["state"], "needs_you");
        assert_eq!(notice["title"], "Add a dark mode switch");
        assert_eq!(notice["place"], "my-app");
        assert_eq!(notice["pane_id"], "w1:p1");
        assert_eq!(judged(&notices, &rest("needs_you", "approval")), None);

        let cleared = judged(&notices, &rest("seen", "none")).expect("a clear");
        assert_eq!(cleared["payload"]["notices"], json!([]));
        assert_eq!(cleared["payload"]["clear"], json!([notice["tag"].clone()]));
    }

    /// The switch follows the last window that has it on, and turning it on
    /// again seeds afresh, so a wait that began while it was off is not
    /// announced late.
    #[test]
    fn the_switch_follows_its_windows_and_reseeds_when_it_turns_on_again() {
        let notices = DesktopNotices::default();
        assert!(notices.set(1, true));
        assert!(!notices.set(2, true), "a second window adds no transition");
        judged(&notices, &rest("working", "none"));
        notices.release(1);
        assert!(notices.wanted());
        notices.set(2, false);
        assert!(!notices.wanted());
        assert!(notices.set(2, true));
        assert_eq!(judged(&notices, &rest("needs_you", "question")), None);
    }

    /// status-model.md: the root's detail, not a child's line or a newly
    /// generated sentence, reaches both transports; copy changes alone are
    /// not state transitions and must not repeat a notification.
    #[test]
    fn each_notice_carries_the_roots_chosen_sentence_and_status() {
        for (group, demand, status, detail) in [
            (
                "needs_you",
                "question",
                "question",
                Some("배포할 환경을 골라 주세요"),
            ),
            ("needs_you", "approval", "approval", Some("계획 승인 대기")),
            ("needs_you", "error", "error", Some("로그인이 필요해요")),
            ("done", "none", "done", Some("배포 완료")),
            ("done", "none", "stopped", Some("테스트 세 개 남음")),
            ("needs_you", "approval", "approval", None),
        ] {
            let notices = DesktopNotices::default();
            notices.set(7, true);
            judged(&notices, &rest("working", "none"));
            let mut asking = rest(group, demand);
            asking["navigator"]["agents"][0]["status_code"] = json!(status);
            asking["navigator"]["agents"][0]["detail"] = json!(detail);
            let frame = judged(&notices, &asking).expect("a new notice");
            let notice = &frame["payload"]["notices"][0];
            assert_eq!(notice["status_code"], status);
            assert_eq!(notice["detail"], json!(detail));
            asking["navigator"]["agents"][0]["detail"] = json!("새 문장");
            assert_eq!(judged(&notices, &asking), None);
        }
    }
}
