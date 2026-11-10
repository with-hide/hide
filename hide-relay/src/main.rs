use hide_relay::{
    Relay,
    apns::{Apns, ProviderKey},
    config::{Config, HELP},
    event::Event,
    limits::{Limits, PushLimits},
};
use std::io::Write;

fn log(event: Event) {
    if let Ok(line) = serde_json::to_string(&event) {
        // A failed collector cannot tear down unrelated socket tasks. The
        // deployment supervises this stderr sink independently of the relay.
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    }
}

fn event(name: &'static str) -> Event {
    Event {
        event: name,
        room: None,
        size: None,
        cap: None,
    }
}

/// Apple's `.p8` and its ids, read once at start. A key that is missing, wide
/// or unreadable stops the relay rather than serving with push quietly off
/// (`docs/RELAY_PROTOCOL.md`, Push relay).
fn apns(config: &hide_relay::config::ApnsConfig) -> Result<Apns, &'static str> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(&config.key).map_err(|_| "apns_key_unreadable")?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("apns_key_permissions");
    }
    let pkcs8 = std::fs::read(&config.key).map_err(|_| "apns_key_unreadable")?;
    let key = ProviderKey::from_pkcs8(
        &pkcs8,
        config.key_id.clone(),
        config.team_id.clone(),
        config.topic.clone(),
    )?;
    let started = std::time::Instant::now();
    let clock: hide_relay::apns::Clock = std::sync::Arc::new(move || started.elapsed());
    // No stand-in means Apple's own hosts, chosen per request from the
    // environment the phone named.
    let endpoint = config.endpoint.clone().unwrap_or_default();
    Apns::new(key, endpoint, PushLimits::default(), clock)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = match Config::parse(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            print!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(event) => {
            log(Event {
                event,
                room: None,
                size: None,
                cap: None,
            });
            return std::process::ExitCode::FAILURE;
        }
    };
    let apns = match config.apns.as_ref().map(apns).transpose() {
        Ok(apns) => apns,
        Err(name) => {
            log(event(name));
            return std::process::ExitCode::FAILURE;
        }
    };
    let Ok(listener) = tokio::net::TcpListener::bind(config.listen).await else {
        log(event("bind_failed"));
        return std::process::ExitCode::FAILURE;
    };
    let started = std::time::Instant::now();
    let relay = Relay::with_push(
        Limits::default(),
        config.trusted_proxy,
        log,
        move || started.elapsed(),
        apns,
    )
    .expect("valid production limits");
    log(Event {
        event: "server_started",
        room: None,
        size: None,
        cap: None,
    });
    if relay.serve(listener, shutdown()).await.is_err() {
        log(Event {
            event: "server_failed",
            room: None,
            size: None,
            cap: None,
        });
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install termination handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("install interrupt handler");
    }
}
