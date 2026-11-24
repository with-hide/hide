//! The Mac app's identity as macOS's consent knows it, from
//! `contracts/app-identity.json`: its bundle identifier, the name a consent
//! window shows, each usage sentence of its `Info.plist`, and the code
//! requirement its consent is kept under.
//!
//! The core a login item runs and the pane holders it starts have no app
//! above them and answer macOS's consent under this identity
//! (`hide_platform::privacy`, [`crate::holders::Source::identity`]). The
//! package reads the same file: it signs both so that they meet the
//! requirement and fails when the app it built says anything else
//! (`desktop/scripts/app-identity.mjs`).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use hide_platform::privacy::Identity;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    identifier: String,
    name: String,
    requirement: String,
    usage: BTreeMap<String, String>,
}

/// Read once and kept for the life of the process, as the borrowed form
/// [`Identity`] takes: a fixed handful of short strings.
static APP: LazyLock<Identity<'static>> = LazyLock::new(|| {
    let contract: Contract =
        serde_json::from_str(include_str!("../../contracts/app-identity.json"))
            .expect("contracts/app-identity.json is the app's identity");
    let kept = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let usage: Vec<(&'static str, &'static str)> = contract
        .usage
        .into_iter()
        .map(|(key, sentence)| (kept(key), kept(sentence)))
        .collect();
    Identity {
        identifier: kept(contract.identifier),
        name: kept(contract.name),
        requirement: kept(contract.requirement),
        usage: Box::leak(usage.into_boxed_slice()),
    }
});

/// The app's identity.
pub fn app() -> Identity<'static> {
    *APP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contract_reads_and_its_requirement_names_its_identifier() {
        let app = app();
        assert!(!app.name.is_empty());
        assert!(
            app.requirement
                .starts_with(&format!("identifier \"{}\" and ", app.identifier)),
            "{}",
            app.requirement
        );
        assert!(!app.usage.is_empty());
        for (key, sentence) in app.usage {
            assert!(
                key.starts_with("NS") && key.ends_with("UsageDescription"),
                "{key}"
            );
            assert!(!sentence.is_empty(), "{key}");
        }
    }
}
