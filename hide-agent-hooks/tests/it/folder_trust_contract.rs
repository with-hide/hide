//! Folder trust is decided by the agent definition files alone: an agent
//! whose screen rules draw a question whether to trust a folder declares the
//! trust file Hide records that trust in, or is named in the support cell
//! that says why Hide cannot; and an agent that declares a trust file has a
//! writer that writes exactly there (docs/agent-hooks.md, Folder trust for
//! the folders Hide starts agents in).

use hide_agent_adapter::definition::{self, Definition, PathRule};
use hide_agent_adapter::{AccountPath, NodeEnv, Profile};
use hide_agent_hooks::folder_trust::Agent;

const INSTALL: &str = include_str!("../../../docs/INSTALL.md");

/// The section every agent Hide cannot record the folder trust of is
/// explained in; the support table's row for those agents links it from its
/// first cell.
const GAP_SECTION: &str = "agent-hooks.md#agents-whose-folder-trust-hide-does-not-record";

/// Whether `definition`'s screen rules draw the agent's question whether to
/// trust a folder: a rule that says so (`asks = "folder_trust"`), which the
/// captured screens of that question are held to (`hide-agent-adapter`'s
/// `screens` tests).
fn asks_folder_trust(definition: &Definition) -> bool {
    definition.screen.as_ref().is_some_and(|screen| {
        screen
            .rules
            .rules()
            .iter()
            .any(|rule| rule.asks == Some(definition::Asks::FolderTrust))
    })
}

/// The first cells of the support table rows that link [`GAP_SECTION`].
fn gap_cells() -> Vec<&'static str> {
    INSTALL
        .lines()
        .filter_map(|line| line.strip_prefix('|')?.split('|').next())
        .filter(|cell| cell.contains(GAP_SECTION))
        .collect()
}

#[test]
fn every_agent_whose_screen_asks_for_folder_trust_records_it_or_names_its_support_gap() {
    let gaps = gap_cells();
    let asking: Vec<&Definition> = definition::builtins()
        .iter()
        .filter(|definition| asks_folder_trust(definition))
        .collect();
    // Every agent whose definition's rules were taken from a real screen of
    // that question says so; a rule that stopped saying it would leave its
    // agent out of this test unseen.
    for id in ["codex", "antigravity", "devin", "qwen", "muse"] {
        assert!(
            asking.iter().any(|definition| definition.id == id),
            "{id}'s screen rules no longer say which rule is its folder trust question"
        );
    }
    for definition in asking {
        let records = Agent::of_kind(&definition.id).is_some();
        let gap = gaps.iter().any(|cell| cell.contains(&definition.name));
        assert!(
            records || gap,
            "{} draws a folder trust question but its definition declares no \
             folder_trust file, and docs/INSTALL.md names it in no row linking {GAP_SECTION}",
            definition.name
        );
        assert!(
            !(records && gap),
            "{} records its folder trust, yet docs/INSTALL.md still names it as a gap",
            definition.name
        );
    }
}

#[test]
fn every_agent_that_declares_a_folder_trust_file_has_a_writer_that_writes_there() {
    let home = tempfile::tempdir().unwrap();
    let mut declaring = 0;
    for definition in definition::builtins() {
        let declared = definition.account.as_ref().is_some_and(|account| {
            !matches!(account.path(AccountPath::FolderTrust), PathRule::Absent)
        });
        let writer = Agent::of_kind(&definition.id);
        assert_eq!(
            declared,
            writer.is_some(),
            "{}: a folder_trust file and a writer come together",
            definition.id
        );
        let Some(writer) = writer else {
            continue;
        };
        declaring += 1;
        let account = hide_agent_adapter::resolve(
            definition,
            &Profile::builtin_default(&definition.id),
            &NodeEnv::new(Some(home.path().to_path_buf())),
        )
        .unwrap();
        assert_eq!(
            writer.settings_path(home.path()),
            account.path(AccountPath::FolderTrust).unwrap(),
            "{}'s writer writes another file than its definition declares",
            definition.id
        );
        // Every spelling of the kind reaches the same writer.
        for alias in &definition.aliases {
            assert_eq!(Agent::of_kind(alias), Some(writer), "{alias}");
        }
    }
    assert!(declaring > 0);
}
