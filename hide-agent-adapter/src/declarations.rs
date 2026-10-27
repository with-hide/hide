use super::*;
use composer::{Composer, Placeholder, Region, Style};
const GROK_CONTEXT_ABSENCE: CapabilityAbsence = CapabilityAbsence {
    reason: "Grok discards SessionStart and UserPromptSubmit output; the installed path cannot inject guidance or Memory.",
    evidence: "docs/agent-hooks.md#which-agents-hide-supports",
    would_change: "Prove a model-context delivery path on the real agent, implement it and add installed-host and journey fixtures.",
};

const RETIRED_CONTEXT_ABSENCE: CapabilityAbsence = CapabilityAbsence {
    reason: "Cursor is retired from active support; only historical conversation reading remains.",
    evidence: "docs/agent-hooks.md",
    would_change: "Restore active support with a pinned Herdr integration and measured context and title contracts.",
};

use usage::{
    AbsenceKind, Attribution, AttributionFrom, CostBasis, LimitDeclaration, LimitSource, ModelFrom,
    Support, TokenSource, UsageDeclaration, absent, not_built,
};

// Each agent's update routes, read from its vendor's documentation
// (docs/agent-hooks.md, Updating an agent that is too old). A command the
// vendor does not document is never written here: a package the vendor
// names for a package manager is updated with that manager's own command for
// it, and a way of installing no route covers gets the install page.

const CLAUDE_UPDATES: &[UpdateRoute] = &[
    // "the native installer manages the launcher at `~/.local/bin/claude` as
    // a symlink into `~/.local/share/claude/versions/`"; on Windows it
    // "places `claude` at `%USERPROFILE%\.local\bin\claude.exe`".
    UpdateRoute {
        method: InstallMethod::Installer(&[
            ".local/share/claude/versions",
            ".local/bin/claude.exe",
        ]),
        systems: Systems::Every,
        command: "claude update",
        source: "https://code.claude.com/docs/en/setup https://code.claude.com/docs/en/troubleshoot-install",
    },
    // "Run `brew upgrade claude-code` or `brew upgrade claude-code@latest`,
    // depending on which cask you installed".
    UpdateRoute {
        method: InstallMethod::Homebrew("claude-code"),
        systems: Systems::MacOsAndLinux,
        command: "brew upgrade claude-code",
        source: "https://code.claude.com/docs/en/setup",
    },
    UpdateRoute {
        method: InstallMethod::Homebrew("claude-code@latest"),
        systems: Systems::MacOsAndLinux,
        command: "brew upgrade claude-code@latest",
        source: "https://code.claude.com/docs/en/setup",
    },
    // "Run `winget upgrade Anthropic.ClaudeCode` periodically".
    UpdateRoute {
        method: InstallMethod::WinGet("Anthropic.ClaudeCode"),
        systems: Systems::Windows,
        command: "winget upgrade Anthropic.ClaudeCode",
        source: "https://code.claude.com/docs/en/setup",
    },
    // "To upgrade an npm installation, run `npm install -g
    // @anthropic-ai/claude-code@latest`. Avoid `npm update -g`".
    UpdateRoute {
        method: InstallMethod::Npm("@anthropic-ai/claude-code"),
        systems: Systems::Every,
        command: "npm install -g @anthropic-ai/claude-code@latest",
        source: "https://code.claude.com/docs/en/setup",
    },
];

const CODEX_UPDATES: &[UpdateRoute] = &[
    // "Update Codex" gives the standalone installer again; it links
    // `~/.local/bin/codex` into `~/.codex/packages/standalone/current`.
    // Windows has no documented update, so its standalone copy gets the page.
    UpdateRoute {
        method: InstallMethod::Installer(&[".codex/packages/standalone"]),
        systems: Systems::MacOsAndLinux,
        command: "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
        source: "https://learn.chatgpt.com/docs/codex/cli https://chatgpt.com/codex/install.sh",
    },
    // The README's install line, which installs the newest release.
    UpdateRoute {
        method: InstallMethod::Npm("@openai/codex"),
        systems: Systems::Every,
        command: "npm install -g @openai/codex",
        source: "https://github.com/openai/codex#installing-and-running-codex-cli",
    },
    // The README installs the `codex` cask; Homebrew upgrades a cask so.
    UpdateRoute {
        method: InstallMethod::Homebrew("codex"),
        systems: Systems::MacOsAndLinux,
        command: "brew upgrade --cask codex",
        source: "https://github.com/openai/codex#installing-and-running-codex-cli https://docs.brew.sh/Manpage#upgrade-options-installed_formulainstalled_cask-",
    },
];

const GROK_UPDATES: &[UpdateRoute] = &[
    // The install script links `~/.grok/bin/grok` into `~/.grok/downloads`
    // on macOS and Linux and writes `%USERPROFILE%\.grok\bin\grok.exe` on
    // Windows; "`grok update` | Check for updates or install a specific
    // version".
    UpdateRoute {
        method: InstallMethod::Installer(&[".grok/downloads", ".grok/bin/grok.exe"]),
        systems: Systems::Every,
        command: "grok update",
        source: "https://docs.x.ai/build/cli/reference https://x.ai/cli/install.sh https://x.ai/cli/install.ps1",
    },
    // "Use `npm install -g @xai-official/grok` as an alternative".
    UpdateRoute {
        method: InstallMethod::Npm("@xai-official/grok"),
        systems: Systems::Every,
        command: "npm install -g @xai-official/grok",
        source: "https://docs.x.ai/build/enterprise",
    },
];

// "`opencode upgrade [target]` ... `--method`: curl, npm, pnpm, bun, brew":
// OpenCode updates itself for every way it is installed that it knows.
const OPENCODE_UPGRADE: &str = "https://opencode.ai/docs/cli/#upgrade";
const OPENCODE_UPDATES: &[UpdateRoute] = &[
    // "The install script installs OpenCode to `$HOME/.opencode/bin`."
    UpdateRoute {
        method: InstallMethod::Installer(&[".opencode/bin/opencode"]),
        systems: Systems::MacOsAndLinux,
        command: "opencode upgrade",
        source: OPENCODE_UPGRADE,
    },
    UpdateRoute {
        method: InstallMethod::Npm("opencode-ai"),
        systems: Systems::Every,
        command: "opencode upgrade",
        source: OPENCODE_UPGRADE,
    },
    UpdateRoute {
        method: InstallMethod::Bun("opencode-ai"),
        systems: Systems::Every,
        command: "opencode upgrade",
        source: OPENCODE_UPGRADE,
    },
    UpdateRoute {
        method: InstallMethod::Homebrew("opencode"),
        systems: Systems::MacOsAndLinux,
        command: "opencode upgrade",
        source: OPENCODE_UPGRADE,
    },
];

const PI_UPDATES: &[UpdateRoute] = &[
    // The installer writes its launcher to `~/.pi/agent/bin/pi` and links
    // it from a folder on `PATH`; "The installer pins all dependencies and
    // updates Pi with `pi update`."
    UpdateRoute {
        method: InstallMethod::Installer(&[".pi/agent/bin"]),
        systems: Systems::MacOsAndLinux,
        command: "pi update",
        source: "https://pi.dev/docs/quickstart https://pi.dev/install.sh",
    },
    // The README's npm line (Windows' installer installs with npm too).
    UpdateRoute {
        method: InstallMethod::Npm("@earendil-works/pi-coding-agent"),
        systems: Systems::Every,
        command: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
        source: "https://github.com/earendil-works/pi#installation",
    },
    UpdateRoute {
        method: InstallMethod::Bun("@earendil-works/pi-coding-agent"),
        systems: Systems::Every,
        command: "bun add -g --ignore-scripts @earendil-works/pi-coding-agent",
        source: "https://pi.dev/",
    },
];

const OMP_UPDATES: &[UpdateRoute] = &[
    // The install script writes `~/.local/bin/omp`, and on Windows
    // `%LOCALAPPDATA%\omp\omp.exe`; "`omp update [flags]` Installs the
    // current channel's update."
    UpdateRoute {
        method: InstallMethod::Installer(&[".local/bin/omp", "AppData/Local/omp/omp.exe"]),
        systems: Systems::Every,
        command: "omp update",
        source: "https://omp.sh/docs/cli https://omp.sh/install https://omp.sh/install.ps1",
    },
    // The README's "Bun (recommended)" line.
    UpdateRoute {
        method: InstallMethod::Bun("@oh-my-pi/pi-coding-agent"),
        systems: Systems::Every,
        command: "bun install -g @oh-my-pi/pi-coding-agent",
        source: "https://github.com/can1357/oh-my-pi#installation",
    },
    // The README installs `can1357/tap/omp`; Homebrew upgrades it so.
    UpdateRoute {
        method: InstallMethod::Homebrew("omp"),
        systems: Systems::MacOsAndLinux,
        command: "brew upgrade can1357/tap/omp",
        source: "https://github.com/can1357/oh-my-pi#installation https://docs.brew.sh/Manpage#upgrade-options-installed_formulainstalled_cask-",
    },
];

// "Default binary location: `~/.local/bin/agy`", and on Windows the user's
// `AppData\Local\agy\bin` under the home folder: the documented page has
// no update command, so the update is its install command, which installs
// the newest release.
const ANTIGRAVITY_INSTALL: &str = "https://antigravity.google/docs/getting-started?tab=cli";
const ANTIGRAVITY_UPDATES: &[UpdateRoute] = &[
    UpdateRoute {
        method: InstallMethod::Installer(&[".local/bin/agy"]),
        systems: Systems::MacOsAndLinux,
        command: "curl -fsSL https://antigravity.google/cli/install.sh | bash",
        source: ANTIGRAVITY_INSTALL,
    },
    UpdateRoute {
        method: InstallMethod::Installer(&["AppData/Local/agy/bin/agy.exe"]),
        systems: Systems::Windows,
        command: "irm https://antigravity.google/cli/install.ps1 | iex",
        source: ANTIGRAVITY_INSTALL,
    },
];

/// The supported order is also the stable Settings order.
pub const ADAPTERS: &[AgentAdapter] = &[
    AgentAdapter {
        key: AgentId::ClaudeCode,
        fake_home: journey::FakeHome::Proven {
            key_env: "ANTHROPIC_API_KEY",
            auto_update: journey::AutoUpdate::Environment {
                name: "DISABLE_AUTOUPDATER",
                value: "1",
            },
            evidence: "https://code.claude.com/docs/en/settings-reference",
        },
        id: "claude-code",
        label: "Claude Code",
        picker_label: "Claude",
        sidebar_label: Some("Claude"),
        terminal_click: true,
        aliases: &["claude", "claude_code"],
        executables: &["claude"],
        minimum: Some(MinimumVersion {
            version: "2.1.278",
            evidence: "docs/agent-hooks.md#the-oldest-version-of-each-agent",
        }),
        skill_location: SkillLocation::Claude,
        hook: HookInstall::Runtime(HookDialect::ClaudeCode),
        guidance: Support::Supported(ContextRoute::NativeHook(HookDialect::ClaudeCode)),
        observation: ObservationSupport::Native(HookDialect::ClaudeCode),
        herdr: HerdrIntegration {
            name: "claude",
            target: "claude",
            folder: &[".claude"],
        },
        default_on: true,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::ClaudeCode,
            context: true,
        }),
        spawn_guard: Some(HookDialect::ClaudeCode),
        subagent_counts: Some(HookDialect::ClaudeCode),
        memory: Support::Supported(ContextRoute::NativeHook(HookDialect::ClaudeCode)),
        bell: true,
        bell_names_inbox: false,
        turn_end_letters: None,
        tool_letters: None,
        session: Some(SessionFormat::Claude),
        sleep: Some(LaunchDialect::Claude),
        fork: Some(LaunchDialect::Claude),
        resume: Some(LaunchDialect::Claude),
        conversation: Some(SessionFormat::Claude),
        titles: Support::Supported(SessionFormat::Claude),
        composer: Support::Supported(Composer {
            region: Region::BetweenRules,
            marker: &["❯"],
            trailer: 0,
            // A suggestion such as `Try "…"` or the next prompt it predicts,
            // which changes from screen to screen, always dim.
            empty: &[Placeholder {
                text: "",
                style: Style::Dim,
            }],
        }),
        start: Some(LaunchDialect::Claude),
        find: Some(FindDialect::ClaudeTranscript),
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: Support::Supported(LimitSource::ClaudeUsageText),
                weekly: Support::Supported(LimitSource::ClaudeUsageText),
                scoped: Support::Supported(LimitSource::ClaudeUsageText),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::ClaudeTranscript),
            cost: Support::Supported(CostBasis::PublicTable),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://code.claude.com/docs/en/skills",
        install_url: "https://code.claude.com/docs/en/setup",
        updates: CLAUDE_UPDATES,
        logo_id: "claude-code",
    },
    AgentAdapter {
        key: AgentId::Codex,
        fake_home: journey::FakeHome::Proven {
            key_env: "JOURNEY_FAKE_KEY",
            auto_update: journey::AutoUpdate::Config {
                path: ".codex/config.toml",
                key: "check_for_update_on_startup",
                value: "false",
            },
            evidence: "https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/core/config.schema.json",
        },
        id: "codex",
        label: "Codex",
        picker_label: "Codex",
        sidebar_label: Some("Codex"),
        terminal_click: false,
        aliases: &[],
        executables: &["codex"],
        minimum: Some(MinimumVersion {
            version: "0.155.1",
            evidence: "docs/agent-hooks.md#the-oldest-version-of-each-agent",
        }),
        skill_location: SkillLocation::Shared,
        hook: HookInstall::Runtime(HookDialect::Codex),
        guidance: Support::Supported(ContextRoute::NativeHook(HookDialect::Codex)),
        observation: ObservationSupport::Native(HookDialect::Codex),
        herdr: HerdrIntegration {
            name: "codex",
            target: "codex",
            folder: &[".codex"],
        },
        default_on: true,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::Codex,
            context: true,
        }),
        spawn_guard: Some(HookDialect::Codex),
        subagent_counts: Some(HookDialect::Codex),
        memory: Support::Supported(ContextRoute::NativeHook(HookDialect::Codex)),
        bell: true,
        bell_names_inbox: true,
        turn_end_letters: None,
        tool_letters: None,
        session: Some(SessionFormat::Codex),
        sleep: Some(LaunchDialect::Codex),
        fork: Some(LaunchDialect::Codex),
        resume: Some(LaunchDialect::Codex),
        conversation: Some(SessionFormat::Codex),
        titles: Support::Supported(SessionFormat::Codex),
        composer: Support::Supported(Composer {
            region: Region::ToBlank,
            marker: &["›"],
            trailer: 0,
            empty: &[Placeholder {
                text: "Ask Codex to do anything",
                style: Style::Dim,
            }],
        }),
        start: Some(LaunchDialect::Codex),
        find: Some(FindDialect::CodexTranscript),
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: Support::Supported(LimitSource::CodexUsage),
                weekly: Support::Supported(LimitSource::CodexUsage),
                scoped: not_built(5),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::CodexRollout),
            cost: Support::Supported(CostBasis::PublicTable),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://learn.chatgpt.com/docs/build-skills",
        install_url: "https://learn.chatgpt.com/docs/codex/cli",
        updates: CODEX_UPDATES,
        logo_id: "codex",
    },
    AgentAdapter {
        key: AgentId::Grok,
        fake_home: journey::FakeHome::Proven {
            key_env: "JOURNEY_FAKE_KEY",
            auto_update: journey::AutoUpdate::Environment {
                name: "GROK_DISABLE_AUTOUPDATER",
                value: "1",
            },
            evidence: "https://docs.x.ai/build/settings/reference.md",
        },
        id: "grok",
        label: "Grok",
        picker_label: "Grok",
        sidebar_label: None,
        terminal_click: false,
        aliases: &[],
        executables: &["grok"],
        minimum: Some(MinimumVersion {
            // The journey ran on 1.0.52 on all three systems; 1.0.50 is a
            // reading of Grok's source, not a run.
            version: "1.0.52",
            evidence: "docs/agent-hooks.md#the-oldest-version-of-each-agent",
        }),
        skill_location: SkillLocation::Shared,
        hook: HookInstall::Guidance(GuidanceDialect::Grok),
        guidance: Support::Unsupported(GROK_CONTEXT_ABSENCE),
        observation: ObservationSupport::Native(HookDialect::Grok),
        herdr: HerdrIntegration {
            name: "grok",
            target: "grok",
            folder: &[".grok"],
        },
        default_on: false,
        // The prompt hook records submission and recognizes a bell; its
        // output is discarded. Tool and turn-end hooks carry the letters.
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::Grok,
            context: false,
        }),
        spawn_guard: Some(HookDialect::Grok),
        subagent_counts: Some(HookDialect::Grok),
        memory: Support::Unsupported(GROK_CONTEXT_ABSENCE),
        bell: true,
        bell_names_inbox: true,
        turn_end_letters: Some(HookDialect::Grok),
        tool_letters: Some(HookDialect::Grok),
        session: Some(SessionFormat::Grok),
        sleep: Some(LaunchDialect::Grok),
        fork: Some(LaunchDialect::Grok),
        resume: Some(LaunchDialect::Grok),
        conversation: Some(SessionFormat::Grok),
        titles: Support::Supported(SessionFormat::Grok),
        composer: Support::Supported(Composer {
            region: Region::Box {
                open: '╭',
                close: '╰',
                side: '│',
            },
            // `> ` on a console Grok takes for Windows' legacy one, which a
            // Herdr pane on Windows is (`glyphs::prompt_arrow`).
            marker: &["❯", ">"],
            trailer: 0,
            // The next prompt Grok predicts after a turn, drawn as ghost text
            // in an empty composer (`ghost_text_style`, always italic); its
            // words change from screen to screen. Typed text and the grey
            // `[Pasted: …]` chip are never italic, so nothing merely faint
            // reads empty.
            empty: &[Placeholder {
                text: "",
                style: Style::Italic,
            }],
        }),
        start: Some(LaunchDialect::Grok),
        find: None,
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                weekly: not_built(5),
                scoped: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::GrokUpdates),
            cost: Support::Supported(CostBasis::Recorded),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://docs.x.ai/build/features/skills-plugins-marketplaces",
        install_url: "https://github.com/xai-org/grok-build",
        updates: GROK_UPDATES,
        logo_id: "grok",
    },
    AgentAdapter {
        key: AgentId::OpenCode,
        fake_home: journey::FakeHome::Proven {
            key_env: "JOURNEY_FAKE_KEY",
            auto_update: journey::AutoUpdate::Config {
                path: ".config/opencode/opencode.json",
                key: "update",
                value: "\"disable\"",
            },
            evidence: "https://opencode.ai/v2/docs/config/",
        },
        id: "opencode",
        label: "OpenCode",
        picker_label: "OpenCode",
        sidebar_label: Some("OpenCode"),
        terminal_click: false,
        aliases: &[],
        executables: &["opencode"],
        minimum: Some(MinimumVersion {
            // Windows loaded the plugin on 1.18.35; 1.18.30's shapes are
            // proven on macOS and Linux only.
            version: "1.18.35",
            evidence: "docs/agent-hooks.md#the-oldest-version-of-each-agent",
        }),
        skill_location: SkillLocation::Shared,
        hook: HookInstall::Plugin(PluginDialect::OpenCode),
        guidance: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::OpenCode)),
        observation: ObservationSupport::Plugin(PluginDialect::OpenCode),
        herdr: HerdrIntegration {
            name: "opencode",
            target: "opencode",
            folder: &[".config", "opencode"],
        },
        default_on: false,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::OpenCode,
            context: true,
        }),
        spawn_guard: Some(HookDialect::OpenCode),
        subagent_counts: Some(HookDialect::OpenCode),
        memory: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::OpenCode)),
        // A bell's Enter in OpenCode's session picker opens another session
        // (measured 2026-10-07); letters reach it on its next prompt instead.
        bell: false,
        bell_names_inbox: false,
        turn_end_letters: None,
        tool_letters: Some(HookDialect::OpenCode),
        session: Some(SessionFormat::OpenCode),
        sleep: Some(LaunchDialect::OpenCode),
        fork: Some(LaunchDialect::OpenCode),
        resume: Some(LaunchDialect::OpenCode),
        conversation: Some(SessionFormat::OpenCode),
        titles: Support::Supported(SessionFormat::OpenCode),
        // The last line inside the bar is the agent and model, never input.
        composer: Support::Supported(Composer {
            region: Region::Bar {
                side: '┃',
                close: '╹',
            },
            marker: &[],
            trailer: 1,
            empty: &[Placeholder {
                text: "Ask anything…",
                style: Style::Rgb(128, 128, 128),
            }],
        }),
        start: Some(LaunchDialect::OpenCode),
        find: None,
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: absent(
                    AbsenceKind::Excluded,
                    "Reading limits requires credentials outside the supported usage boundary.",
                    "An official delegated usage reader becomes available.",
                ),
                weekly: absent(
                    AbsenceKind::Excluded,
                    "Reading limits requires credentials outside the supported usage boundary.",
                    "An official delegated usage reader becomes available.",
                ),
                scoped: absent(
                    AbsenceKind::Excluded,
                    "Reading limits requires credentials outside the supported usage boundary.",
                    "An official delegated usage reader becomes available.",
                ),
                monthly: absent(
                    AbsenceKind::Excluded,
                    "Reading limits requires credentials outside the supported usage boundary.",
                    "An official delegated usage reader becomes available.",
                ),
            },
            tokens: Support::Supported(TokenSource::OpenCodeParts),
            cost: Support::Supported(CostBasis::Recorded),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://opencode.ai/docs/skills/",
        install_url: "https://opencode.ai/docs/",
        updates: OPENCODE_UPDATES,
        logo_id: "opencode",
    },
    AgentAdapter {
        key: AgentId::Pi,
        fake_home: journey::FakeHome::Proven {
            key_env: "JOURNEY_FAKE_KEY",
            auto_update: journey::AutoUpdate::Environment {
                name: "PI_OFFLINE",
                value: "1",
            },
            evidence: "https://unpkg.com/@earendil-works/pi-coding-agent@1.0.4/docs/models.md",
        },
        id: "pi",
        label: "Pi",
        picker_label: "Pi",
        sidebar_label: None,
        terminal_click: false,
        aliases: &[],
        executables: &["pi"],
        minimum: Some(MinimumVersion {
            version: "1.0.4",
            evidence: "hide-agent-hooks/tests/fixtures/pi-extension/pi-1.0.4.json",
        }),
        skill_location: SkillLocation::Shared,
        hook: HookInstall::Plugin(PluginDialect::Pi),
        guidance: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::Pi)),
        observation: ObservationSupport::Plugin(PluginDialect::Pi),
        herdr: HerdrIntegration {
            name: "pi",
            target: "pi",
            folder: &[".pi", "agent"],
        },
        default_on: false,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::Pi,
            context: true,
        }),
        spawn_guard: Some(HookDialect::Pi),
        // Pi runs no subagents.
        subagent_counts: None,
        memory: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::Pi)),
        // Pi's previous-session picker reads `done` and a bell's Enter there
        // resumes another session; its trust dialog reads `idle` and Enter
        // picks the highlighted item (measured 2026-10-07). Letters reach it on
        // its next prompt instead.
        bell: false,
        bell_names_inbox: false,
        turn_end_letters: None,
        tool_letters: Some(HookDialect::Pi),
        session: Some(SessionFormat::Pi),
        sleep: Some(LaunchDialect::Pi),
        fork: Some(LaunchDialect::Pi),
        resume: Some(LaunchDialect::Pi),
        conversation: Some(SessionFormat::Pi),
        titles: Support::Supported(SessionFormat::Pi),
        composer: Support::Supported(Composer {
            region: Region::BetweenRules,
            marker: &[],
            trailer: 0,
            empty: &[],
        }),
        start: Some(LaunchDialect::Pi),
        find: None,
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                weekly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                scoped: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::PiSession),
            cost: Support::Supported(CostBasis::Recorded),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/skills.md",
        install_url: "https://pi.dev/",
        updates: PI_UPDATES,
        logo_id: "pi",
    },
    AgentAdapter {
        key: AgentId::Omp,
        fake_home: journey::FakeHome::Proven {
            key_env: "JOURNEY_FAKE_KEY",
            auto_update: journey::AutoUpdate::Config {
                path: ".omp/agent/config.yml",
                key: "startup",
                value: "{\"checkUpdate\":false}",
            },
            evidence: "https://github.com/can1357/oh-my-pi/blob/v18.7.0/packages/coding-agent/src/main.ts",
        },
        id: "omp",
        label: "omp",
        picker_label: "omp",
        sidebar_label: None,
        terminal_click: false,
        aliases: &[],
        executables: &["omp"],
        minimum: Some(MinimumVersion {
            version: "18.7.0",
            evidence: "hide-agent-hooks/tests/fixtures/pi-extension/omp-18.7.0.json",
        }),
        skill_location: SkillLocation::Shared,
        hook: HookInstall::Plugin(PluginDialect::Omp),
        guidance: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::Omp)),
        observation: ObservationSupport::Plugin(PluginDialect::Omp),
        herdr: HerdrIntegration {
            name: "omp",
            target: "omp",
            folder: &[".omp", "agent"],
        },
        default_on: false,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::Omp,
            context: true,
        }),
        spawn_guard: Some(HookDialect::Omp),
        subagent_counts: Some(HookDialect::Omp),
        memory: Support::Supported(ContextRoute::PluginPrompt(PluginDialect::Omp)),
        // omp's previous-session picker reads `done` and a bell's Enter there
        // resumes another session (measured 2026-10-07). Letters reach it on
        // its next prompt instead.
        bell: false,
        bell_names_inbox: false,
        turn_end_letters: None,
        tool_letters: Some(HookDialect::Omp),
        session: Some(SessionFormat::Omp),
        sleep: Some(LaunchDialect::Omp),
        fork: Some(LaunchDialect::Omp),
        resume: Some(LaunchDialect::Omp),
        conversation: Some(SessionFormat::Omp),
        titles: Support::Supported(SessionFormat::Omp),
        composer: Support::Supported(Composer {
            region: Region::Line,
            marker: &["╰─"],
            trailer: 0,
            empty: &[Placeholder {
                text: "⇧⇥ to change thinking effort",
                style: Style::Any,
            }],
        }),
        start: Some(LaunchDialect::Omp),
        find: None,
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                weekly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                scoped: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::OmpSession),
            cost: Support::Supported(CostBasis::Recorded),
            models: Support::Supported(ModelFrom::UsageRecord),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://omp.sh/docs/skills",
        install_url: "https://omp.sh/docs/quickstart",
        updates: OMP_UPDATES,
        logo_id: "omp",
    },
    AgentAdapter {
        key: AgentId::Antigravity,
        fake_home: journey::FakeHome::Exception {
            auto_update: Some(journey::AutoUpdate::Environment {
                name: "AGY_CLI_DISABLE_AUTO_UPDATE",
                value: "1",
            }),
            reason: "The API-key environment has not been proven.",
        },
        id: "antigravity",
        label: "Antigravity",
        picker_label: "Antigravity",
        sidebar_label: Some("Antigravity"),
        terminal_click: false,
        aliases: &["agy", "antigravity-cli", "antigravity_cli"],
        executables: &["agy"],
        minimum: Some(MinimumVersion {
            // The Windows guard was proven on 1.4.2; 1.4.0 on macOS only.
            version: "1.4.2",
            evidence: "docs/agent-hooks.md#the-oldest-version-of-each-agent",
        }),
        skill_location: SkillLocation::Antigravity,
        hook: HookInstall::Guidance(GuidanceDialect::Antigravity),
        guidance: Support::Supported(ContextRoute::GuidanceHook(GuidanceDialect::Antigravity)),
        observation: ObservationSupport::Native(HookDialect::Antigravity),
        herdr: HerdrIntegration {
            name: "agy",
            target: "antigravity-cli",
            folder: &[".gemini", "config"],
        },
        // On by default like Claude Code and Codex: the operator's amendment
        // to PRD antigravity-first-class B1/D-14 (2026-10-16).
        default_on: true,
        prompt_hook: Some(PromptHook {
            dialect: HookDialect::Antigravity,
            context: true,
        }),
        spawn_guard: Some(HookDialect::Antigravity),
        subagent_counts: Some(HookDialect::Antigravity),
        memory: Support::Supported(ContextRoute::GuidanceHook(GuidanceDialect::Antigravity)),
        // Herdr 0.9.3 reads agy's permission question as an approval, but
        // its folder-trust question is unmeasured, and a bell's Enter into a
        // dialog Herdr reads as `idle` would answer it; letters reach agy on
        // its next prompt instead (PRD antigravity-first-class D-06), and
        // while it works on a later model call of the turn, which its
        // `PreInvocation` hook runs before and Hide answers with the letters
        // due as an injected system message (`tool_letters`).
        bell: false,
        bell_names_inbox: false,
        turn_end_letters: None,
        tool_letters: Some(HookDialect::Antigravity),
        session: Some(SessionFormat::Antigravity),
        sleep: Some(LaunchDialect::Antigravity),
        fork: Some(LaunchDialect::Antigravity),
        resume: Some(LaunchDialect::Antigravity),
        conversation: Some(SessionFormat::Antigravity),
        titles: Support::Supported(SessionFormat::Antigravity),
        composer: Support::Supported(Composer {
            region: Region::BetweenRules,
            marker: &[">"],
            trailer: 0,
            empty: &[],
        }),
        start: Some(LaunchDialect::Antigravity),
        find: None,
        usage: UsageDeclaration {
            limits: LimitDeclaration {
                five_hour: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
                weekly: Support::Supported(LimitSource::AntigravityUsage),
                scoped: Support::Supported(LimitSource::AntigravityUsage),
                monthly: absent(
                    AbsenceKind::NoSuchData,
                    "The agent has no independent subscription window.",
                    "The agent exposes its own subscription limits.",
                ),
            },
            tokens: Support::Supported(TokenSource::AntigravityTranscript),
            cost: absent(
                AbsenceKind::NoSuchData,
                "Google publishes no price table for the agent's models.",
                "A public price table for the agent's models.",
            ),
            models: absent(
                AbsenceKind::NoSuchData,
                "The transcript does not name the model of a call.",
                "The agent records the model beside each call's tokens.",
            ),
            attribution: Attribution {
                session: Support::Supported(AttributionFrom::UsageRecord),
                project: Support::Supported(AttributionFrom::UsageRecord),
            },
        },
        doc_url: "https://antigravity.google/docs/skills/",
        install_url: "https://antigravity.google/docs/install?tab=cli",
        updates: ANTIGRAVITY_UPDATES,
        logo_id: "antigravity",
    },
];

/// Historical descriptor retained for stored Cursor conversations and retirement only.
/// Active support and installation always iterate `ADAPTERS`.
pub(super) const RETIRED_CURSOR: AgentAdapter = AgentAdapter {
    key: AgentId::Cursor,
    fake_home: journey::FakeHome::Exception {
        auto_update: None,
        reason: "The API-key environment and auto-update policy have not been proven.",
    },
    id: "cursor",
    label: "Cursor",
    picker_label: "Cursor",
    sidebar_label: None,
    terminal_click: false,
    aliases: &[],
    executables: &["cursor-agent"],
    minimum: None,
    skill_location: SkillLocation::Shared,
    hook: HookInstall::None,
    guidance: Support::Unsupported(RETIRED_CONTEXT_ABSENCE),
    observation: ObservationSupport::Unsupported {
        reason: "Cursor is retired; historical hooks perform no observation.",
    },
    herdr: HerdrIntegration {
        name: "cursor",
        target: "cursor",
        folder: &[".cursor"],
    },
    default_on: false,
    prompt_hook: None,
    spawn_guard: None,
    subagent_counts: None,
    memory: Support::Unsupported(RETIRED_CONTEXT_ABSENCE),
    bell: false,
    bell_names_inbox: false,
    turn_end_letters: None,
    tool_letters: None,
    session: Some(SessionFormat::Cursor),
    sleep: None,
    fork: None,
    resume: None,
    conversation: Some(SessionFormat::Cursor),
    titles: Support::Unsupported(RETIRED_CONTEXT_ABSENCE),
    composer: Support::Unsupported(RETIRED_CONTEXT_ABSENCE),
    start: None,
    find: None,
    usage: UsageDeclaration {
        limits: LimitDeclaration {
            five_hour: absent(
                AbsenceKind::Excluded,
                "Reading limits requires credentials outside the supported usage boundary.",
                "An official delegated usage reader becomes available.",
            ),
            weekly: absent(
                AbsenceKind::Excluded,
                "Reading limits requires credentials outside the supported usage boundary.",
                "An official delegated usage reader becomes available.",
            ),
            scoped: absent(
                AbsenceKind::Excluded,
                "Reading limits requires credentials outside the supported usage boundary.",
                "An official delegated usage reader becomes available.",
            ),
            monthly: absent(
                AbsenceKind::Excluded,
                "Reading limits requires credentials outside the supported usage boundary.",
                "An official delegated usage reader becomes available.",
            ),
        },
        tokens: absent(
            AbsenceKind::NoSuchData,
            "No supported local usage record is available.",
            "The agent provides a documented local usage record.",
        ),
        cost: absent(
            AbsenceKind::NoSuchData,
            "No supported local usage record is available.",
            "The agent provides a documented local usage record.",
        ),
        models: absent(
            AbsenceKind::NoSuchData,
            "No supported local usage record is available.",
            "The agent provides a documented local usage record.",
        ),
        attribution: Attribution {
            session: absent(
                AbsenceKind::NoSuchData,
                "No supported local usage record is available.",
                "The agent provides a documented local usage record.",
            ),
            project: absent(
                AbsenceKind::NoSuchData,
                "No supported local usage record is available.",
                "The agent provides a documented local usage record.",
            ),
        },
    },
    doc_url: "https://cursor.com/docs/context/skills",
    install_url: "https://cursor.com/docs/cli/installation",
    updates: &[],
    logo_id: "cursor",
};
