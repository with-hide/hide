pub mod agent_cli;
pub mod attach;
pub mod attachments;
pub mod backoff;
pub mod boundary;
mod browser_assets;
pub mod browser_cli;
pub mod browser_control;
pub mod browser_page;
pub mod browser_relay;
pub mod browser_routes;
pub mod build_id;
pub mod build_order;
pub mod cli;
pub mod cli_contract;
pub mod core;
pub mod core_move;
pub mod core_update;
pub mod delivery_cli;
pub mod demand;
pub mod desktop_notices;
pub mod ending;
pub mod env;
pub mod file_url;
pub mod guide;
pub mod herdr_handoff;
pub mod index;
pub mod login_item;
pub mod mission_cli;
pub mod mobile;
pub mod node_browser;
pub mod node_cli;
pub mod node_daemon;
pub mod node_pages;
pub mod node_uploads;

mod hook_request;
pub mod node_panes;
pub mod node_role;
mod owned_task;
pub mod pane_auth;
mod pane_sizes;
pub mod placement;
mod product_analytics;
pub mod relay;
pub mod screen_event;
pub mod seat;
pub mod server;
pub mod shell_exec;
pub mod spawn;
pub mod state_file;
// The old folder is this Mac's history; no Windows build ever wrote it.
#[cfg(unix)]
pub mod state_move;
mod supervisor;
pub mod terminal_hub;
pub mod watch;
pub mod workspace_cli;

use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use herdr_core::{CoreOptions, SCHEMA_VERSION};
use tokio::sync::Notify;

use crate::attachments::Attachments;
use crate::boundary::{Boundary, Root};
use crate::core::CoreHandle;
use crate::env::Env;
use crate::index::IndexService;
use crate::server::AppState;
use crate::state_file::{DaemonState, acquire_lock, forget_daemon, new_token, write_state};

/// The release this build ships as (`HIDE_VERSION` when a package was
/// built, else the crate's own), which the shell shows as the app's version
/// (PRD auto-update D-26) rather than the crate number every build shares.
pub const VERSION: &str = env!("HIDE_BUILD_VERSION");

/// The name of the machine this daemon runs on, which Settings names as the
/// owner of every value the daemon stores (PRD S5.5 B35); `None` when the
/// system will not say, which the page shows as unavailable rather than a guess.
pub(crate) fn host_name() -> Option<String> {
    hide_platform::host::name().ok()
}

/// A name for this start of the core role, drawn fresh each time, so a page
/// that outlived it (a move, a rollback, an update) never applies a delta
/// over state another core numbered.
fn core_instance() -> String {
    let mut instance = [0_u8; 8];
    getrandom::getrandom(&mut instance).expect("getrandom");
    hex::encode(instance)
}

pub(crate) fn find_ui_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("HIDED_UI_DIR") {
        let path = std::path::PathBuf::from(dir);
        if path.join("index.html").is_file() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(std::path::Path::to_path_buf);
        for _ in 0..5 {
            let Some(current) = dir else { break };
            candidates.push(current.join("web/dist"));
            dir = current.parent().map(std::path::Path::to_path_buf);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("web/dist"));
        candidates.push(cwd.join("dist"));
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join("web/dist"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.join("index.html").is_file())
}

pub struct RunningDaemon {
    pub port: u16,
    pub token: String,
    shutdown: ending::Ending,
    /// Ends the daemon after its screens left; aborted with the role.
    idle: tokio::task::JoinHandle<()>,
    pane_capabilities: Arc<pane_auth::Registry>,
    pane_bootstrap_socket: std::path::PathBuf,
    pane_bootstrap_record: std::path::PathBuf,
    /// Where nodes on other machines reach this core (PRD
    /// core-host-node-remote-core D-07); removed with the daemon.
    node_attach_socket: std::path::PathBuf,
    node_attach_record: std::path::PathBuf,
    pub mobile: Arc<mobile::Mobile>,
    /// Ended on drop, before the instance lock is released: the core's last
    /// layout and state saves are on disk before another daemon can start,
    /// and before whoever owns the state folder can remove it.
    core: Arc<CoreHandle>,
    /// Closed while this core runs on a move's copy that has not committed.
    pub move_gate: Arc<core_move::gate::MoveGate>,
    /// Told when the role ends, before the core stops, so the role's sockets
    /// close (`AppState::role_ended`).
    role_ended: tokio::sync::watch::Sender<bool>,
    /// The server and instance lock of a daemon started on its own
    /// (`start_daemon`); a process's supervisor keeps them otherwise.
    own: Option<OwnSeat>,
}

/// A seat and instance lock a role holds for itself, released after the
/// role ends, in this order.
struct OwnSeat {
    _seat: seat::Seat,
    state_dir: std::path::PathBuf,
    _lock: hide_platform::fs::lock::Lock,
}

impl Drop for OwnSeat {
    fn drop(&mut self) {
        forget_daemon(&self.state_dir, std::process::id());
    }
}

impl RunningDaemon {
    fn remove_bootstrap_socket(&self) {
        for (record, socket) in [
            (&self.pane_bootstrap_record, &self.pane_bootstrap_socket),
            (&self.node_attach_record, &self.node_attach_socket),
        ] {
            let _ = std::fs::remove_file(record);
            let _ = std::fs::remove_file(socket);
            if let Some(directory) = socket.parent() {
                let _ = std::fs::remove_dir(directory);
            }
        }
    }

    pub fn stop(&self) {
        self.pane_capabilities.revoke_all();
        self.remove_bootstrap_socket();
        self.shutdown.end();
    }
}

impl Drop for RunningDaemon {
    fn drop(&mut self) {
        self.role_ended.send_replace(true);
        self.shutdown.end();
        self.idle.abort();
        self.pane_capabilities.revoke_all();
        self.remove_bootstrap_socket();
        self.core.shutdown();
    }
}

/// The daemon in the node role (PRD core-host-node-remote-core D-02): this
/// machine's node, linked to a core on another machine, and its screens.
/// No core runs here.
pub struct RunningNode {
    pub port: u16,
    pub token: String,
    shutdown: ending::Ending,
    idle: tokio::task::JoinHandle<()>,
    /// Ended on drop, before the instance lock below is released (fields
    /// drop in this order): the terminals relay, the route reaper, and the
    /// link with its SSH connection and threads, which only this holds.
    pub node: node_daemon::NodeDaemon,
    own: Option<OwnSeat>,
}

impl RunningNode {
    pub fn stop(&self) {
        self.shutdown.end();
    }
}

impl Drop for RunningNode {
    fn drop(&mut self) {
        self.shutdown.end();
        self.idle.abort();
    }
}

/// The process's supervisor: it holds the seat (port, token, instance lock)
/// for the process's life and runs the role the state folder names in it.
pub async fn run_daemon(env: Env) -> Result<(), String> {
    supervisor::run(env).await
}

/// A core daemon on a seat of its own, for a caller that is not a process's
/// supervisor (the integration tests); the seat and lock end with it.
pub async fn start_daemon(env: Env) -> Result<RunningDaemon, String> {
    // Refused before anything is written: a daemon that came up on this
    // path would answer healthy and then fail every pane attach in turn.
    if let Some(error) = env::herdr_bin_error(&env) {
        return Err(error);
    }
    let lock = acquire_lock(&env.state_dir).map_err(|error| error.to_string())?;
    let seat = seat::Seat::serve(
        server::bind(env.bind).await?,
        new_token(),
        ending::Ending::new(),
    )?;
    let state_dir = env.state_dir.clone();
    let herdr_socket = env.herdr_socket_path.clone();
    let mut running = start_core_role(env, seat.parts()).await?;
    // Recorded once the daemon runs, so a start that failed leaves no record
    // naming a process that is gone.
    record_daemon(&state_dir, &seat.parts(), herdr_socket)?;
    seat.end_with(running.shutdown.clone(), state_dir.clone());
    running.own = Some(OwnSeat {
        _seat: seat,
        state_dir,
        _lock: lock,
    });
    Ok(running)
}

/// Records the process holding `seat` in `state_dir` (`hided.json`), which
/// `hide connect` and the desktop host find it by, whichever role it runs.
pub(crate) fn record_daemon(
    state_dir: &std::path::Path,
    seat: &seat::SeatParts,
    herdr_socket: Option<String>,
) -> Result<(), String> {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into());
    let pid = std::process::id();
    let state = DaemonState {
        pid,
        port: seat.port,
        token: seat.token.clone(),
        socket: herdr_socket,
        started_at,
        pid_started: hide_platform::process::start_time(pid).ok(),
    };
    write_state(state_dir, &state)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// [`Env::herdr_program`] off the async runtime: the account's search can
/// wait on its login shell.
async fn herdr_program(env: &Env) -> Result<Option<std::path::PathBuf>, String> {
    let env = env.clone();
    tokio::task::spawn_blocking(move || env.herdr_program())
        .await
        .map_err(|error| format!("the herdr search did not finish: {error}"))
}

/// The core role on `seat`, which already holds the instance lock.
pub async fn start_core_role(env: Env, seat: seat::SeatParts) -> Result<RunningDaemon, String> {
    seat.starting("core.state");
    // This machine's name in every key that names it. A machine Hide cannot
    // name refuses to start rather than guess one (PRD core-host-node D-28).
    let node = herdr_core::node::NodeId::of_this_machine()?;
    // A machine whose core runs elsewhere starts the node role instead.
    if placement::read(&env.state_dir, node.as_str())?.is_some() {
        return Err(
            "the core runs on another machine: this state folder starts the node role".to_owned(),
        );
    }
    // A core on a move's copy waits for the move's own link; a folder whose
    // core left it in a move starts none (PRD core-host-node-move B8).
    let move_gate = core_move::gate::MoveGate::for_start(&env.state_dir)?;
    // Stored state from before node ids is converted once, before the core
    // reads it; a store that cannot be converted stops the start and names
    // the file (PRD core-host-node B2).
    let converted =
        herdr_core::node_migration::convert(&env.state_dir, &node).map_err(|refusal| {
            let record = serde_json::json!({
                "component": "hided",
                "kind": "node_migration.refused",
                "pid": std::process::id(),
                "at_unix_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or_default(),
                "node": node.as_str(),
                "file": refusal.file.display().to_string(),
                "reason": &refusal.reason,
            });
            if let Err(error) =
                herdr_core::diagnostics::record_now(&env.state_dir.join("core-state.json"), record)
            {
                eprintln!("the start refusal could not be logged: {error}");
            }
            refusal.to_string()
        })?;
    if !converted.files.is_empty() {
        let record = serde_json::json!({
            "component": "hided",
            "kind": "node_migration.converted",
            "node": node.as_str(),
            "files": converted.files,
            "projects": converted.projects,
            "retention_rows": converted.retention_rows,
        });
        if let Err(error) =
            herdr_core::diagnostics::record_now(&env.state_dir.join("core-state.json"), record)
        {
            eprintln!("the conversion could not be logged: {error}");
        }
    }
    let host_id = state_file::host_id(&env.state_dir)
        .map_err(|error| format!("the daemon host id could not be read or written: {error}"))?;
    let token = seat.token.clone();
    let port = seat.port;
    seat.starting("core.herdr_program");
    let herdr_bin = herdr_program(&env).await?;
    match herdr_bin.as_ref() {
        Some(path) => eprintln!(
            "{}",
            serde_json::json!({
                "component": "hided",
                "kind": "herdr_bin.resolved",
                "path": path.display().to_string(),
            })
        ),
        None => eprintln!(
            "{}",
            serde_json::json!({
                "component": "hided",
                "kind": "herdr_bin.missing",
                "message": "no herdr binary: HERDR_BIN_PATH is unset and neither this build's pinned copy nor the account's search has one; pane terminals cannot attach",
            })
        ),
    }
    let options = CoreOptions {
        schema_version: SCHEMA_VERSION,
        // A relative HOME names no account folder; the core then reports
        // HOME invalid and turns off what needs it, as it always has.
        home: env
            .home
            .is_absolute()
            .then(|| env.home.display().to_string()),
        node_id: node.clone(),
        herdr_socket_path: env.herdr_socket_path.clone(),
        herdr_bin_path: herdr_bin.as_ref().map(|path| path.display().to_string()),
        app_state_path: env.state_dir.join("core-state.json").display().to_string(),
        host_helper_root: env.host_helper_root.clone(),
        host_cli_dir: env.host_cli_dir.clone(),
        // The web shell draws separate Agent and View areas; each
        // Workspace's presentation lives in its own versioned file (S6 D-10).
        workspace_views_path: Some(
            env.state_dir
                .join("workspace-views.json")
                .display()
                .to_string(),
        ),
        // The removed native app's state at its release path, read once for the pane
        // chords the operator set there (desktop PRD follow-up, user decision
        // 2026-09-26). It follows HOME, so an isolated run reads its own. The
        // app only ever ran on macOS.
        shortcut_import_path: cfg!(target_os = "macos").then(|| {
            env.home
                .join("Library/Application Support/hide/state.json")
                .display()
                .to_string()
        }),
        local_issues_path: Some(
            env.state_dir
                .join("local-issues.json")
                .display()
                .to_string(),
        ),
        // A core on a move's copy acts on nothing outside this machine
        // until the move's link commits it (PRD core-host-node-move
        // amendment 3).
        effects_held: !move_gate.is_open(),
        machine_name: host_name(),
        account_state_folder: env.account_state_folder,
    };
    let effects_held = options.effects_held;
    let boundary = Arc::new(boundary::Boundary::for_node(&env.home, node)?);
    // Each device link's pane traffic goes here from the start; answered
    // once the server's state exists, below.
    let node_panes = Arc::new(node_panes::NodePanes::new(tokio::runtime::Handle::current()));
    let pane_events: hide_node::ssh::PaneEventsSlot = Default::default();
    let _ = pane_events.set(Arc::new(node_panes::Events(Arc::clone(&node_panes))));
    seat.starting("core.owner");
    let core = Arc::new(CoreHandle::spawn(options, Arc::clone(&pane_events))?);
    seat.starting("core.services");
    if effects_held {
        let mut open = move_gate.subscribe();
        let core = Arc::clone(&core);
        tokio::spawn(async move {
            if open.wait_for(|open| *open).await.is_ok()
                && let Ok(Err(message)) =
                    tokio::task::spawn_blocking(move || core.release_effects()).await
            {
                herdr_core::diagnostic!(serde_json::json!({
                    "component": "core_move",
                    "kind": "effects.release_failed",
                    "message": message,
                }));
            }
        });
    }
    // After the core installed the diagnostic log beside its state.
    #[cfg(unix)]
    state_move::log_left_behind(env.legacy_state_dir.as_deref(), &env.state_dir);
    let pane_capabilities = Arc::new(pane_auth::Registry::new(&env.state_dir)?);
    let (pane_listener, pane_bootstrap_socket) = pane_auth::bind(&env.state_dir)?;
    let (attach_listener, node_attach_socket) = attach::bind(&env.state_dir)?;
    let relay_grants = Arc::new(attach::RelayGrants::default());
    let watch = Arc::new(watch::WatchService::new(Arc::clone(&core)));
    let index = Arc::new(IndexService::new());
    let attachments = Arc::new(Attachments::new(&env.state_dir));
    let shutdown = ending::Ending::new();
    let (role_ended, _) = tokio::sync::watch::channel(false);
    let supervisor_exe = std::env::current_exe()
        .map_err(|error| format!("cannot resolve opener supervisor executable: {error}"))?;
    let opener = hide_node::opener::OpenHandler::new(
        env.open_command.clone(),
        shutdown.subscribe(),
        supervisor_exe,
    );
    let roots = Arc::new(RootFollower::new(
        Arc::clone(&core),
        Arc::clone(&boundary),
        Arc::clone(&watch),
        Arc::clone(&index),
    ));
    let browser_routes = browser_routes::BrowserRoutes::new(
        Arc::clone(&core) as Arc<dyn browser_routes::PageSources>
    );
    let desktop_renderers = Arc::new(AtomicUsize::new(0));
    let _reaper = browser_routes.spawn_reaper(Arc::clone(&desktop_renderers), shutdown.clone());
    let renderers = Arc::new(AtomicUsize::new(0));
    let start_demand = Arc::new(demand::ObservationDemand::default());
    let desktop_notices = Arc::new(desktop_notices::DesktopNotices::default());
    seat.starting("core.mobile");
    let mobile = mobile::Mobile::start(mobile::Config {
        state_dir: env.state_dir.clone(),
        home: env.home.clone(),
        port,
        cli: mobile::tailscale::CliSource {
            pinned: env.tailscale_bin.clone(),
            search_path: env.search_path.clone(),
        },
        host_name: host_name(),
        core: Arc::clone(&core),
        input_gate: core.input_gate()?,
        herdr_socket: env.herdr_socket_path.as_ref().map(std::path::PathBuf::from),
        renderers: Arc::clone(&renderers),
        start_demand: Arc::clone(&start_demand),
        open: move_gate.subscribe(),
        desktop: Arc::clone(&desktop_notices),
    });
    let app = AppState {
        core: Arc::clone(&core),
        boundary,
        roots,
        watch: Arc::clone(&watch),
        index: Arc::clone(&index),
        attachments: Arc::clone(&attachments),
        opener,
        token: Arc::new(token.clone()),
        pane_capabilities: Arc::clone(&pane_capabilities),
        browser_routes,
        browser_control: Arc::new(browser_control::BrowserControl::default()),
        browser_comments: Arc::default(),
        herdr_socket: env.herdr_socket_path.as_ref().map(std::path::PathBuf::from),
        allowed_origins: Arc::new(server::allowed_origins(port, env.vite_origin.as_deref())),
        clients: Arc::new(AtomicUsize::new(0)),
        renderers,
        desktop_renderers,
        connections: Arc::new(AtomicU64::new(0)),
        last_client_gone: Arc::new(Mutex::new(Instant::now())),
        keep_alive: env.keep_alive,
        idle_secs: env.idle_secs,
        build: env.build.as_deref().map(Arc::from),
        starter_program: env
            .starter_program
            .as_deref()
            .map(|program| Arc::from(program.to_string_lossy().as_ref())),
        renderer_transitions: Arc::new(Mutex::new(())),
        shutdown: shutdown.clone(),
        role_ended: role_ended.subscribe(),
        move_gate: Arc::clone(&move_gate),
        ui_dir: if server::has_embedded_ui() {
            None
        } else {
            find_ui_dir()
        },
        version: VERSION,
        seat: seat.clone(),
        demand: Arc::new(demand::ObservationDemand::default()),
        start_demand,
        request_view_demand: Arc::new(demand::ObservationDemand::default()),
        origin_holds: Arc::new(demand::OriginHolds::default()),
        daemon_info: Arc::new(serde_json::json!({
            "version": VERSION,
            "schema_version": SCHEMA_VERSION,
            "host_id": host_id,
            "host_name": host_name(),
            "pid": std::process::id(),
            "started_at_unix": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_else(|_| "0".into()),
            "state_dir": env.state_dir.display().to_string(),
            "core_state_path": env.state_dir.join("core-state.json").display().to_string(),
            "herdr_bin_path": herdr_bin.as_ref().map(|path| path.display().to_string()),
            "herdr_socket_path": env.herdr_socket_path.clone(),
            "keep_alive": env.keep_alive,
            "idle_secs": env.idle_secs,
            "core_instance": core_instance(),
        })),
        mobile: Arc::clone(&mobile),
        desktop_notices,
        relay_grants: Arc::clone(&relay_grants),
        own_screens: Arc::new(Mutex::new(0)),
        pane_sizes: core.outputs.pane_sizes(),
    };
    seat.starting("core.roots");
    node_panes.serve(app.clone());
    // Seeded before the server accepts a client with the registrations the
    // core loaded in `CoreHandle::spawn`. A checkout the core learns later,
    // from Herdr's first sync, reaches a client's snapshot and these roots on
    // separate reads; a refused event catches them up (`server::admit_event`).
    if let Err(error) = app.roots.catch_up() {
        roots_failed(&error);
    }
    spawn_root_refresh(Arc::clone(&app.roots), Arc::clone(&app.clients));
    // The core and the Explorer watch start out drawn; until a window says
    // otherwise, they are not.
    server::renderers_drawn(&app, false);
    tokio::spawn(pane_auth::serve(
        pane_listener,
        Arc::clone(&pane_capabilities),
        Arc::clone(&core),
        env.herdr_socket_path.as_ref().map(std::path::PathBuf::from),
        port,
        shutdown.clone(),
        Arc::clone(&app.desktop_renderers),
    ));
    tokio::spawn(attach::serve(
        attach_listener,
        Arc::new(attach::AttachService {
            core: Arc::clone(&core),
            build: env.build.clone(),
            port,
            panes: pane_events,
            terminals: Arc::clone(&core.terminals)
                as Arc<dyn hide_node::terminal::device::DeviceSink>,
            grants: Arc::clone(&relay_grants),
            attaching: attach::Attaching::default(),
            move_gate: Arc::clone(&move_gate),
            moves: Arc::clone(&seat.moves),
        }),
        shutdown.clone(),
    ));
    let idle = server::mount(&seat, app);
    Ok(RunningDaemon {
        port,
        token,
        idle,
        shutdown,
        pane_capabilities,
        pane_bootstrap_socket,
        pane_bootstrap_record: pane_auth::bootstrap_socket_record(&env.state_dir),
        node_attach_socket,
        node_attach_record: attach::attach_record(&env.state_dir),
        mobile,
        core,
        move_gate,
        role_ended,
        own: None,
    })
}

/// The daemon in the node role: the link to the core the state folder's
/// placement record names, and this machine's screens, with no core here.
/// A node daemon on a seat of its own; see `start_daemon`.
pub async fn start_node_daemon(env: Env) -> Result<RunningNode, String> {
    if let Some(error) = env::herdr_bin_error(&env) {
        return Err(error);
    }
    let lock = acquire_lock(&env.state_dir).map_err(|error| error.to_string())?;
    let seat = seat::Seat::serve(
        server::bind(env.bind).await?,
        new_token(),
        ending::Ending::new(),
    )?;
    let state_dir = env.state_dir.clone();
    let herdr_socket = env.herdr_socket_path.clone();
    let mut running = start_node_role(env, seat.parts()).await?;
    record_daemon(&state_dir, &seat.parts(), herdr_socket)?;
    seat.end_with(running.shutdown.clone(), state_dir.clone());
    running.own = Some(OwnSeat {
        _seat: seat,
        state_dir,
        _lock: lock,
    });
    Ok(running)
}

/// The node role on `seat`, which already holds the instance lock.
pub async fn start_node_role(env: Env, seat: seat::SeatParts) -> Result<RunningNode, String> {
    seat.starting("node");
    let node = herdr_core::node::NodeId::of_this_machine()?;
    let placement = placement::read(&env.state_dir, node.as_str())?
        .ok_or("no core placement record: this state folder starts a core")?;
    let herdr_socket = env
        .herdr_socket_path
        .clone()
        .ok_or("the node role needs this machine's Herdr socket (HERDR_SOCKET_PATH)")?;
    // Its records go where the core's would: Logs beside the state folder.
    if let Err(error) = herdr_core::diagnostics::install(&env.state_dir.join("core-state.json")) {
        eprintln!(
            "{}",
            serde_json::json!({"kind": "diagnostics.open_failed", "message": error.to_string()})
        );
    }
    let build = match env.build.clone() {
        Some(build) => build,
        None => build_id::of_current_exe()?,
    };
    let token = seat.token.clone();
    let port = seat.port;
    // The browser relay listens on its own loopback port, which answers
    // nothing else: a stream the core opens to it through the link reaches
    // only a relay ticket, never this daemon's other routes.
    let relay_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("the browser relay could not listen: {error}"))?;
    let browser_relay_port = relay_listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let shutdown = ending::Ending::new();
    let daemon = node_daemon::NodeDaemon::start(
        &env.home,
        placement.clone(),
        node_role::NodeIdentity {
            node: node.as_str().to_owned(),
            label: host_name().unwrap_or_else(|| node.as_str().to_owned()),
            build: build.clone(),
            release: crate::build_order::Release::of_this_build(),
            herdr_socket: std::path::PathBuf::from(&herdr_socket),
            herdr_bin: herdr_program(&env)
                .await?
                .ok_or("the node role needs this machine's Herdr binary")?,
            worktree_facts: Some(hide_kit::layout::worktree_facts(&env.state_dir)),
        },
        node_daemon::ServerParts {
            token: Arc::new(token.clone()),
            allowed_origins: Arc::new(server::allowed_origins(port, env.vite_origin.as_deref())),
            ui_dir: if server::has_embedded_ui() {
                None
            } else {
                find_ui_dir()
            },
            version: VERSION,
            build: Some(Arc::from(build.as_str())),
            seat: seat.clone(),
            shutdown: shutdown.clone(),
            state_dir: env.state_dir.clone(),
            browser_relay_port,
        },
        // This build's programs ship beside this binary, as the core's
        // devices' do; a daemon anywhere else carries none and updates no
        // core.
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
            .map(|packages| node_role::Updates {
                packages,
                state_dir: env.state_dir.clone(),
            }),
    )?;
    herdr_core::diagnostic!(serde_json::json!({
        "component": "node_daemon",
        "kind": "started",
        "node": node.as_str(),
        "core": placement.node,
        "alias": placement.alias,
        "port": port,
    }));
    // The node lives as the core's daemon does: while a screen is open or
    // its Herdr server answers, so its agents keep reaching their core.
    let idle = server::watch_idle(
        server::Idle {
            idle_secs: env.idle_secs,
            keep_alive: env.keep_alive,
            clients: Arc::clone(&daemon.state.clients),
            last_client_gone: Arc::clone(&daemon.state.last_client_gone),
            shutdown: shutdown.clone(),
            herdr_socket: Some(std::path::PathBuf::from(&herdr_socket)),
        },
        || false,
    );
    let app = node_daemon::router(daemon.state.clone());
    seat.mount(app);
    let relay_app = node_daemon::relay_router(daemon.state.clone());
    let relay_stopping = shutdown.clone();
    tokio::spawn(async move {
        let served = axum::serve(relay_listener, relay_app)
            .with_graceful_shutdown(async move { relay_stopping.ended().await })
            .await;
        if let Err(error) = served {
            herdr_core::diagnostic!(serde_json::json!({
                "component": "node_daemon",
                "kind": "browser_relay.exit",
                "message": error.to_string(),
            }));
        }
    });
    Ok(RunningNode {
        port,
        token,
        shutdown,
        idle,
        node: daemon,
        own: None,
    })
}

pub async fn wait_shutdown(running: &RunningDaemon) {
    running.shutdown.ended().await;
}

/// Follows the core's checkout roots into the boundary, the watch service and
/// the index: each read takes what the core changed since the last one, and
/// the checkouts in it replace the root set wholesale.
///
/// Three readers share it: the seed in `start_daemon` before the server
/// serves, the notification reader (`spawn_root_refresh`), and a client event
/// the boundary refused as outside every checkout (`server::admit_event`). One
/// cursor behind one lock means each read starts where the last one
/// ended, a reader never applies an older snapshot over a newer one, and a
/// read that waited for another to finish finds the roots that one applied.
///
/// Reading with a cursor keeps the whole rest/editor/changes payload off the
/// notify path: a delta that changed nothing of it carries no `rest`, and
/// roots and expanded folders both live in it. Nothing here runs under
/// the runtime mutex.
pub struct RootFollower {
    core: Arc<CoreHandle>,
    boundary: Arc<Boundary>,
    watch: Arc<watch::WatchService>,
    index: Arc<IndexService>,
    /// The revision the last read reached.
    cursor: Mutex<u64>,
    /// Wakes the refresh loop when a client arrives after none were
    /// connected, so the reads it skipped meanwhile catch up at once.
    resumed: Notify,
}

impl RootFollower {
    pub fn new(
        core: Arc<CoreHandle>,
        boundary: Arc<Boundary>,
        watch: Arc<watch::WatchService>,
        index: Arc<IndexService>,
    ) -> Self {
        Self {
            core,
            boundary,
            watch,
            index,
            cursor: Mutex::new(0),
            resumed: Notify::new(),
        }
    }

    /// A client arrived after none were connected.
    pub fn resume(&self) {
        self.resumed.notify_one();
    }

    /// Reads what the core changed since the last read and applies the roots
    /// in it; true when the read carried them. An error means the core's
    /// owner thread is gone.
    pub fn catch_up(&self) -> Result<bool, String> {
        let mut cursor = self
            .cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reply = self.core.snapshot(*cursor)?;
        *cursor = reply.revision;
        let rest = &reply.current_rest;
        let have = reply.have_revision;
        if !roots_changed(rest, have) {
            return Ok(false);
        }
        apply_snapshot(rest, &self.core, &self.boundary, &self.watch, &self.index);
        Ok(true)
    }
}

fn roots_changed(rest: &herdr_core::RestSections, have: u64) -> bool {
    rest.navigator.workspaces.revision > have
        || rest.status.remote.revision > have
        || rest.navigator.root_path.revision > have
        || rest.navigator.focused_device_id.revision > have
        || rest.navigator.devices.revision > have
        || rest.ui_state.revision > have
}

/// The one line a failed root read leaves.
pub(crate) fn roots_failed(message: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "hided",
            "kind": "boundary.roots_failed",
            "message": message,
        })
    );
}

/// Keeps the boundary's checkout roots in step with the core: one read per
/// change notification.
///
/// The notification the snapshot stream already consumes is the only trigger,
/// so the roots change exactly when a snapshot can change and nothing here runs
/// on a clock (`docs/PERFORMANCE_TESTING.md`). The reads a single burst asks for
/// beyond the first are drained before it, and the ones that do run take a
/// snapshot that already carries every change the burst announced, so a repeat
/// read converges on the same root set instead of piling up work.
///
/// With no client connected nothing reads the roots but a refused event,
/// which catches them up itself (`server::admit_event`), so the reads rest
/// until a client arrives and one read then covers everything skipped (PRD
/// labels-in-hided B29): the core changes on every label while no window is
/// open, and a snapshot per change would be the daemon's largest cost then.
fn spawn_root_refresh(roots: Arc<RootFollower>, clients: Arc<AtomicUsize>) {
    let mut changes = roots.core.notify.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                change = changes.recv() => match change {
                    Ok(()) => {}
                    // This reader fell behind; the next read catches it up.
                    // Only a closed channel means the daemon is done.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                () = roots.resumed.notified() => {}
            }
            while changes.try_recv().is_ok() {}
            if clients.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                continue;
            }
            if let Err(error) = roots.catch_up() {
                roots_failed(&error);
                return;
            }
        }
    });
}

/// One snapshot value's roots, watch folders and registered index roots.
fn apply_snapshot(
    rest: &herdr_core::RestSections,
    core: &CoreHandle,
    boundary: &Boundary,
    watch: &watch::WatchService,
    index: &IndexService,
) {
    let roots = roots_from_rest(rest);
    boundary.set_roots(roots.clone());
    boundary.set_device_roots(device_roots_from_rest(rest, boundary.node().as_str()));
    if let Err(error) = core.set_file_roots(boundary.opened_roots()) {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "hided", "kind": "boundary.core_roots_failed", "message": error
            })
        );
    }
    let device_roots = device_roots_from_rest(rest, boundary.node().as_str());
    index.set_roots(
        &roots
            .iter()
            .map(|root| (boundary.node().to_string(), root.path.display().to_string()))
            .chain(
                device_roots
                    .iter()
                    .map(|root| (root.device_id.clone(), root.path.clone())),
            )
            .collect::<Vec<_>>(),
    );
    let root = rest.navigator.root_path.as_ref().cloned();
    let expanded = rest.ui_state.expanded_paths.clone();
    let root = root.and_then(|root| {
        boundary
            .root_identity(&root)
            .map(|identity| (root, identity))
    });
    let expanded: Vec<String> = expanded
        .into_iter()
        .filter(|path| boundary.resolve_target(path).is_ok())
        .collect();
    watch.reconcile(root.map(|(root, identity)| {
        watch::Target::of(boundary.node().as_str(), root, &expanded).pinned_by(identity)
    }));
    watch.reconcile_device(watch::device_target_from_rest(
        rest,
        boundary.node().as_str(),
    ));
}

fn roots_from_rest(rest: &herdr_core::RestSections) -> Vec<Root> {
    rest.navigator
        .workspaces
        .iter()
        .filter(|workspace| workspace.remote_target_id.is_none())
        .flat_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .filter(|checkout| checkout.exists)
                .map(|checkout| Root {
                    workspace_id: workspace.id.clone(),
                    checkout_id: checkout.id.clone(),
                    path: std::path::PathBuf::from(&checkout.path),
                })
        })
        .collect()
}

fn device_roots_from_rest(
    rest: &herdr_core::RestSections,
    node: &str,
) -> Vec<boundary::DeviceRoot> {
    rest.navigator
        .workspaces
        .iter()
        .chain(
            rest.status
                .remote
                .iter()
                .filter_map(|remote| remote.session.as_ref())
                .flat_map(|session| &session.workspaces),
        )
        .filter(|workspace| workspace.device_id != node)
        .flat_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .map(|checkout| boundary::DeviceRoot {
                    device_id: workspace.device_id.clone(),
                    path: checkout.path.clone(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::env::{HOME, REGISTRY};
    use super::*;

    #[test]
    fn env_registry_lists_home_as_required() {
        assert!(REGISTRY.iter().any(|key| key.key == HOME && key.required));
    }

    #[test]
    fn retained_typed_roots_preserve_local_device_and_confirmed_removal_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let options: herdr_core::CoreOptions = serde_json::from_value(serde_json::json!({
            "schema_version": 2, "node_id": "test-node", "app_state_path": ""
        }))
        .unwrap();
        let mut snapshot = herdr_core::Snapshot::initial(&options);
        let paths = herdr_core::workspace::PathIndex::ask(
            &hide_node::Local::of_process(),
            std::collections::BTreeSet::from([path.clone()]),
        )
        .unwrap();
        snapshot.navigator.workspaces = herdr_core::workspace::build_catalog(
            &herdr_core::node::NodeId::parse("test-node").unwrap(),
            &[],
            &[herdr_core::workspace::SessionSpace {
                id: "fixture".to_owned(),
                label: "fixture".to_owned(),
                cwds: vec![path.clone()],
                purpose: None,
            }],
            &Default::default(),
            &paths,
        );
        let mut device = snapshot.navigator.workspaces[0].clone();
        device.remote_target_id = Some("fixture-device".to_owned());
        device.device_id = "fixture-device".to_owned();
        device.checkouts[0].path = "/remote/fixture".to_owned();
        snapshot.navigator.workspaces.push(device);
        let initial = herdr_core::RestSections::capture(&snapshot, None, 1);
        assert_eq!(roots_from_rest(&initial).len(), 1);
        assert_eq!(
            roots_from_rest(&initial)[0].path,
            std::path::PathBuf::from(&path)
        );
        assert_eq!(
            device_roots_from_rest(&initial, "test-node"),
            vec![boundary::DeviceRoot {
                device_id: "fixture-device".to_owned(),
                path: "/remote/fixture".to_owned(),
            }]
        );
        snapshot.zoomed = Some("fixture:pane".to_owned());
        let selected = herdr_core::RestSections::capture(&snapshot, Some(&initial), 2);
        assert_eq!(roots_from_rest(&selected), roots_from_rest(&initial));
        assert_eq!(
            device_roots_from_rest(&selected, "test-node"),
            device_roots_from_rest(&initial, "test-node")
        );
        snapshot.navigator.workspaces[0].checkouts[0].exists = false;
        snapshot.navigator.workspaces.pop();
        let removed = herdr_core::RestSections::capture(&snapshot, Some(&selected), 3);
        assert!(roots_from_rest(&removed).is_empty());
        assert!(device_roots_from_rest(&removed, "test-node").is_empty());
        assert_eq!(
            roots_from_rest(&initial).len(),
            1,
            "a slower reader's immutable projection remains intact"
        );
    }

    #[test]
    fn root_follower_detects_remote_session_changes_for_independent_and_reset_cursors() {
        let mut snapshot = watch::tests::remote_snapshot();
        let initial = herdr_core::RestSections::capture(&snapshot, None, 1);
        assert_eq!(
            device_roots_from_rest(&initial, "test-node"),
            vec![boundary::DeviceRoot {
                device_id: "mac".into(),
                path: "/r".into(),
            }]
        );
        assert!(roots_changed(&initial, 0));
        assert!(!roots_changed(&initial, 1));
        snapshot.zoomed = Some("fixture:p1".into());
        let zoom = herdr_core::RestSections::capture(&snapshot, Some(&initial), 2);
        assert!(
            !roots_changed(&zoom, 1),
            "another reader's zoom does not renew roots"
        );
        snapshot.status.remote[0]
            .session
            .as_mut()
            .unwrap()
            .workspaces[0]
            .checkouts[0]
            .path = "/r/new".into();
        let moved = herdr_core::RestSections::capture(&snapshot, Some(&zoom), 3);
        assert!(roots_changed(&moved, 2));
        assert!(roots_changed(&moved, 1));
        assert_eq!(
            device_roots_from_rest(&moved, "test-node")[0].path,
            "/r/new"
        );
        snapshot.status.remote.clear();
        let removed = herdr_core::RestSections::capture(&snapshot, Some(&moved), 4);
        assert!(roots_changed(&removed, 3));
        assert!(roots_changed(&removed, 1));
        assert!(
            roots_changed(&removed, 0),
            "normalized reset re-applies complete authority"
        );
        assert!(device_roots_from_rest(&removed, "test-node").is_empty());
        assert_eq!(device_roots_from_rest(&initial, "test-node")[0].path, "/r");
    }
}
