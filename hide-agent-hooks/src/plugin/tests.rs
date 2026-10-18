//! Each script file's states, its bytes and its writes, and each plugin's as
//! the set of its files (PRD opencode-plugin B1, B14, B15; PRD
//! pi-omp-extension B1, B12, B13). Every test builds its own `HOME`; the
//! operator's real agent folders are never a target.

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::opencode;

const PLUGINS: [&Plugin; 3] = [
    &opencode::PLUGIN,
    &crate::pi_extension::PI_PLUGIN,
    &crate::pi_extension::OMP_PLUGIN,
];

#[test]
fn extension_routes_are_updated_without_treating_operator_edits_as_generated() {
    for plugin in PLUGINS {
        let fixture = Fixture::new(plugin);
        let state = fixture.home().join("daemon ' $경로");
        let target = HookTarget::new(&fixture.helper, &state).unwrap();
        plugin.install(fixture.home(), target).unwrap();
        assert_eq!(
            plugin.observe(fixture.home(), target),
            PluginObserved::Current
        );
        assert!(matches!(
            plugin.observe(fixture.home(), &fixture.helper),
            PluginObserved::Stale(_)
        ));
        let path = plugin.path(fixture.home());
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("--state-dir-b64"));
        fs::write(path, format!("{text}\n// operator edit\n")).unwrap();
        assert_eq!(
            plugin.observe(fixture.home(), target),
            PluginObserved::Edited
        );
    }
}

fn files() -> impl Iterator<Item = &'static PluginFile> {
    PLUGINS.into_iter().flat_map(|plugin| plugin.files)
}

/// Whether the file names the helper itself; OpenCode 2's terminal half takes
/// it from the server half.
fn names_helper(file: &PluginFile) -> bool {
    file.template.contains(HELPER_PLACEHOLDER)
}

struct Fixture {
    plugin: &'static Plugin,
    home: tempfile::TempDir,
    helper: PathBuf,
}

impl Fixture {
    /// A home where the agent has made its own folder, and a helper.
    fn new(plugin: &'static Plugin) -> Self {
        let fixture = Self::without_agent(plugin);
        fs::create_dir_all(plugin.config_directory(fixture.home())).unwrap();
        fixture
    }

    fn without_agent(plugin: &'static Plugin) -> Self {
        let home = tempfile::tempdir().unwrap();
        let helper = home
            .path()
            .join("hide.app/Contents/Resources/hide-agent-hooks");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(&helper, b"binary").unwrap();
        Self {
            plugin,
            home,
            helper,
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn observe(&self) -> PluginObserved {
        self.plugin.observe(self.home(), &self.helper)
    }

    fn install(&self) -> Result<bool, String> {
        self.plugin.install(self.home(), &self.helper)
    }

    fn remove(&self) -> Result<bool, String> {
        self.plugin.remove(self.home())
    }

    /// The folder the agent loads scripts from.
    fn folder(&self) -> PathBuf {
        let entry = self.plugin.entry();
        entry.config_directory(self.home()).join(entry.folder)
    }

    /// The file Settings names.
    fn read(&self) -> String {
        fs::read_to_string(self.plugin.path(self.home())).unwrap()
    }

    /// Whether every file of the plugin is this build's.
    fn all_current(&self) -> bool {
        self.plugin.files.iter().all(|file| {
            fs::read_to_string(file.path(self.home())).ok() == Some(file.text(&self.helper))
        })
    }
}

#[test]
fn each_file_is_its_template_with_the_helper_and_a_marker_that_hashes_the_rest() {
    for file in files() {
        let text = file.text(Path::new("/kit path/it's \"here\"/hide-agent-hooks"));
        let (first, body) = text.split_once('\n').unwrap();
        assert_eq!(
            first,
            format!(
                "// {}@{} sha256={}",
                file.source_name,
                file.version,
                digest(body)
            )
        );
        assert!(!body.contains(HELPER_PLACEHOLDER), "{}", file.name());
        if names_helper(file) {
            // A JSON string literal: quotes and the apostrophe survive as JavaScript.
            assert!(body.contains(r#"const HELPER = "/kit path/it's \"here\"/hide-agent-hooks";"#));
            assert_eq!(
                helper_of(body).as_deref(),
                Some("/kit path/it's \"here\"/hide-agent-hooks")
            );
        }
        for (placeholder, _) in file.constants {
            assert!(!body.contains(placeholder), "{}: {placeholder}", file.agent);
        }
    }
}

#[test]
fn each_plugin_names_its_helper_and_each_template_names_it_and_its_constants_once_at_most() {
    for plugin in PLUGINS {
        assert!(plugin.files.iter().any(names_helper), "{}", plugin.agent());
    }
    for file in files() {
        let named = usize::from(names_helper(file));
        assert_eq!(file.template.matches(HELPER_PLACEHOLDER).count(), named);
        assert_eq!(
            file.template
                .lines()
                .filter(|line| line.starts_with(HELPER_LINE_START))
                .count(),
            named,
            "{}",
            file.name()
        );
        for (placeholder, _) in file.constants {
            assert_eq!(
                file.template.matches(placeholder).count(),
                1,
                "{placeholder}"
            );
        }
    }
}

#[test]
fn a_helper_path_that_spells_a_placeholder_keeps_it_and_the_constants_still_go_in() {
    for file in files().filter(|file| names_helper(file)) {
        let helper = Path::new("/kit/__HIDE_AGENT__/__HIDE_VERSION__/hide-agent-hooks");
        let body = file.body(helper);
        assert_eq!(helper_of(&body).as_deref(), Some(helper.to_str().unwrap()));
        for (placeholder, value) in file.constants {
            assert_eq!(
                body.matches(placeholder).count(),
                1,
                "{placeholder}: the path's own"
            );
            assert!(body.contains(&format!("= {value};")), "{placeholder}");
        }
    }
}

#[test]
fn pi_and_omp_get_one_source_that_differs_only_in_the_agents_name() {
    let pi = &crate::pi_extension::PI;
    let omp = &crate::pi_extension::OMP;
    assert_eq!(pi.template, omp.template);
    let helper = Path::new("/kit/hide-agent-hooks");
    let (pi_body, omp_body) = (pi.body(helper), omp.body(helper));
    let differing: Vec<_> = pi_body
        .lines()
        .zip(omp_body.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        differing,
        [(r#"const AGENT = "pi";"#, r#"const AGENT = "omp";"#)]
    );
}

#[test]
fn install_writes_beside_herdrs_file_and_leaves_it_byte_for_byte() {
    for plugin in PLUGINS {
        let fixture = Fixture::new(plugin);
        fs::create_dir_all(fixture.folder()).unwrap();
        let herdr = b"export default function () {}\n";
        fs::write(fixture.folder().join("herdr-agent-state.ts"), herdr).unwrap();
        assert_eq!(fixture.observe(), PluginObserved::Missing);

        assert_eq!(fixture.install(), Ok(true));

        assert_eq!(fixture.observe(), PluginObserved::Current);
        assert!(fixture.all_current(), "{}", plugin.agent());
        assert_eq!(
            fs::read(fixture.folder().join("herdr-agent-state.ts")).unwrap(),
            herdr
        );
        // Installing twice converges and writes nothing.
        assert_eq!(fixture.install(), Ok(false));
    }
}

#[test]
fn install_makes_the_extension_folder_and_hides_own_but_never_the_agents() {
    for plugin in PLUGINS {
        let missing = Fixture::without_agent(plugin);
        assert_eq!(missing.observe(), PluginObserved::ConfigAbsent);
        let refused = missing.install().unwrap_err();
        assert!(refused.contains("does not exist"), "{refused}");
        assert!(!plugin.config_directory(missing.home()).exists());

        let fixture = Fixture::new(plugin);
        assert!(!fixture.folder().exists());
        assert_eq!(fixture.install(), Ok(true));
        assert_eq!(fixture.observe(), PluginObserved::Current);
    }
    let fixture = Fixture::new(&opencode::PLUGIN);
    fixture.install().unwrap();
    assert!(fixture.folder().join("hide-opencode/server.js").is_file());
    assert!(fixture.folder().join("hide-opencode/tui.js").is_file());
}

#[test]
fn an_edit_of_any_file_reads_edited_and_only_install_puts_hides_back() {
    for file in files() {
        let plugin = PLUGINS
            .into_iter()
            .find(|plugin| plugin.files.iter().any(|own| std::ptr::eq(own, file)))
            .unwrap();
        let fixture = Fixture::new(plugin);
        fixture.install().unwrap();
        let path = file.path(fixture.home());
        let edited = format!(
            "{}\n// the operator's line\n",
            fs::read_to_string(&path).unwrap()
        );
        fs::write(&path, &edited).unwrap();

        assert_eq!(fixture.observe(), PluginObserved::Edited, "{}", file.name());
        // The kit's switch-off takes no file of an edited plugin.
        assert_eq!(fixture.remove(), Ok(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        for own in plugin.files {
            assert!(own.path(fixture.home()).is_file(), "{}", own.name());
        }
        // Reinstall is the kit calling install on an edited plugin.
        assert_eq!(fixture.install(), Ok(true));
        assert_eq!(fixture.observe(), PluginObserved::Current);
        assert!(fixture.all_current());
    }
}

#[test]
fn another_builds_plugin_or_a_gone_helper_reads_stale_with_the_reason() {
    for plugin in PLUGINS {
        let fixture = Fixture::new(plugin);
        let other = fixture.home().join("other/hide-agent-hooks");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, b"binary").unwrap();
        plugin.install(fixture.home(), &other).unwrap();
        match fixture.observe() {
            PluginObserved::Stale(reason) => {
                assert!(reason.contains("not this build's"), "{reason}");
                assert!(reason.contains(plugin.noun()), "{reason}");
            }
            observed => panic!("{observed:?}"),
        }

        fs::remove_file(&other).unwrap();
        match fixture.observe() {
            PluginObserved::Stale(reason) => assert!(reason.contains("which is gone"), "{reason}"),
            observed => panic!("{observed:?}"),
        }
        assert_eq!(fixture.install(), Ok(true));
        assert!(fixture.all_current());

        // An older version Hide wrote, unedited, is replaced without Reinstall.
        let entry = plugin.entry();
        let body = entry.body(&fixture.helper);
        fs::write(
            entry.path(fixture.home()),
            format!(
                "// {}@0 sha256={}\n{body}",
                entry.source_name,
                digest(&body)
            ),
        )
        .unwrap();
        match fixture.observe() {
            PluginObserved::Stale(reason) => assert!(reason.contains("version 0"), "{reason}"),
            observed => panic!("{observed:?}"),
        }
        assert_eq!(fixture.install(), Ok(true));
        assert_eq!(fixture.observe(), PluginObserved::Current);
    }
}

#[test]
fn opencodes_1x_file_alone_reads_stale_and_a_pass_adds_the_2x_folder_beside_it() {
    let fixture = Fixture::new(&opencode::PLUGIN);
    fs::create_dir_all(fixture.folder()).unwrap();
    // What an earlier build wrote: the 1.x file and nothing else.
    let body = opencode::V1.body(&fixture.helper);
    fs::write(
        opencode::V1.path(fixture.home()),
        format!("// hide-opencode-plugin@5 sha256={}\n{body}", digest(&body)),
    )
    .unwrap();
    match fixture.observe() {
        PluginObserved::Stale(reason) => assert!(reason.contains("version 5"), "{reason}"),
        observed => panic!("{observed:?}"),
    }
    assert_eq!(fixture.install(), Ok(true));
    assert!(fixture.all_current());
}

#[test]
fn a_file_hide_did_not_write_anywhere_in_a_plugin_holds_every_file_back() {
    for file in files() {
        let plugin = PLUGINS
            .into_iter()
            .find(|plugin| plugin.files.iter().any(|own| std::ptr::eq(own, file)))
            .unwrap();
        let fixture = Fixture::new(plugin);
        let path = file.path(fixture.home());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "export default function () {}\n";
        fs::write(&path, theirs).unwrap();

        assert_eq!(fixture.observe(), PluginObserved::Foreign(file.name()));
        assert!(fixture.install().is_err());
        assert_eq!(fixture.remove(), Ok(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
        // Nothing of Hide's went in beside it.
        for own in plugin.files.iter().filter(|own| !std::ptr::eq(*own, file)) {
            assert!(!own.path(fixture.home()).exists(), "{}", own.name());
        }
    }
}

#[test]
fn the_other_agents_file_in_an_agents_folder_reads_stale_and_is_replaced() {
    let fixture = Fixture::new(&crate::pi_extension::PI_PLUGIN);
    fs::create_dir_all(fixture.folder()).unwrap();
    // omp's file copied into Pi's folder by hand is Hide's, but not Pi's.
    fs::write(
        crate::pi_extension::PI.path(fixture.home()),
        crate::pi_extension::OMP.text(&fixture.helper),
    )
    .unwrap();
    assert!(matches!(fixture.observe(), PluginObserved::Stale(_)));
    assert_eq!(fixture.install(), Ok(true));
    assert!(fixture.read().contains(r#"const AGENT = "pi";"#));
}

#[test]
fn remove_takes_hides_unedited_files_and_its_emptied_folder_and_nothing_else() {
    for plugin in PLUGINS {
        let fixture = Fixture::new(plugin);
        fixture.install().unwrap();
        fs::write(fixture.folder().join("herdr-agent-state.ts"), b"herdr").unwrap();

        assert_eq!(fixture.remove(), Ok(true));

        assert_eq!(fixture.observe(), PluginObserved::Missing);
        assert_eq!(
            fs::read(fixture.folder().join("herdr-agent-state.ts")).unwrap(),
            b"herdr"
        );
        assert_eq!(fs::read_dir(fixture.folder()).unwrap().count(), 1);
        assert_eq!(fixture.remove(), Ok(false));
    }
    // A file of the operator's in Hide's folder keeps the folder.
    let fixture = Fixture::new(&opencode::PLUGIN);
    fixture.install().unwrap();
    let theirs = fixture.folder().join("hide-opencode/notes.md");
    fs::write(&theirs, b"mine").unwrap();
    assert_eq!(fixture.remove(), Ok(true));
    assert_eq!(fs::read(&theirs).unwrap(), b"mine");
    assert_eq!(fs::read_dir(theirs.parent().unwrap()).unwrap().count(), 1);
}
