//! The per-agent layer of the kit (issue #517): the adapter table, detection,
//! the skill stub and the guidance hooks, against a private HOME.

use super::*;
use crate::agents::{ADAPTERS, BROWSER_SKILL, CONDUCTOR_SKILL, GUIDE_SKILL, SKILLS};

fn agent<'a>(report: &'a KitReport, id: &str) -> &'a AgentReport {
    report
        .agents
        .iter()
        .find(|agent| agent.id == id)
        .unwrap_or_else(|| panic!("no agent {id} in {report:?}"))
}

fn shared_skill(fixture: &Fixture) -> PathBuf {
    fixture
        .home()
        .join(".agents/skills")
        .join(BROWSER_SKILL.name)
        .join("SKILL.md")
}

/// A folder the agent makes under the home, such as its settings folder.
fn set_up(fixture: &Fixture, folder: &str) {
    std::fs::create_dir_all(fixture.home().join(folder)).unwrap();
}

/// An agent installed the way its installer leaves it: its program in the
/// fixture's `~/.local/bin` (the only folder a test searches) and its own
/// folder under the home.
fn install(fixture: &Fixture, program: &str, folder: &str) {
    executable(
        &fixture.home().join(".local/bin").join(program),
        "#!/bin/sh\n",
    );
    set_up(fixture, folder);
}

fn record(fixture: &Fixture) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(fixture.home().join(".hide/kit/installed.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn every_adapter_names_an_official_page_and_a_unique_id() {
    let mut seen = std::collections::BTreeSet::new();
    for adapter in ADAPTERS {
        assert!(seen.insert(adapter.id), "{} is listed twice", adapter.id);
        assert!(
            adapter.doc_url.starts_with("https://") && adapter.doc_url.len() > "https://".len(),
            "{} has no official page: {:?}",
            adapter.id,
            adapter.doc_url
        );
        assert!(!adapter.label.is_empty());
        assert!(
            !adapter.executables.is_empty(),
            "{} names no program, so it could never be installed",
            adapter.id
        );
        if let HookSupport::Guidance(guidance) = adapter.hook {
            assert_eq!(guidance.id(), adapter.id);
        }
    }
    let defaults: Vec<_> = ADAPTERS
        .iter()
        .filter(|adapter| adapter.default_on)
        .map(|adapter| adapter.id)
        .collect();
    assert_eq!(defaults, ["claude-code", "codex", "antigravity"]);
}

#[test]
fn an_agent_that_is_not_on_gets_nothing_until_the_operator_switches_it_on() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");

    let report = apply(&fixture.target, &Scope::automatic());

    let grok = agent(&report, "grok");
    assert!(!grok.enabled);
    assert_eq!(grok.skill.state, ComponentState::Off);
    assert_eq!(grok.hook.as_ref().unwrap().state, ComponentState::Off);
    assert!(!grok_hooks(&fixture).exists());
    assert!(!shared_skill(&fixture).exists());

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));

    let grok = agent(&report, "grok");
    assert!(grok.enabled);
    assert_eq!(grok.skill.state, ComponentState::Installed, "{grok:?}");
    assert_eq!(
        grok.hook.as_ref().unwrap().state,
        ComponentState::Installed,
        "{grok:?}"
    );
    assert!(shared_skill(&fixture).is_file());
    assert!(
        std::fs::read_to_string(grok_hooks(&fixture))
            .unwrap()
            .contains(&format!(
                "hide-guidance@{}",
                hide_agent_hooks::guidance::GuidanceAgent::Grok.version()
            ))
    );
    assert_eq!(record(&fixture)["agents"]["grok"], true);
}

#[test]
fn switching_on_an_agent_that_is_not_installed_here_records_nothing() {
    let fixture = Fixture::new();

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));

    let grok = agent(&report, "grok");
    assert_eq!(grok.availability, Availability::NotInstalled);
    assert!(
        record(&fixture).get("agents").is_none(),
        "{}",
        record(&fixture)
    );
    assert!(!grok_hooks(&fixture).exists());
}

#[test]
fn a_program_on_the_home_bin_folder_is_the_agent_being_installed() {
    let fixture = Fixture::new();
    executable(&fixture.home().join(".local/bin/grok"), "#!/bin/sh\n");

    let report = status(&fixture.target);

    assert_eq!(agent(&report, "grok").availability, Availability::Available);
    assert_eq!(
        agent(&report, "pi").availability,
        Availability::NotInstalled
    );
}

#[test]
fn a_second_apply_of_an_agent_that_is_on_changes_nothing() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    let tree = home_tree(fixture.home());

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(home_tree(fixture.home()), tree);
    assert!(agent(&report, "grok").enabled);
}

#[test]
fn removed_by_hand_stays_removed_until_the_operator_reinstalls() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    std::fs::remove_file(grok_hooks(&fixture)).unwrap();
    std::fs::remove_dir_all(fixture.home().join(".agents")).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    let grok = agent(&report, "grok");
    assert_eq!(grok.hook.as_ref().unwrap().state, ComponentState::Removed);
    assert_eq!(grok.skill.state, ComponentState::Removed);
    assert!(!grok_hooks(&fixture).exists());
    assert!(!shared_skill(&fixture).exists());

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));

    let grok = agent(&report, "grok");
    assert_eq!(grok.hook.as_ref().unwrap().state, ComponentState::Installed);
    assert_eq!(grok.skill.state, ComponentState::Installed);
}

#[test]
fn an_older_stub_is_replaced_without_the_operator_asking() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    let path = shared_skill(&fixture);
    let current = std::fs::read_to_string(&path).unwrap();
    let older = format!("hide-skill@{}", BROWSER_SKILL.version - 1);
    let marker = format!("hide-skill@{}", BROWSER_SKILL.version);
    std::fs::write(&path, current.replace(&marker, &older)).unwrap();

    let before = status(&fixture.target);
    assert_eq!(agent(&before, "grok").skill.state, ComponentState::Outdated);
    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        agent(&report, "grok").skill.state,
        ComponentState::Installed
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), current);
}

#[test]
fn a_version_1_stub_with_a_plain_description_is_replaced() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    let path = shared_skill(&fixture);
    // What builds before quoted descriptions wrote.
    let version_1 = "---\nname: hide-browser\ndescription: Read and drive a browser display inside Hide. Use when a task involves a web page, a local app in a browser tab, or checking what a page shows.\n---\n\n<!-- hide-skill@1: written by Hide; remove it from Settings, Agents -->\n\nRun `hide browser help` and follow what it prints. It explains how to open a page, read it by refs, check a change with `--diff`, and what to do when a command fails.\n";
    std::fs::write(&path, version_1).unwrap();
    assert_eq!(
        agent(&status(&fixture.target), "grok").skill.state,
        ComponentState::Outdated
    );

    let report = apply(&fixture.target, &Scope::automatic());

    assert_eq!(
        agent(&report, "grok").skill.state,
        ComponentState::Installed
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(
        "---\nname: hide-browser\ndescription: \"Read and drive a browser display inside Hide;"
    ));
    assert_eq!(text, BROWSER_SKILL.text());
}

#[test]
fn switching_an_agent_off_takes_only_hides_pieces_and_keeps_a_folder_another_agent_reads() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    install(&fixture, "codex", ".codex");
    let settings = r#"{"version":1,"hooks":{"SessionStart":[{"command":"/opt/other/start.sh"}]}}"#;
    std::fs::create_dir_all(grok_hooks(&fixture).parent().unwrap()).unwrap();
    std::fs::write(grok_hooks(&fixture), settings).unwrap();
    apply(&fixture.target, &Scope::agents(["grok"], []));
    assert!(shared_skill(&fixture).is_file());

    let report = apply(&fixture.target, &Scope::agents([], ["grok"]));

    let grok = agent(&report, "grok");
    assert!(!grok.enabled);
    assert_eq!(grok.hook.as_ref().unwrap().state, ComponentState::Off);
    let left: Value =
        serde_json::from_str(&std::fs::read_to_string(grok_hooks(&fixture)).unwrap()).unwrap();
    assert_eq!(left["version"], 1);
    assert_eq!(left["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert!(left.to_string().contains("/opt/other/start.sh"));
    assert!(!left.to_string().contains("hide-guidance"));
    // Codex reads the shared folder and is still on.
    assert!(shared_skill(&fixture).is_file());
    assert_eq!(record(&fixture)["agents"]["grok"], false);

    // No later pass puts it back.
    apply(&fixture.target, &Scope::automatic());
    assert!(
        !std::fs::read_to_string(grok_hooks(&fixture))
            .unwrap()
            .contains("hide-guidance")
    );

    // Once no agent that reads the folder is on, the stub goes.
    apply(&fixture.target, &Scope::agents([], ["codex"]));
    assert!(!shared_skill(&fixture).exists());
}

#[test]
fn every_skill_folder_gets_every_stub_and_a_folder_from_before_a_stub_gets_it() {
    let fixture = Fixture::new();
    install(&fixture, "claude", ".claude");
    install(&fixture, "grok", ".grok");
    install(&fixture, "agy", ".gemini/config");
    let report = apply(
        &fixture.target,
        &Scope::agents(["claude-code", "grok", "antigravity"], []),
    );
    for id in ["claude-code", "grok", "antigravity"] {
        assert_eq!(
            agent(&report, id).skill.state,
            ComponentState::Installed,
            "{id}"
        );
    }
    for folder in [".claude/skills", ".agents/skills", ".gemini/config/skills"] {
        for skill in SKILLS {
            let path = fixture
                .home()
                .join(folder)
                .join(skill.name)
                .join("SKILL.md");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                skill.text(),
                "{path:?}"
            );
        }
    }
    assert!(CONDUCTOR_SKILL.text().contains("hide mission help"));
    assert!(GUIDE_SKILL.text().contains("run `hide guide`"));

    // A folder an older Hide wrote holds the browser stub alone.
    let conductor = fixture
        .home()
        .join(".agents/skills")
        .join(CONDUCTOR_SKILL.name);
    std::fs::remove_dir_all(&conductor).unwrap();
    let report = status(&fixture.target);
    assert_eq!(agent(&report, "grok").skill.state, ComponentState::Outdated);
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        agent(&report, "grok").skill.state,
        ComponentState::Installed
    );
    assert!(conductor.join("SKILL.md").is_file());

    // A folder written before the guide holds the browser and conductor
    // stubs only.
    let guide = fixture.home().join(".claude/skills").join(GUIDE_SKILL.name);
    std::fs::remove_dir_all(&guide).unwrap();
    let report = status(&fixture.target);
    assert_eq!(
        agent(&report, "claude-code").skill.state,
        ComponentState::Outdated
    );
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        agent(&report, "claude-code").skill.state,
        ComponentState::Installed
    );
    assert_eq!(
        std::fs::read_to_string(guide.join("SKILL.md")).unwrap(),
        GUIDE_SKILL.text()
    );

    // Switching every reader off takes both stubs out.
    apply(&fixture.target, &Scope::agents([], ["grok", "codex"]));
    for skill in SKILLS {
        assert!(
            !fixture
                .home()
                .join(".agents/skills")
                .join(skill.name)
                .exists()
        );
    }
}

#[test]
fn a_skill_that_hide_did_not_write_is_left_alone_and_reported() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    let own = shared_skill(&fixture);
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "---\nname: hide-browser\n---\nmine\n").unwrap();

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));

    assert_eq!(agent(&report, "grok").skill.state, ComponentState::Absent);
    assert!(!agent(&report, "grok").needs_attention());
    assert_eq!(
        std::fs::read_to_string(&own).unwrap(),
        "---\nname: hide-browser\n---\nmine\n"
    );
    apply(&fixture.target, &Scope::agents([], ["grok", "codex"]));
    assert_eq!(
        std::fs::read_to_string(&own).unwrap(),
        "---\nname: hide-browser\n---\nmine\n"
    );
}

#[test]
fn an_operators_own_hide_skill_stays_beside_hides_other_stubs() {
    let fixture = Fixture::new();
    install(&fixture, "claude", ".claude");
    let mine = "---\nname: hide\n---\nmy notes on hiding files\n";
    let own = fixture
        .home()
        .join(".claude/skills")
        .join(GUIDE_SKILL.name)
        .join("SKILL.md");
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, mine).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    let skill = &agent(&report, "claude-code").skill;
    assert_eq!(skill.state, ComponentState::Absent);
    assert_eq!(
        skill.reason.as_deref(),
        Some("a skill named hide that Hide did not write is already there; Hide left it alone")
    );
    assert_eq!(std::fs::read_to_string(&own).unwrap(), mine);
    for stub in [BROWSER_SKILL, CONDUCTOR_SKILL] {
        let path = fixture
            .home()
            .join(".claude/skills")
            .join(stub.name)
            .join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), stub.text());
    }

    // Switching Claude Code off takes Hide's stubs out and leaves the
    // operator's.
    apply(&fixture.target, &Scope::agents([], ["claude-code"]));
    assert_eq!(std::fs::read_to_string(&own).unwrap(), mine);
    assert!(!fixture.home().join(".claude/skills/hide-browser").exists());
}

#[test]
fn an_agents_own_folder_is_not_created_for_its_skill() {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(fixture.home().join(".claude")).unwrap();
    executable(&fixture.home().join(".local/bin/claude"), "#!/bin/sh\n");

    apply(&fixture.target, &Scope::automatic());

    assert!(!fixture.home().join(".claude").exists());
}

#[test]
fn claude_code_and_codex_are_on_without_a_choice_and_off_removes_the_hook_part() {
    let fixture = Fixture::new();

    let report = apply(&fixture.target, &Scope::automatic());

    let claude = agent(&report, "claude-code");
    assert!(claude.enabled);
    assert_eq!(
        claude.hook.as_ref().unwrap().state,
        ComponentState::Installed
    );
    assert_eq!(claude.skill.state, ComponentState::Installed);
    assert!(
        fixture
            .home()
            .join(".claude/skills")
            .join(BROWSER_SKILL.name)
            .join("SKILL.md")
            .is_file()
    );

    let report = apply(&fixture.target, &Scope::agents([], ["claude-code"]));

    let claude = agent(&report, "claude-code");
    assert!(!claude.enabled);
    assert_eq!(claude.hook.as_ref().unwrap().state, ComponentState::Off);
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Off
    );
    assert!(!fixture.settings().contains("hide-subagents@"));
    assert!(fixture.settings().contains("/opt/other/notify.sh start"));
    assert!(
        !fixture
            .home()
            .join(".claude/skills")
            .join(BROWSER_SKILL.name)
            .exists()
    );

    apply(&fixture.target, &Scope::automatic());
    assert!(!fixture.settings().contains("hide-subagents@"));

    let report = apply(&fixture.target, &Scope::agents(["claude-code"], []));
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
    assert!(fixture.settings().contains("hide-subagents@"));
}

#[test]
fn removing_a_machine_takes_every_marked_piece_and_nothing_else() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));

    let removed = remove(&fixture.target);

    assert!(removed.agents.iter().any(|(code, _)| code == "hook:grok"));
    assert!(!shared_skill(&fixture).exists());
    assert!(
        !std::fs::read_to_string(grok_hooks(&fixture))
            .map(|text| text.contains("hide-guidance"))
            .unwrap_or(false)
    );
}

#[test]
fn the_shared_stub_goes_when_the_last_agent_reading_it_is_switched_off_even_with_codex_not_installed()
 {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    assert!(shared_skill(&fixture).is_file());

    // Codex is on by default but not installed here, so it reads nothing.
    apply(&fixture.target, &Scope::agents([], ["grok"]));

    assert!(!shared_skill(&fixture).exists());
}

#[test]
fn a_switch_off_that_left_hides_stub_behind_does_not_read_as_off() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    // The choice is off and Hide's stub is still there, as a removal that
    // failed would leave it.
    let mut text = record(&fixture);
    text["agents"]["grok"] = serde_json::json!(false);
    std::fs::write(
        fixture.home().join(".hide/kit/installed.json"),
        text.to_string(),
    )
    .unwrap();

    let report = status(&fixture.target);

    let grok = agent(&report, "grok");
    assert_eq!(grok.skill.state, ComponentState::Failed, "{grok:?}");
    assert!(
        grok.skill
            .reason
            .as_deref()
            .unwrap()
            .contains("switch the agent on and off again")
    );
}

#[test]
fn an_agent_found_only_by_its_program_reports_its_folder_as_not_made_yet() {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(fixture.home().join(".claude")).unwrap();
    executable(&fixture.home().join(".local/bin/claude"), "#!/bin/sh\n");

    let report = apply(&fixture.target, &Scope::automatic());

    let claude = agent(&report, "claude-code");
    assert_eq!(claude.skill.state, ComponentState::Absent);
    assert!(!claude.needs_attention());
}

#[test]
fn a_file_that_only_mentions_the_marker_is_not_hides_stub() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    let own = shared_skill(&fixture);
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    // The marker's words sit in the body, not in the line Hide writes.
    let mine = "---\nname: hide-browser\n---\nsee hide-skill@1 in the docs\n";
    std::fs::write(&own, mine).unwrap();

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));
    assert_eq!(agent(&report, "grok").skill.state, ComponentState::Absent);
    apply(&fixture.target, &Scope::agents([], ["grok", "codex"]));

    assert_eq!(std::fs::read_to_string(&own).unwrap(), mine);
}

#[test]
fn an_edited_stub_is_reported_and_only_reinstall_puts_hides_text_back() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    let path = shared_skill(&fixture);
    let hide_text = std::fs::read_to_string(&path).unwrap();
    let edited = format!("{hide_text}\nMy own note for this agent.\n");
    std::fs::write(&path, &edited).unwrap();

    // No automatic pass overwrites the operator's change.
    let report = apply(&fixture.target, &Scope::automatic());
    let grok = agent(&report, "grok");
    assert_eq!(grok.skill.state, ComponentState::Outdated);
    assert!(grok.skill.reason.as_deref().unwrap().contains("edited"));
    assert!(grok.needs_attention());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

    // Switching the agent off keeps what they wrote as well.
    let off = apply(&fixture.target, &Scope::agents([], ["grok", "codex"]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    assert_eq!(
        agent(&off, "grok").skill.state,
        ComponentState::Off,
        "an edited stub is the operator's file, so the switch is Off"
    );

    // Reinstall (switching on again) restores Hide's text.
    apply(&fixture.target, &Scope::agents(["grok"], []));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), hide_text);
}

#[test]
fn a_folder_the_agent_makes_is_not_the_agent_being_installed() {
    // Each agent's folder alone: an editor makes `~/.grok` without the
    // `grok` CLI, and Pi's installer keeps `~/.pi/agent` around.
    for (program, id, folder) in [
        ("pi", "pi", ".pi/agent"),
        ("grok", "grok", ".grok"),
        ("opencode", "opencode", ".config/opencode"),
        ("omp", "omp", ".omp/agent"),
    ] {
        let fixture = Fixture::new();
        set_up(&fixture, folder);
        let report = apply(&fixture.target, &Scope::agents([id], []));
        let row = agent(&report, id);
        assert_eq!(row.availability, Availability::NotInstalled, "{id}");
        assert!(record(&fixture).get("agents").is_none(), "{id}");

        // Its program is what makes it installed.
        executable(
            &fixture.home().join(".local/bin").join(program),
            "#!/bin/sh\n",
        );
        assert_eq!(
            agent(&status(&fixture.target), id).availability,
            Availability::Available,
            "{id}"
        );
    }
}

#[test]
fn an_agent_whose_program_is_gone_keeps_what_hide_put_down_until_it_is_switched_off() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    apply(&fixture.target, &Scope::agents(["grok"], []));
    assert!(shared_skill(&fixture).is_file());
    std::fs::remove_file(fixture.home().join(".local/bin/grok")).unwrap();
    let tree = home_tree(fixture.home());

    let report = apply(&fixture.target, &Scope::automatic());

    // Nothing is taken out or written: the operator's choice and Hide's
    // pieces stay as they were, so the agent is whole again once its
    // program is back.
    assert_eq!(home_tree(fixture.home()), tree);
    assert_eq!(record(&fixture)["agents"]["grok"], true);
    let grok = agent(&report, "grok");
    assert_eq!(grok.availability, Availability::NotInstalled);
    assert!(grok.enabled, "the switch stays so it can be turned off");
    assert!(
        grok.chosen,
        "the record holds the operator's choice, which is what keeps the row's switch"
    );
    assert!(!grok.needs_attention(), "nothing for Reinstall to do");
    for piece in [&grok.skill, grok.hook.as_ref().unwrap()] {
        assert_eq!(piece.state, ComponentState::Absent, "{grok:?}");
        assert!(
            piece
                .reason
                .as_deref()
                .unwrap()
                .contains("`grok` is not found"),
            "{piece:?}"
        );
    }

    // The switch still takes Hide's pieces out.
    let report = apply(&fixture.target, &Scope::agents([], ["grok"]));

    assert!(!agent(&report, "grok").enabled);
    assert!(!shared_skill(&fixture).exists());
    assert!(
        !std::fs::read_to_string(grok_hooks(&fixture))
            .map(|text| text.contains("hide-guidance"))
            .unwrap_or(false)
    );
}

#[test]
fn an_agent_on_only_by_default_with_no_program_reports_no_operator_choice() {
    let fixture = Fixture::new();
    // A machine that already has a record, so the first-run hold does not
    // switch the default-on agents off.
    apply(&fixture.target, &Scope::agents([], ["grok"]));

    let report = status(&fixture.target);

    let codex = agent(&report, "codex");
    assert_eq!(codex.availability, Availability::NotInstalled);
    assert!(codex.enabled, "Codex is on by default");
    assert!(!codex.chosen, "{codex:?}");
    // Switched off by the operator, the choice is on record whatever it is.
    assert!(agent(&report, "grok").chosen);
}

#[test]
fn a_report_from_a_build_that_predates_chosen_reads_as_no_choice() {
    let fixture = Fixture::new();
    let report = status(&fixture.target);
    let mut wire = serde_json::to_value(agent(&report, "codex")).unwrap();
    wire.as_object_mut().unwrap().remove("chosen");

    let parsed: AgentReport = serde_json::from_value(wire).unwrap();

    assert!(!parsed.chosen);
}

/// A stand-in login shell that puts `~/.grok/bin` on its `PATH`, as Grok's
/// installer does in `~/.zshrc`, and counts how often it is asked. Its `PATH`
/// is that folder alone, so this process's own `PATH` stays out of the
/// search as in every other test.
fn grok_login_shell(fixture: &Fixture) -> PathBuf {
    let shell = fixture.root.join("bin/login-shell");
    program(
        &shell,
        "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 64\necho x >> \"${HOME%/*}/shell-asked\"\necho 'Last login: today'\nPATH=\"$HOME/.grok/bin\"\nexport PATH\neval \"$2\"\n",
    );
    shell
}

#[test]
fn a_program_on_the_login_shells_path_is_installed_and_the_shell_is_asked_again_only_after_its_files_change()
 {
    let mut fixture = Fixture::new();
    executable(&fixture.home().join(".grok/bin/grok"), "#!/bin/sh\n");
    assert_eq!(
        agent(&status(&fixture.target), "grok").availability,
        Availability::NotInstalled,
        "no other folder this test searches holds it"
    );
    fixture.target.login_shell = Some(grok_login_shell(&fixture));
    let asked = || {
        std::fs::read_to_string(fixture.root.join("shell-asked"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    };

    for _ in 0..3 {
        assert_eq!(
            agent(&status(&fixture.target), "grok").availability,
            Availability::Available
        );
    }
    assert_eq!(asked(), 1, "Settings re-reads the kit every few seconds");

    // An installer that adds its folder edits a startup file.
    std::fs::write(
        fixture.home().join(".zshrc"),
        "export PATH=\"$HOME/.kilo/bin:$PATH\"\n",
    )
    .unwrap();
    status(&fixture.target);
    assert_eq!(asked(), 2);
}

#[test]
fn a_machine_the_kit_has_never_touched_waits_for_the_first_choice_before_any_agent_gets_anything() {
    let fixture = Fixture::fresh();
    install(&fixture, "codex", ".codex");
    install(&fixture, "agy", ".gemini/config");

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(report.held_for_onboarding);
    for id in ["claude-code", "codex", "antigravity"] {
        let held = agent(&report, id);
        assert!(!held.enabled, "{held:?}");
        assert_eq!(held.skill.state, ComponentState::Off);
    }
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Off
    );
    assert!(!fixture.settings().contains("hide-subagents@"));
    assert!(
        !fixture
            .home()
            .join(".claude/skills")
            .join(BROWSER_SKILL.name)
            .exists()
    );
    assert!(!shared_skill(&fixture).exists());
    assert!(!agy_hooks(&fixture).exists());
    // The CLI link and the retirement stage are not agents and are applied.
    assert_eq!(state(&report, ComponentId::Cli), ComponentState::Installed);

    // The next pass is not the first, and still waits: the record says so,
    // so a launch after a quit between the hold and the answer asks again.
    let again = apply(&fixture.target, &Scope::automatic());
    assert!(again.held_for_onboarding);
    assert!(!agent(&again, "claude-code").enabled);
    assert!(crate::status(&fixture.target).held_for_onboarding);

    // The operator's choice puts back what they chose and nothing else.
    let chosen = apply(&fixture.target, &Scope::agents(["claude-code"], []));
    assert!(!chosen.held_for_onboarding);
    assert!(agent(&chosen, "claude-code").enabled);
    assert_eq!(
        state(&chosen, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
    assert!(!agent(&chosen, "codex").enabled);
    assert!(!shared_skill(&fixture).exists());
    assert!(!crate::status(&fixture.target).held_for_onboarding);
}

#[test]
fn answering_with_nothing_chosen_is_still_an_answer() {
    let fixture = Fixture::fresh();
    install(&fixture, "codex", ".codex");
    assert!(apply(&fixture.target, &Scope::automatic()).held_for_onboarding);

    let answered = apply(&fixture.target, &Scope::first_run([]));

    assert!(!answered.held_for_onboarding);
    assert!(!agent(&answered, "claude-code").enabled);
    assert!(!agent(&answered, "codex").enabled);
    // Later passes and a status read agree, and nothing is asked again.
    assert!(!apply(&fixture.target, &Scope::automatic()).held_for_onboarding);
    assert!(!crate::status(&fixture.target).held_for_onboarding);
}

#[test]
fn the_choice_made_in_the_first_pass_wins_over_the_hold() {
    let fixture = Fixture::fresh();

    let report = apply(&fixture.target, &Scope::agents(["claude-code"], []));

    // The choice is an answer too: the machine does not ask afterwards.
    assert!(!report.held_for_onboarding);
    assert!(agent(&report, "claude-code").enabled);
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
}

#[test]
fn a_machine_with_a_record_keeps_what_it_had_when_this_build_arrives() {
    let fixture = Fixture::new();

    let report = apply(&fixture.target, &Scope::automatic());

    assert!(!report.held_for_onboarding);
    assert!(agent(&report, "claude-code").enabled);
    assert_eq!(
        state(&report, ComponentId::ClaudeCodeHook),
        ComponentState::Installed
    );
}

fn agy_hooks(fixture: &Fixture) -> std::path::PathBuf {
    fixture.home().join(".gemini/config/hooks.json")
}

#[test]
fn agy_is_on_by_default_where_its_program_is_found_and_gets_nothing_where_it_is_not() {
    let fixture = Fixture::new();
    // Gemini CLI's folder alone is not agy.
    set_up(&fixture, ".gemini");

    let report = apply(&fixture.target, &Scope::automatic());

    let agy = agent(&report, "antigravity");
    assert!(agy.enabled, "{agy:?}");
    assert_eq!(agy.availability, Availability::NotInstalled);
    assert!(!agy_hooks(&fixture).exists());

    install(&fixture, "agy", ".gemini/config");
    let report = apply(&fixture.target, &Scope::automatic());

    let hook = agent(&report, "antigravity").hook.as_ref().unwrap();
    assert_eq!(hook.state, ComponentState::Installed, "{hook:?}");
    let block: Value =
        serde_json::from_str(&std::fs::read_to_string(agy_hooks(&fixture)).unwrap()).unwrap();
    assert!(block["hide"].is_object(), "{block}");
    // On by default, not by the operator's choice.
    assert!(!agent(&report, "antigravity").chosen);
}

#[test]
fn grok_gets_the_guidance_hook_with_the_switch_and_loses_it_with_it() {
    let file = ".grok/hooks/hide.json";
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));

    let hook = agent(&report, "grok").hook.as_ref().unwrap();
    assert_eq!(hook.state, ComponentState::Installed, "{hook:?}");
    let written = std::fs::read_to_string(fixture.home().join(file)).unwrap();
    assert!(written.contains(&format!(
        "hide-guidance@{}",
        hide_agent_hooks::guidance::GuidanceAgent::Grok.version()
    )));

    let report = apply(&fixture.target, &Scope::agents([], ["grok"]));

    assert_eq!(
        agent(&report, "grok").hook.as_ref().unwrap().state,
        ComponentState::Off
    );
    // Hide created the file and nothing else was in it, so the file goes.
    assert!(!fixture.home().join(file).exists());
}

#[test]
fn claude_code_and_codex_do_everything_opencode_pi_omp_and_antigravity_collaborate_and_grok_is_partial()
 {
    use crate::agents::Feature::{self, *};
    // The expected rows come from the PRDs and the hook research, not from
    // the table: what Hide does for each agent in this build (D-10, B18;
    // opencode-plugin D-11: OpenCode takes letters and is refused a launch,
    // so it is no longer Basic, while no bell rings for it; grok-grok-hooks:
    // Grok and Grok get the spawn guard from their official hooks, and Grok
    // the subagent count (Grok's hooks never fire for a Task subagent, issue
    // 911), take no letters and stay Basic, and Grok discards a
    // session-start hook's output). The session-reader common contract B1/B6
    // enables starts for the five agents besides Claude Code and Codex
    // without enabling their future reader/sleep/fork features. The complete
    // Pi and OMP slices add native title and exact sleep/fork beside the
    // accepted extension's collaboration features (reader B1/B2/B3/B14, D13);
    // the Grok slice adds the same to its hook features and Grok stays Basic;
    // the OpenCode slice does the same beside its plugin.
    let opencode = [
        Skill,
        Guidance,
        Letters,
        Memory,
        SpawnGuard,
        HerdrIntegration,
        Sleep,
        Fork,
        Start,
        Titles,
    ];
    // pi-omp-extension D-08, D-09, D-10: Pi and omp take letters and Memory
    // and are refused a launch through Hide's extension; only omp runs
    // subagents, and no bell rings for either. Their session readers add sleep, fork and titles.
    let pi = [
        Skill,
        Guidance,
        Letters,
        Memory,
        SpawnGuard,
        HerdrIntegration,
        Sleep,
        Fork,
        Start,
        Titles,
    ];
    let omp = [
        Skill,
        Guidance,
        Letters,
        Memory,
        SpawnGuard,
        HerdrIntegration,
        Sleep,
        Fork,
        Start,
        Titles,
    ];
    let expected: [(&str, &[Feature], bool); 7] = [
        ("claude-code", &Feature::ALL, false),
        ("codex", &Feature::ALL, false),
        (
            "grok",
            &[
                Skill,
                Bell,
                SpawnGuard,
                HerdrIntegration,
                Sleep,
                Fork,
                Start,
                Titles,
            ],
            true,
        ),
        ("opencode", &opencode, false),
        ("pi", &pi, false),
        ("omp", &omp, false),
        ("antigravity", &omp, false),
    ];
    assert_eq!(
        ADAPTERS.iter().map(|row| row.id).collect::<Vec<_>>(),
        expected.map(|(id, _, _)| id)
    );
    for (id, supported, partial) in expected {
        let row = crate::agents::adapter(id).unwrap();
        for feature in Feature::ALL {
            assert_eq!(
                row.supports(feature),
                supported.contains(&feature),
                "{id}: {feature:?}"
            );
        }
        assert_eq!(row.partial(), partial, "{id}");
    }
}

#[test]
fn the_features_the_hook_gives_are_the_ones_a_hook_runtime_exists_for() {
    // Letters and Memory come from the six-event hook, a script file of
    // Hide's (OpenCode's plugin, Pi's and omp's extension) or agy's hook
    // block, which answers its prompt event,
    // which the hook crate speaks a dialect for; the subagent count and the
    // spawn guard also come from Grok's and Grok's own hook files. An agent
    // claiming them without a hook would be a popover that says more than
    // the kit installs.
    let with_runtime: Vec<&str> = ADAPTERS
        .iter()
        .filter(|row| row.supports(crate::agents::Feature::Letters))
        .map(|row| row.id)
        .collect();
    assert_eq!(
        with_runtime.len(),
        hide_agent_adapter::HookDialect::ALL
            .iter()
            .filter(|dialect| ADAPTERS.iter().any(|row| {
                hide_agent_adapter::adapter(row.id).is_some_and(|adapter| {
                    adapter
                        .prompt_hook
                        .is_some_and(|hook| hook.context && hook.dialect == **dialect)
                })
            }))
            .count()
    );
    for row in ADAPTERS {
        let declared = hide_agent_adapter::adapter(row.id).unwrap();
        // agy's hook entry answers its prompt event, so it carries letters.
        let instrumented = matches!(row.hook, HookSupport::Part(_) | HookSupport::Plugin(_))
            || matches!(row.hook, HookSupport::Guidance(agent)
                if agent.dialect().is_some() && agent.dialect() == declared.prompt_hook.filter(|hook| hook.context).map(|hook| hook.dialect));
        assert_eq!(
            row.supports(crate::agents::Feature::Letters),
            instrumented,
            "{}",
            row.id
        );
        // Memory needs the hook too, and its own declaration.
        assert_eq!(
            row.supports(crate::agents::Feature::Memory),
            instrumented && declared.memory.is_some(),
            "{}: memory",
            row.id
        );
        // The spawn guard needs any hook of Hide's, including Grok's and
        // Grok's own hook files, and its own declaration.
        let hook = !matches!(row.hook, HookSupport::None);
        assert_eq!(
            row.supports(crate::agents::Feature::SpawnGuard),
            hook && declared.spawn_guard.is_some(),
            "{}: spawn guard",
            row.id
        );
    }
}

fn grok_hooks(fixture: &Fixture) -> PathBuf {
    fixture.home().join(".grok/hooks/hide.json")
}

#[test]
fn grok_gets_its_own_hook_file_beside_the_others_and_only_inside_its_own_folder() {
    let fixture = Fixture::new();
    install(&fixture, "grok", ".grok");
    // Herdr's and Orca's files are already there and stay byte for byte.
    let herdr = br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/herdr.sh"}]}]}}"#;
    set_up(&fixture, ".grok/hooks");
    std::fs::write(fixture.home().join(".grok/hooks/herdr.json"), herdr).unwrap();

    let report = apply(&fixture.target, &Scope::agents(["grok"], []));
    let grok = agent(&report, "grok");
    assert_eq!(
        grok.hook.as_ref().unwrap().state,
        ComponentState::Installed,
        "{grok:?}"
    );
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(grok_hooks(&fixture)).unwrap()).unwrap();
    for event in [
        "SessionStart",
        "PreToolUse",
        "SubagentStart",
        "SubagentStop",
        "Stop",
    ] {
        let command = written["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{event}: {written}"));
        assert!(command.contains("--runtime grok"), "{command}");
        assert!(command.contains(&format!("--event {event}")), "{command}");
    }
    assert!(written["hooks"]["PreToolUse"][0].get("matcher").is_none());
    assert_eq!(
        std::fs::read(fixture.home().join(".grok/hooks/herdr.json")).unwrap(),
        herdr
    );

    // A generated entry for a previous daemon state is safe to refresh;
    // the operator edit below must still be left alone by an automatic pass.
    let helper = fixture.target.kit_dir.join("hide-agent-hooks");
    let previous = fixture.home().join("previous-daemon-state");
    hide_agent_hooks::guidance::install(
        hide_agent_hooks::guidance::GuidanceAgent::Grok,
        fixture.home(),
        hide_agent_hooks::route::HookTarget::new(&helper, &previous).unwrap(),
    )
    .unwrap();
    let old = std::fs::read_to_string(grok_hooks(&fixture)).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    assert_eq!(
        agent(&report, "grok").hook.as_ref().unwrap().state,
        ComponentState::Installed
    );
    assert_ne!(std::fs::read_to_string(grok_hooks(&fixture)).unwrap(), old);

    // An entry the operator edited reads Outdated and is theirs until
    // Reinstall, which puts Hide's back (B8).
    let edited = std::fs::read_to_string(grok_hooks(&fixture))
        .unwrap()
        .replace("--event Stop", "--event Stop --verbose");
    std::fs::write(grok_hooks(&fixture), &edited).unwrap();
    let report = apply(&fixture.target, &Scope::automatic());
    let hook = agent(&report, "grok").hook.clone().unwrap();
    assert_eq!(hook.state, ComponentState::Outdated, "{hook:?}");
    assert!(hook.reason.unwrap().contains("edited"));
    assert_eq!(
        std::fs::read_to_string(grok_hooks(&fixture)).unwrap(),
        edited
    );
    let report = apply(&fixture.target, &Scope::agents(["grok"], []));
    assert_eq!(
        agent(&report, "grok").hook.as_ref().unwrap().state,
        ComponentState::Installed
    );
    assert!(
        !std::fs::read_to_string(grok_hooks(&fixture))
            .unwrap()
            .contains("--verbose")
    );

    // Switched off, Hide's own file goes and Herdr's stays.
    let report = apply(&fixture.target, &Scope::agents([], ["grok"]));
    assert_eq!(
        agent(&report, "grok").hook.as_ref().unwrap().state,
        ComponentState::Off
    );
    assert!(!grok_hooks(&fixture).exists());
    assert_eq!(
        std::fs::read(fixture.home().join(".grok/hooks/herdr.json")).unwrap(),
        herdr
    );
}

#[test]
fn grok_without_its_own_folder_gets_no_folder_and_its_row_says_why() {
    let fixture = Fixture::new();
    executable(&fixture.home().join(".local/bin/grok"), "#!/bin/sh\n");
    let report = apply(&fixture.target, &Scope::agents(["grok"], []));
    let hook = agent(&report, "grok").hook.clone().unwrap();
    assert_eq!(hook.state, ComponentState::Absent, "{hook:?}");
    assert!(hook.reason.unwrap().contains(".grok"));
    assert!(!fixture.home().join(".grok").exists());
}

fn opencode_plugin(fixture: &Fixture) -> PathBuf {
    hide_agent_hooks::opencode::PLUGIN.path(fixture.home())
}

#[test]
fn opencode_switched_on_gets_hides_plugin_beside_herdrs_and_reads_installed() {
    let fixture = Fixture::new();
    install(&fixture, "opencode", ".config/opencode");
    let herdr = fixture
        .home()
        .join(".config/opencode/plugins/herdr-agent-state.js");
    std::fs::create_dir_all(herdr.parent().unwrap()).unwrap();
    std::fs::write(&herdr, b"herdr's plugin").unwrap();

    let report = apply(&fixture.target, &Scope::agents(["opencode"], []));

    let opencode = agent(&report, "opencode");
    let hook = opencode.hook.as_ref().unwrap();
    assert_eq!(hook.state, ComponentState::Installed, "{opencode:?}");
    assert_eq!(hook.location.as_deref(), opencode_plugin(&fixture).to_str());
    assert_eq!(
        std::fs::read_to_string(opencode_plugin(&fixture)).unwrap(),
        hide_agent_hooks::opencode::V1.text(
            hide_agent_hooks::route::HookTarget::new(
                &fixture.target.kit_dir.join("hide-agent-hooks"),
                &crate::layout::state_dir_from_process(fixture.home()),
            )
            .unwrap()
        )
    );
    assert_eq!(std::fs::read(&herdr).unwrap(), b"herdr's plugin");
    assert!(
        record(&fixture)["installed"]
            .as_array()
            .unwrap()
            .contains(&json!("hook:opencode"))
    );
}

#[test]
fn an_edited_opencode_plugin_reads_outdated_and_stays_until_reinstall() {
    let fixture = Fixture::new();
    install(&fixture, "opencode", ".config/opencode");
    apply(&fixture.target, &Scope::agents(["opencode"], []));
    let edited = std::fs::read_to_string(opencode_plugin(&fixture))
        .unwrap()
        .replace("RUNNING_LIMIT = 8", "RUNNING_LIMIT = 3");
    std::fs::write(opencode_plugin(&fixture), &edited).unwrap();

    let report = apply(&fixture.target, &Scope::automatic());

    let hook = agent(&report, "opencode").hook.clone().unwrap();
    assert_eq!(hook.state, ComponentState::Outdated);
    assert!(hook.reason.unwrap().contains("edited"));
    assert_eq!(
        std::fs::read_to_string(opencode_plugin(&fixture)).unwrap(),
        edited
    );

    // Reinstall is the agent's switch made again.
    let report = apply(&fixture.target, &Scope::agents(["opencode"], []));
    assert_eq!(
        agent(&report, "opencode").hook.as_ref().unwrap().state,
        ComponentState::Installed
    );
}

#[test]
fn switching_opencode_off_removes_only_an_unedited_plugin() {
    let fixture = Fixture::new();
    install(&fixture, "opencode", ".config/opencode");
    apply(&fixture.target, &Scope::agents(["opencode"], []));

    let report = apply(&fixture.target, &Scope::agents([], ["opencode"]));

    assert_eq!(
        agent(&report, "opencode").hook.as_ref().unwrap().state,
        ComponentState::Off
    );
    assert!(!opencode_plugin(&fixture).exists());
    assert!(
        !record(&fixture)["installed"]
            .as_array()
            .unwrap()
            .contains(&json!("hook:opencode"))
    );

    // An edited plugin is the operator's: switching off leaves it.
    apply(&fixture.target, &Scope::agents(["opencode"], []));
    let edited = std::fs::read_to_string(opencode_plugin(&fixture))
        .unwrap()
        .replace("RUNNING_LIMIT = 8", "RUNNING_LIMIT = 3");
    std::fs::write(opencode_plugin(&fixture), &edited).unwrap();
    apply(&fixture.target, &Scope::agents([], ["opencode"]));
    assert_eq!(
        std::fs::read_to_string(opencode_plugin(&fixture)).unwrap(),
        edited
    );
}

#[test]
fn opencode_without_its_config_folder_gets_no_folder_made_and_says_why() {
    let fixture = Fixture::new();
    executable(&fixture.home().join(".local/bin/opencode"), "#!/bin/sh\n");

    let report = apply(&fixture.target, &Scope::agents(["opencode"], []));

    let hook = agent(&report, "opencode").hook.clone().unwrap();
    assert_eq!(hook.state, ComponentState::Absent);
    assert!(hook.reason.unwrap().contains("has not created"));
    assert!(!fixture.home().join(".config/opencode").exists());
}

#[test]
fn a_plugin_named_like_hides_that_hide_did_not_write_is_left_alone() {
    let fixture = Fixture::new();
    install(&fixture, "opencode", ".config/opencode");
    std::fs::create_dir_all(opencode_plugin(&fixture).parent().unwrap()).unwrap();
    std::fs::write(
        opencode_plugin(&fixture),
        "export const Mine = async () => ({})\n",
    )
    .unwrap();

    let report = apply(&fixture.target, &Scope::agents(["opencode"], []));

    let hook = agent(&report, "opencode").hook.clone().unwrap();
    assert_eq!(hook.state, ComponentState::Absent);
    assert!(hook.reason.unwrap().contains("did not write"));
    assert_eq!(
        std::fs::read_to_string(opencode_plugin(&fixture)).unwrap(),
        "export const Mine = async () => ({})\n"
    );
    apply(&fixture.target, &Scope::agents([], ["opencode"]));
    assert!(opencode_plugin(&fixture).exists());
}
