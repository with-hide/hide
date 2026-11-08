//! The built-in agent definitions and the operator's own files (PRD
//! engine-bundle-3-core-owns D-16, D-17, B21, B23, B24).

use hide_agent_adapter::definition::{
    Catalog, Hooks, MAX_FILE_BYTES, MAX_USER_FILES, Source, UserFile, builtin, builtins,
};
use hide_agent_adapter::{ADAPTERS, AgentId};

#[test]
fn built_in_package_identities_match_their_install_routes() {
    use hide_agent_adapter::InstallMethod;
    for adapter in ADAPTERS {
        let installed: std::collections::BTreeSet<_> = adapter
            .updates
            .iter()
            .filter_map(|route| match route.method {
                InstallMethod::Npm(name) | InstallMethod::Bun(name) => Some(name),
                _ => None,
            })
            .collect();
        let declared: std::collections::BTreeSet<_> = builtin(adapter.id)
            .unwrap()
            .packages
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(declared, installed, "{}", adapter.id);
    }
}

#[test]
fn package_identities_are_full_names_and_a_bad_file_is_left_out_alone() {
    let file = |id: &str, package: &str| {
        format!(
            "schema = 1\nid = {id:?}\nname = {id:?}\nexecutables = [{id:?}]\nhooks = \"none\"\npackages = [{package:?}]\n"
        )
    };
    for bad in [
        "",
        "@scope",
        "@/name",
        "@scope/",
        "@scope/name/extra",
        "../name",
        "name@1",
        "Name",
        "name with space",
        "C:\\name",
    ] {
        let catalog = Catalog::read(vec![
            text("bad.toml", &file("bad", bad)),
            text("good.toml", &file("good", "@local/good")),
        ]);
        assert_eq!(catalog.rejected.len(), 1, "{bad:?}");
        assert_eq!(catalog.rejected[0].file, "bad.toml");
        assert!(catalog.rejected[0].reason.contains("package"));
        assert_eq!(
            catalog.definitions.last().unwrap().packages,
            ["@local/good"]
        );
    }
    let catalog = Catalog::read(vec![
        text("first.toml", &file("first", "@local/shared")),
        text("second.toml", &file("second", "@local/shared")),
        text("third.toml", &file("third", "@other/shared")),
    ]);
    assert_eq!(catalog.rejected.len(), 1);
    assert!(
        catalog.rejected[0]
            .reason
            .contains("package @local/shared is taken by first")
    );
    assert_eq!(catalog.definitions.last().unwrap().id, "third");
}

fn text(name: &str, text: &str) -> UserFile {
    UserFile::Text {
        name: name.to_owned(),
        text: text.to_owned(),
    }
}

const RULES: &str = r#"
[screen]
version = "1"
min_engine_version = 1

[[screen.rules]]
id = "ready"
state = "idle"
contains = ["ready"]
"#;

#[test]
fn the_built_in_definitions_are_herdrs_twenty_two_and_omp() {
    let mut ids: Vec<&str> = builtins()
        .iter()
        .map(|definition| definition.id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "amp",
            "antigravity",
            "claude-code",
            "cline",
            "codex",
            "copilot",
            "cursor",
            "devin",
            "droid",
            "gemini",
            "grok",
            "hermes",
            "kilo",
            "kimi",
            "kiro",
            "letta",
            "maki",
            "muse",
            "omp",
            "opencode",
            "pi",
            "qodercli",
            "qwen",
        ]
    );
    for definition in builtins() {
        assert_eq!(definition.source, Source::Builtin);
        // omp's state comes from its extension; every other has rules.
        assert_eq!(
            definition.screen.is_none(),
            definition.id == "omp",
            "{}",
            definition.id
        );
    }
}

#[test]
fn each_agent_with_hides_features_takes_its_identity_from_its_definition() {
    let dialects = [
        (AgentId::ClaudeCode, Hooks::Claude),
        (AgentId::Codex, Hooks::Codex),
        (AgentId::Grok, Hooks::Grok),
        (AgentId::OpenCode, Hooks::Opencode),
        (AgentId::Pi, Hooks::Pi),
        (AgentId::Omp, Hooks::Omp),
        (AgentId::Antigravity, Hooks::Agy),
        (AgentId::Cursor, Hooks::None),
    ];
    assert_eq!(
        dialects.len(),
        ADAPTERS.len() + 1,
        "every adapter and retired Cursor"
    );
    for (key, hooks) in dialects {
        let adapter = key.adapter();
        let definition =
            builtin(adapter.id).unwrap_or_else(|| panic!("{}: no definition", adapter.id));
        assert_eq!(definition.hooks, hooks, "{}", adapter.id);
        assert_eq!(definition.name, adapter.label);
        assert_eq!(definition.executables, adapter.executables);
        assert_eq!(definition.aliases, adapter.aliases);
        // The definition's `home` and its default account's own folder are
        // one folder; retired Cursor keeps no account (PRD agent-profiles D-25).
        if key == AgentId::Cursor {
            assert!(definition.account.is_none());
            continue;
        }
        assert_eq!(
            definition.home,
            hide_agent_adapter::default_folder(definition, hide_agent_adapter::AccountPath::State),
            "{}",
            adapter.id
        );
        if let Some(dialect) = adapter.resume {
            let session = definition
                .resume_session
                .as_deref()
                .expect("a resumed agent's session words");
            assert_eq!(session, [dialect.resume_flag(), "{session}"]);
        }
    }
}

#[test]
fn an_empty_folder_leaves_the_built_in_agents() {
    let catalog = Catalog::read(Vec::new());
    assert_eq!(catalog.definitions.len(), builtins().len());
    assert!(catalog.rejected.is_empty());
}

#[test]
fn a_file_based_on_pi_gets_pis_hooks_resume_and_rules_with_its_own_program_and_folder() {
    let catalog = Catalog::read(vec![text(
        "mypi.toml",
        "schema = 1\nid = \"mypi\"\nname = \"My Pi\"\nbased_on = \"pi\"\nexecutables = [\"mypi\"]\nhome = \"~/.mypi/agent\"\n",
    )]);
    assert!(catalog.rejected.is_empty(), "{:?}", catalog.rejected);
    let pi = builtin("pi").unwrap();
    let mine = catalog
        .definitions
        .iter()
        .find(|definition| definition.id == "mypi")
        .unwrap();
    assert_eq!(mine.hooks, Hooks::Pi);
    assert_eq!(mine.based_on.as_deref(), Some("pi"));
    assert_eq!(mine.source, Source::User("mypi.toml".into()));
    assert_eq!(mine.executables, ["mypi"]);
    assert_eq!(
        mine.home.as_deref(),
        Some(&[".mypi".to_owned(), "agent".to_owned()][..])
    );
    assert_eq!(mine.resume_session, pi.resume_session);
    let (mine_rules, pi_rules) = (mine.screen.as_ref().unwrap(), pi.screen.as_ref().unwrap());
    assert_eq!(mine_rules.source, Source::Builtin);
    assert_eq!(mine_rules.rules.version(), pi_rules.rules.version());
}

#[test]
fn a_file_with_a_built_in_id_replaces_only_its_screen_rules() {
    let catalog = Catalog::read(vec![text(
        "pi.toml",
        &format!("schema = 1\nid = \"pi\"\n{RULES}"),
    )]);
    assert!(catalog.rejected.is_empty(), "{:?}", catalog.rejected);
    let pi = catalog
        .definitions
        .iter()
        .find(|definition| definition.id == "pi")
        .unwrap();
    assert_eq!(pi.source, Source::Builtin);
    let screen = pi.screen.as_ref().unwrap();
    assert_eq!(screen.source, Source::User("pi.toml".into()));
    assert_eq!(screen.rules.version().as_str(), "1");
}

#[test]
fn a_starts_ready_rule_is_an_idle_rule_of_the_screen_rules_the_agent_ends_up_with() {
    let opencode = builtin("opencode").unwrap();
    assert_eq!(opencode.start_ready.as_deref(), Some("model_line_idle"));
    let model_line = "\n[[screen.rules]]\nid = \"model_line_idle\"\nstate = \"idle\"\ncontains = [\"build ·\"]\n";
    let busy = "\n[[screen.rules]]\nid = \"busy\"\nstate = \"working\"\ncontains = [\"busy\"]\n";
    let new_agent =
        "schema = 1\nid = \"mine\"\nname = \"Mine\"\nexecutables = [\"mine\"]\nhooks = \"none\"\n";
    let catalog = Catalog::read(vec![
        // Rules without the ready rule would hold every start to its deadline.
        text(
            "opencode-a.toml",
            &format!("schema = 1\nid = \"opencode\"\n{RULES}"),
        ),
        text(
            "opencode-b.toml",
            &format!("schema = 1\nid = \"opencode\"\n{RULES}{model_line}"),
        ),
        text(
            "copy.toml",
            "schema = 1\nid = \"copy\"\nname = \"Copy\"\nbased_on = \"opencode\"\nexecutables = [\"copy\"]\n",
        ),
        text(
            "mine.toml",
            &format!("{new_agent}[start]\nready = \"ready\"\n{RULES}"),
        ),
        text(
            "busy.toml",
            &format!(
                "{}[start]\nready = \"busy\"\n{RULES}{busy}",
                new_agent.replace("mine", "busy")
            ),
        ),
    ]);
    let reasons: Vec<&str> = catalog
        .rejected
        .iter()
        .map(|rejected| rejected.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            "opencode-a.toml: start.ready names screen rule \"model_line_idle\", which [screen] does not have",
            "busy.toml: start.ready names screen rule \"busy\", which is not an idle rule",
        ]
    );
    let find = |id: &str| {
        catalog
            .definitions
            .iter()
            .find(|definition| definition.id == id)
            .unwrap()
    };
    assert_eq!(
        find("opencode").screen.as_ref().unwrap().source,
        Source::User("opencode-b.toml".into())
    );
    assert_eq!(find("copy").start_ready.as_deref(), Some("model_line_idle"));
    assert_eq!(find("mine").start_ready.as_deref(), Some("ready"));
}

#[test]
fn every_later_file_for_the_same_built_in_names_the_one_whose_rules_were_taken() {
    let pi = format!("schema = 1\nid = \"pi\"\n{RULES}");
    let catalog = Catalog::read(
        ["pi-a.toml", "pi-b.toml", "pi-c.toml"]
            .into_iter()
            .map(|name| text(name, &pi))
            .collect(),
    );
    let reasons: Vec<&str> = catalog
        .rejected
        .iter()
        .map(|rejected| rejected.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            "pi-b.toml: pi-a.toml already replaces the screen rules of pi",
            "pi-c.toml: pi-a.toml already replaces the screen rules of pi",
        ]
    );
}

#[test]
fn a_new_agent_names_its_program_and_reads_its_state_from_the_screen() {
    let catalog = Catalog::read(vec![text(
        "tool.toml",
        &format!(
            "schema = 1\nid = \"tool\"\nname = \"Tool\"\nexecutables = [\"tool\"]\nhooks = \"none\"\n\n[resume]\nlast = [\"--continue\"]\n{RULES}"
        ),
    )]);
    assert!(catalog.rejected.is_empty(), "{:?}", catalog.rejected);
    let tool = catalog.definitions.last().unwrap();
    assert_eq!((tool.id.as_str(), tool.hooks), ("tool", Hooks::None));
    assert_eq!(
        tool.resume_last.as_deref(),
        Some(&["--continue".to_owned()][..])
    );
}

#[test]
fn each_unusable_file_is_left_out_alone_with_one_line_naming_it() {
    let ok =
        "schema = 1\nid = \"good\"\nname = \"Good\"\nexecutables = [\"good\"]\nhooks = \"none\"\n";
    let cases: Vec<(UserFile, &str)> = vec![
        (
            text("syntax.toml", "schema = 1\nid = "),
            "not a definition file",
        ),
        (
            text(
                "field.toml",
                "schema = 1\nid = \"field\"\nname = \"F\"\nexecutables = [\"f\"]\nhooks = \"none\"\n[resume]\nsesion = [\"x\"]\n",
            ),
            "not a definition file",
        ),
        (
            text("schema.toml", "schema = 2\nid = \"schema\"\n"),
            "schema 2",
        ),
        (
            text("id.toml", "schema = 1\nid = \"Bad Id\"\n"),
            "id \"Bad Id\"",
        ),
        (
            text(
                "builtin.toml",
                "schema = 1\nid = \"codex\"\nexecutables = [\"codex2\"]\n",
            ),
            "may change [screen] only, not executables",
        ),
        (
            text(
                "base.toml",
                "schema = 1\nid = \"base\"\nname = \"B\"\nbased_on = \"nothing\"\nexecutables = [\"b\"]\n",
            ),
            "based_on nothing is not a built-in agent",
        ),
        (
            text(
                "chain.toml",
                "schema = 1\nid = \"chain\"\nname = \"C\"\nbased_on = \"good\"\nexecutables = [\"c\"]\n",
            ),
            "based_on good is not a built-in agent",
        ),
        (
            text(
                "hooks.toml",
                "schema = 1\nid = \"hooks\"\nname = \"H\"\nexecutables = [\"h\"]\nhooks = \"claude\"\n",
            ),
            "hooks must be \"none\"",
        ),
        (
            text(
                "taken.toml",
                "schema = 1\nid = \"taken\"\nname = \"T\"\nexecutables = [\"pi\"]\nhooks = \"none\"\n",
            ),
            "program pi is taken by pi",
        ),
        (
            text(
                "again.toml",
                "schema = 1\nid = \"again\"\nname = \"A\"\nexecutables = [\"good\"]\nhooks = \"none\"\n",
            ),
            "program good is taken by good",
        ),
        (
            text(
                "path.toml",
                "schema = 1\nid = \"path\"\nname = \"P\"\nexecutables = [\"/usr/bin/p\"]\nhooks = \"none\"\n",
            ),
            "is not a file name",
        ),
        (
            text(
                "space.toml",
                "schema = 1\nid = \"space\"\nname = \"S\"\nexecutables = [\"space\", \"space cli\"]\nhooks = \"none\"\n",
            ),
            "program \"space cli\" is not a file name",
        ),
        (
            text(
                "resume.toml",
                "schema = 1\nid = \"resume\"\nname = \"R\"\nexecutables = [\"r\"]\nhooks = \"none\"\n[resume]\nsession = [\"--resume\"]\n",
            ),
            "names {session} 0 times",
        ),
        (
            text(
                "rules.toml",
                &format!(
                    "schema = 1\nid = \"rules\"\nname = \"R\"\nexecutables = [\"rr\"]\nhooks = \"none\"\n{}",
                    RULES.replace("contains = [\"ready\"]", "regex = [\"[\"]")
                ),
            ),
            "screen rules: rule ready",
        ),
        (
            UserFile::TooLarge {
                name: "large.toml".into(),
                bytes: MAX_FILE_BYTES + 1,
            },
            "more than the 262144",
        ),
        (
            UserFile::BeyondLimit {
                name: "zz.toml".into(),
            },
            "past the first 64 files",
        ),
    ];
    let mut files = vec![text("good.toml", ok)];
    let expected: Vec<(String, &str)> = cases
        .iter()
        .map(|(file, reason)| {
            let name = match file {
                UserFile::Text { name, .. }
                | UserFile::TooLarge { name, .. }
                | UserFile::BeyondLimit { name }
                | UserFile::Unreadable { name, .. } => name.clone(),
            };
            (name, *reason)
        })
        .collect();
    files.extend(cases.into_iter().map(|(file, _)| file));
    let catalog = Catalog::read(files);
    assert!(
        catalog
            .definitions
            .iter()
            .any(|definition| definition.id == "good"),
        "the good file stays"
    );
    assert_eq!(catalog.definitions.len(), builtins().len() + 1);
    assert_eq!(
        catalog.rejected.len(),
        expected.len(),
        "{:#?}",
        catalog.rejected
    );
    for (rejected, (name, reason)) in catalog.rejected.iter().zip(expected) {
        assert_eq!(rejected.file, name);
        assert!(
            rejected.reason.starts_with(&format!("{name}: ")),
            "{}",
            rejected.reason
        );
        assert!(
            rejected.reason.contains(reason),
            "{name}: {}",
            rejected.reason
        );
        assert!(
            !rejected.reason.contains('\n'),
            "one line: {}",
            rejected.reason
        );
    }
    assert_eq!(MAX_USER_FILES, 64);
}
