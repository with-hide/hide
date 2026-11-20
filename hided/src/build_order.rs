//! Which of two builds of hided is newer (PRD core-host-node-move D-10, Q3).
//!
//! A build is named by its hash (`build_id`), which says whether two builds
//! are the same and nothing about their order. A package also carries the
//! version it ships and its place in main's history, the first-parent
//! commit count `desktop/scripts/package.mjs` passes as `HIDE_BUILD_ORDER`.
//! Two builds are ordered by their release (`MAJOR.MINOR.PATCH`, whatever
//! follows it ignored) and then by that count. Builds that differ while
//! neither key orders them, or whose version names no release, are
//! unordered, as layer 4 refused every other build.

use serde::{Deserialize, Serialize};

/// What a hello says of the build that sent it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Release {
    pub version: String,
    pub order: u64,
    /// The commit it was built from, for the operator's eyes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

impl Release {
    /// This build's, as a fixture may present it (`env`,
    /// `HIDE_BUILD_VERSION_OVERRIDE` and `HIDE_BUILD_ORDER_OVERRIDE`, read
    /// only under a fixture HOME).
    pub fn of_this_build() -> Self {
        let overridden = crate::env::fixture_build_overrides();
        Self {
            version: overridden
                .version
                .unwrap_or_else(|| crate::cli::VERSION.to_owned()),
            order: overridden.order.unwrap_or(crate::cli::ORDER),
            commit: crate::cli::COMMIT.map(str::to_owned),
        }
    }

    /// `0.4.2 (3f1c2a9)`: the version and the commit's first seven
    /// characters, as the status page shows a build.
    pub fn shown(&self) -> String {
        match &self.commit {
            Some(commit) => format!(
                "{} ({})",
                self.version,
                commit.chars().take(7).collect::<String>()
            ),
            None => self.version.clone(),
        }
    }

    /// Whether this build is an official release: a package numbered it
    /// and its version is the release's own `MAJOR.MINOR.PATCH`. A local
    /// package is named `<newest release>-main.<commit>` and a build no
    /// package made has no number (docs/BUILD.md).
    pub fn is_official(&self) -> bool {
        self.order > 0
            && release_numbers(&self.version).is_some()
            && !self.version.contains(['-', '+'])
    }

    fn key(&self) -> Option<((u64, u64, u64), u64)> {
        Some((release_numbers(&self.version)?, self.order))
    }
}

/// How the build named by `ours` stands to `theirs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Standing {
    Same,
    Newer,
    Older,
    /// Different builds that neither key orders.
    Unordered,
}

/// Compares two builds by hash first: one hash is one build whatever its
/// hello says.
pub fn standing(ours: (&str, &Release), theirs: (&str, &Release)) -> Standing {
    if ours.0 == theirs.0 {
        return Standing::Same;
    }
    match (ours.1.key(), theirs.1.key()) {
        (Some(ours), Some(theirs)) if ours > theirs => Standing::Newer,
        (Some(ours), Some(theirs)) if ours < theirs => Standing::Older,
        _ => Standing::Unordered,
    }
}

/// The leading `MAJOR.MINOR.PATCH` of a version; what follows a `-` or `+`
/// (a pre-release, `git describe`'s distance and hash, `-dirty`) is not
/// part of the release, since the count orders builds of one release.
fn release_numbers(version: &str) -> Option<(u64, u64, u64)> {
    let release = version.split(['-', '+']).next()?;
    let mut parts = release.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u64>().ok())
            .flatten()
    });
    let numbers = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(numbers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str, order: u64) -> Release {
        Release {
            version: version.to_owned(),
            order,
            commit: None,
        }
    }

    #[test]
    fn only_a_numbered_package_of_a_plain_release_version_is_official() {
        assert!(release("1.2.10", 1042).is_official());
        for (version, order) in [
            // A local package, a build no package made, and versions that
            // name no release.
            ("1.2.10-main.f77c2a43", 1042),
            ("1.2.10+dirty", 1042),
            ("0.1.0", 0),
            ("1.2", 7),
        ] {
            assert!(!release(version, order).is_official(), "{version} {order}");
        }
    }

    #[test]
    fn a_build_is_ordered_by_its_release_then_its_count_and_one_hash_is_one_build() {
        let compare = |ours: (&str, Release), theirs: (&str, Release)| {
            standing((ours.0, &ours.1), (theirs.0, &theirs.1))
        };
        assert_eq!(
            compare(("a", release("0.4.2", 10)), ("b", release("0.4.1", 99))),
            Standing::Newer
        );
        assert_eq!(
            compare(("a", release("0.4.1", 99)), ("b", release("0.4.2", 10))),
            Standing::Older
        );
        assert_eq!(
            compare(
                ("a", release("0.4.2-3-gabc-dirty", 12)),
                ("b", release("0.4.2", 11))
            ),
            Standing::Newer,
            "the count orders builds of one release"
        );
        assert_eq!(
            compare(("a", release("0.4.2", 11)), ("b", release("0.4.2", 11))),
            Standing::Unordered,
            "another hash at an equal key is another build"
        );
        assert_eq!(
            compare(("a", release("nightly", 11)), ("b", release("0.4.2", 1))),
            Standing::Unordered
        );
        assert_eq!(
            compare(("a", release("1.2", 11)), ("b", release("0.4.2", 1))),
            Standing::Unordered
        );
        assert_eq!(
            compare(("a", release("nightly", 1)), ("a", release("0.4.2", 9))),
            Standing::Same,
            "one hash is one build"
        );
    }

    #[test]
    fn a_build_is_shown_by_its_version_and_short_commit() {
        let mut shown = release("0.4.2", 1);
        assert_eq!(shown.shown(), "0.4.2");
        shown.commit = Some("3f1c2a9fdeadbeef".to_owned());
        assert_eq!(shown.shown(), "0.4.2 (3f1c2a9)");
    }
}
