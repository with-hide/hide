use hide_relay::{
    Relay,
    apns::{load, unix_clock},
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

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let config = match Config::parse(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            print!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(event) => {
            log(Event::new(event, None));
            return std::process::ExitCode::FAILURE;
        }
    };
    let apns = match config
        .apns
        .as_ref()
        .map(|apns| load(apns, PushLimits::default(), unix_clock()))
        .transpose()
    {
        Ok(apns) => apns,
        Err(name) => {
            log(Event::new(name, None));
            return std::process::ExitCode::FAILURE;
        }
    };
    let Ok(listener) = tokio::net::TcpListener::bind(config.listen).await else {
        log(Event::new("bind_failed", None));
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
    log(Event::new("server_started", None));
    if relay.serve(listener, shutdown()).await.is_err() {
        log(Event::new("server_failed", None));
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
