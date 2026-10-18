//! Hide's OpenCode plugin: Hide's files in OpenCode's global configuration,
//! `~/.config/opencode/plugins/`, owned the way every script file of Hide's is
//! (`crate::plugin`).
//!
//! OpenCode 1.x loads `hide.js`, which asks the helper
//! (`hide-agent-hooks opencode <operation>`) on each root prompt, each shell
//! or `question` tool call, each file tool call while a project guardrail is
//! on, and each change of a subagent's state. OpenCode 2 loads `hide.js` as a
//! plugin that does nothing, and the `hide-opencode` folder beside it: one
//! shared server runs every session of the machine, so its half
//! (`server.js`) asks the terminal showing the session (`tui.js`), which runs
//! in that session's Herdr pane, to run the helper for it.

use crate::plugin::{Plugin, PluginFile};

/// OpenCode's global configuration folder as Herdr's integration finds it,
/// `~/.config/opencode` on every system, so Hide's plugin sits beside Herdr's.
/// OpenCode itself reads `$XDG_CONFIG_HOME/opencode` when that is set; such a
/// machine loads neither plugin from here (known limitation, shared with Herdr).
const CONFIG: &[&str] = &[".config", "opencode"];

/// The folder OpenCode 2 loads as one plugin, its halves by their names.
const FOLDER: &str = "hide-opencode";

pub const V1: PluginFile = PluginFile {
    agent: "OpenCode",
    noun: "plugin",
    source_name: "hide-opencode-plugin",
    version: 7,
    config: CONFIG,
    folder: "plugins",
    own_folder: None,
    file_name: "hide.js",
    template: include_str!("opencode/plugin.js"),
    constants: &[],
};

pub const SERVER: PluginFile = PluginFile {
    agent: "OpenCode",
    noun: "plugin",
    source_name: "hide-opencode-server",
    version: 2,
    config: CONFIG,
    folder: "plugins",
    own_folder: Some(FOLDER),
    file_name: "server.js",
    template: include_str!("opencode/server.js"),
    constants: &[],
};

/// The terminal's half, which takes the helper's path from the server's.
pub const TUI: PluginFile = PluginFile {
    agent: "OpenCode",
    noun: "plugin",
    source_name: "hide-opencode-tui",
    version: 1,
    config: CONFIG,
    folder: "plugins",
    own_folder: Some(FOLDER),
    file_name: "tui.js",
    template: include_str!("opencode/tui.js"),
    constants: &[],
};

pub const PLUGIN: Plugin = Plugin {
    files: &[V1, SERVER, TUI],
};
