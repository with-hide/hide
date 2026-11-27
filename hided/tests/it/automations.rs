//! Automations through the daemon, on every system: the idle exit beside
//! them (PRD automations B32), where an automation that is on keeps a daemon
//! with no screen and no pane past its idle window and, once none is on, the
//! daemon ends after the window as before; and `hide automation` (B9, B10),
//! the built binary's commands against the daemon and its screen section.

use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A daemon of its own state folder and home that ends one second after its
/// last screen left, when nothing holds it.
fn daemon_env(root: &Path) -> Env {
    let state = root.join("state");
    hide_platform::fs::private::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(root.join("operator")).unwrap();
    Env {
        home: root.join("operator"),
        state_dir: state,
        legacy_state_dir: None,
        account_state_folder: false,
        keep_alive: false,
        vite_origin: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        idle_secs: 1,
        build: None,
        starter_program: None,
        open_command: None,
        host_helper_root: None,
        host_cli_dir: None,
        pane_id: None,
        install: None,
        access: None,
        tailscale_bin: Some(root.join("no-tailscale")),
        relay_url: "ws://127.0.0.1:9/ws".to_owned(),
        search_path: None,
        peer_direct: None,
        peer_relay: None,
        peer_relay_only: false,
        screen_net_hosts: Default::default(),
    }
}

/// The operator's window, the screen that makes and removes automations.
async fn window(running: &hided::RunningDaemon) -> Socket {
    let mut request = format!("ws://127.0.0.1:{}/ws", running.port)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        ORIGIN,
        format!("http://127.0.0.1:{}", running.port)
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let hello = json!({"token": running.token, "schema_version": 2, "client_kind": "web"});
    socket
        .send(Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    socket
}

async fn send(socket: &mut Socket, kind: &str, payload: Value) {
    let event =
        json!({"schema_version": hide_core::SCHEMA_VERSION, "kind": kind, "payload": payload});
    socket
        .send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}

/// The names in the `automations` section of the first frame carrying one.
fn automation_names(frame: &Value) -> Option<Vec<String>> {
    match frame {
        Value::Object(map) => map
            .get("automations")
            .and_then(|section| section["items"].as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .or_else(|| map.values().find_map(automation_names)),
        Value::Array(values) => values.iter().find_map(automation_names),
        _ => None,
    }
}

/// Reads frames until the automations section lists exactly `names`.
async fn listed(socket: &mut Socket, names: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = tokio::time::timeout(left, socket.next())
            .await
            .unwrap_or_else(|_| panic!("the section never listed {names:?}"))
            .expect("the connection closed")
            .unwrap();
        let Message::Text(text) = frame else { continue };
        let frame: Value = serde_json::from_str(&text).unwrap();
        if automation_names(&frame).is_some_and(|listed| listed == names) {
            return;
        }
    }
}

async fn serving(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_automation_that_is_on_keeps_an_idle_daemon_until_none_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(dir.path())).await.unwrap();

    let mut screen = window(&running).await;
    send(
        &mut screen,
        "automation_add",
        json!({
            "name": "Nightly", "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": "03:00"}, "agent": {"kind": "claude"},
        }),
    )
    .await;
    listed(&mut screen, &["Nightly"]).await;
    screen.close(None).await.unwrap();
    drop(screen);

    // Past the idle window with no screen and no pane: held by the
    // automation that is on.
    tokio::time::sleep(Duration::from_secs(4)).await; // wait-ok: the 1 s idle window has passed with no screen and no pane
    assert!(
        serving(running.port).await,
        "the daemon ended with an automation on"
    );

    let mut screen = window(&running).await;
    // An automation is named by its id or its name.
    send(&mut screen, "automation_remove", json!({"id": "Nightly"})).await;
    listed(&mut screen, &[]).await;
    screen.close(None).await.unwrap();
    drop(screen);

    let deadline = Instant::now() + Duration::from_secs(15);
    while serving(running.port).await {
        assert!(
            Instant::now() < deadline,
            "the daemon outlived its idle window with no automation on"
        );
        tokio::time::sleep(Duration::from_millis(200)).await; // wait-ok: the daemon has ended and closed its port
    }
}

/// `hide automation …` run as an approved bot in `mode`: the `hide` binary
/// itself against this daemon, with no pane.
async fn acting_tool(
    root: &Path,
    window: &mut crate::access::Window,
    name: &str,
    mode: Value,
) -> crate::access::Tool {
    let tool = crate::access::Tool::new(root, &format!("{name}-home"));
    let id = tool.put_up(name);
    window
        .send("access_approve", json!({"id": id, "mode": mode}))
        .await;
    let connected = tool.request_until_answered(name).await;
    assert!(connected.ok, "{}", connected.out);
    tool
}

/// A refusal with `code`, and the sentence it gives, which names what to
/// change.
fn refused(ran: &crate::access::Ran, code: &str) -> String {
    assert!(!ran.ok, "{code} expected, the command ran: {}", ran.out);
    let line = ran.json();
    assert_eq!(line["error"]["code"], code, "{}", ran.out);
    line["error"]["message"].as_str().unwrap().to_owned()
}

fn wall_clock_at(value: &Value, time: &str) -> bool {
    value
        .as_str()
        .is_some_and(|at| at.len() == "2026-11-20 09:00".len() && at.ends_with(&format!(" {time}")))
}

/// PRD automations c1 (B9, B10): the CLI makes, reads, changes, pauses,
/// resumes and removes an automation through the core, answers what its
/// schedule reads as and when it runs next, the screen's section shows it
/// at once, and a period under five minutes or a cron line that is none is
/// refused with why and what to do.
// The daemon runs on this test's runtime while the tool's `hide` blocks a
// thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cli_makes_changes_and_removes_an_automation_the_screen_shows() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = daemon_env(dir.path());
    env.keep_alive = true;
    env.idle_secs = 600;
    // A plain-folder project Hide has open, by its real path.
    let project = hide_platform::fs::identity::canonical(&env.home)
        .unwrap()
        .join("projects")
        .join("issues");
    std::fs::create_dir_all(project.join("notes")).unwrap();
    let project_wire = hide_platform::path::to_wire(&project).unwrap();
    std::fs::write(
        env.state_dir.join("core-state.json"),
        json!({
            "schema_version": 1,
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "workspace_registrations": [{"id": "w-issues", "label": "issues", "path": project_wire}],
        })
        .to_string(),
    )
    .unwrap();
    let running = hided::start_daemon(env).await.unwrap();
    let mut settings = crate::access::Window::open(&running).await;
    let tool = acting_tool(
        dir.path(),
        &mut settings,
        "scheduler",
        json!({"kind": "all"}),
    )
    .await;
    let mut screen = window(&running).await;

    let made = tool.run(&[
        "automation",
        "add",
        "--name",
        "Morning issues",
        "--task",
        "Sort the new issues\nand label them",
        "--schedule",
        "weekdays 09:00",
        "--kind",
        "claude",
        "--effort",
        "high",
    ]);
    assert!(made.ok, "{}", made.out);
    let automation = &made.json()["value"]["automation"];
    assert_eq!(automation["name"], "Morning issues");
    assert_eq!(automation["task"], "Sort the new issues\nand label them");
    assert_eq!(automation["reads_as_text"], "weekdays at 09:00");
    assert!(
        wall_clock_at(&automation["next_run_at"], "09:00"),
        "{automation}"
    );
    assert_eq!(
        automation["agent"],
        json!({"kind": "claude", "effort": "high"})
    );
    // No folder named: the automation's own, under the home's hide folder.
    assert_eq!(automation["folder"]["default"], true);
    assert!(
        automation["folder"]["path"]
            .as_str()
            .unwrap()
            .contains("hide/automations/Morning-issues-"),
        "{automation}"
    );
    let id = automation["id"].as_str().unwrap().to_owned();
    // The screen's section lists it at once (B9).
    listed(&mut screen, &["Morning issues"]).await;

    // What may not run is refused with why (B7, B10).
    let draft = |schedule: &str, name: &str| {
        tool.run(&[
            "automation",
            "add",
            "--name",
            name,
            "--task",
            "Check the stale pull requests",
            "--schedule",
            schedule,
            "--kind",
            "codex",
        ])
    };
    let too_often = refused(
        &draft("cron */4 * * * *", "Too often"),
        "schedule_too_often",
    );
    assert!(
        too_often.contains("at most every five minutes"),
        "{too_often}"
    );
    refused(
        &draft("every 7m", "Seven minutes"),
        "schedule_minutes_out_of_range",
    );
    refused(
        &draft("cron 0 9 * *", "Four fields"),
        "schedule_cron_invalid",
    );
    refused(&draft("cron 0 9 30 2 *", "February"), "schedule_never");
    refused(
        &draft("every 13h 09:00", "Too far apart"),
        "schedule_hours_out_of_range",
    );
    refused(&draft("every morning", "Words"), "schedule_unreadable");
    refused(&draft("daily 9am", "Morning"), "schedule_time_invalid");
    refused(
        &draft("weekdays 09:00", "Morning issues"),
        "automation_name_taken",
    );
    // A project's root folder is the project's; a folder inside it is not
    // one Hide has open (B5).
    let inside = project.join("notes");
    refused(
        &tool.run(&[
            "automation",
            "add",
            "--name",
            "Notes",
            "--task",
            "Tidy the notes",
            "--schedule",
            "daily 08:00",
            "--kind",
            "claude",
            "--folder",
            inside.to_str().unwrap(),
        ]),
        "automation_folder_unknown",
    );
    let in_project = tool.run(&[
        "automation",
        "add",
        "--name",
        "Project notes",
        "--task",
        "Tidy the notes",
        "--schedule",
        "daily 08:00",
        "--kind",
        "claude",
        "--folder",
        project.to_str().unwrap(),
    ]);
    assert!(in_project.ok, "{}", in_project.out);
    let folder = &in_project.json()["value"]["automation"]["folder"];
    assert_eq!(folder["default"], false, "{folder}");
    assert_eq!(folder["path"], project_wire.as_str(), "{folder}");
    assert!(tool.run(&["automation", "remove", "Project notes"]).ok);
    let preview = tool.run(&["automation", "preview", "--schedule", "cron 0 9 * * 1-5"]);
    assert!(preview.ok, "{}", preview.out);
    // A cron line equal to a choice reads as that choice (B6).
    assert_eq!(
        preview.json()["value"]["reads_as_text"],
        "weekdays at 09:00"
    );
    // Every N minutes is a choice too, and the cron line it equals reads as
    // it (B6, D-16).
    for schedule in ["every 10m", "cron */10 * * * *"] {
        let preview = tool.run(&["automation", "preview", "--schedule", schedule]);
        assert!(preview.ok, "{}", preview.out);
        assert_eq!(
            preview.json()["value"]["reads_as_text"],
            "every 10 minutes",
            "{schedule}"
        );
    }

    // Read by its name or its id.
    let shown = tool.run(&["automation", "show", "Morning issues"]);
    assert!(shown.ok, "{}", shown.out);
    assert_eq!(shown.json()["value"]["automation"]["id"], id.as_str());
    let list = tool.run(&["automation", "list"]).json();
    assert_eq!(list["value"]["items"][0]["id"], id.as_str());
    assert!(list["value"]["items"][0].get("records").is_none());

    // Changed from the next run on (B26).
    let edited = tool.run(&[
        "automation",
        "edit",
        &id,
        "--schedule",
        "weekly mon,thu 18:30",
        "--name",
        "Issue sweep",
    ]);
    assert!(edited.ok, "{}", edited.out);
    let automation = &edited.json()["value"]["automation"];
    assert_eq!(automation["reads_as_text"], "every mon, thu at 18:30");
    assert!(
        wall_clock_at(&automation["next_run_at"], "18:30"),
        "{automation}"
    );
    listed(&mut screen, &["Issue sweep"]).await;

    // Paused, it has no next time; on again, it has one (B25).
    let paused = tool.run(&["automation", "pause", "Issue sweep"]).json();
    assert_eq!(
        paused["value"]["automation"]["paused"]["reason"],
        "operator"
    );
    assert!(paused["value"]["automation"]["next_run_at"].is_null());
    let resumed = tool.run(&["automation", "resume", &id]).json();
    assert!(resumed["value"]["automation"]["paused"].is_null());
    assert!(wall_clock_at(
        &resumed["value"]["automation"]["next_run_at"],
        "18:30"
    ));

    // The automations are no one bot's work: a bot of its own work neither
    // reads nor changes them (mission-87).
    let held = acting_tool(dir.path(), &mut settings, "watcher", json!({"kind": "own"})).await;
    for args in [&["automation", "list"][..], &["automation", "pause", &id]] {
        let ran = held.run(args);
        assert!(
            !ran.ok && ran.out.contains("access_not_own_work"),
            "{args:?}: {}",
            ran.out
        );
    }

    let removed = tool.run(&["automation", "remove", "Issue sweep"]);
    assert!(removed.ok, "{}", removed.out);
    assert_eq!(removed.json()["value"]["removed"]["name"], "Issue sweep");
    listed(&mut screen, &[]).await;
    refused(
        &tool.run(&["automation", "show", &id]),
        "automation_unknown",
    );
    running.stop();
}
