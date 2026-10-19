//! Explicit, personless PostHog events. No SDK, automatic properties or content.
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use herdr_core::product_analytics::{Event, Sink};
use serde_json::json;

const ENDPOINT: &str = "https://us.i.posthog.com/i/v0/e/";
const QUEUE_LIMIT: usize = 128;
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

struct Capture {
    event: Event,
    timestamp: String,
}

#[derive(Default)]
struct State {
    enabled: bool,
    closed: bool,
    queue: VecDeque<Capture>,
}

#[derive(Default)]
pub(crate) struct Client {
    state: Mutex<State>,
    wake: Condvar,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProductAnalyticsClient")
    }
}

impl Sink for Client {
    fn set_enabled(&self, enabled: bool) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.enabled = enabled;
        // Disabled events are never retained or replayed after re-enabling.
        if !enabled {
            state.queue.clear();
        }
    }

    fn capture(&self, event: Event) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.enabled || state.closed {
            return;
        }
        if state.queue.len() >= QUEUE_LIMIT {
            drop(state);
            diagnostic("queue_full");
            return;
        }
        state.queue.push_back(Capture {
            event,
            timestamp: jiff::Timestamp::now().to_string(),
        });
        self.wake.notify_one();
    }
}

pub(crate) struct Worker {
    pub(crate) client: Arc<Client>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(crate) fn start(
        home: Option<&Path>,
        state_dir: &Path,
        token: String,
    ) -> Result<Option<Self>, &'static str> {
        if let Some(home) = home {
            let disabled = hide_kit::layout::product_analytics_disabled(home)
                .try_exists()
                .map_err(|_| "disable_marker_unreadable")?;
            if disabled || crate::env::fixture_home(home) {
                return Ok(None);
            }
        }
        let id = installation_id(state_dir)?;
        Self::spawn(id, token, ENDPOINT.to_owned()).map(Some)
    }

    fn spawn(id: String, token: String, endpoint: String) -> Result<Self, &'static str> {
        let client = Arc::new(Client::default());
        let shared = Arc::clone(&client);
        let thread = std::thread::Builder::new()
            .name("product-analytics".into())
            .spawn(move || run(shared, id, token, endpoint))
            .map_err(|_| "worker_start_failed")?;
        Ok(Self {
            client,
            thread: Some(thread),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        {
            let mut state = self
                .client
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            state.queue.clear();
            self.client.wake.notify_one();
        }
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            diagnostic("worker_join_failed");
        }
    }
}

fn run(client: Arc<Client>, id: String, token: String, endpoint: String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    loop {
        let capture = {
            let mut state = client
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !state.closed && state.queue.is_empty() {
                state = client
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                return;
            }
            // Taking this item starts the single in-flight send. Opt-out drops
            // every waiting item; it cannot recall a request already started.
            state.queue.pop_front().expect("a nonempty queue")
        };
        let uuid = match random_id() {
            Ok(uuid) => uuid,
            Err(reason) => {
                diagnostic(reason);
                continue;
            }
        };
        let body = json!({
            "api_key": token,
            "event": capture.event.name(),
            "uuid": uuid,
            "timestamp": capture.timestamp,
            "properties": {
                "distinct_id": id,
                "$process_person_profile": false,
                "$geoip_disable": true,
                "$ip": null
            }
        })
        .to_string();
        // No retries or disk spool: delivery is best effort, and external
        // availability never holds up the IDE or a provider request.
        match agent
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .send(body.as_bytes())
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => herdr_core::diagnostic!(json!({
                "component": "product_analytics", "kind": "http_refused",
                "status": response.status().as_u16()
            })),
            Err(_) => diagnostic("send_failed"),
        }
    }
}

fn random_id() -> Result<String, &'static str> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "random_unavailable")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn installation_id(state_dir: &Path) -> Result<String, &'static str> {
    let path = hide_kit::layout::product_analytics_id(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(id) if valid_id(&id) => Ok(id),
        Ok(_) => Err("installation_id_invalid"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let id = random_id()?;
            hide_platform::fs::private::create_dir_all(state_dir)
                .map_err(|_| "identity_directory_failed")?;
            hide_platform::fs::atomic::write_file(
                &path,
                id.as_bytes(),
                hide_platform::fs::Access::Private,
            )
            .map_err(|_| "identity_write_failed")?;
            Ok(id)
        }
        Err(_) => Err("identity_read_failed"),
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn diagnostic(kind: &str) {
    herdr_core::diagnostic!(json!({ "component": "product_analytics", "kind": kind }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn a_fixture_marker_prevents_transport_even_with_a_configured_token() {
        let folder = tempfile::tempdir().unwrap();
        let home = folder.path().join("home");
        let state = home.join(".hide/state");
        std::fs::create_dir_all(home.join(".hide")).unwrap();
        // A general fixture, without the privileged /tmp device-home marker.
        assert!(!crate::env::fixture_home(&home));
        std::fs::write(hide_kit::layout::product_analytics_disabled(&home), "").unwrap();
        let worker = Worker::start(Some(&home), &state, "phc_fixture_public_token".into()).unwrap();
        assert!(worker.is_none(), "a fixture must start no network worker");
        assert!(!hide_kit::layout::product_analytics_id(&state).exists());
    }

    // The real transport talks to an HTTP peer held at the response boundary.
    // Opt-out must discard waiting/disabled calls while allowing the one
    // already sent to finish, without leaking provider input into its JSON.
    #[test]
    fn wire_is_personless_and_opt_out_drops_waiting_events() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/", server.local_addr().unwrap());
        let (received, bodies) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let peer = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let mut length = None;
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let mut body = vec![0; length.unwrap()];
                reader.read_exact(&mut body).unwrap();
                received
                    .send(serde_json::from_slice::<serde_json::Value>(&body).unwrap())
                    .unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            }
        });
        let folder = tempfile::tempdir().unwrap();
        let id = installation_id(folder.path()).unwrap();
        assert_eq!(installation_id(folder.path()).unwrap(), id);
        let worker =
            Worker::spawn(id.clone(), "phc_fixture_public_token".into(), endpoint).unwrap();
        worker.client.set_enabled(true);
        worker.client.capture(Event::AiCalled);
        let first = bodies.recv_timeout(Duration::from_secs(10)).unwrap();
        worker.client.capture(Event::AiCalled);
        worker.client.set_enabled(false);
        worker.client.capture(Event::AiCalled);
        worker.client.set_enabled(true);
        worker.client.capture(Event::Connected);
        release.send(()).unwrap();
        let second = bodies.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first["event"], "hide_ai_called");
        assert_eq!(second["event"], "hide_connected");
        assert_ne!(first["uuid"], second["uuid"]);
        for body in [first, second] {
            let mut keys: Vec<_> = body
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["api_key", "event", "properties", "timestamp", "uuid"]
            );
            assert_eq!(
                body["properties"],
                json!({
                    "distinct_id": id, "$process_person_profile": false,
                    "$geoip_disable": true, "$ip": null
                })
            );
            assert!(valid_id(body["uuid"].as_str().unwrap()));
            body["timestamp"]
                .as_str()
                .unwrap()
                .parse::<jiff::Timestamp>()
                .unwrap();
        }
        release.send(()).unwrap();
        drop(worker);
        peer.join().unwrap();
    }
}
