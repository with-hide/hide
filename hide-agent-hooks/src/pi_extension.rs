//! Hide's extension for Pi and omp: one source (`pi_extension/extension.js`),
//! written as `~/.pi/agent/extensions/hide.ts` and
//! `~/.omp/agent/extensions/hide.ts`, the two differing only in the agent's
//! name (PRD pi-omp-extension D-01). omp is a fork of Pi with the same
//! extension shape; what only one host has (omp's subagents and `ask`) is
//! used where the host reports it.
//!
//! The file is plain JavaScript that is also valid TypeScript, in a `.ts`
//! file as Herdr's own extension for these agents is, so both hosts load it
//! from their user extension folder without a build step. Each prompt and
//! shell call asks the helper (`hide-agent-hooks pi|omp <operation>`,
//! `src/plugin/helper.rs`), and it tells the helper its version, so a file
//! another build wrote does nothing (B13).
//!
//! It goes only into the default agent folders, the ones Herdr's integration
//! uses; an omp profile or a `PI_CODING_AGENT_DIR` folder gets none (PRD
//! non-goal).

use crate::plugin::{Plugin, PluginFile};

/// The extension's version: the marker's, and the one the extension tells the
/// helper. Raise both when the text or the helper protocol changes.
pub const VERSION: u32 = 9;
const VERSION_TEXT: &str = "9";

/// The marker's name, one for both files: they differ only in the agent's name.
pub const SOURCE_NAME: &str = "hide-extension";

const TEMPLATE: &str = include_str!("pi_extension/extension.js");

pub const PI: PluginFile = PluginFile {
    agent: "Pi",
    noun: "extension",
    source_name: SOURCE_NAME,
    version: VERSION,
    config: &[".pi", "agent"],
    folder: "extensions",
    own_folder: None,
    file_name: "hide.ts",
    template: TEMPLATE,
    constants: &[
        ("__HIDE_AGENT__", "\"pi\""),
        ("__HIDE_VERSION__", VERSION_TEXT),
    ],
};

pub const OMP: PluginFile = PluginFile {
    agent: "omp",
    noun: "extension",
    source_name: SOURCE_NAME,
    version: VERSION,
    config: &[".omp", "agent"],
    folder: "extensions",
    own_folder: None,
    file_name: "hide.ts",
    template: TEMPLATE,
    constants: &[
        ("__HIDE_AGENT__", "\"omp\""),
        ("__HIDE_VERSION__", VERSION_TEXT),
    ],
};

pub const PI_PLUGIN: Plugin = Plugin { files: &[PI] };

pub const OMP_PLUGIN: Plugin = Plugin { files: &[OMP] };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_the_extension_tells_the_helper_is_its_markers() {
        assert_eq!(VERSION_TEXT.parse::<u32>(), Ok(VERSION));
        for file in [&PI, &OMP] {
            let text = file.text(std::path::Path::new("/kit/hide-agent-hooks"));
            assert!(text.starts_with(&format!("// {}@{VERSION} ", file.source_name)));
            assert!(text.contains(&format!("const VERSION = {VERSION};")));
        }
    }
}
