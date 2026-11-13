//! Ordered delivery regressions: the real Mobile delivery boundary and a
//! loopback relay, decrypting the push as its phone would. No provider is used.
use super::*;
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::admission::{
    ClientMessage, PendingAdmission, PushOutcome, PushResult, ServerMessage,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[path = "../../tests/fixtures/push_key.rs"]
mod push_key;
use push_key::AppKey;

const GUARD: Duration = Duration::from_secs(30);
const PHONE: &str = "phone";

fn key(pane: &str) -> AgentKey {
    AgentKey {
        device_id: "node".into(),
        pane_id: pane.into(),
    }
}

fn notice(pane: &str) -> push::Notice {
    push::Notice {
        key: key(pane),
        title: pane.into(),
        state: push::NoticeState::NeedsYou,
        status_code: None,
        detail: None,
        subject: None,
        place: String::new(),
    }
}

fn deliver(mobile: &Arc<Mobile>, notices: Vec<push::Notice>, closed: &[&str]) {
    mobile.deliver(
        notices,
        closed.iter().map(|pane| key(pane)).collect(),
        &Projection::default(),
    );
}

/// The runtime has one blocking worker: this fence completes after every
/// delivery job queued before it. It orders work, without waiting a guessed time.
async fn fence() {
    tokio::task::spawn_blocking(|| ()).await.unwrap();
}

/// Release on panic too, so a failed assertion cannot strand the runtime's worker.
struct Gate(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

async fn block_jobs() -> Gate {
    let (release, wait) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        wait.recv_timeout(GUARD)
            .expect("test releases queued delivery jobs");
    });
    ready.await.unwrap();
    Gate(Some(release))
}

#[derive(Clone, Copy, Debug)]
enum Discard {
    Job,
    Close,
    Replace,
    Cap,
    Expire,
    Withdraw,
    Preflight,
}

fn with_runtime(scenario: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(GUARD, scenario)
                .await
                .expect("clear reaches the loopback phone");
        });
}

fn run(discard: Discard) {
    with_runtime(scenario(discard));
}

async fn start(directory: &std::path::Path, relay_port: u16) -> crate::RunningDaemon {
    let state = directory.join("state");
    let mut env = crate::env::load_from(|name| match name {
        "HOME" | "USERPROFILE" => Some(directory.to_string_lossy().into_owned()),
        "HIDE_STATE_DIR" => Some(state.to_string_lossy().into_owned()),
        "HIDE_RELAY_URL" => Some(format!("ws://127.0.0.1:{}/ws", relay_port)),
        _ => None,
    })
    .unwrap();
    env.bind = "127.0.0.1:0".parse().unwrap();
    env.keep_alive = true;
    crate::start_daemon(env).await.unwrap()
}

async fn scenario(discard: Discard) {
    let directory = tempfile::tempdir().unwrap();
    let phone_key = AppKey::new();
    let registration = phone_key.registration(&"ab".repeat(32));
    let state = directory.path().join("state");
    assert!(store::write_logged(
        &store::settings_path(&state),
        &json!({
            "enabled": true, "method": "relay", "push_mode": "always",
        }),
    ));
    assert!(store::write_logged(
        &store::phones_path(&state),
        &json!({"phones": [{
            "id": PHONE, "name": "Phone", "credential_sha256": "0".repeat(64),
            "paired_at_ms": now_ms(), "last_seen_ms": now_ms(),
            "notifications": "on", "origin": "relay",
            "apns": {"token": registration["token"], "environment": "sandbox",
                "p256dh": registration["key"]["p256dh"], "auth": registration["key"]["auth"]},
        }]}),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = start(directory.path(), listener.local_addr().unwrap().port()).await;
    let mobile = &running.mobile;
    // A was already shown on the phone; its close must outlive B, the
    // next notice that would have carried it, and arrive with a later notice.
    let gate = if matches!(discard, Discard::Job) {
        Some(block_jobs().await)
    } else {
        None
    };
    let mut b = notice("b");
    if matches!(discard, Discard::Preflight) {
        b.title = "x".repeat(push::MAX_APNS_PLAINTEXT + 1);
    }
    deliver(mobile, vec![b], &["a"]);
    if gate.is_none() {
        fence().await;
        assert_eq!(
            mobile.lock().held[PHONE].len(),
            1,
            "B is held before its discard"
        );
    }
    let mut expected = 1;
    match discard {
        Discard::Job | Discard::Close => deliver(mobile, vec![], &["b"]),
        Discard::Replace => {
            let mut replacement = notice("b");
            replacement.title = "replacement".into();
            deliver(mobile, vec![replacement], &[]);
            fence().await;
            assert_eq!(mobile.lock().held[PHONE].len(), 1);
        }
        Discard::Cap => {
            for index in 0..HELD_PER_PHONE {
                deliver(mobile, vec![notice(&format!("new-{index}"))], &[]);
            }
            fence().await;
            let inner = mobile.lock();
            assert_eq!(inner.held[PHONE].len(), HELD_PER_PHONE);
            assert!(
                inner.held[PHONE]
                    .iter()
                    .all(|held| held.payload["tag"] != "node|b")
            );
            expected = HELD_PER_PHONE;
        }
        Discard::Expire => {
            mobile.lock().held.get_mut(PHONE).unwrap()[0].at -=
                HELD_MAX_AGE + Duration::from_secs(1);
            assert_eq!(
                mobile.flush_held().unwrap().await.unwrap(),
                [Handoff::Expired]
            );
        }
        Discard::Withdraw => mobile.withdraw_waiting(),
        Discard::Preflight => {}
    }
    if !matches!(discard, Discard::Replace | Discard::Cap) {
        deliver(mobile, vec![notice("c")], &[]);
    }
    drop(gate);
    fence().await;
    // Accepting admission only now makes every earlier relay send a hold.
    let (stream, _) = listener.accept().await.unwrap();
    let mut relay = tokio_tungstenite::accept_async(stream).await.unwrap();
    let Message::Text(hello) = relay.next().await.unwrap().unwrap() else {
        panic!("device hello")
    };
    let ClientMessage::Device { hello } = ClientMessage::decode(&hello).unwrap() else {
        panic!("device hello")
    };
    let pending = PendingAdmission::new(hello, 0).unwrap();
    relay
        .send(Message::Text(
            ServerMessage::Challenge {
                challenge: pending.challenge().clone(),
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let Message::Text(proof) = relay.next().await.unwrap().unwrap() else {
        panic!("device proof")
    };
    let ClientMessage::Proof { signature } = ClientMessage::decode(&proof).unwrap() else {
        panic!("device proof")
    };
    pending.verify(&signature, 1).unwrap();
    relay
        .send(Message::Text(
            ServerMessage::Registered {
                push: true,
                fcm: false,
            }
            .encode()
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let mut first = None;
    for _ in 0..expected {
        let request = loop {
            match relay.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let ClientMessage::Push(request) = ClientMessage::decode(&text).unwrap() else {
                        panic!("push request")
                    };
                    break request;
                }
                Message::Ping(bytes) => relay.send(Message::Pong(bytes)).await.unwrap(),
                _ => {}
            }
        };
        if first.is_none() {
            first = Some(request.payload.clone());
        }
        relay
            .send(Message::Text(
                ServerMessage::PushResult(PushResult {
                    id: request.id,
                    result: PushOutcome::Delivered,
                })
                .encode()
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
    }
    let body = URL_SAFE_NO_PAD.decode(first.unwrap()).unwrap();
    let received: Value = serde_json::from_slice(&phone_key.decrypt(&body)).unwrap();
    running.stop();
    assert!(
        received["clear"]
            .as_array()
            .unwrap()
            .contains(&json!("node|a")),
        "{discard:?}: the next actual push must close A, received {received}"
    );
}

#[test]
fn a_clear_survives_a_withdrawn_job() {
    run(Discard::Job);
}
#[test]
fn a_clear_survives_a_closed_held_notice() {
    run(Discard::Close);
}
#[test]
fn a_clear_survives_a_replaced_held_notice() {
    run(Discard::Replace);
}
#[test]
fn a_clear_survives_the_held_notice_cap() {
    run(Discard::Cap);
}
#[test]
fn a_clear_survives_an_expired_held_notice() {
    run(Discard::Expire);
}
#[test]
fn a_clear_survives_follower_withdrawal() {
    run(Discard::Withdraw);
}
#[test]
fn a_clear_survives_app_preflight_failure() {
    run(Discard::Preflight);
}

/// Web preparation can fail before its HTTP request exists, too. Observe the
/// next request at a local push service, rather than only the pending set.
#[test]
fn a_clear_survives_web_preflight_failure() {
    with_runtime(web_preflight());
}

async fn web_preflight() {
    let directory = tempfile::tempdir().unwrap();
    let phone_key = AppKey::new();
    let registration = phone_key.registration(&"ab".repeat(32));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = directory.path().join("state");
    assert!(store::write_logged(
        &store::settings_path(&state),
        &json!({
            "enabled": true, "method": "relay", "push_mode": "always",
        })
    ));
    assert!(store::write_logged(
        &store::phones_path(&state),
        &json!({"phones": [{
            "id": PHONE, "name": "Phone", "credential_sha256": "0".repeat(64),
            "paired_at_ms": now_ms(), "last_seen_ms": now_ms(),
            "notifications": "on", "origin": "relay",
            "push": {"endpoint": format!("http://{}/push", listener.local_addr().unwrap()),
                "p256dh": registration["key"]["p256dh"], "auth": registration["key"]["auth"]},
        }]})
    ));
    let (sent, mut received) = tokio::sync::mpsc::channel(1);
    let app = axum::Router::new().route(
        "/push",
        axum::routing::post(move |body: axum::body::Bytes| {
            let sent = sent.clone();
            async move {
                sent.send(body).await.unwrap();
                axum::http::StatusCode::NO_CONTENT
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = start(directory.path(), relay.local_addr().unwrap().port()).await;
    let mobile = &running.mobile;
    let mut oversized = notice("b");
    oversized.title = "x".repeat(4096);
    deliver(mobile, vec![oversized], &["a"]);
    fence().await;
    assert!(
        received.try_recv().is_err(),
        "preflight never posts the oversized notice"
    );
    // A close arrives while another sender owns the earlier subset.
    let failed: Result<(), &str> = mobile.prepare_with_clear(PHONE, &json!({}), |_| {
        deliver(mobile, vec![], &["d"]);
        Err("preparation failed")
    });
    assert_eq!(failed, Err("preparation failed"));
    deliver(mobile, vec![notice("c")], &[]);
    let body = received.recv().await.unwrap();
    fence().await;
    server.abort();
    let _ = server.await;
    let payload: Value = serde_json::from_slice(&phone_key.decrypt(&body)).unwrap();
    assert!(
        payload["clear"]
            .as_array()
            .unwrap()
            .contains(&json!("node|a")),
        "{payload}"
    );
    assert!(
        payload["clear"]
            .as_array()
            .unwrap()
            .contains(&json!("node|d")),
        "{payload}"
    );
    deliver(mobile, vec![], &["e"]);
    let failed: Result<(), &str> = mobile.prepare_with_clear(PHONE, &json!({}), |_| {
        mobile.revoke(PHONE, "test");
        Err("preparation failed")
    });
    assert_eq!(failed, Err("preparation failed"));
    assert!(
        !mobile.lock().pending_clear.contains_key(PHONE),
        "revocation ends ownership"
    );
    running.stop();
}

/// The follower's shared judge/settle pass and actual HTTP push boundary,
/// with a stand-in host. A bare core keeps the automatic follower parked;
/// the fixture supplies projections and a monotonic clock explicitly.
type SendHook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

struct RestFixture {
    mobile: Arc<Mobile>,
    received: tokio::sync::mpsc::UnboundedReceiver<(String, String, axum::body::Bytes)>,
    on_send: SendHook,
    server: tokio::task::JoinHandle<()>,
    phone_key: Option<AppKey>,
    _directory: tempfile::TempDir,
}

impl RestFixture {
    async fn new(phones: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = tokio::sync::mpsc::unbounded_channel();
        let on_send: SendHook = Arc::default();
        let hook = Arc::clone(&on_send);
        let app = axum::Router::new().route(
            "/{phone}",
            axum::routing::post(
                move |axum::extract::Path(phone): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let sent = sent.clone();
                    let hook = Arc::clone(&hook);
                    async move {
                        sent.send((phone, headers["topic"].to_str().unwrap().to_owned(), body))
                            .unwrap();
                        if let Some(hook) = hook.lock().unwrap().take() {
                            hook();
                        }
                        axum::http::StatusCode::NO_CONTENT
                    }
                },
            ),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let phone_key = AppKey::new();
        let registration = phone_key.registration(&"ab".repeat(32));
        let state = directory.path().join("state");
        assert!(store::write_logged(
            &store::settings_path(&state),
            &json!({
                "enabled": true, "method":"relay", "push_mode":"app_closed",
            })
        ));
        let phones: Vec<_> = (0..phones).map(|index| json!({
            "id":format!("phone-{index}"), "name":"Phone", "credential_sha256":"0".repeat(64),
            "paired_at_ms":now_ms(), "last_seen_ms":now_ms(), "notifications":"on", "origin":"relay",
            "push":{"endpoint":format!("http://{address}/phone-{index}"),
                "p256dh":registration["key"]["p256dh"], "auth":registration["key"]["auth"]},
        })).collect();
        assert!(store::write_logged(
            &store::phones_path(&state),
            &json!({"phones":phones})
        ));
        let mobile = test_mobile(directory.path(), &Arc::default());
        mobile.relay_keys().unwrap();
        mobile.config.desktop.set_waiting(1, true);
        // Use the injected observation clock, with the initial computer input
        // outside the one-second tie window of a subsequent phone action.
        mobile.config.desktop.activity_at(
            1,
            true,
            std::time::Instant::now() - std::time::Duration::from_secs(2),
        );
        Self {
            mobile,
            received,
            on_send,
            server,
            phone_key: Some(phone_key),
            _directory: directory,
        }
    }

    fn queue_pass(&self, projection: &Projection, at: std::time::Instant, changed: bool) {
        Self::queue(&self.mobile, projection, at, changed);
    }

    fn queue(mobile: &Arc<Mobile>, projection: &Projection, at: std::time::Instant, changed: bool) {
        let mut inner = mobile.lock();
        let mode = inner.settings.push_mode;
        let judged = judge_pass(
            &mut inner.phone_notices,
            Some(mode),
            &mobile.config.desktop,
            projection,
            changed,
            true,
            at,
        );
        drop(inner);
        if let Some((notices, closed)) = judged {
            mobile.deliver(notices, closed, projection);
        }
    }

    async fn pass(&self, projection: &Projection, at: std::time::Instant, changed: bool) {
        self.queue_pass(projection, at, changed);
        fence().await;
    }

    fn take(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        while let Ok((phone, topic, body)) = self.received.try_recv() {
            assert!(!body.is_empty(), "the phone receives an encrypted request");
            out.push((phone, topic));
        }
        out.sort();
        out
    }
}

impl Drop for RestFixture {
    fn drop(&mut self) {
        self.mobile.stopping.send_replace(true);
        self.server.abort();
    }
}

fn roots(count: usize, working: &[usize], turns: &[&str]) -> Projection {
    let agents: Vec<_> = (0..count).map(|index| {
        let mut row = json!({"pane_id":format!("root-{index}"), "group":if working.contains(&index) { "working" } else { "needs_you" },
            "mark":"going", "identity_label":format!("Task {index}"), "agent_kind":"claude", "demand":if working.contains(&index) { "none" } else { "question" },
            "status_code":if working.contains(&index) { "working" } else { "question" }, "unread":true});
        if !turns.is_empty() {
            row["raised"] = json!(turns.iter().map(|id| json!({"subject":{
                "cause":"human_turn", "id":id, "kind":"auth", "place":"login", "device":"mini",
            }})).collect::<Vec<_>>());
            row["human_turn"] = json!({"id":turns[0],"own":false});
        }
        row
    }).collect();
    projection::project(
        &json!({"navigator":{"agents":agents,"workspaces":[],"devices":[{"id":"node","label":null}]}}),
        "node",
    )
}

// Web Push's public collapse topic identifies each received root without
// needing a reusable private ECDH key in this fixture.
fn topic(index: usize) -> String {
    URL_SAFE_NO_PAD.encode(
        &ring::digest::digest(
            &ring::digest::SHA256,
            format!("node|root-{index}").as_bytes(),
        )
        .as_ref()[..24],
    )
}

#[test]
fn computer_use_discards_old_waits_and_only_new_away_waits_send() {
    with_runtime(async {
        for leave_before_settle in [false, true] {
            let mut fixture = RestFixture::new(1).await;
            let t = std::time::Instant::now();
            fixture
                .pass(&roots(11, &(0..11).collect::<Vec<_>>(), &[]), t, true)
                .await;
            fixture.pass(&roots(11, &[10], &[]), t, true).await;
            let away = t + if leave_before_settle {
                push::SETTLE / 2
            } else {
                push::SETTLE
            };
            fixture.pass(&roots(11, &[0, 10], &[]), away, true).await;
            fixture.mobile.config.desktop.activity(1, false);
            fixture.pass(&Projection::default(), away, true).await;
            let current = roots(11, &[0, 10], &[]);
            fixture.pass(&current, t + push::SETTLE, true).await;
            assert!(
                fixture.take().is_empty(),
                "away sends no old wait, including one still settling"
            );
            fixture.pass(&current, t + push::SETTLE * 2, false).await;
            assert!(
                fixture.take().is_empty(),
                "repeated away never replays an old wait"
            );
            let new_wait = roots(11, &[0], &[]);
            fixture.pass(&new_wait, t + push::SETTLE * 2, true).await;
            fixture
                .pass(
                    &new_wait,
                    t + push::SETTLE * 3 - Duration::from_millis(1),
                    false,
                )
                .await;
            assert!(
                fixture.take().is_empty(),
                "new away wait keeps its settle window"
            );
            fixture.pass(&new_wait, t + push::SETTLE * 3, false).await;
            assert_eq!(
                fixture.take(),
                [("phone-0".into(), topic(10))],
                "only the new away wait sends"
            );
            fixture.pass(&new_wait, t + push::SETTLE * 4, false).await;
            assert!(fixture.take().is_empty(), "new wait sends once");
        }
    });
}

#[test]
fn ending_rest_does_not_replay_and_always_still_sends_during_use() {
    with_runtime(async {
        for release in 0..3 {
            let mut fixture = RestFixture::new(1).await;
            let t = std::time::Instant::now();
            fixture.pass(&roots(1, &[0], &[]), t, true).await;
            fixture.pass(&roots(1, &[], &[]), t, true).await;
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
                .await;
            assert!(fixture.take().is_empty());
            match release {
                0 => fixture.mobile.config.desktop.set_waiting(1, false),
                1 => fixture.mobile.lock().settings.push_mode = PushMode::Always,
                _ => fixture.mobile.config.desktop.release(1),
            }
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
                .await;
            assert!(
                fixture.take().is_empty(),
                "ending rest never sends the old wait"
            );
            fixture
                .pass(&roots(1, &[0], &[]), t + push::SETTLE, true)
                .await;
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE, true)
                .await;
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE * 2, false)
                .await;
            assert_eq!(
                fixture.take(),
                [("phone-0".into(), topic(0))],
                "new wait sends, including Always with active input"
            );
        }
    });
}

#[test]
fn rested_declarations_never_replay_and_a_new_away_declaration_sends() {
    with_runtime(async {
        for cancel_newest in [false, true] {
            let mut fixture = RestFixture::new(1).await;
            let t = std::time::Instant::now();
            fixture.pass(&roots(1, &[0], &[]), t, true).await;
            fixture.pass(&roots(1, &[], &["A"]), t, true).await;
            fixture
                .pass(&roots(1, &[], &["A"]), t + push::SETTLE, false)
                .await;
            fixture
                .pass(&roots(1, &[], &["A", "B"]), t + push::SETTLE, true)
                .await;
            let turns = if cancel_newest {
                vec!["A"]
            } else {
                vec!["A", "B"]
            };
            fixture
                .pass(&roots(1, &[], &turns), t + push::SETTLE * 2, true)
                .await;
            assert!(fixture.take().is_empty());
            fixture.mobile.config.desktop.activity(1, false);
            fixture
                .pass(&roots(1, &[], &turns), t + push::SETTLE * 2, false)
                .await;
            assert!(
                fixture.take().is_empty(),
                "neither a current nor a cancelled rested declaration replays"
            );
            let mut new_turns = turns;
            new_turns.push("C");
            let current = roots(1, &[], &new_turns);
            fixture.pass(&current, t + push::SETTLE * 2, true).await;
            assert!(
                fixture.take().is_empty(),
                "new declaration waits for settle"
            );
            fixture.pass(&current, t + push::SETTLE * 3, false).await;
            let (phone, received_topic, body) = fixture.received.try_recv().unwrap();
            assert_eq!((phone, received_topic), ("phone-0".into(), topic(0)));
            let received: Value =
                serde_json::from_slice(&fixture.phone_key.take().unwrap().decrypt(&body)).unwrap();
            assert_eq!(received["subject"]["id"], "C");
            fixture.pass(&current, t + push::SETTLE * 4, false).await;
            assert!(fixture.take().is_empty(), "new declaration sends once");
        }
    });
}

#[test]
fn input_resuming_during_transport_discards_the_remaining_phone_attempt() {
    with_runtime(async {
        let mut fixture = RestFixture::new(2).await;
        let t = std::time::Instant::now();
        fixture.mobile.config.desktop.activity(1, false);
        fixture.pass(&roots(1, &[0], &[]), t, true).await;
        fixture.pass(&roots(1, &[], &[]), t, true).await;
        let desktop = Arc::clone(&fixture.mobile.config.desktop);
        *fixture.on_send.lock().unwrap() = Some(Box::new(move || desktop.activity(1, true)));
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        assert_eq!(fixture.take(), [("phone-0".into(), topic(0))]);
        fixture.mobile.config.desktop.activity(1, false);
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        assert!(
            fixture.take().is_empty(),
            "the second phone never receives a rested attempt later"
        );
        fixture
            .pass(&roots(1, &[0], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE * 2, false)
            .await;
        assert_eq!(
            fixture.take(),
            [("phone-0".into(), topic(0)), ("phone-1".into(), topic(0))]
        );
    });
}

#[test]
fn a_discarded_wait_still_closes_with_the_next_away_notification() {
    with_runtime(async {
        let mut fixture = RestFixture::new(1).await;
        let t = std::time::Instant::now();
        fixture.pass(&roots(2, &[0, 1], &[]), t, true).await;
        fixture.pass(&roots(2, &[1], &[]), t, true).await;
        fixture
            .pass(&roots(2, &[1], &[]), t + push::SETTLE, false)
            .await;
        assert!(fixture.take().is_empty());
        fixture.mobile.config.desktop.activity(1, false);
        fixture
            .pass(&roots(2, &[0, 1], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(2, &[0], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(2, &[0], &[]), t + push::SETTLE * 2, false)
            .await;
        let (phone, received_topic, body) = fixture.received.try_recv().unwrap();
        assert_eq!((phone, received_topic), ("phone-0".into(), topic(1)));
        let received: Value =
            serde_json::from_slice(&fixture.phone_key.take().unwrap().decrypt(&body)).unwrap();
        assert_eq!(received["clear"], json!(["node|root-0"]));
        assert!(fixture.take().is_empty());
    });
}

#[test]
fn a_relay_reconnect_rests_again_before_the_transport_handoff() {
    with_runtime(async {
        let mut fixture = RestFixture::new(1).await;
        let registration = AppKey::new().registration(&"ab".repeat(32));
        let web = fixture
            .mobile
            .lock()
            .phones
            .get("phone-0")
            .unwrap()
            .push
            .clone()
            .unwrap();
        fixture.mobile.lock().phones.set_apns(
            "phone-0",
            Some(
                serde_json::from_value(json!({
                    "token":registration["token"], "environment":"sandbox",
                    "p256dh":registration["key"]["p256dh"], "auth":registration["key"]["auth"],
                }))
                .unwrap(),
            ),
        );
        let t = std::time::Instant::now();
        fixture.mobile.config.desktop.activity(1, false);
        fixture.pass(&roots(1, &[0], &[]), t, true).await;
        fixture.pass(&roots(1, &[], &[]), t, true).await;
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        // The actual app route is held while its relay is away.
        fixture.mobile.config.desktop.activity(1, true);
        assert_eq!(
            fixture.mobile.flush_held().unwrap().await.unwrap(),
            [],
            "reconnect admits no transport handoff while the computer is in use"
        );
        // The same paired phone registers its Web Push route before the next away.
        fixture
            .mobile
            .lock()
            .phones
            .set_subscription("phone-0", Some(web));
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        assert!(fixture.take().is_empty());
        fixture.mobile.config.desktop.activity(1, false);
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        assert!(
            fixture.take().is_empty(),
            "reconnect's rested attempt never replays on away"
        );
        fixture
            .pass(&roots(1, &[0], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE * 2, false)
            .await;
        assert_eq!(
            fixture.take(),
            [("phone-0".into(), topic(0))],
            "a new away wait still sends"
        );
        fixture
            .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
            .await;
        assert!(fixture.take().is_empty());
    });
}

#[test]
fn an_accepted_phone_action_only_sends_new_waits_to_all_phones_until_computer_input() {
    with_runtime(async {
        let mut fixture = RestFixture::new(2).await;
        let t = std::time::Instant::now();
        fixture
            .mobile
            .config
            .desktop
            .activity_at(1, true, t - std::time::Duration::from_secs(2));
        fixture.pass(&roots(3, &[0, 1, 2], &[]), t, true).await;
        fixture.pass(&roots(3, &[1, 2], &[]), t, true).await;
        assert!(matches!(
            fixture.mobile.reserve_input("phone-1", "accepted"),
            Reservation::Write
        ));
        fixture
            .mobile
            .settle_input("phone-1", "accepted", Some(InputState::Written));
        fixture
            .pass(&roots(3, &[1, 2], &[]), t + push::SETTLE, false)
            .await;
        assert!(
            fixture.take().is_empty(),
            "taking over never sends the old wait"
        );
        fixture
            .pass(&roots(3, &[2], &[]), t + push::SETTLE, true)
            .await;
        fixture
            .pass(
                &roots(3, &[2], &[]),
                t + push::SETTLE * 2 - std::time::Duration::from_millis(1),
                false,
            )
            .await;
        assert!(fixture.take().is_empty());
        fixture
            .pass(&roots(3, &[2], &[]), t + push::SETTLE * 2, false)
            .await;
        assert_eq!(
            fixture.take(),
            [("phone-0".into(), topic(1)), ("phone-1".into(), topic(1))]
        );
        fixture.mobile.config.desktop.activity(1, true);
        fixture
            .pass(&roots(3, &[], &[]), t + push::SETTLE * 2, true)
            .await;
        fixture
            .pass(&roots(3, &[], &[]), t + push::SETTLE * 3, false)
            .await;
        assert!(
            fixture.take().is_empty(),
            "resumed computer input rests new waits again"
        );
    });
}

#[test]
fn an_eligible_root_survives_rest_and_uses_the_current_wait_at_settle() {
    with_runtime(async {
        let declared = |turns: &[&str]| {
            projection::project(
                &json!({"navigator":{"agents":[{
                "pane_id":"root-0", "identity_label":"Task", "agent_kind":"claude",
                "group":if turns.is_empty() {"working"} else {"needs_you"},
                "mark":"going", "demand":"none", "status_code":"human_turn", "unread":true,
                "human_turn":turns.first().map(|id| json!({"id":id,"own":false})),
                "raised":turns.iter().map(|id| json!({"subject":{
                    "cause":"human_turn","id":id,"kind":"auth","place":"login","device":"mini"
                }})).collect::<Vec<_>>()
            }],"workspaces":[],"devices":[{"id":"node","label":null}]}}),
                "node",
            )
        };
        for cancel_a in [false, true] {
            for (disappears, check_c) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let mut fixture = RestFixture::new(1).await;
                let t = std::time::Instant::now();
                let desktop = Arc::clone(&fixture.mobile.config.desktop);
                desktop.phone_input(t);
                fixture.pass(&declared(&[]), t, true).await;
                fixture.pass(&declared(&["A"]), t, true).await;
                desktop.activity_at(1, true, t + Duration::from_millis(100));
                fixture
                    .pass(&declared(&["A", "B"]), t + Duration::from_millis(200), true)
                    .await;
                let turns = if cancel_a { vec!["B"] } else { vec!["A", "B"] };
                if disappears {
                    fixture
                        .pass(&roots(0, &[], &[]), t + Duration::from_millis(300), true)
                        .await;
                }
                desktop.phone_input(t + Duration::from_millis(1300));
                let settled_at = if disappears {
                    fixture
                        .pass(&roots(0, &[], &[]), t + push::SETTLE, false)
                        .await;
                    assert!(
                        fixture.take().is_empty(),
                        "no payload while the root is absent"
                    );
                    t + push::SETTLE + Duration::from_secs(1)
                } else {
                    t + push::SETTLE
                };
                fixture.pass(&declared(&turns), settled_at, true).await;
                let (_, _, body) = fixture
                    .received
                    .try_recv()
                    .expect("eligible root survives rest");
                if !check_c {
                    let payload: Value =
                        serde_json::from_slice(&fixture.phone_key.take().unwrap().decrypt(&body))
                            .unwrap();
                    // The lead accepts B replacing resolved A inside this
                    // root's original three-second SETTLE window.
                    assert_eq!(payload["subject"]["id"], if cancel_a { "B" } else { "A" });
                }
                assert!(fixture.take().is_empty(), "one notice per pending root");
                let current = declared(&["B", "C"]);
                fixture.pass(&current, settled_at, true).await;
                fixture
                    .pass(&current, settled_at + push::SETTLE, false)
                    .await;
                let (_, _, body) = fixture.received.try_recv().expect("new eligible C sends");
                if check_c {
                    let payload: Value =
                        serde_json::from_slice(&fixture.phone_key.take().unwrap().decrypt(&body))
                            .unwrap();
                    assert_eq!(payload["subject"]["id"], "C");
                }
                assert!(fixture.take().is_empty(), "a new C sends once");
            }
        }
    });
}

#[test]
fn a_native_wait_still_notifies_when_a_rested_declaration_becomes_its_representative() {
    with_runtime(async {
        let native = |with_b: bool| {
            projection::project(
                &json!({"navigator":{"agents":[{
            "pane_id":"root-0", "identity_label":"Task", "agent_kind":"claude",
            "group":"needs_you", "mark":"going", "demand":"none", "status_code":"mission_decision", "unread":true,
            "ask_subject":{"cause":"mission_decision","line":"A","views":0},
            "human_turn":with_b.then(|| json!({"id":"B","own":false})),
            "raised":if with_b { json!([{"subject":{"cause":"human_turn","id":"B","kind":"auth","place":"login","device":"mini"}}]) } else { json!([]) }
        }],"workspaces":[],"devices":[{"id":"node","label":null}]}}),
                "node",
            )
        };
        let mut fixture = RestFixture::new(1).await;
        let t = std::time::Instant::now();
        let desktop = Arc::clone(&fixture.mobile.config.desktop);
        desktop.phone_input(t);
        fixture.pass(&roots(1, &[0], &[]), t, true).await;
        fixture.pass(&native(false), t, true).await;
        desktop.activity_at(1, true, t + Duration::from_millis(100));
        fixture
            .pass(&native(true), t + Duration::from_millis(200), true)
            .await;
        desktop.phone_input(t + Duration::from_millis(1300));
        fixture.pass(&native(true), t + push::SETTLE, false).await;
        let (_, _, body) = fixture
            .received
            .try_recv()
            .expect("native A keeps the root eligible");
        let payload: Value =
            serde_json::from_slice(&fixture.phone_key.take().unwrap().decrypt(&body)).unwrap();
        assert_eq!(
            payload["subject"]["id"], "B",
            "current representative is an accepted settle boundary"
        );
        assert!(fixture.take().is_empty(), "one notice for the root");
    });
}

#[test]
fn an_eligible_settle_uses_the_device_at_send_and_a_resting_candidate_never_replays() {
    with_runtime(async {
        for phone_at_send in [true, false] {
            let mut fixture = RestFixture::new(1).await;
            let t = std::time::Instant::now();
            let desktop = Arc::clone(&fixture.mobile.config.desktop);
            desktop.activity_at(1, true, t - std::time::Duration::from_secs(2));
            desktop.phone_input(t);
            fixture.pass(&roots(1, &[0], &[]), t, true).await;
            fixture.pass(&roots(1, &[], &[]), t, true).await;
            desktop.activity_at(1, true, t + std::time::Duration::from_millis(10));
            fixture
                .pass(
                    &roots(1, &[], &[]),
                    t + std::time::Duration::from_millis(10),
                    false,
                )
                .await;
            if phone_at_send {
                desktop.phone_input(t + std::time::Duration::from_secs(2));
            }
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE, false)
                .await;
            assert_eq!(fixture.take().len(), usize::from(phone_at_send));
            desktop.phone_input(t + push::SETTLE * 2);
            fixture
                .pass(&roots(1, &[], &[]), t + push::SETTLE * 2, false)
                .await;
            assert!(
                fixture.take().is_empty(),
                "no second send or replay after a discarded settle"
            );
        }
    });
}
