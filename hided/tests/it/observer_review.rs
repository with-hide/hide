//! One real-daemon journey for the approved thirty-minute Observer review.
//! The clock is backdated only while the private daemon is stopped; neither
//! the operator's account nor a real provider is used. Pure clock boundaries
//! belong to the watch unit tests, while this covers persistence, real pane
//! authority, the existing doorbell and explicit review completion together.
#![cfg(unix)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use herdr_core::delivery::ledger::{Ledger, State};
use herdr_core::delivery::watch::{PeriodicReview, ReviewAssessment, ReviewPolicy};
use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

use crate::private_herdr::pinned_version;
use crate::support::remote_delivery::renderer::Renderer;
use crate::support::remote_delivery::{
    Environment, Herdr, install_provider, quote, read, successful, wait_for, wait_within,
};
use crate::support::run::{Run, finish};

struct Fixture {
    root: PathBuf,
    _ipc: tempfile::TempDir,
    state: PathBuf,
    cli: PathBuf,
    herdr_bin: PathBuf,
    herdr: Herdr,
    child: String,
    daemon: Option<OwnedChild>,
    removed: bool,
}

impl Fixture {
    fn journey(body: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let mut fixture = Self::start()?;
        let result = body(&mut fixture);
        finish(&mut fixture, result)
    }

    fn start() -> Result<Self> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let cli = std::env::var_os("HIDE_E2E_CLI_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("target/debug"));
        for name in ["hide", "hided"] {
            ensure!(
                cli.join(name).is_file(),
                "build this worktree's CLI binaries first"
            );
        }
        let bin = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN")
                .context("set HIDE_E2E_HERDR_BIN to the pinned Herdr")?,
        );
        let artifacts = repository.join("agents/runs/observer-review");
        fs::create_dir_all(&artifacts)?;
        let root = tempfile::Builder::new()
            .prefix("runtime-")
            .tempdir_in(artifacts)?
            .keep();
        // Only IPC endpoints need a short path. Every account, state file,
        // executable and log stays in the ignored worktree run directory.
        let ipc = tempfile::Builder::new().prefix("hor-").tempdir_in("/tmp")?;
        let state = root.join("state");
        hide_platform::fs::private::create_dir_all(&state)?;
        let mut environment = Environment::new(
            &root.join("local"),
            &ipc.path().join("h.sock"),
            &bin,
            Some(&state),
        )?;
        environment.set("HIDE_DELIVERY_FIXTURE_ECHO", "1");
        let mut version = environment.command(&bin);
        version.arg("--version");
        ensure!(
            String::from_utf8(successful(version)?)?
                .split_whitespace()
                .last()
                == Some(pinned_version().as_str()),
            "fixture Herdr is not the repository pin"
        );
        install_provider(&environment, &root)?;
        // Stop discovery at the fixture's project instead of the real
        // checkout surrounding agents/runs.
        let mut git = environment.command("/usr/bin/git");
        git.arg("-C")
            .arg(environment.home.join("project"))
            .args(["init", "-q"]);
        successful(git)?;
        let herdr = Herdr::start(root.join("local"), environment, bin.clone(), Some(&state))?;
        let parent = herdr.run(&["pane", "get", &herdr.pane])?;
        let tab = herdr.run(&[
            "tab",
            "create",
            "--workspace",
            parent
                .pointer("/result/pane/workspace_id")
                .and_then(Value::as_str)
                .context("private workspace id")?,
            "--cwd",
            herdr
                .environment
                .home
                .join("project")
                .to_str()
                .context("private project UTF-8")?,
            "--no-focus",
        ])?;
        let child = tab
            .pointer("/result/root_pane/pane_id")
            .and_then(Value::as_str)
            .context("private child's pane")?
            .to_owned();
        herdr.run(&[
            "agent",
            "start",
            "local-child",
            "--kind",
            "claude",
            "--pane",
            &child,
        ])?;
        wait_for("private child provider command loop", || {
            let mut command = herdr.environment.command(&bin);
            command.args([
                "pane", "read", &child, "--source", "visible", "--format", "text",
            ]);
            let output = String::from_utf8(successful(command)?)?;
            Ok(output.contains("DELIVERY_FIXTURE_READY").then_some(()))
        })?;
        herdr.report_session(&child, "fixture-review-child")?;
        let mut fixture = Self {
            root,
            _ipc: ipc,
            state,
            cli,
            herdr_bin: bin,
            herdr,
            child,
            daemon: None,
            removed: false,
        };
        fixture.start_daemon()?;
        Ok(fixture)
    }

    fn start_daemon(&mut self) -> Result<()> {
        ensure!(self.daemon.is_none(), "private daemon already started");
        let mut environment = self.herdr.environment.clone();
        environment.set("HIDE_KEEP_ALIVE", "1");
        environment.set("HIDE_TAILSCALE_BIN", self.root.join("no-tailscale"));
        environment.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.root.join("hided.log"))?;
        let mut command = environment.command(self.cli.join("hided"));
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let daemon = OwnedChild::spawn(&mut command)?;
        let pid = daemon.id();
        self.daemon = Some(daemon);
        let state: Value = wait_for("this private daemon's state", || {
            let path = self.state.join("hided.json");
            if !path.exists() {
                return Ok(None);
            }
            let state: Value = serde_json::from_slice(&read(&path)?)?;
            Ok((state["pid"] == pid).then_some(state))
        })?;
        let port = state["port"].as_u64().context("private daemon port")? as u16;
        let token = state["token"].as_str().context("private daemon token")?;
        Renderer::connect(port, token)?
            .event("create_workspace", json!({"path":self.herdr.environment.home.join("project"), "label":"Review fixture", "initialize_git":false}))?;
        // Registration queues an asynchronous native projection. Wait for
        // that observation before issuing a pane-authorized command after
        // a restart, rather than retrying a refused mutation.
        wait_for("both private native agents projected after restart", || {
            let renderer = Renderer::connect(port, token)?;
            let agents = renderer.snapshot()["navigator"]["agents"]
                .as_array()
                .context("private navigator agents")?;
            Ok([self.herdr.pane.as_str(), self.child.as_str()]
                .iter()
                .all(|pane| agents.iter().any(|agent| agent["pane_id"] == *pane))
                .then_some(()))
        })?;
        self.ledger()?;
        Ok(())
    }

    fn stop_daemon(&mut self) -> Result<()> {
        if self.daemon.is_none() {
            return Ok(());
        }
        let mut command = self.herdr.environment.command(self.cli.join("hide"));
        command.arg("stop");
        successful(command)?;
        let daemon = self.daemon.as_mut().context("owned private daemon")?;
        wait_for("private daemon confirmed exit", || {
            Ok(daemon.try_wait()?.map(|_| ()))
        })?;
        self.daemon = None;
        Ok(())
    }

    fn ledger(&self) -> Result<Ledger> {
        let ledger: Ledger =
            serde_json::from_slice(&read(&self.state.join("delivery-ledger.json"))?)?;
        ledger.validate().map_err(anyhow::Error::msg)?;
        Ok(ledger)
    }

    fn save_ledger(&self, ledger: &Ledger) -> Result<()> {
        ensure!(
            self.daemon.is_none(),
            "never edit a running daemon's ledger"
        );
        ledger.validate().map_err(anyhow::Error::msg)?;
        hide_platform::fs::atomic::write_file(
            &self.state.join("delivery-ledger.json"),
            &serde_json::to_vec(ledger)?,
            hide_platform::fs::Access::Private,
        )?;
        Ok(())
    }

    fn parent(&mut self, args: &str) -> Result<Value> {
        self.herdr
            .run_in_pane(&format!("{} {args}", quote(self.cli.join("hide"))))?
            .json()
    }
}

impl Run for Fixture {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stop(&mut self) -> Result<()> {
        self.stop_daemon()?;
        self.herdr.stop()
    }

    fn removed(&mut self) {
        self.removed = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.removed
            && let Err(error) = self.stop()
        {
            eprintln!(
                "private review fixture cleanup failed; run kept at {}: {error:#}",
                self.root.display()
            );
        }
    }
}

#[test]
#[ignore = "needs this worktree's CLI binaries and pinned HIDE_E2E_HERDR_BIN"]
fn a_due_review_wakes_its_observer_and_only_explicit_review_advances_it() -> Result<()> {
    Fixture::journey(|fixture| {
        let watch = fixture.parent("watch start local-child")?;
        let watch_id = watch["id"].as_str().context("started watch id")?.to_owned();
        fixture.stop_daemon()?;
        let mut ledger = fixture.ledger()?;
        let due = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64 - 1;
        ledger
            .watches
            .iter_mut()
            .find(|watch| watch.id == watch_id)
            .context("persisted watch")?
            .review = Some(PeriodicReview {
            policy: ReviewPolicy::Observer30m,
            due_at_unix_ms: due,
            pending_letter: None,
            completed_at_unix_ms: None,
            assessment: None,
            cycle: 0,
        });
        fixture.save_ledger(&ledger)?;
        fixture.start_daemon()?;
        let letter = wait_within(
            "daemon-generated review and successful private doorbell",
            Duration::from_secs(90),
            || {
                let ledger = fixture.ledger()?;
                let pending = ledger
                    .watches
                    .iter()
                    .find(|watch| watch.id == watch_id)
                    .and_then(|watch| watch.review.as_ref())
                    .and_then(|review| review.pending_letter.as_deref());
                Ok(ledger
                    .letters
                    .iter()
                    .find(|letter| {
                        Some(letter.id.as_str()) == pending
                            && letter.bell_sent
                            && letter.bell_line.is_some()
                    })
                    .cloned())
            },
        )?;
        ensure!(
            letter.kind == "watch" && letter.immediate,
            "review must use the existing immediate watch-letter path"
        );
        ensure!(
            letter.sender == letter.recipient,
            "the designated Observer owns the review letter"
        );
        ensure!(
            letter.body.contains("local-child") && letter.body.contains(&watch_id),
            "review does not identify its target and watch"
        );
        let bell_line = letter
            .bell_line
            .as_deref()
            .context("persisted review bell")?;
        wait_for(
            "the actual parent provider received the exact review wake",
            || {
                let mut command = fixture.herdr.environment.command(&fixture.herdr_bin);
                command.args([
                    "pane",
                    "read",
                    &fixture.herdr.pane,
                    "--source",
                    "visible",
                    "--format",
                    "text",
                ]);
                let text = String::from_utf8(successful(command)?)?;
                Ok(text.contains(bell_line).then_some(()))
            },
        )?;
        eprintln!(
            "review wake observed: watch={watch_id} request={} line={bell_line:?}",
            letter.id
        );
        let id = letter.id;
        fixture.parent(&format!("request ack {id}"))?;
        let acknowledged = fixture.ledger()?;
        let review = acknowledged
            .watches
            .iter()
            .find(|watch| watch.id == watch_id)
            .and_then(|watch| watch.review.as_ref())
            .context("acknowledged review")?;
        ensure!(
            review.pending_letter.as_deref() == Some(id.as_str())
                && review.completed_at_unix_ms.is_none()
                && review.due_at_unix_ms == due,
            "receipt incorrectly completed or postponed the review"
        );
        ensure!(
            acknowledged.letters.iter().any(|letter| letter.id == id
                && letter.state == State::Acknowledged
                && letter.hook_confirmed == Some(true)),
            "the real Observer receipt was not persisted"
        );
        fixture
            .herdr
            .run_in(
                &fixture.child.clone(),
                &format!("{} inbox", quote(fixture.cli.join("hide"))),
            )?
            .json()?;
        let wrong = fixture.herdr.run_in(&fixture.child.clone(), &format!("{} request reply {id} --review scope-appropriate --intent child-review --body 'child cannot review itself'", quote(fixture.cli.join("hide"))))?;
        let refused: Value = serde_json::from_str(&wrong.stdout)?;
        ensure!(
            wrong.status != 0
                && refused["ok"] == false
                && refused["reason"] == "letter_unavailable",
            "target child did not receive the letter authority refusal: {refused}"
        );
        eprintln!("review remains pending after actual receipt; child reply refused: {id}");
        fixture.stop_daemon()?;
        fixture.start_daemon()?;
        fixture.parent("watch list")?;
        let restored = fixture.ledger()?;
        let review = restored
            .watches
            .iter()
            .find(|watch| watch.id == watch_id)
            .and_then(|watch| watch.review.as_ref())
            .context("restored pending review")?;
        ensure!(
            review.pending_letter.as_deref() == Some(id.as_str())
                && review.due_at_unix_ms == due
                && review.completed_at_unix_ms.is_none(),
            "restart or wrong actor advanced the review"
        );
        ensure!(
            restored
                .letters
                .iter()
                .filter(|letter| letter.kind == "watch")
                .count()
                == 1,
            "restart duplicated the outstanding review"
        );
        let reply = format!(
            "request reply {id} --review diagnosis-needed --intent observer-conclusion --body 'Repeated failures need a minimal reproduction; diagnose the first failure next'"
        );
        let first = fixture.parent(&reply)?;
        let completed = fixture.ledger()?;
        let review = completed
            .watches
            .iter()
            .find(|watch| watch.id == watch_id)
            .and_then(|watch| watch.review.as_ref())
            .context("completed review")?
            .clone();
        ensure!(
            review.pending_letter.is_none()
                && review.assessment == Some(ReviewAssessment::DiagnosisNeeded),
            "explicit conclusion was not recorded"
        );
        ensure!(
            review.due_at_unix_ms
                == review.completed_at_unix_ms.context("completion time")? + 30 * 60_000,
            "next review must be thirty minutes from explicit completion"
        );
        ensure!(
            fixture.parent(&reply)?["id"] == first["id"],
            "retry duplicated the review conclusion"
        );
        eprintln!(
            "review explicitly completed: request={id} result={} next_due={} completion={:?}",
            first["id"], review.due_at_unix_ms, review.completed_at_unix_ms
        );
        fixture.stop_daemon()?;
        fixture.start_daemon()?;
        fixture.parent("watch list")?;
        ensure!(
            fixture
                .ledger()?
                .watches
                .iter()
                .find(|watch| watch.id == watch_id)
                .and_then(|watch| watch.review.as_ref())
                == Some(&review),
            "restart lost completed review state"
        );
        // Exercise stop with a fresh outstanding request. The initial tick
        // makes it due; the ordinary post-restart quiet period keeps its
        // wake unreserved while the Observer stops the watch.
        fixture.stop_daemon()?;
        let mut ledger = fixture.ledger()?;
        ledger
            .watches
            .iter_mut()
            .find(|watch| watch.id == watch_id)
            .and_then(|watch| watch.review.as_mut())
            .context("next review")?
            .due_at_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64 - 1;
        fixture.save_ledger(&ledger)?;
        fixture.start_daemon()?;
        let next = wait_for("second daemon-generated pending review", || {
            let ledger = fixture.ledger()?;
            let pending = ledger
                .watches
                .iter()
                .find(|watch| watch.id == watch_id)
                .and_then(|watch| watch.review.as_ref())
                .and_then(|review| review.pending_letter.as_deref());
            Ok(ledger
                .letters
                .iter()
                .find(|letter| Some(letter.id.as_str()) == pending)
                .cloned())
        })?;
        ensure!(
            next.id != id && !next.bell_sent,
            "next review was not held before stop"
        );
        fixture.parent(&format!("watch stop {watch_id}"))?;
        let stopped = fixture.ledger()?;
        ensure!(
            stopped.watches.is_empty(),
            "explicit stop did not end the review watch"
        );
        ensure!(
            stopped.letters.iter().any(|letter| letter.id == next.id
                && letter.state == State::Cancelled
                && !letter.bell_sent),
            "stop left the outstanding review wake live"
        );
        fixture.stop_daemon()?;
        fixture.start_daemon()?;
        fixture.parent("watch list")?;
        let final_ledger = fixture.ledger()?;
        ensure!(
            final_ledger.watches.is_empty() && final_ledger.letters.len() == stopped.letters.len(),
            "restart recreated a stopped review"
        );
        eprintln!(
            "pending next review cancelled before wake and still stopped after restart: {}",
            next.id
        );
        Ok(())
    })
}
