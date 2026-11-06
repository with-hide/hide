//! Durable existence before the core publishes a pane. The request digest
//! is stable before shell setup, and only the worker claiming this exact
//! incarnation expands the shell or starts a process.

use std::time::Instant;

use hide_holder_protocol::PROTOCOL_VERSION;
use hide_holder_protocol::record::{
    self, End, EndReason, PaneRecord, Phase, RECORD_FORMAT, Reservation,
};

use super::{HOLDER_CAP, Panes, StartRefusal, StartRequest, StartStages, Started, spec_of};
use crate::{holders, layout};

impl Panes {
    /// Persist a new intent or answer that exact intent's reservation. A
    /// replacement explicitly names the earlier incarnation it consumes.
    pub fn reserve(
        &self,
        request_guard: &super::Request<'_>,
        pane: &str,
        request: &str,
        cwd: &str,
        previous: Option<u64>,
    ) -> Result<PaneRecord, StartRefusal> {
        let owner = request_guard.owner();
        self.check_owner(owner).map_err(StartRefusal::Failed)?;
        validate(pane, request, cwd)?;
        let _lock = self.lock().map_err(StartRefusal::Failed)?;
        let path = layout::pane_record(&self.state_dir, pane);
        let existing = record::read(&path).map_err(StartRefusal::RecordUnreadable)?;
        let replaces_live = existing.as_ref().is_some_and(|record| {
            record.phase != Phase::Ended
                || record.reservation.as_ref().is_some_and(|intent| {
                    intent.owner == owner.incarnation() && owner.request_pending(intent.number)
                })
        });
        if let Some(existing) = existing {
            if existing.owner != self.owner {
                return Err(StartRefusal::OtherOwner(Box::new(existing)));
            }
            if existing.phase != Phase::Ended
                && existing.reservation.as_ref().is_some_and(|intent| {
                    intent.owner == owner.incarnation()
                        && intent.request == request
                        && intent.number == request_guard.number()
                })
            {
                return Ok(existing);
            }
            if previous != Some(existing.incarnation) {
                return Err(StartRefusal::Conflict(Box::new(existing)));
            }
            if existing.reservation.as_ref().is_some_and(|intent| {
                intent.owner == owner.incarnation() && owner.request_pending(intent.number)
            }) {
                return Err(StartRefusal::Uncertain(
                    "the earlier reservation still has an unresolved request".to_owned(),
                ));
            }
            // Replacing an unclaimed attempt fences a late holder's Claim.
            // A named holder must be established dead, not merely unreadable.
            if let Some(holder) = existing.holder
                && hide_platform::process::is_alive(holder.pid)
            {
                match hide_platform::process::start_time(holder.pid) {
                    Ok(started) if started != holder.started => {}
                    Ok(_) => return Err(StartRefusal::EarlierRuns(vec![holder.pid])),
                    Err(error) => return Err(StartRefusal::Uncertain(error.to_string())),
                }
            }
            let left = self.still_running(&existing);
            if !left.is_empty() {
                return Err(StartRefusal::EarlierRuns(left));
            }
        } else if let Some(previous) = previous {
            // A record goes only once nothing it names runs, so a replaced
            // incarnation whose record is gone (the ended-record cap dropped
            // it, or a person deleted it) holds nothing: the pane starts
            // afresh under a new incarnation. Refusing it left a rebooted
            // pane closed for good once its record was dropped.
            self.emit(
                "pane_holder.replaced_record_missing",
                serde_json::json!({ "pane": pane, "incarnation": previous }),
            );
        }
        let count = self.count_live_for(owner, &self.scan().map_err(StartRefusal::Failed)?);
        // A replacement already has its place in the record bound.
        if !replaces_live && count >= HOLDER_CAP {
            drop(_lock);
            self.emit(
                "pane_holder.start_refused",
                serde_json::json!({ "pane": pane, "reason": "holder_cap", "count": count, "cap": HOLDER_CAP }),
            );
            return Err(StartRefusal::Cap { count });
        }
        let reserved = self.new_reservation(request_guard, pane, request, cwd)?;
        record::write(&path, &reserved)
            .map_err(|error| StartRefusal::Uncertain(error.to_string()))?;
        Ok(reserved)
    }

    // Caller holds the records lock; allocation and installation are one
    // decision even when cancel arrives before the original reserve.
    fn new_reservation(
        &self,
        request_guard: &super::Request<'_>,
        pane: &str,
        request: &str,
        cwd: &str,
    ) -> Result<PaneRecord, StartRefusal> {
        let owner = request_guard.owner();
        Ok(PaneRecord {
            format: RECORD_FORMAT,
            pane: pane.to_owned(),
            owner: self.owner.clone(),
            incarnation: self.next_incarnation().map_err(StartRefusal::Failed)?,
            phase: Phase::Reserved,
            protocol: PROTOCOL_VERSION,
            version_folder: String::new(),
            spec: String::new(),
            reservation: Some(Reservation {
                owner: owner.incarnation(),
                request: request.to_owned(),
                cwd: cwd.to_owned(),
                cancelled: false,
                number: request_guard.number(),
            }),
            reserved_at_unix_ms: record::now_unix_ms(),
            boot: self.boot.clone(),
            holder: None,
            program: None,
            end: None,
        })
    }

    /// Cancel the exact original intent, including an unpublished reserve
    /// that has not reached this node yet. Its terminal record stays pinned
    /// until this connection's workers can no longer submit a late call.
    pub fn cancel_reservation(
        &self,
        request_guard: &super::Request<'_>,
        pane: &str,
        digest: &str,
        cwd: &str,
    ) -> Result<PaneRecord, StartRefusal> {
        let owner = request_guard.owner();
        self.check_owner(owner).map_err(StartRefusal::Failed)?;
        validate(pane, digest, cwd)?;
        let path = layout::pane_record(&self.state_dir, pane);
        let current = {
            let _lock = self.lock().map_err(StartRefusal::Failed)?;
            let mut current = match self.record(pane).map_err(StartRefusal::RecordUnreadable)? {
                Some(current)
                    if current.owner == self.owner
                        && current.reservation.as_ref().is_some_and(|intent| {
                            intent.owner == owner.incarnation()
                                && intent.request == digest
                                && intent.number == request_guard.number()
                        }) =>
                {
                    current
                }
                Some(current) => return Err(StartRefusal::Conflict(Box::new(current))),
                None => {
                    let count =
                        self.count_live_for(owner, &self.scan().map_err(StartRefusal::Failed)?);
                    if count >= HOLDER_CAP {
                        return Err(StartRefusal::Cap { count });
                    }
                    self.new_reservation(request_guard, pane, digest, cwd)?
                }
            };
            self.prepare_close(&mut current);
            record::write(&path, &current)
                .map_err(|error| StartRefusal::Uncertain(error.to_string()))?;
            current
        };
        self.finish_close(owner, current)
            .map_err(|error| StartRefusal::Uncertain(error.to_string()))
    }

    /// Claim before expanding the command, and retain the connection's
    /// ownership through preparation, process start and failure cleanup.
    pub fn start_reserved(
        &self,
        request_guard: &super::Request<'_>,
        pane: &str,
        incarnation: u64,
        digest: &str,
        source: &holders::Source,
        request: impl FnOnce() -> Result<StartRequest, StartRefusal>,
    ) -> Result<Started, StartRefusal> {
        let owner = request_guard.owner();
        self.check_owner(owner).map_err(StartRefusal::Failed)?;
        let begun = Instant::now();
        let path = layout::pane_record(&self.state_dir, pane);
        let mut current = {
            let _lock = self.lock().map_err(StartRefusal::Failed)?;
            let mut current = self.exact_reservation(request_guard, pane, incarnation, digest)?;
            match current.phase {
                Phase::Running => return Ok(Started::Existing(current)),
                Phase::Preparing => {
                    return Err(StartRefusal::Uncertain(
                        "this holder is still starting".to_owned(),
                    ));
                }
                Phase::Reserved => {}
                _ => return Err(StartRefusal::Conflict(Box::new(current))),
            }
            current.phase = Phase::Preparing;
            record::write(&path, &current)
                .map_err(|error| StartRefusal::Uncertain(error.to_string()))?;
            current
        };
        let prepared = (|| {
            let request = request()?;
            if request.pane != pane {
                return Err(StartRefusal::Failed(
                    "the expanded request names another pane".to_owned(),
                ));
            }
            let version = holders::prepare(&self.home, PROTOCOL_VERSION, source, &self.names)
                .map_err(StartRefusal::Failed)?;
            let _lock = self.lock().map_err(StartRefusal::Failed)?;
            let found = self.exact_reservation(request_guard, pane, incarnation, digest)?;
            if found != current {
                return Err(StartRefusal::Conflict(Box::new(found)));
            }
            current.spec = spec_of(&request);
            current.version_folder = version
                .dir
                .file_name()
                .ok_or_else(|| StartRefusal::Failed("the holder version has no name".to_owned()))?
                .to_string_lossy()
                .into_owned();
            record::write(&path, &current)
                .map_err(|error| StartRefusal::Uncertain(error.to_string()))?;
            Ok((request, version))
        })();
        let (request, version) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.fail_reservation(&current, &error.to_string())?;
                return Err(error);
            }
        };
        let stages = StartStages {
            begun,
            prepared: begun.elapsed(),
            reserved: begun.elapsed(),
        };
        self.start_holder(&request, &version, &current, &stages)
            .map(Started::Started)
            .map_err(StartRefusal::Uncertain)
    }

    fn exact_reservation(
        &self,
        request_guard: &super::Request<'_>,
        pane: &str,
        incarnation: u64,
        digest: &str,
    ) -> Result<PaneRecord, StartRefusal> {
        let owner = request_guard.owner();
        let current = self
            .record(pane)
            .map_err(StartRefusal::RecordUnreadable)?
            .ok_or_else(|| StartRefusal::Failed("the reservation is missing".to_owned()))?;
        if current.owner != self.owner
            || current.incarnation != incarnation
            || !current.reservation.as_ref().is_some_and(|intent| {
                intent.owner == owner.incarnation()
                    && intent.request == digest
                    && intent.number == request_guard.number()
            })
        {
            return Err(StartRefusal::Conflict(Box::new(current)));
        }
        Ok(current)
    }

    fn fail_reservation(&self, current: &PaneRecord, reason: &str) -> Result<(), StartRefusal> {
        let mut ended = current.clone();
        ended.phase = Phase::Ended;
        ended.end = Some(End::new(EndReason::StartFailed, reason));
        if let Err(why) = self.write_if_unchanged(current, &ended) {
            self.emit(
                "pane_holder.reservation_failure_unwritten",
                serde_json::json!({
                    "pane": current.pane, "incarnation": current.incarnation, "error": why,
                }),
            );
            return Err(StartRefusal::Uncertain(why));
        }
        Ok(())
    }
}

fn validate(pane: &str, digest: &str, cwd: &str) -> Result<(), StartRefusal> {
    record::check_pane(pane).map_err(StartRefusal::InvalidPane)?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StartRefusal::Failed(
            "the request digest is invalid".to_owned(),
        ));
    }
    hide_platform::path::from_wire(cwd).map_err(|error| StartRefusal::Failed(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn cancelling_before_reserve_fences_the_late_request_without_starting_a_program() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let request = owner.issue("p1", DIGEST).unwrap();
        let cwd = hide_platform::path::to_wire(home.path()).unwrap();
        let canceled = panes
            .cancel_reservation(&request, "p1", DIGEST, &cwd)
            .unwrap();
        assert!(matches!(
            panes.reserve(&request, "p1", DIGEST, &cwd, None),
            Err(StartRefusal::Conflict(_))
        ));
        let source = holders::Source {
            program: home.path().join("must-not-run"),
            conpty: None,
        };
        assert!(matches!(
            panes.start_reserved(
                &request,
                "p1",
                canceled.incarnation,
                DIGEST,
                &source,
                || panic!("a cancelled request must not expand the shell")
            ),
            Err(StartRefusal::Conflict(_))
        ));
        assert_eq!(panes.record("p1").unwrap().unwrap(), canceled);
        let number = request.number();
        assert!(!owner.retire(owner.incarnation(), number).unwrap());
        drop(request);
        panes.forget(&owner, "p1").unwrap();
        assert!(
            owner
                .admit(owner.incarnation(), number, "p1", DIGEST)
                .is_err()
        );
    }

    #[test]
    fn a_new_owner_recovers_the_id_but_fences_the_previous_start_incarnation() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let cwd = hide_platform::path::to_wire(home.path()).unwrap();
        let request = owner.issue("p1", DIGEST).unwrap();
        let first = panes.reserve(&request, "p1", DIGEST, &cwd, None).unwrap();
        assert_eq!(
            panes.reserve(&request, "p1", DIGEST, &cwd, None).unwrap(),
            first
        );
        let previous_owner = owner.incarnation();
        drop(request);
        drop(owner);
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let recovered = panes.reconcile(&owner, Duration::from_secs(2));
        assert!(matches!(
            recovered.panes.get("p1"),
            Some(super::super::Judgment::Recoverable(_))
        ));
        assert!(owner.admit(previous_owner, 1, "p1", DIGEST).is_err());
        let request = owner.issue("p1", DIGEST).unwrap();
        let next = panes
            .reserve(&request, "p1", DIGEST, &cwd, Some(first.incarnation))
            .unwrap();
        assert_ne!(first.incarnation, next.incarnation);
        let source = holders::Source {
            program: home.path().join("must-not-run"),
            conpty: None,
        };
        assert!(matches!(
            panes.start_reserved(
                &request,
                "p1",
                first.incarnation,
                DIGEST,
                &source,
                || panic!("an old incarnation must not expand the shell")
            ),
            Err(StartRefusal::Conflict(_))
        ));
        assert_eq!(panes.record("p1").unwrap().unwrap(), next);
    }

    /// 2026-11-06: the ended-record cap dropped the records of 44 rebooted
    /// panes before their restarts replaced them, and each restart was
    /// refused for good. A replaced record that is gone names nothing that
    /// runs, so the pane starts again under a new incarnation.
    #[test]
    fn a_replacement_whose_record_was_dropped_starts_the_pane_afresh() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let cwd = hide_platform::path::to_wire(home.path()).unwrap();
        let request = owner.issue("p1", DIGEST).unwrap();
        let first = panes.reserve(&request, "p1", DIGEST, &cwd, None).unwrap();
        drop(request);
        std::fs::remove_file(layout::pane_record(&panes.state_dir, "p1")).unwrap();

        let request = owner.issue("p1", DIGEST).unwrap();
        let next = panes
            .reserve(&request, "p1", DIGEST, &cwd, Some(first.incarnation))
            .unwrap();
        assert!(next.incarnation > first.incarnation);
        assert_eq!(next.phase, Phase::Reserved);
        assert_eq!(panes.record("p1").unwrap().unwrap(), next);
    }

    /// A connection's numbers come from one place: a number handed out for
    /// a later admission and a library caller's whole request never share
    /// one, and an unsent number is retired like any other.
    #[test]
    fn a_handed_out_number_and_an_issued_request_never_share_a_number() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let handed = owner.number().unwrap();
        let issued = owner.issue("p1", DIGEST).unwrap();
        assert_ne!(handed, issued.number());
        let admitted = owner
            .admit(owner.incarnation(), handed, "p2", DIGEST)
            .unwrap();
        assert!(
            owner
                .admit(owner.incarnation(), handed, "p1", DIGEST)
                .is_err()
        );
        drop(admitted);
        drop(issued);
        let unsent = owner.number().unwrap();
        assert!(owner.retire(owner.incarnation(), unsent).unwrap());
        assert!(
            owner
                .admit(owner.incarnation(), unsent, "p3", DIGEST)
                .is_err()
        );
    }

    #[test]
    fn unresolved_requests_keep_the_cap_but_a_confirmed_completion_releases_it() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let cwd = hide_platform::path::to_wire(home.path()).unwrap();
        for n in 0..HOLDER_CAP {
            let request = owner.issue(&format!("p{n}"), DIGEST).unwrap();
            panes
                .reserve(&request, &format!("p{n}"), DIGEST, &cwd, None)
                .unwrap();
            // Its answer was lost; no retirement is implied by worker exit.
        }
        assert!(owner.issue("new", DIGEST).is_err());
        let request = owner.admit(owner.incarnation(), 1, "p0", DIGEST).unwrap();
        panes
            .cancel_reservation(&request, "p0", DIGEST, &cwd)
            .unwrap();
        assert!(panes.forget(&owner, "p0").is_err());
        assert!(!owner.retire(owner.incarnation(), 1).unwrap());
        drop(request);
        let request = owner.issue("new", DIGEST).unwrap();
        panes.reserve(&request, "new", DIGEST, &cwd, None).unwrap();
        assert_eq!(panes.live().unwrap(), HOLDER_CAP);
    }

    #[test]
    fn normal_completion_does_not_turn_the_live_cap_into_a_lifetime_limit() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let cwd = hide_platform::path::to_wire(home.path()).unwrap();
        for n in 1..=HOLDER_CAP * 3 {
            let pane = format!("p{n}");
            let request = owner.issue(&pane, DIGEST).unwrap();
            let mut current = panes.reserve(&request, &pane, DIGEST, &cwd, None).unwrap();
            // Exercise storage transitions without starting 384 processes.
            current.finish(End::new(EndReason::ProgramExited, "completed"));
            {
                let _lock = panes.lock().unwrap();
                record::write(&layout::pane_record(&panes.state_dir, &pane), &current).unwrap();
            }
            assert!(!owner.retire(owner.incarnation(), request.number()).unwrap());
            drop(request);
        }
        assert_eq!(panes.live().unwrap(), 0);
        let request = owner.issue("next", DIGEST).unwrap();
        panes.reserve(&request, "next", DIGEST, &cwd, None).unwrap();
        assert!(panes.record("p1").unwrap().is_none());
        assert!(owner.admit(owner.incarnation(), 1, "p1", DIGEST).is_err());
        assert_eq!(panes.scan().unwrap().len(), super::super::ENDED_KEPT + 1);
    }
}
