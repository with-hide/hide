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
            ServerMessage::Registered { push: true }
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
