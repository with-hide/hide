//! The status reports a hook could not hand to the core, kept on its machine
//! until that machine's link to the core takes them there, oldest first
//! (with-hide/hide-private#674; docs/agent-hooks.md, Reports kept while the
//! core is away).
//!
//! A hook's status report ([`crate::report`]) that nothing answered for
//! (no daemon or node published a socket, nothing took the connection, the
//! request was not written whole, no answer came in time, or the answer was
//! unreadable) goes into this file in the state folder its requests go to,
//! with the pane and installation it came from and the hook's own time. The
//! node that serves that state folder's panes takes the file's reports up
//! its link once the link stands, and the daemon of a core takes its own
//! machine's ([`take`]); each removes what the core answered for
//! ([`remove`]). A report made at or before the newest one its pane applied
//! changes nothing there, so a report sent twice, or after a later one, is
//! counted once and never undoes the later one.
//!
//! The file holds at most [`LIMIT`] reports: one more drops the oldest and
//! counts it, and the count goes with the next batch, which the core logs.
//! A report that waits is metadata only (`HookStatusReport`): no prompt,
//! path or letter is ever kept here.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::{self, private};
use serde::{Deserialize, Serialize};

use crate::report::HookStatusReport;

/// The most reports one machine keeps: a few hours of five agents' turns.
pub const LIMIT: usize = 512;

/// The most reports one batch carries up a link.
pub const BATCH: usize = 32;

/// The most bytes of reports one batch carries: what one hook request may
/// be (`HOOK_REQUEST_LIMIT`), which a link line already carries. A report
/// is a few hundred bytes, so a batch is usually [`BATCH`] reports.
const BATCH_BYTES: usize = crate::hook_request::HOOK_REQUEST_LIMIT;

/// The file is read whole; past this it is no file this module wrote.
const READ_LIMIT: u64 = 4 * 1024 * 1024;

/// How long a push or a take waits for the file's lock.
const LOCK_WAIT: Duration = Duration::from_millis(250);

/// The causes a report waits for: nothing of the pane's installation
/// answered, so the core never decided on it. A refusal the core gave
/// (`refused`, `identity`, `capacity`, `ledger`), an untrusted socket record
/// (`record`) and another installation's answer (`other_install`,
/// `unconfirmed`) are not waited out.
pub const WAITED_OUT: &[&str] = &[
    "unavailable",
    "connect",
    "unsent",
    "timeout",
    "answer",
    "deadline",
];

/// One report that waits, as the hook left it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Queued {
    /// Names this report in [`remove`]; the hook's fresh request id.
    pub id: String,
    /// The pane the hook ran in (`HIDE_PANE_ID`).
    pub pane_id: String,
    /// The installation the pane names, when it names one: only that
    /// installation's node or daemon takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    pub report: HookStatusReport,
}

/// What [`take`] hands a node or daemon to send.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Batch {
    pub reports: Vec<Queued>,
    /// Reports dropped at the limit since the core was last told.
    pub dropped: u64,
    /// The file could not be read and was set aside
    /// (`hook-outbox.damaged.json`) since the core was last told: the
    /// reports it held are lost, uncounted.
    pub damaged: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    dropped: u64,
    #[serde(default)]
    damaged: bool,
    reports: VecDeque<Queued>,
}

/// The file in `state_dir`.
pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join("hook-outbox.json")
}

/// Keeps `report`, which nothing answered for, for the pane this process
/// runs in, in the state folder its requests go to. A report that cannot
/// be kept is written to the hook log, beside the failure that sent it
/// here (engineering rule 10).
pub fn keep(report: HookStatusReport) {
    let Some(pane_id) = crate::pane::from_env() else {
        return;
    };
    let Ok(home) = hide_platform::host::home_dir() else {
        return;
    };
    let kept = crate::route::state_dir(&home)
        .map_err(io::Error::other)
        .and_then(|state_dir| {
            let id = crate::hook_request::fresh_request_id().map_err(io::Error::other)?;
            push(
                &state_dir,
                Queued {
                    id,
                    pane_id: pane_id.clone(),
                    install: crate::route::pane_install(),
                    report,
                },
            )
        });
    if let Err(error) = kept {
        let _ = crate::hook_log::append_line(
            &crate::hook_log::log_path(&home),
            &serde_json::json!({
                "component": "agent_hooks",
                "kind": "outbox.failed",
                "pane": pane_id,
                "error": error.kind().to_string(),
            })
            .to_string(),
        );
    }
}

/// Adds `queued` after every report already waiting, dropping the oldest
/// past [`LIMIT`].
pub fn push(state_dir: &Path, queued: Queued) -> io::Result<()> {
    edit(state_dir, |stored| {
        stored.reports.push_back(queued);
        while stored.reports.len() > LIMIT {
            stored.reports.pop_front();
            stored.dropped += 1;
        }
    })
}

/// The oldest reports, at most [`BATCH`], that a node or daemon of
/// installation `own` takes: those naming `own`, and those naming none.
/// Nothing is removed until [`remove`].
pub fn take(state_dir: &Path, own: Option<&str>) -> io::Result<Batch> {
    let _lock = lock(state_dir)?;
    let stored = read(state_dir)?;
    let mut bytes = 0;
    let mut reports = Vec::new();
    for queued in stored
        .reports
        .into_iter()
        .filter(|queued| queued.install.is_none() || queued.install.as_deref() == own)
    {
        bytes += serde_json::to_vec(&queued).map_err(io::Error::other)?.len();
        if reports.len() == BATCH || (bytes > BATCH_BYTES && !reports.is_empty()) {
            break;
        }
        reports.push(queued);
    }
    Ok(Batch {
        reports,
        dropped: stored.dropped,
        damaged: stored.damaged,
    })
}

/// Whether any report waits, without reading the file.
pub fn waiting(state_dir: &Path) -> bool {
    path(state_dir).is_file()
}

/// Hands the reports that wait for installation `own` to `send` a batch at
/// a time, oldest first, and removes each batch `send` says the core
/// answered for; a batch it did not take stays, with everything after it,
/// for the next try. Answers how many reports went.
///
/// One drain runs on an outbox at a time, whichever node or daemon starts
/// it: it holds the drain lock from its first [`take`] to its last
/// [`remove`], so no report goes twice and no drop is counted twice. A drain
/// that finds the lock held sends nothing and leaves the reports to the one
/// holding it; a hook's [`push`] takes only the file's own lock, so it never
/// waits for a drain.
pub fn drain(
    state_dir: &Path,
    own: Option<&str>,
    mut send: impl FnMut(&Batch) -> bool,
) -> io::Result<usize> {
    let Some(_draining) = drain_lock(state_dir)? else {
        return Ok(0);
    };
    let mut sent = 0;
    // Each round removes a report or the count, so the file empties within
    // this many rounds; a file another writer keeps filling waits for the
    // next drain.
    for _ in 0..=LIMIT / BATCH {
        let batch = take(state_dir, own)?;
        if batch.reports.is_empty() && batch.dropped == 0 && !batch.damaged {
            break;
        }
        if !send(&batch) {
            break;
        }
        remove(state_dir, &batch)?;
        sent += batch.reports.len();
    }
    Ok(sent)
}

/// Removes what `batch` handed on, which the core answered for: its
/// reports, its dropped count and its damage; the file goes once nothing is
/// left.
pub fn remove(state_dir: &Path, batch: &Batch) -> io::Result<()> {
    edit(state_dir, |stored| {
        stored
            .reports
            .retain(|queued| !batch.reports.iter().any(|sent| sent.id == queued.id));
        stored.dropped = stored.dropped.saturating_sub(batch.dropped);
        stored.damaged &= !batch.damaged;
    })
}

fn edit(state_dir: &Path, change: impl FnOnce(&mut Stored)) -> io::Result<()> {
    let _lock = lock(state_dir)?;
    let mut stored = read(state_dir)?;
    change(&mut stored);
    let file = path(state_dir);
    if stored.reports.is_empty() && stored.dropped == 0 && !stored.damaged {
        return match std::fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    let bytes = serde_json::to_vec(&stored).map_err(io::Error::other)?;
    // A report outlives no reboot: the panes it speaks of end with it.
    fs::atomic::write_file(
        &file,
        &bytes,
        fs::atomic::WriteOptions::UntilReboot(fs::Access::Private),
    )
    .map(|_| ())
}

/// The file's lock, waited for inside the time a hook keeps after its
/// report (`report::EXIT_MARGIN`): a hook and a node hold it for one small
/// read and write each.
fn lock(state_dir: &Path) -> io::Result<fs::lock::Lock> {
    private::create_dir_all(state_dir)?;
    let file = private::open_own_file(&state_dir.join("hook-outbox.lock"), true)?;
    match fs::lock::lock_file(file, fs::lock::Mode::Exclusive, LOCK_WAIT, &|| false)? {
        fs::lock::Waited::Locked(lock) => Ok(lock),
        _ => Err(io::Error::new(io::ErrorKind::TimedOut, "hook outbox busy")),
    }
}

/// The drain lock, tried once: `None` while another drain holds it.
fn drain_lock(state_dir: &Path) -> io::Result<Option<fs::lock::Lock>> {
    private::create_dir_all(state_dir)?;
    let file = private::open_own_file(&state_dir.join("hook-outbox.drain.lock"), true)?;
    match fs::lock::lock_file(file, fs::lock::Mode::Exclusive, Duration::ZERO, &|| false)? {
        fs::lock::Waited::Locked(lock) => Ok(Some(lock)),
        fs::lock::Waited::TimedOut | fs::lock::Waited::Cancelled => Ok(None),
    }
}

/// The file as it stands, under its lock. Writes replace it whole, so one
/// that does not read back was never this module's: it is set aside for
/// whoever looks into it, and the outbox starts again, saying so.
fn read(state_dir: &Path) -> io::Result<Stored> {
    let file = path(state_dir);
    let opened = match private::open_own_file(&file, false) {
        Ok(opened) => opened,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    opened.take(READ_LIMIT + 1).read_to_end(&mut bytes)?;
    match serde_json::from_slice(&bytes) {
        Ok(stored) if bytes.len() as u64 <= READ_LIMIT => Ok(stored),
        _ => {
            fs::atomic::replace_file(&file, &state_dir.join("hook-outbox.damaged.json"))?;
            Ok(Stored {
                damaged: true,
                ..Stored::default()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::HookTurn;

    fn queued(n: usize, install: Option<&str>) -> Queued {
        Queued {
            id: format!("r-{n}"),
            pane_id: "w1:p1".to_owned(),
            install: install.map(str::to_owned),
            report: HookStatusReport {
                runtime: "claude".to_owned(),
                version: crate::HOOK_VERSION,
                session: Some("s-1".to_owned()),
                turn: Some(HookTurn::Working),
                counts: None,
                session_start: false,
                waiting: None,
                at_unix_us: Some(n as u64),
            },
        }
    }

    fn ids(batch: &Batch) -> Vec<&str> {
        batch
            .reports
            .iter()
            .map(|queued| queued.id.as_str())
            .collect()
    }

    #[test]
    fn reports_wait_oldest_first_and_past_the_limit_the_oldest_go_and_are_counted() {
        let state = tempfile::tempdir().unwrap();
        for n in 0..LIMIT + 3 {
            push(state.path(), queued(n, None)).unwrap();
        }
        let batch = take(state.path(), None).unwrap();
        assert_eq!(batch.dropped, 3);
        assert_eq!(batch.reports.len(), BATCH);
        assert_eq!(ids(&batch)[..2], ["r-3", "r-4"]);
        // A take removes nothing.
        assert_eq!(take(state.path(), None).unwrap(), batch);
    }

    #[test]
    fn a_node_takes_its_own_installation_s_reports_and_those_naming_none() {
        let state = tempfile::tempdir().unwrap();
        push(state.path(), queued(1, Some("a"))).unwrap();
        push(state.path(), queued(2, Some("b"))).unwrap();
        push(state.path(), queued(3, None)).unwrap();
        assert_eq!(ids(&take(state.path(), Some("a")).unwrap()), ["r-1", "r-3"]);
        assert_eq!(ids(&take(state.path(), Some("b")).unwrap()), ["r-2", "r-3"]);
    }

    #[test]
    fn a_file_that_does_not_read_back_is_set_aside_and_the_core_is_told_once() {
        let state = tempfile::tempdir().unwrap();
        std::fs::write(path(state.path()), b"{not json").unwrap();
        push(state.path(), queued(1, None)).unwrap();
        assert!(state.path().join("hook-outbox.damaged.json").is_file());
        let mut told = Vec::new();
        drain(state.path(), None, |batch| {
            told.push((ids(batch).len(), batch.damaged));
            true
        })
        .unwrap();
        assert_eq!(told, [(1, true)]);
        assert!(!waiting(state.path()));
    }

    #[test]
    fn a_drain_sends_every_batch_in_order_and_keeps_what_the_core_did_not_take() {
        let state = tempfile::tempdir().unwrap();
        for n in 0..LIMIT + 5 {
            push(state.path(), queued(n, None)).unwrap();
        }
        // The core's link drops at the second batch: the first is gone, the
        // rest wait, the count went with the first.
        let mut batches = 0;
        let sent = drain(state.path(), None, |_| {
            batches += 1;
            batches == 1
        })
        .unwrap();
        assert_eq!(sent, BATCH);
        let left = take(state.path(), None).unwrap();
        assert_eq!((ids(&left)[0], left.dropped), ("r-37", 0));

        let mut seen = Vec::new();
        let sent = drain(state.path(), None, |batch| {
            seen.extend(batch.reports.iter().map(|queued| queued.id.clone()));
            true
        })
        .unwrap();
        assert_eq!(sent, LIMIT - BATCH);
        let expected: Vec<String> = (5 + BATCH..LIMIT + 5).map(|n| format!("r-{n}")).collect();
        assert_eq!(seen, expected);
        assert!(!waiting(state.path()), "an empty outbox leaves no file");
    }

    /// A second drain while one runs (a sweep that came while the core was
    /// slow to answer, or the node and the daemon of one state folder) sends
    /// nothing: every report goes once and the drops are counted once.
    #[test]
    fn one_drain_at_a_time_sends_each_report_once_and_counts_the_drops_once() {
        let state = tempfile::tempdir().unwrap();
        for n in 0..LIMIT + 3 {
            push(state.path(), queued(n, None)).unwrap();
        }
        let (mut sent, mut dropped, mut second) = (Vec::new(), 0, Vec::new());
        drain(state.path(), None, |batch| {
            second.push(
                drain(state.path(), None, |_| {
                    panic!("a second drain sent a batch")
                })
                .unwrap(),
            );
            sent.extend(batch.reports.iter().map(|queued| queued.id.clone()));
            dropped += batch.dropped;
            true
        })
        .unwrap();
        assert_eq!(second, vec![0; LIMIT / BATCH]);
        assert_eq!((sent.len(), dropped), (LIMIT, 3));
        assert!(!waiting(state.path()));
        // Once it ends, the next drain runs.
        push(state.path(), queued(LIMIT + 3, None)).unwrap();
        assert_eq!(drain(state.path(), None, |_| true).unwrap(), 1);
    }
}
