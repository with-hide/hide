//! The generated shell contract is a projection, never another registry.

#[derive(serde::Serialize)]
pub struct WebAdapter {
    pub usage: super::usage::UsageDeclaration,
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub label: &'static str,
    pub picker_label: &'static str,
    pub herdr_kind: &'static str,
    pub doc_url: &'static str,
    pub install_url: &'static str,
    pub logo_id: &'static str,
    pub can_start: bool,
    pub can_resume: bool,
    /// Reasoning efforts the start command takes; empty means the CLI default only.
    pub efforts: &'static [&'static str],
    /// Whether the start command takes a model.
    pub can_pick_model: bool,
    /// The program a start runs, as Settings › Agents shows its command.
    pub program: &'static str,
    /// The options a start adds for full access (Settings › Agents, 권한),
    /// each as its words with the option's name first, the one that turns
    /// questions off before any that keeps the CLI from stopping to confirm
    /// it: empty for an agent that asks no permission question, absent for
    /// one Hide does not start.
    pub full_access: Option<&'static [&'static [&'static str]]>,
    /// What Settings calls the agent's hook piece: a hook entry, a plugin
    /// file or an extension file; absent for an agent Hide writes no hook for.
    pub hook_kind: Option<HookKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Hook,
    Plugin,
    Extension,
}

pub fn web_contract() -> impl Iterator<Item = WebAdapter> {
    super::ADAPTERS.iter().map(|row| WebAdapter {
        usage: row.usage,
        id: row.id,
        aliases: row.aliases,
        label: row.label,
        picker_label: row.picker_label,
        herdr_kind: row.herdr.name,
        doc_url: row.doc_url,
        install_url: row.install_url,
        logo_id: row.logo_id,
        can_start: row.start.is_some(),
        can_resume: row.resume.is_some(),
        efforts: row.start.map_or(&[], |start| start.options().efforts),
        can_pick_model: row
            .start
            .is_some_and(|start| !start.options().model.is_empty()),
        program: row.executables[0],
        full_access: row.start.map(|start| start.options().full_access.options()),
        hook_kind: match row.hook {
            super::HookInstall::Runtime(_) | super::HookInstall::Guidance(_) => {
                Some(HookKind::Hook)
            }
            super::HookInstall::Plugin(super::PluginDialect::OpenCode) => Some(HookKind::Plugin),
            super::HookInstall::Plugin(super::PluginDialect::Pi | super::PluginDialect::Omp) => {
                Some(HookKind::Extension)
            }
            super::HookInstall::None => None,
        },
    })
}
