//! The daemon's private desktop registration is not a public CDP discovery
//! route and remains unavailable to tailnet and unauthenticated callers.

use hided::env::Env;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};

async fn request(
    port: u16,
    path: &str,
    token: Option<&str>,
    forwarded: bool,
    body: Value,
) -> (u16, String) {
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let mut request = hyper::Request::builder()
        .method("POST")
        .uri(format!("http://127.0.0.1:{port}{path}"))
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if forwarded {
        request = request.header("X-Forwarded-For", "100.100.100.100");
    }
    let response = client
        .request(
            request
                .body(Full::new(Bytes::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn desktop_registration_requires_daemon_auth_and_local_origin() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        home: dir.path().into(),
        state_dir: dir.path().into(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        login_shell: None,
        legacy_state_dir: None,
        account_state_folder: false,
        keep_alive: true,
        vite_origin: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        idle_secs: 600,
        build: None,
        starter_program: None,
        open_command: None,
        host_helper_root: None,
        host_cli_dir: None,
        pane_id: None,
        tailscale_bin: Some(dir.path().join("missing-tailscale")),
        search_path: None,
    };
    let running = hided::start_daemon(env).await.unwrap();
    let registration = json!({"owner_pid":std::process::id(),"endpoint":"http://127.0.0.1:9322",
        "token":"0123456789abcdef0123456789abcdef"});
    for path in ["/browser-control", "/browser-control/action"] {
        let body = if path.ends_with("/action") {
            json!({"owner_pid":std::process::id(),"device_id":"local","checkout_path":"/checkout",
                "area_id":"area-a","action":"open","url":"about:blank","request_id":"100-open"})
        } else {
            registration.clone()
        };
        assert_eq!(
            request(running.port, path, None, false, body.clone())
                .await
                .0,
            401
        );
        assert_eq!(
            request(running.port, path, Some(&running.token), true, body)
                .await
                .0,
            404
        );
    }
    let accepted = request(
        running.port,
        "/browser-control",
        Some(&running.token),
        false,
        registration.clone(),
    )
    .await;
    assert_eq!(accepted.0, 204);
    assert!(accepted.1.is_empty());
    let mut external = registration;
    external["endpoint"] = json!("http://external.invalid:9322");
    let rejected = request(
        running.port,
        "/browser-control",
        Some(&running.token),
        false,
        external,
    )
    .await;
    assert_eq!(rejected.0, 409);
    assert_eq!(
        serde_json::from_str::<Value>(&rejected.1).unwrap()["reason"],
        "invalid_browser_control"
    );
    running.stop();
}
