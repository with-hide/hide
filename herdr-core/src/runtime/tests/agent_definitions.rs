//! Agent definition files through the core (PRD engine-bundle-3-core-owns
//! B1, B21, B24): with the development switch off nothing is read and
//! `hide agent catalog` and the kinds a spawn starts are the seven agents
//! they were; on, the catalog lists every built-in and operator definition
//! with its source and screen rules, and each file left out with why.

use super::*;
use hide_agent_adapter::definition::{Catalog, UserFile, builtins};
use serde_json::Value;

const SEVEN: [&str; 7] = [
    "claude-code",
    "codex",
    "grok",
    "opencode",
    "pi",
    "omp",
    "antigravity",
];
/// The fields every catalog row carried before definitions existed.
const ROW_FIELDS: [&str; 14] = [
    "agent",
    "cli_default_model",
    "efforts",
    "enabled",
    "id",
    "installed",
    "label",
    "models",
    "models_unavailable_reason",
    "past_usage_limit",
    "signed_in",
    "takes_model",
    "weekly_resets_at_ms",
    "weekly_used_percent",
];

fn ids(catalog: &Value) -> Vec<String> {
    catalog["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

fn row<'a>(catalog: &'a Value, id: &str) -> &'a Value {
    catalog["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("no {id} in {catalog}"))
}

#[test]
fn with_the_switch_off_the_catalog_and_spawn_keep_the_seven_agents_and_nothing_is_read() {
    let mut runtime = runtime();
    let node = crate::node::TEST_NODE;
    let catalog = runtime.agent_catalog(node, None, true).unwrap();
    assert_eq!(ids(&catalog), SEVEN);
    for item in catalog["items"].as_array().unwrap() {
        let mut fields: Vec<&str> = item
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ROW_FIELDS, "{item}");
    }
    let mut top: Vec<&str> = catalog
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    top.sort_unstable();
    assert_eq!(
        top,
        ["installed_known", "items", "machine", "models_read_pending"]
    );
    // A spawn starts the same kinds; a definition-only agent is not one.
    for id in SEVEN {
        assert!(hide_agent_adapter::start_kind(id).is_some(), "{id}");
    }
    for id in builtins().iter().map(|definition| definition.id.as_str()) {
        assert_eq!(
            hide_agent_adapter::start_kind(id).is_some(),
            SEVEN.contains(&id),
            "{id}"
        );
    }
    // Settings asking for a read starts none, and the snapshot carries none.
    assert!(!runtime.request_agent_definitions());
    assert!(runtime.agent_definitions.is_none());
    assert!(runtime.snapshot.status.agent_definitions.is_none());
}

#[test]
fn with_the_switch_on_the_catalog_lists_every_definition_its_source_and_rules_and_what_was_left_out()
 {
    let mut runtime = runtime();
    let node = crate::node::TEST_NODE;
    let files = vec![
        UserFile::Text {
            name: "mypi.toml".into(),
            text: "schema = 1\nid = \"mypi\"\nname = \"My Pi\"\nbased_on = \"pi\"\nexecutables = [\"mypi\"]\n".into(),
        },
        UserFile::Text {
            name: "codex.toml".into(),
            text: "schema = 1\nid = \"codex\"\n[screen]\nversion = \"9\"\nmin_engine_version = 1\n[[screen.rules]]\nid = \"r\"\nstate = \"idle\"\ncontains = [\"ready\"]\n".into(),
        },
        UserFile::Text {
            name: "broken.toml".into(),
            text: "schema = 1\nid = ".into(),
        },
    ];
    assert!(runtime.settle_agent_definitions(PathBuf::from("/home"), Ok(Catalog::read(files))));
    let catalog = runtime.agent_catalog(node, None, true).unwrap();
    let listed = ids(&catalog);
    assert_eq!(listed.len(), builtins().len() + 1);
    assert_eq!(
        &listed[..7],
        SEVEN,
        "the seven keep their order and come first"
    );
    assert!(listed.contains(&"gemini".to_owned()) && listed.contains(&"mypi".to_owned()));

    let gemini = row(&catalog, "gemini");
    assert_eq!(gemini["source"]["kind"], "built-in");
    assert_eq!(gemini["hooks"], "none");
    assert_eq!(gemini["spawn"], false);
    assert_eq!(gemini["screen"]["source"]["kind"], "built-in");
    assert_eq!(gemini["screen"]["version"], "2026.06.10.1");

    let mine = row(&catalog, "mypi");
    assert_eq!(mine["source"]["file"], "~/.hide/agents/mypi.toml");
    assert_eq!(mine["based_on"], "pi");
    assert_eq!(mine["hooks"], "pi");
    assert_eq!(
        mine["screen"]["version"],
        row(&catalog, "pi")["screen"]["version"]
    );

    let codex = row(&catalog, "codex");
    assert_eq!(codex["spawn"], true);
    assert_eq!(codex["source"]["kind"], "built-in");
    assert_eq!(
        codex["screen"]["source"]["file"],
        "~/.hide/agents/codex.toml"
    );
    assert_eq!(codex["screen"]["version"], "9");

    let rejected = catalog["definitions_rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0]["file"], "broken.toml");
    assert!(
        rejected[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("broken.toml: ")
    );
    let status = runtime.snapshot.status.agent_definitions.as_ref().unwrap();
    assert_eq!(status.rejected.len(), 1);
    assert_eq!(status.rejected[0].file, "broken.toml");

    // The same read again changes nothing on the screen.
    let again = Catalog::read(vec![UserFile::Text {
        name: "broken.toml".into(),
        text: "schema = 1\nid = ".into(),
    }]);
    assert!(!runtime.settle_agent_definitions(PathBuf::from("/home"), Ok(again)));
}

#[test]
fn a_folder_that_cannot_be_listed_leaves_the_built_in_agents_and_says_why() {
    let mut runtime = runtime();
    assert!(runtime.settle_agent_definitions(PathBuf::from("/home"), Err("denied".into())));
    let catalog = runtime
        .agent_catalog(crate::node::TEST_NODE, None, true)
        .unwrap();
    assert_eq!(ids(&catalog).len(), builtins().len());
    assert_eq!(catalog["definitions_unreadable"], "denied");
}
