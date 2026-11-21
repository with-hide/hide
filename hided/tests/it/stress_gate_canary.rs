//! The gate canary of a lane that repeats its tests.
//!
//! nextest's `--stress-count N` ends 0 when its last iteration passed,
//! whatever the earlier ones did (docs/TESTING.md, The repeat lane), so a
//! repeated lane only means something while a failed iteration fails the job
//! (`scripts/check-stress-report.py`). This test is how that gate is shown to
//! work on a real runner: `remote-mailbox.yml` hands it
//! `HIDE_TEST_STRESS_CANARY_FAIL_ITERATION`, a 1-based iteration, and it fails
//! in that one and passes in the rest, so the run ends with a summary that
//! counts a failure nextest exits 0 for.
//!
//! It injects nothing by itself. With the variable unset, or outside a
//! `--stress-count` run, it passes, so the lane's ordinary runs are unchanged
//! and no product test carries a fault path.

/// The iteration to fail, as the lane names it: 1-based.
const FAIL_ITERATION: &str = "HIDE_TEST_STRESS_CANARY_FAIL_ITERATION";

/// Whether this process is the iteration the lane asked to fail.
///
/// `current` is nextest's own `NEXTEST_STRESS_CURRENT`: the 0-based iteration
/// of the stress run this process runs in, and `none` for a run without
/// `--stress-count` (nextest sets it for every test it starts). A name that is
/// missing, not a number, or past the iterations the run makes matches no
/// iteration, so a mistyped input fails nothing and says so in the run's
/// summary rather than passing for the wrong reason.
fn fails_here(named: Option<&str>, current: Option<&str>) -> bool {
    let Some(named) = named.and_then(|text| text.trim().parse::<u32>().ok()) else {
        return false;
    };
    let Some(current) = current.and_then(|text| text.trim().parse::<u32>().ok()) else {
        return false;
    };
    current + 1 == named
}

#[test]
#[ignore = "runs in the repeated lane, which runs ignored tests; a lane with no repeat has nothing for it to show"]
fn a_named_stress_iteration_fails_and_the_others_pass() {
    let named = std::env::var(FAIL_ITERATION).ok();
    let current = std::env::var("NEXTEST_STRESS_CURRENT").ok();
    assert!(
        !fails_here(named.as_deref(), current.as_deref()),
        "the lane asked iteration {named:?} to fail and nextest is running {current:?}: \
         this run must end red, so `scripts/check-stress-report.py` did not fail a repeated \
         run that had a failed iteration (docs/TESTING.md, The repeat lane)"
    );
}

#[cfg(test)]
mod tests {
    use super::fails_here;

    #[test]
    fn only_the_named_iteration_fails() {
        // The lane names the iteration from 1; nextest counts from 0.
        assert!(fails_here(Some("1"), Some("0")));
        assert!(fails_here(Some("2"), Some("1")));
        assert!(!fails_here(Some("2"), Some("0")));
        assert!(!fails_here(Some("1"), Some("1")));
    }

    #[test]
    fn a_run_nothing_asked_to_fail_passes() {
        assert!(!fails_here(None, Some("0")));
        assert!(!fails_here(Some(""), Some("0")));
        // nextest's own name for a run without `--stress-count`.
        assert!(!fails_here(Some("1"), Some("none")));
        assert!(!fails_here(Some("1"), None));
    }

    #[test]
    fn a_name_that_is_no_iteration_fails_nothing() {
        assert!(!fails_here(Some("0"), Some("0")));
        assert!(!fails_here(Some("-1"), Some("0")));
        assert!(!fails_here(Some("first"), Some("0")));
        assert!(!fails_here(Some("1"), Some("first")));
    }
}
