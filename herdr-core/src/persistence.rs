use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use hide_platform::fs::identity::{self, FileId};

use serde::{Deserialize, Serialize};

use crate::model::{
    AgentLogoMode, ColorTheme, DetailSlot, DeviceRegistration, OverviewGraphChoices,
    PRACTICE_CHAPTER_LIMIT, PRACTICE_STEP_LIMIT, PaneCompletion, PaneReadRecord, PaneTurnRowMode,
    PetOriginSnapshot, PracticeProgress, RightPanelSection, SidebarLens, SidebarProject, TaskView,
    ThemePreference, UiStateSnapshot, WorkspaceRegistration, default_accent_hex, default_font_size,
    default_pane_text_scale, default_panel_visible, default_sidebar_width,
    default_terminal_font_size, practice_chapter_id_valid,
};

const UI_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredUiState {
    #[serde(default)]
    resolved_sessions: BTreeMap<String, crate::agent_state::sessions::Resolution>,
    #[serde(default)]
    revisit_sessions: BTreeMap<String, crate::agent_state::sessions::RevisitMark>,
    #[serde(default)]
    session_resolution_inputs: BTreeMap<String, u64>,
    #[serde(default)]
    session_collapsed_checkout_ids: Vec<String>,
    #[serde(default)]
    session_open_folds: Vec<String>,
    schema_version: u32,
    #[serde(default = "default_panel_visible")]
    left_sidebar_visible: bool,
    #[serde(default = "default_panel_visible")]
    device_rail_visible: bool,
    #[serde(default = "default_panel_visible")]
    right_panel_visible: bool,
    #[serde(default)]
    right_panel_section: RightPanelSection,
    expanded_paths: Vec<String>,
    #[serde(default)]
    device_expanded_paths: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    collapsed_workspace_ids: Vec<String>,
    #[serde(default)]
    collapsed_checkout_ids: Vec<String>,
    /// The web Projects list's opened checkouts; absent in an older store,
    /// which loads with every checkout closed.
    #[serde(default)]
    expanded_checkout_ids: Vec<String>,
    /// Project paths whose Inactive checkout row is open. The project path is
    /// stable across navigator rebuilds and matches the existing path-keyed
    /// expansion contract.
    #[serde(default)]
    expanded_inactive_checkout_project_paths: Vec<String>,
    /// Device ids whose Inactive projects row is open.
    #[serde(default)]
    expanded_inactive_project_device_ids: Vec<String>,
    #[serde(default)]
    project_base_branches: BTreeMap<String, String>,
    #[serde(default)]
    project_settings: BTreeMap<String, crate::project_settings::ProjectSettings>,
    /// The folded-by-default agent tree's open rows. The former
    /// `collapsed_agent_pane_ids` key is ignored on load and dropped on the
    /// next write: the ids it held name panes of some earlier Herdr server,
    /// and a first launch after the upgrade starts every parent folded
    /// (PRD D-11).
    #[serde(default)]
    expanded_agent_pane_ids: Vec<String>,
    #[serde(default)]
    sessions_expanded_agent_pane_ids: Vec<String>,
    /// Kept as text, like `theme`: absent in a store written before the
    /// 태스크 list existed, which loads on the projects (PRD sidebar-task-lens
    /// D-25), and a value this build does not know loads the same way.
    #[serde(default)]
    sidebar_lens: Option<String>,
    #[serde(default)]
    task_view: Option<String>,
    /// Kept as JSON so a value this build cannot read loads as every project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sidebar_project: Option<serde_json::Value>,
    /// Kept as JSON and read chapter by chapter (`read_practice`), so a
    /// chapter this build cannot read is dropped and the rest still load.
    #[serde(default)]
    practice: Option<serde_json::Value>,
    #[serde(default)]
    practice_invite_closed: bool,
    /// Kept as JSON so a slot this build cannot read loads empty instead of
    /// making the whole store unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<serde_json::Value>,
    /// Kept as JSON and read record by record
    /// (`OverviewGraphChoices::read_stored`), so a record this build cannot
    /// read is left out and the rest of the store still loads.
    #[serde(default)]
    overview_graph: Option<serde_json::Value>,
    selected_path: Option<String>,
    selected_pane_id: Option<String>,
    /// Written only by a core move's rekey and left out once taken, so an
    /// ordinary store never carries it and an older build ignores it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    focus_from_herdr: bool,
    #[serde(default)]
    shortcut_bindings: BTreeMap<String, String>,
    /// Absent in a store written before the shortcut import existed, which loads
    /// as not imported and so imports once.
    #[serde(default)]
    shortcut_bindings_imported: bool,
    #[serde(default)]
    browser_shortcut_bindings: BTreeMap<String, String>,
    #[serde(default = "default_pet_visible")]
    pet_visible: bool,
    #[serde(default)]
    pet_origin: Option<PetOriginSnapshot>,
    #[serde(default)]
    pet_shortcut: Option<String>,
    #[serde(default)]
    focused_device_id: Option<String>,
    #[serde(default)]
    focused_checkout_id: Option<String>,
    #[serde(default)]
    workspace_registrations: Vec<WorkspaceRegistration>,
    /// Read one by one, so a registration this build cannot place (an SSH
    /// alias and a node that dials in together, or neither, as only a hand
    /// edit writes) is dropped with a diagnostic instead of making the whole
    /// store unreadable.
    #[serde(default, deserialize_with = "readable_device_registrations")]
    device_registrations: Vec<DeviceRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    machine_label: Option<String>,
    #[serde(default = "default_accent_hex")]
    accent_hex: String,
    /// Kept as text so a value this build does not know loads as Dark with a
    /// diagnostic instead of making the whole store unreadable. Absent in a
    /// store written before the web theme existed, which loads as Dark.
    #[serde(default)]
    theme: Option<String>,
    /// `shown` or `hidden`; a store written before the logo mode existed
    /// loads as hidden.
    #[serde(default)]
    agent_logo_mode: Option<String>,
    /// `one_line`, `two_lines` or `off`; a store written before the mode
    /// existed loads as one line.
    #[serde(default)]
    pane_turn_row_mode: Option<String>,
    /// Kept as text for the same reason as `theme`; absent in a store written
    /// before the color theme existed, which loads as Hide (D-04).
    #[serde(default)]
    color_theme: Option<String>,
    #[serde(default)]
    interface_language: Option<serde_json::Value>,
    #[serde(default)]
    usage: crate::usage::prefs::UsagePreferences,
    #[serde(default)]
    agent_onboarding: Option<crate::model::AgentOnboarding>,
    #[serde(default)]
    agent_onboarding_agents: Vec<String>,
    #[serde(default)]
    first_run: Option<crate::model::FirstRun>,
    #[serde(default)]
    first_run_choices: crate::model::FirstRunChoices,
    #[serde(default)]
    desktop_notifications: bool,
    #[serde(default = "default_panel_visible")]
    product_analytics_enabled: bool,
    /// Absent in a store from before the split: keep the old shared choice.
    #[serde(default)]
    desktop_notifications_done: Option<bool>,
    #[serde(default)]
    screen_intros_seen: BTreeSet<crate::model::ScreenIntro>,
    #[serde(default = "default_sidebar_width")]
    sidebar_width: u32,
    #[serde(default = "default_font_size")]
    font_size: f32,
    /// Absent in a store from before the Terminal text slider: every
    /// terminal keeps the size it drew.
    #[serde(default = "default_terminal_font_size")]
    terminal_font_size: f32,
    #[serde(default)]
    pane_text_scales: BTreeMap<String, f32>,
    /// Absent in a store written while the editor's zoom still lived in
    /// `pane_text_scales`, where the pane prune kept deleting it. It loads at
    /// the default scale, so the operator's editor opens unzoomed once.
    #[serde(default = "default_pane_text_scale")]
    editor_text_scale: f32,
    /// Absent in a store written before Hide owned the read axis. It loads as
    /// an empty map, which reads as everything unread, rather than bumping the
    /// schema version and discarding the rest of the operator's state.
    #[serde(default)]
    pane_read_records: BTreeMap<String, PaneReadRecord>,
    /// Absent in a store written before completions were kept; loads empty,
    /// so a completion a restored server no longer reports is lost once.
    #[serde(default)]
    pane_completions: BTreeMap<String, PaneCompletion>,
    /// Absent in a store written before Hide kept each pane's launch choice;
    /// loads empty, so an agent started before then restarts on its CLI's
    /// defaults, as it did.
    #[serde(default)]
    pane_launches: BTreeMap<String, crate::model::PaneLaunch>,
    /// Absent in a store written before the request view; loads empty, and
    /// each row's wait starts over once.
    #[serde(default)]
    request_verbs: BTreeMap<String, crate::request_view::VerbRecord>,
    /// Each pane's last reported terminal size, as (rows, cols). Herdr sizes
    /// a pane's PTY from the attach, so a launch that already knows a pane's
    /// size attaches at it and takes one full frame instead of one at a guess
    /// and a second after the resize. Absent in a store written before that,
    /// which loads empty and makes the first launch after it wait one canvas
    /// build for the size, as a first launch always does.
    #[serde(default)]
    pane_terminal_sizes: BTreeMap<String, (u16, u16)>,
    /// Absent in a store written before agent sleep existed, which loads as
    /// Never; a value that is not one of the choices loads as Never too.
    #[serde(default)]
    agent_sleep_after_hours: Option<u32>,
    /// Absent in a store written before issue sources could be chosen, which
    /// loads every project at its default source.
    #[serde(default)]
    project_issue_sources: BTreeMap<String, String>,
    #[serde(default)]
    issue_settings: StoredIssueSettings,
    /// Absent in a store written before Settings › Conductor, which loads
    /// the defaults. Kept as a value, as `detail` is, so one this build
    /// cannot read or refuses loads the defaults instead of making the whole
    /// store unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conductor_settings: Option<serde_json::Value>,
    /// The agent kind and per-kind model last started with. Absent in a
    /// store written before it existed, which seeds it once from the removed
    /// `issue_settings.default_agent` (PRD home-device-rail D-18).
    #[serde(default)]
    agent_start: Option<crate::model::AgentStartChoice>,
    /// The stamps and the sleeping agents; absent loads as none.
    #[serde(default)]
    agent_sleep: crate::agent_sleep::AgentSleepStore,
    /// The archived sessions; absent loads as none.
    #[serde(default)]
    session_archive: crate::session_archive::SessionArchive,
    /// Absent in a store written before the recent list existed, which loads
    /// empty. More entries than the limit (a hand edit) are cut to it.
    #[serde(default)]
    recent_checkouts: Vec<crate::model::RecentCheckout>,
    /// Absent in a store written before the recent pane order existed, which
    /// loads empty. More entries than the limit (a hand edit) are cut to it.
    #[serde(default)]
    recent_pane_ids: Vec<String>,
    /// Absent in a store written before Rename was recorded, which loads
    /// empty: no tab named before then counts as the operator's until it is
    /// renamed again. More entries than the limit (a hand edit) are cut to it.
    #[serde(default)]
    operator_tab_names: BTreeMap<String, String>,
}

/// Settings › Issues as stored. `default_agent` is only ever read: it named
/// the Start dialog's first agent until every start surface shared one
/// remembered choice, and it seeds that choice once; the next save drops it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct StoredIssueSettings {
    #[serde(flatten)]
    settings: crate::model::IssueSettingsSnapshot,
    #[serde(default, skip_serializing)]
    default_agent: Option<String>,
}

/// A store written before the pet existed carries no visibility, and the pet
/// shows itself by default (D-09).
fn default_pet_visible() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadDisposition {
    Loaded,
    /// Loaded, but a stored appearance value was not one this build knows;
    /// each named field took its default.
    Unrecognized(Vec<UnrecognizedField>),
    Missing,
    Corrupt,
}

/// A stored appearance field whose value this build does not know.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnrecognizedField {
    /// The mode; Dark was used.
    Theme,
    /// The color theme; Hide was used.
    ColorTheme,
    /// The agent logo mode; hidden was used.
    AgentLogoMode,
    /// The pane turn row mode; one line was used.
    PaneTurnRowMode,
    /// The sidebar's lens or the 태스크 view option; the projects, grouped
    /// by state, was used.
    SidebarLens,
    /// The project the sidebar's lists are narrowed to; every project was used.
    SidebarProject,
    /// Practice-mode progress; the chapters that read loaded.
    Practice,
    /// The lane's 보기 slot; it loaded empty.
    Detail,
    /// Records of the Overview map's choices; the ones that read loaded.
    OverviewGraph,
    /// Settings › Conductor; the defaults were used.
    ConductorSettings,
}

/// What the removed native app's state file holds for `shortcut_bindings`.
#[derive(Debug, PartialEq)]
pub enum NativeAppShortcuts {
    /// No file: that app never saved state under this account.
    Missing,
    Found(BTreeMap<String, String>),
    /// The file is there but its bindings could not be read.
    Unreadable,
}

/// A state file larger than this is not read: the import is one field of a
/// file another app wrote, never a reason to load an unbounded one.
const NATIVE_APP_STATE_READ_CAP: u64 = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct NativeAppShortcutFields {
    #[serde(default)]
    shortcut_bindings: BTreeMap<String, String>,
}

/// Reads only `shortcut_bindings` from the removed native app's state file;
/// the rest of that file was that app's, and the core takes nothing else from it.
pub fn read_native_app_shortcuts(path: &Path) -> NativeAppShortcuts {
    let bytes = match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return NativeAppShortcuts::Missing;
        }
        Err(_) => return NativeAppShortcuts::Unreadable,
        Ok(metadata) if metadata.len() > NATIVE_APP_STATE_READ_CAP => {
            return NativeAppShortcuts::Unreadable;
        }
        Ok(_) => match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return NativeAppShortcuts::Missing;
            }
            Err(_) => return NativeAppShortcuts::Unreadable,
        },
    };
    match serde_json::from_slice::<NativeAppShortcutFields>(&bytes) {
        Ok(fields) => NativeAppShortcuts::Found(fields.shortcut_bindings),
        Err(_) => NativeAppShortcuts::Unreadable,
    }
}

fn readable_device_registrations<'de, D>(
    deserializer: D,
) -> Result<Vec<DeviceRegistration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| match serde_json::from_value(value) {
            Ok(registration) => Some(registration),
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "ui_state",
                    "kind": "device_registration.unreadable",
                    "reason": error.to_string(),
                }));
                None
            }
        })
        .collect())
}

/// Terminal sizes are kept beside the UI state rather than inside it: they
/// are PTY geometry the core attaches with, and the shell never draws them,
/// so they do not belong on the snapshot wire.
pub type PaneTerminalSizes = BTreeMap<String, (u16, u16)>;

pub fn load(path: &Path) -> (UiStateSnapshot, PaneTerminalSizes, LoadDisposition) {
    match fs::read(path) {
        Ok(bytes) => decode(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            UiStateSnapshot::default(),
            PaneTerminalSizes::new(),
            LoadDisposition::Missing,
        ),
        Err(_) => (
            UiStateSnapshot::default(),
            PaneTerminalSizes::new(),
            LoadDisposition::Corrupt,
        ),
    }
}

/// Why `load` would read the file at `path` as corrupt, or `None` when it
/// reads it; for a refusal that names what this build cannot read.
pub(crate) fn unreadable(path: &Path) -> Option<String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return Some(error.to_string()),
    };
    match serde_json::from_slice::<StoredUiState>(&bytes) {
        Err(error) => Some(error.to_string()),
        Ok(stored) if stored.schema_version != UI_STATE_SCHEMA_VERSION => Some(format!(
            "schema version {}, not {UI_STATE_SCHEMA_VERSION}",
            stored.schema_version
        )),
        Ok(_) => None,
    }
}

/// Reads the stored practice progress chapter by chapter and returns it with
/// how many entries were dropped: one that is not a chapter id with a
/// readable progress, a step past the limit, or past the chapter limit.
fn read_practice(value: serde_json::Value) -> (BTreeMap<String, PracticeProgress>, usize) {
    let serde_json::Value::Object(entries) = value else {
        return (BTreeMap::new(), 1);
    };
    let mut practice = BTreeMap::new();
    let mut dropped = 0;
    for (chapter, entry) in entries {
        let progress = serde_json::from_value::<PracticeProgress>(entry)
            .ok()
            .filter(|progress| progress.step <= PRACTICE_STEP_LIMIT);
        match progress {
            Some(progress)
                if practice_chapter_id_valid(&chapter)
                    && practice.len() < PRACTICE_CHAPTER_LIMIT =>
            {
                practice.insert(chapter, progress);
            }
            _ => dropped += 1,
        }
    }
    (practice, dropped)
}

fn decode(bytes: &[u8]) -> (UiStateSnapshot, PaneTerminalSizes, LoadDisposition) {
    let Ok(stored) = serde_json::from_slice::<StoredUiState>(bytes) else {
        return (
            UiStateSnapshot::default(),
            PaneTerminalSizes::new(),
            LoadDisposition::Corrupt,
        );
    };
    if stored.schema_version != UI_STATE_SCHEMA_VERSION {
        return (
            UiStateSnapshot::default(),
            PaneTerminalSizes::new(),
            LoadDisposition::Corrupt,
        );
    }
    let mut unrecognized = Vec::new();
    let theme = match stored.theme.as_deref() {
        None => ThemePreference::Dark,
        Some(value) => ThemePreference::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::Theme);
            ThemePreference::Dark
        }),
    };
    let color_theme = match stored.color_theme.as_deref() {
        None => ColorTheme::Hide,
        Some(value) => ColorTheme::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::ColorTheme);
            ColorTheme::Hide
        }),
    };
    let agent_logo_mode = match stored.agent_logo_mode.as_deref() {
        None => AgentLogoMode::Hidden,
        Some(value) => AgentLogoMode::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::AgentLogoMode);
            AgentLogoMode::Hidden
        }),
    };
    let pane_turn_row_mode = match stored.pane_turn_row_mode.as_deref() {
        None => PaneTurnRowMode::OneLine,
        Some(value) => PaneTurnRowMode::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::PaneTurnRowMode);
            PaneTurnRowMode::OneLine
        }),
    };
    let sidebar_lens = match stored.sidebar_lens.as_deref() {
        None => SidebarLens::default(),
        Some(value) => SidebarLens::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::SidebarLens);
            SidebarLens::default()
        }),
    };
    let task_view = match stored.task_view.as_deref() {
        None => TaskView::default(),
        Some(value) => TaskView::parse(value).unwrap_or_else(|| {
            unrecognized.push(UnrecognizedField::SidebarLens);
            TaskView::default()
        }),
    };
    let sidebar_project = stored.sidebar_project.and_then(|value| {
        serde_json::from_value::<SidebarProject>(value)
            .map_err(|_| unrecognized.push(UnrecognizedField::SidebarProject))
            .ok()
    });
    let practice = match stored.practice {
        None => BTreeMap::new(),
        Some(value) => {
            let (practice, dropped) = read_practice(value);
            if dropped > 0 {
                unrecognized.push(UnrecognizedField::Practice);
            }
            practice
        }
    };
    let detail = match stored.detail {
        None => DetailSlot::default(),
        Some(value) => serde_json::from_value::<DetailSlot>(value)
            .ok()
            .filter(|slot| slot.items.len() <= crate::model::DETAIL_HISTORY_LIMIT)
            .unwrap_or_else(|| {
                unrecognized.push(UnrecognizedField::Detail);
                DetailSlot::default()
            }),
    };
    let overview_graph = match stored.overview_graph {
        None => OverviewGraphChoices::default(),
        Some(value) => match OverviewGraphChoices::read_stored(value) {
            Ok((choices, 0)) => choices,
            Ok((choices, _)) => {
                unrecognized.push(UnrecognizedField::OverviewGraph);
                choices
            }
            Err(_) => {
                unrecognized.push(UnrecognizedField::OverviewGraph);
                OverviewGraphChoices::default()
            }
        },
    };
    let conductor_settings = match stored.conductor_settings {
        None => crate::missions::ConductorSettings::default(),
        Some(value) => serde_json::from_value::<crate::missions::ConductorSettings>(value)
            .ok()
            .filter(|settings| settings.validate().is_ok())
            .unwrap_or_else(|| {
                unrecognized.push(UnrecognizedField::ConductorSettings);
                crate::missions::ConductorSettings::default()
            }),
    };
    let disposition = if unrecognized.is_empty() {
        LoadDisposition::Loaded
    } else {
        LoadDisposition::Unrecognized(unrecognized)
    };
    (
        UiStateSnapshot {
            resolved_sessions: stored.resolved_sessions,
            revisit_sessions: stored.revisit_sessions,
            session_resolution_inputs: stored.session_resolution_inputs,
            session_collapsed_checkout_ids: stored.session_collapsed_checkout_ids,
            session_open_folds: stored.session_open_folds,
            left_sidebar_visible: stored.left_sidebar_visible,
            device_rail_visible: stored.device_rail_visible,
            right_panel_visible: stored.right_panel_visible,
            right_panel_section: stored.right_panel_section,
            expanded_paths: stored.expanded_paths,
            device_expanded_paths: stored.device_expanded_paths,
            collapsed_workspace_ids: stored.collapsed_workspace_ids,
            collapsed_checkout_ids: stored.collapsed_checkout_ids,
            expanded_checkout_ids: stored.expanded_checkout_ids,
            expanded_inactive_checkout_project_paths: stored
                .expanded_inactive_checkout_project_paths,
            expanded_inactive_project_device_ids: stored.expanded_inactive_project_device_ids,
            project_base_branches: stored.project_base_branches,
            project_settings: stored.project_settings,
            project_settings_writes: BTreeMap::new(),
            expanded_agent_pane_ids: stored.expanded_agent_pane_ids,
            sessions_expanded_agent_pane_ids: stored.sessions_expanded_agent_pane_ids,
            sidebar_lens,
            task_view,
            sidebar_project,
            practice,
            practice_invite_closed: stored.practice_invite_closed,
            detail,
            overview_graph,
            selected_path: stored.selected_path,
            selected_pane_id: stored.selected_pane_id,
            focus_from_herdr: stored.focus_from_herdr,
            shortcut_bindings: stored.shortcut_bindings,
            shortcut_bindings_imported: stored.shortcut_bindings_imported,
            browser_shortcut_bindings: stored.browser_shortcut_bindings,
            pet_visible: stored.pet_visible,
            pet_origin: stored.pet_origin,
            pet_shortcut: stored.pet_shortcut,
            focused_device_id: stored.focused_device_id,
            focused_checkout_id: stored.focused_checkout_id,
            workspace_registrations: stored.workspace_registrations,
            device_registrations: stored.device_registrations,
            machine_label: stored.machine_label,
            accent_hex: stored.accent_hex,
            theme,
            agent_logo_mode,
            pane_turn_row_mode,
            color_theme,
            interface_language: stored.interface_language,
            usage: stored.usage.restored(),
            agent_onboarding: stored.agent_onboarding,
            agent_onboarding_agents: stored.agent_onboarding_agents,
            first_run: stored.first_run,
            first_run_choices: stored.first_run_choices,
            desktop_notifications: stored.desktop_notifications,
            product_analytics_enabled: stored.product_analytics_enabled,
            desktop_notifications_done: stored
                .desktop_notifications_done
                .unwrap_or(stored.desktop_notifications),
            screen_intros_seen: stored.screen_intros_seen,
            sidebar_width: stored.sidebar_width,
            font_size: stored.font_size,
            terminal_font_size: stored.terminal_font_size,
            pane_text_scales: stored.pane_text_scales,
            editor_text_scale: stored.editor_text_scale,
            conversation_pane_ids: Default::default(),
            pane_read_records: stored.pane_read_records,
            pane_completions: stored.pane_completions,
            pane_launches: stored.pane_launches,
            request_verbs: stored.request_verbs,
            agent_sleep_after_hours: stored
                .agent_sleep_after_hours
                .filter(|hours| crate::agent_sleep::valid_after_hours(Some(*hours))),
            project_issue_sources: stored
                .project_issue_sources
                .into_iter()
                .filter(|(_, source)| {
                    source == crate::tasks::GITHUB || source == crate::tasks::LOCAL
                })
                .collect(),
            agent_start: stored.agent_start.unwrap_or_else(|| {
                crate::model::AgentStartChoice::seeded(
                    stored.issue_settings.default_agent.as_deref(),
                )
            }),
            issue_settings: stored.issue_settings.settings,
            conductor_settings,
            agent_sleep: {
                let mut store = stored.agent_sleep;
                store.after_load();
                store
            },
            session_archive: {
                let mut archive = stored.session_archive;
                archive.after_load();
                archive
            },
            recent_checkouts: {
                let mut recent = stored.recent_checkouts;
                recent.truncate(crate::model::RECENT_CHECKOUT_LIMIT);
                recent
            },
            recent_pane_ids: {
                let mut recent = stored.recent_pane_ids;
                recent.truncate(crate::model::RECENT_PANE_LIMIT);
                recent
            },
            operator_tab_names: stored
                .operator_tab_names
                .into_iter()
                .take(crate::model::OPERATOR_TAB_NAME_LIMIT)
                .collect(),
        },
        stored.pane_terminal_sizes,
        disposition,
    )
}

/// The folders whose chain of folders from the volume's root this process has
/// made durable, by the folder's file identity. That chain is a fact about
/// the folder, not about one save, so a save reaches it again only for a
/// folder it has not seen, a replaced one included; every save still flushes
/// its file and the file's folder.
static DURABLE_FOLDERS: Mutex<Vec<FileId>> = Mutex::new(Vec::new());

/// More folders than a process keeps its UI state in; past it the oldest is
/// forgotten and made durable again on its next save.
const DURABLE_FOLDER_LIMIT: usize = 8;

fn establish_folder_once(folder: &Path) -> std::io::Result<()> {
    let id = identity::file_id(folder)?;
    let known = |folders: &Vec<FileId>| folders.contains(&id);
    if known(&DURABLE_FOLDERS.lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(());
    }
    let canonical = hide_platform::fs::private::establish_dir_durability(folder)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    #[cfg(test)]
    tests::CHAINS_ESTABLISHED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(id);
    // A folder replaced while its chain was made durable is not the one
    // recorded, and its next save makes its own chain durable.
    if identity::file_id(&canonical)? == id {
        let mut folders = DURABLE_FOLDERS.lock().unwrap_or_else(|e| e.into_inner());
        if !known(&folders) {
            if folders.len() == DURABLE_FOLDER_LIMIT {
                folders.remove(0);
            }
            folders.push(id);
        }
    }
    Ok(())
}

pub fn save(
    path: &Path,
    state: &UiStateSnapshot,
    pane_terminal_sizes: &PaneTerminalSizes,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "UI state path has no parent directory".to_owned())?;
    // Existence alone cannot authorize a destructive close. Reuse the
    // platform's checked ancestor barriers, outside the runtime lock.
    // An untrusted directory refuses the receipt.
    if !state.agent_sleep.dormant.is_empty() || !state.session_archive.is_empty() {
        establish_folder_once(parent).map_err(|_| {
            "UI state directory persistence could not be confirmed; no new close or start was sent"
                .to_owned()
        })?;
    }
    hide_platform::fs::private::create_dir_all(parent)
        .map_err(|_| "UI state directory could not be prepared".to_owned())?;
    let stored = StoredUiState {
        resolved_sessions: state.resolved_sessions.clone(),
        revisit_sessions: state.revisit_sessions.clone(),
        session_resolution_inputs: state.session_resolution_inputs.clone(),
        session_collapsed_checkout_ids: state.session_collapsed_checkout_ids.clone(),
        session_open_folds: state.session_open_folds.clone(),
        schema_version: UI_STATE_SCHEMA_VERSION,
        left_sidebar_visible: state.left_sidebar_visible,
        device_rail_visible: state.device_rail_visible,
        right_panel_visible: state.right_panel_visible,
        right_panel_section: state.right_panel_section,
        expanded_paths: state.expanded_paths.clone(),
        device_expanded_paths: state.device_expanded_paths.clone(),
        collapsed_workspace_ids: state.collapsed_workspace_ids.clone(),
        collapsed_checkout_ids: state.collapsed_checkout_ids.clone(),
        expanded_checkout_ids: state.expanded_checkout_ids.clone(),
        expanded_inactive_checkout_project_paths: state
            .expanded_inactive_checkout_project_paths
            .clone(),
        expanded_inactive_project_device_ids: state.expanded_inactive_project_device_ids.clone(),
        project_base_branches: state.project_base_branches.clone(),
        project_settings: state.project_settings.clone(),
        expanded_agent_pane_ids: state.expanded_agent_pane_ids.clone(),
        sessions_expanded_agent_pane_ids: state.sessions_expanded_agent_pane_ids.clone(),
        sidebar_lens: Some(state.sidebar_lens.as_str().to_owned()),
        task_view: Some(state.task_view.as_str().to_owned()),
        sidebar_project: state.sidebar_project.as_ref().map(|project| {
            serde_json::to_value(project).expect("the sidebar's project serializes")
        }),
        practice: Some(
            serde_json::to_value(&state.practice).expect("practice progress serializes"),
        ),
        practice_invite_closed: state.practice_invite_closed,
        detail: (state.detail != DetailSlot::default())
            .then(|| serde_json::to_value(&state.detail).expect("the 보기 slot serializes")),
        overview_graph: Some(
            serde_json::to_value(&state.overview_graph)
                .expect("the Overview map's choices serialize"),
        ),
        selected_path: state.selected_path.clone(),
        selected_pane_id: state.selected_pane_id.clone(),
        focus_from_herdr: state.focus_from_herdr,
        shortcut_bindings: state.shortcut_bindings.clone(),
        shortcut_bindings_imported: state.shortcut_bindings_imported,
        browser_shortcut_bindings: state.browser_shortcut_bindings.clone(),
        pet_visible: state.pet_visible,
        pet_origin: state.pet_origin,
        pet_shortcut: state.pet_shortcut.clone(),
        focused_device_id: state.focused_device_id.clone(),
        focused_checkout_id: state.focused_checkout_id.clone(),
        workspace_registrations: state.workspace_registrations.clone(),
        device_registrations: state.device_registrations.clone(),
        machine_label: state.machine_label.clone(),
        accent_hex: state.accent_hex.clone(),
        theme: Some(state.theme.as_str().to_owned()),
        agent_logo_mode: Some(state.agent_logo_mode.as_str().to_owned()),
        pane_turn_row_mode: Some(state.pane_turn_row_mode.as_str().to_owned()),
        color_theme: Some(state.color_theme.as_str().to_owned()),
        interface_language: state.interface_language.clone(),
        usage: state.usage.clone(),
        agent_onboarding: state.agent_onboarding,
        agent_onboarding_agents: state.agent_onboarding_agents.clone(),
        first_run: state.first_run,
        first_run_choices: state.first_run_choices.clone(),
        desktop_notifications: state.desktop_notifications,
        product_analytics_enabled: state.product_analytics_enabled,
        desktop_notifications_done: Some(state.desktop_notifications_done),
        screen_intros_seen: state.screen_intros_seen.clone(),
        sidebar_width: state.sidebar_width,
        font_size: state.font_size,
        terminal_font_size: state.terminal_font_size,
        pane_text_scales: state.pane_text_scales.clone(),
        editor_text_scale: state.editor_text_scale,
        pane_read_records: state.pane_read_records.clone(),
        pane_completions: state.pane_completions.clone(),
        pane_launches: state.pane_launches.clone(),
        request_verbs: state.request_verbs.clone(),
        pane_terminal_sizes: pane_terminal_sizes.clone(),
        agent_sleep_after_hours: state.agent_sleep_after_hours,
        project_issue_sources: state.project_issue_sources.clone(),
        issue_settings: StoredIssueSettings {
            settings: state.issue_settings.clone(),
            default_agent: None,
        },
        conductor_settings: Some(
            serde_json::to_value(&state.conductor_settings)
                .expect("Settings › Conductor serializes"),
        ),
        agent_start: Some(state.agent_start.clone()),
        agent_sleep: state.agent_sleep.clone(),
        session_archive: state.session_archive.clone(),
        recent_checkouts: state.recent_checkouts.clone(),
        recent_pane_ids: state.recent_pane_ids.clone(),
        operator_tab_names: state.operator_tab_names.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|_| "UI state could not be encoded".to_owned())?;
    hide_platform::fs::atomic::write_file_durable(
        path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| {
        let phase = match error {
            hide_platform::fs::atomic::DurableWriteError::BeforeReplace { .. } => "before_replace",
            hide_platform::fs::atomic::DurableWriteError::ReplacementUncertain { .. } => {
                "replacement_uncertain"
            }
            hide_platform::fs::atomic::DurableWriteError::ReplacedNotDurable { .. } => {
                "replaced_not_durable"
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "ui_state",
            "kind": "ui_state.durable_save_failed",
            "phase": phase,
        }));
        "UI state persistence could not be confirmed".to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each folder whose whole chain a save made durable, once per time.
    pub(super) static CHAINS_ESTABLISHED: Mutex<Vec<FileId>> = Mutex::new(Vec::new());

    fn chains_established(folder: &Path) -> usize {
        let id = identity::file_id(folder).unwrap();
        CHAINS_ESTABLISHED
            .lock()
            .unwrap()
            .iter()
            .filter(|established| **established == id)
            .count()
    }

    #[test]
    fn a_save_makes_its_folders_chain_durable_once_until_the_folder_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("state");
        hide_platform::fs::private::create_dir(&folder).unwrap();
        let path = folder.join("ui-state.json");
        let mut state = UiStateSnapshot::default();
        state.agent_sleep.dormant.insert(
            crate::agent_sleep::SleepId::new().unwrap(),
            crate::agent_sleep::fixture_record("w1:p1"),
        );

        save(&path, &state, &BTreeMap::new()).unwrap();
        save(&path, &state, &BTreeMap::new()).unwrap();
        assert_eq!(chains_established(&folder), 1);

        let earlier = root.path().join("earlier");
        fs::rename(&folder, &earlier).unwrap();
        hide_platform::fs::private::create_dir(&folder).unwrap();
        save(&path, &state, &BTreeMap::new()).unwrap();
        save(&path, &state, &BTreeMap::new()).unwrap();

        assert_eq!(chains_established(&folder), 1);
        assert_eq!(chains_established(&earlier), 1);
        assert_eq!(load(&path).0.agent_sleep.dormant.len(), 1);
    }

    /// An archived session saves through the state folder's barrier and
    /// loads back, on every system (PRD session-archive B33, D-24).
    #[test]
    fn an_archived_session_saves_through_the_folders_barrier_and_loads_back() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("state");
        hide_platform::fs::private::create_dir(&folder).unwrap();
        let path = folder.join("core-state.json");
        let mut state = UiStateSnapshot::default();
        let id = crate::session_archive::ArchiveId::new().unwrap();
        state
            .session_archive
            .admit(
                id.clone(),
                crate::session_archive::fixture_record(
                    vec![
                        crate::session_archive::fixture_member("root", None),
                        crate::session_archive::fixture_member("child", Some("root")),
                    ],
                    7,
                ),
            )
            .unwrap();

        save(&path, &state, &BTreeMap::new()).unwrap();
        assert_eq!(chains_established(&folder), 1);
        let loaded = load(&path).0.session_archive;
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[&id].members.len(), 2);
        assert_eq!(loaded.records[&id].archived_at_unix_ms, 7);
    }

    #[test]
    fn retired_git_panel_restores_overview_without_losing_other_ui_state() {
        let mut persisted = serde_json::to_value(UiStateSnapshot::default()).unwrap();
        persisted["right_panel_section"] = serde_json::json!("git");
        persisted["expanded_agent_pane_ids"] = serde_json::json!(["parent"]);
        let state: UiStateSnapshot = serde_json::from_value(persisted).unwrap();
        assert_eq!(state.right_panel_section, RightPanelSection::Overview);
        assert_eq!(state.expanded_agent_pane_ids, ["parent"]);
        assert_eq!(
            serde_json::to_value(&state).unwrap()["right_panel_section"],
            "overview"
        );
        assert_eq!(
            RightPanelSection::parse("git"),
            Some(RightPanelSection::Overview)
        );
    }

    /// A registration keeps the shape stores were written in: a dialed
    /// device by its alias, a node that dials in by its flag. One that names
    /// both or neither is dropped alone and the rest of the store loads.
    #[test]
    fn device_registrations_load_by_how_their_link_opens() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        save(
            &path,
            &UiStateSnapshot::default(),
            &PaneTerminalSizes::new(),
        )
        .unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let dialed = serde_json::json!({"id": "studio", "label": "studio", "ssh_alias": "studio-host", "herdr_socket_path": null, "host_consent": null});
        let inbound = serde_json::json!({"id": "laptop", "label": "MacBook", "ssh_alias": null, "herdr_socket_path": null, "host_consent": null, "inbound": true});
        persisted["device_registrations"] = serde_json::json!([
            dialed,
            inbound,
            {"id": "both", "label": "both", "ssh_alias": "both-host", "inbound": true},
            {"id": "neither", "label": "neither"},
        ]);
        persisted["expanded_agent_pane_ids"] = serde_json::json!(["parent"]);
        fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();

        let (state, _, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(state.expanded_agent_pane_ids, ["parent"]);
        let origins: Vec<_> = state
            .device_registrations
            .iter()
            .map(|registration| (registration.id.as_str(), registration.origin.clone()))
            .collect();
        assert_eq!(
            origins,
            [
                (
                    "studio",
                    crate::model::LinkOrigin::Dialed {
                        ssh_alias: "studio-host".to_owned()
                    }
                ),
                ("laptop", crate::model::LinkOrigin::Inbound),
            ]
        );
        assert_eq!(
            serde_json::to_value(&state.device_registrations).unwrap(),
            serde_json::json!([dialed, inbound])
        );
    }

    // PRD D-11: the folded set is not migrated. An old store's collapsed
    // ids name panes of a server that has since been restarted, so they are
    // dropped and every parent starts folded.
    #[test]
    fn a_stored_collapsed_set_is_ignored_and_every_parent_starts_folded() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        save(
            &path,
            &UiStateSnapshot::default(),
            &PaneTerminalSizes::new(),
        )
        .unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        persisted["collapsed_agent_pane_ids"] = serde_json::json!(["w1:p1", "w1:p2"]);
        fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();
        let (state, _, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(state.expanded_agent_pane_ids.is_empty());
        save(&path, &state, &PaneTerminalSizes::new()).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(rewritten.get("collapsed_agent_pane_ids").is_none());
        assert_eq!(rewritten["expanded_agent_pane_ids"], serde_json::json!([]));
    }

    /// Tabs are segments of a checkout's agent list, not rows that fold, so
    /// the stored fold choice is ignored on load and dropped on the next write.
    #[test]
    fn a_stored_tab_fold_choice_loads_and_is_dropped_on_the_next_write() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        save(
            &path,
            &UiStateSnapshot::default(),
            &PaneTerminalSizes::new(),
        )
        .unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        persisted["sidebar_collapsed_tabs"] = serde_json::json!([
            {"device_id": "local", "checkout_id": "checkout:a", "tab_id": "t1"}
        ]);
        fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();
        let (state, _, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        save(&path, &state, &PaneTerminalSizes::new()).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(rewritten.get("sidebar_collapsed_tabs").is_none());
    }

    // PRD home-device-rail D-18: the Settings default agent is gone. A store
    // written before the remembered choice seeds it once from that field,
    // and the next save drops the field.
    #[test]
    fn a_stored_default_agent_seeds_the_remembered_choice_and_is_dropped() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        save(
            &path,
            &UiStateSnapshot::default(),
            &PaneTerminalSizes::new(),
        )
        .unwrap();
        let mut persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        persisted.as_object_mut().unwrap().remove("agent_start");
        persisted["issue_settings"]["default_agent"] = serde_json::json!("codex");
        fs::write(&path, serde_json::to_vec(&persisted).unwrap()).unwrap();

        let (state, _, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(state.agent_start.kind.as_deref(), Some("codex"));
        assert!(state.agent_start.models.is_empty());

        save(&path, &state, &PaneTerminalSizes::new()).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(rewritten["issue_settings"].get("default_agent").is_none());
        assert_eq!(rewritten["agent_start"]["kind"], "codex");

        // Once the choice is stored, an old field no longer decides it.
        let mut again = rewritten;
        again["issue_settings"]["default_agent"] = serde_json::json!("claude");
        fs::write(&path, serde_json::to_vec(&again).unwrap()).unwrap();
        assert_eq!(load(&path).0.agent_start.kind.as_deref(), Some("codex"));
    }

    #[test]
    fn read_records_survive_a_restart() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let mut state = UiStateSnapshot::default();
        state.pane_read_records.insert(
            "w1:p1".to_owned(),
            PaneReadRecord {
                state_change_seq: Some(7),
                session_id: Some("session-7".to_owned()),
                demand: "question".to_owned(),
                activity: "stopped".to_owned(),
                completed: false,
                descendant_signals: [crate::model::DescendantSignal {
                    pane_id: "w1:p2".to_owned(),
                    kind: "question".to_owned(),
                }]
                .into_iter()
                .collect(),
                server: Some("server-7".to_owned()),
                unread: true,
                checks_ended: None,
            },
        );
        state.pane_completions.insert(
            "w1:p1".to_owned(),
            crate::model::PaneCompletion {
                session: Some("session-7".to_owned()),
                server: Some("server-7".to_owned()),
                state_change_seq: Some(7),
            },
        );
        // How long each row has waited survives too (PRD overview-request-view D-40).
        state.request_verbs.insert(
            "w1:p1".to_owned(),
            crate::request_view::VerbRecord {
                verb: crate::agent_state::RequestVerb::Answer,
                since_unix_ms: 1_790_000_000_000,
                result_opened_unix_ms: Some(1_790_000_100_000),
            },
        );
        save(&path, &state, &PaneTerminalSizes::new()).expect("state saves");

        let (reloaded, _sizes, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(reloaded.pane_read_records, state.pane_read_records);
        assert_eq!(reloaded.pane_completions, state.pane_completions);
        assert_eq!(reloaded.request_verbs, state.request_verbs);
    }

    #[test]
    fn read_records_written_before_session_identity_still_load() {
        let mut persisted = serde_json::to_value(UiStateSnapshot::default()).unwrap();
        persisted["pane_read_records"] = serde_json::json!({
            "w1:p1": {
                "state_change_seq": 41,
                "demand": "none",
                "activity": "stopped"
            }
        });

        let state: UiStateSnapshot = serde_json::from_value(persisted).unwrap();
        assert_eq!(state.pane_read_records["w1:p1"].session_id, None);
    }

    /// AC4. A store written before Hide owned the read axis still loads. Its
    /// missing record reads as everything unread, which is the honest answer,
    /// rather than discarding the operator's other settings.
    #[test]
    fn read_records_default_to_empty_on_an_older_store() {
        let source = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#;
        let (state, _sizes, disposition) = decode(source);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(state.pane_read_records.is_empty());
    }

    /// A store written before Rename was recorded names no tab as the
    /// operator's, so no tab outlives its agents until it is renamed again;
    /// a recorded name survives a save and a load.
    #[test]
    fn operator_tab_names_load_empty_from_an_older_store_and_survive_a_save() {
        let older = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#;
        let (mut state, _sizes, disposition) = decode(older);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(state.operator_tab_names.is_empty());

        state
            .operator_tab_names
            .insert("w1:t1".to_owned(), "Planning".to_owned());
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        save(&path, &state, &PaneTerminalSizes::new()).expect("state saves");
        let (reloaded, _sizes, _) = load(&path);
        assert_eq!(reloaded.operator_tab_names, state.operator_tab_names);
    }

    #[test]
    fn a_stored_sessions_mode_from_an_older_build_loads_and_is_not_saved_again() {
        // The Overview's Sessions/Memory switch is gone (PRD
        // project-memory-v2): its per-Project choice is read past and dropped.
        let older = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null,"sessions_mode_by_project":{"project-1":"memory"}}"#;
        let (state, _sizes, disposition) = decode(older);
        assert_eq!(disposition, LoadDisposition::Loaded);

        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        save(&path, &state, &PaneTerminalSizes::new()).expect("state saves");
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("sessions_mode_by_project")
        );
    }

    /// AC4, SC3. A damaged store loads empty and says so through the
    /// disposition the runtime turns into a diagnostic. Nothing is silently
    /// treated as read.
    #[test]
    fn read_records_load_empty_from_a_corrupt_store() {
        let (state, _sizes, disposition) =
            decode(b"{\"schema_version\":1,\"pane_read_records\":\"not-a-map\"}");
        assert_eq!(disposition, LoadDisposition::Corrupt);
        assert!(
            state.pane_read_records.is_empty(),
            "a damaged record is emptied, never read"
        );
    }

    #[test]
    fn valid_state_round_trips_through_the_stable_schema() {
        let source = br#"{"schema_version":1,"expanded_paths":["/repo/src"],"selected_path":"/repo/src/lib.rs","selected_pane_id":"p1"}"#;
        let (state, _sizes, disposition) = decode(source);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(state.expanded_paths, ["/repo/src"]);
        assert_eq!(state.selected_pane_id.as_deref(), Some("p1"));
        assert!(state.left_sidebar_visible);
        assert!(
            state.device_rail_visible,
            "a pre-rail-toggle store still shows the rail"
        );
        assert!(state.right_panel_visible);
        assert!(state.shortcut_bindings.is_empty());
        assert!(state.pet_visible, "a pre-pet store still shows the pet");
        assert_eq!(state.pet_origin, None);
        assert_eq!(state.pet_shortcut, None);
    }

    /// A moved store's hint stays until a session takes it, and an ordinary
    /// store never carries the key.
    #[test]
    fn a_moved_stores_focus_hint_is_kept_until_taken_and_never_written_otherwise() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        let moved = UiStateSnapshot {
            focus_from_herdr: true,
            ..UiStateSnapshot::default()
        };
        save(&path, &moved, &PaneTerminalSizes::new()).unwrap();
        assert!(load(&path).0.focus_from_herdr);
        save(
            &path,
            &UiStateSnapshot::default(),
            &PaneTerminalSizes::new(),
        )
        .unwrap();
        assert!(
            !String::from_utf8(fs::read(&path).unwrap())
                .unwrap()
                .contains("focus_from_herdr")
        );
        assert!(!load(&path).0.focus_from_herdr);
    }

    #[test]
    fn panel_visibility_survives_save_and_relaunch_load() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let state = UiStateSnapshot {
            left_sidebar_visible: false,
            device_rail_visible: false,
            right_panel_visible: false,
            ..UiStateSnapshot::default()
        };

        save(&path, &state, &PaneTerminalSizes::new()).expect("persist panel visibility");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(!restored.left_sidebar_visible);
        assert!(!restored.device_rail_visible);
        assert!(!restored.right_panel_visible);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn pet_position_visibility_and_shortcut_survive_a_relaunch() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let mut state = UiStateSnapshot {
            pet_visible: false,
            pet_origin: Some(PetOriginSnapshot { x: 120.0, y: 640.0 }),
            pet_shortcut: Some("command+option+p".to_owned()),
            ..UiStateSnapshot::default()
        };

        save(&path, &state, &PaneTerminalSizes::new()).expect("persist pet state");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(
            !restored.pet_visible,
            "hidden at exit means hidden at start"
        );
        assert_eq!(
            restored.pet_origin,
            Some(PetOriginSnapshot { x: 120.0, y: 640.0 })
        );
        assert_eq!(restored.pet_shortcut.as_deref(), Some("command+option+p"));

        // Saving the same state twice is a no-op the next load cannot tell apart.
        state.pet_visible = false;
        save(&path, &state, &PaneTerminalSizes::new()).expect("persist pet state again");
        assert_eq!(load(&path).0.pet_origin, restored.pet_origin);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn native_app_shortcuts_read_only_the_bindings_field() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        assert_eq!(
            read_native_app_shortcuts(&path),
            NativeAppShortcuts::Missing
        );

        // That store carries fields the core's own schema would refuse.
        fs::write(
            &path,
            br#"{"schema_version":7,"pet_visible":"yes","shortcut_bindings":{"toggle_zoom":"command+shift+return"}}"#,
        )
        .unwrap();
        assert_eq!(
            read_native_app_shortcuts(&path),
            NativeAppShortcuts::Found(BTreeMap::from([(
                "toggle_zoom".to_owned(),
                "command+shift+return".to_owned()
            )]))
        );

        fs::write(&path, br#"{"shortcut_bindings":["not","a","map"]}"#).unwrap();
        assert_eq!(
            read_native_app_shortcuts(&path),
            NativeAppShortcuts::Unreadable
        );
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn shortcut_bindings_survive_save_and_relaunch_load() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let mut state = UiStateSnapshot::default();
        state
            .shortcut_bindings
            .insert("split_right".to_owned(), "command+option+r".to_owned());
        state
            .browser_shortcut_bindings
            .insert("split_right".to_owned(), "alt+KeyR".to_owned());
        state.shortcut_bindings_imported = true;

        save(&path, &state, &PaneTerminalSizes::new()).expect("persist shortcut binding");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert!(restored.shortcut_bindings_imported);
        assert_eq!(
            restored
                .shortcut_bindings
                .get("split_right")
                .map(String::as_str),
            Some("command+option+r")
        );
        assert_eq!(
            restored
                .browser_shortcut_bindings
                .get("split_right")
                .map(String::as_str),
            Some("alt+KeyR")
        );
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn workspace_collapse_survives_relaunch_without_changing_file_tree_expansion() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let mut state = UiStateSnapshot {
            expanded_paths: vec!["/repo/src".to_owned()],
            collapsed_workspace_ids: vec!["workspace:alpha".to_owned()],
            collapsed_checkout_ids: vec!["checkout:main".to_owned()],
            expanded_checkout_ids: vec!["checkout:feature".to_owned()],
            ..UiStateSnapshot::default()
        };

        save(&path, &state, &PaneTerminalSizes::new())
            .expect("persist independent expansion state");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(restored.expanded_paths, ["/repo/src"]);
        assert_eq!(restored.collapsed_workspace_ids, ["workspace:alpha"]);
        assert_eq!(restored.collapsed_checkout_ids, ["checkout:main"]);
        assert_eq!(restored.expanded_checkout_ids, ["checkout:feature"]);

        state.expanded_paths.push("/repo/tests".to_owned());
        save(&path, &state, &PaneTerminalSizes::new())
            .expect("persist file tree expansion independently");
        let restored_again = load(&path).0;
        assert_eq!(restored_again.expanded_paths, ["/repo/src", "/repo/tests"]);
        assert_eq!(restored_again.collapsed_workspace_ids, ["workspace:alpha"]);
        assert_eq!(restored_again.collapsed_checkout_ids, ["checkout:main"]);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    /// B5, B12. Fold disclosure is independent at the project and device
    /// levels, survives a relaunch, and an older store defaults both closed.
    #[test]
    fn inactive_fold_expansion_survives_relaunch_and_defaults_closed() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let state = UiStateSnapshot {
            expanded_inactive_checkout_project_paths: vec!["/repo/alpha".to_owned()],
            expanded_inactive_project_device_ids: vec!["local".to_owned()],
            ..UiStateSnapshot::default()
        };

        save(&path, &state, &PaneTerminalSizes::new()).expect("persist inactive fold expansion");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(
            restored.expanded_inactive_checkout_project_paths,
            ["/repo/alpha"]
        );
        assert_eq!(restored.expanded_inactive_project_device_ids, ["local"]);

        let legacy = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#;
        let (legacy_state, _sizes, legacy_disposition) = decode(legacy);
        assert_eq!(legacy_disposition, LoadDisposition::Loaded);
        assert!(
            legacy_state
                .expanded_inactive_checkout_project_paths
                .is_empty()
        );
        assert!(legacy_state.expanded_inactive_project_device_ids.is_empty());

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    /// B5. A pinned registration comes back pinned, and a store written before
    /// registrations carried the flag loads every project unpinned with no
    /// warning.
    #[test]
    fn registration_pin_survives_relaunch_and_an_older_store_loads_unpinned() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.join("state.json");
        let state = UiStateSnapshot {
            workspace_registrations: vec![WorkspaceRegistration {
                primary_checkout_id: None,
                id: "workspace:alpha".to_owned(),
                label: "Alpha".to_owned(),
                path: "/repo/alpha".to_owned(),
                device_id: "local".to_owned(),
                pinned: true,
                home: false,
            }],
            ..UiStateSnapshot::default()
        };

        save(&path, &state, &PaneTerminalSizes::new()).expect("persist the pin");
        let (restored, _sizes, disposition) = load(&path);

        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(
            restored.workspace_registrations,
            state.workspace_registrations
        );

        let older = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null,
            "workspace_registrations":[{"id":"workspace:alpha","label":"Alpha","path":"/repo/alpha","device_id":"test-node"}]}"#;
        let (older_state, _sizes, older_disposition) = decode(older);
        assert_eq!(older_disposition, LoadDisposition::Loaded);
        assert_eq!(older_state.workspace_registrations.len(), 1);
        assert!(!older_state.workspace_registrations[0].pinned);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(root);
    }

    /// PRD overview-graph-remembered B1-B6, B15: the Overview map's choices
    /// survive a relaunch; a stored record this build cannot read is left
    /// out, the rest of the map's choices and of the UI state load, and the
    /// load says so; an unreadable value loads the defaults.
    #[test]
    fn the_overview_map_s_choices_survive_a_relaunch_and_an_unreadable_record_alone_is_dropped() {
        use crate::model::OverviewGraphChange;
        use crate::model::overview_graph_choices::OverviewGraphFocus;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        let mut state = UiStateSnapshot {
            task_view: TaskView::Project,
            ..UiStateSnapshot::default()
        };
        for change in [
            serde_json::json!({"kind": "focus", "focus": "results", "include_resting": true}),
            serde_json::json!({"kind": "project_hidden", "project": {"device_id": "n", "workspace_id": "workspace:a"}, "hidden": true}),
            serde_json::json!({"kind": "project_folded", "project": {"device_id": "n", "workspace_id": "workspace:b"}, "folded": true}),
            serde_json::json!({"kind": "team_folded", "session": {"device_id": "n", "pane_id": "w1:p1", "session": "d"}, "folded": false}),
            serde_json::json!({"kind": "place", "scope": {"kind": "all", "device_id": "n"},
                "projects": [{"key": {"device_id": "n", "workspace_id": "workspace:b"}, "x": 120.5, "y": -40.0}],
                "sessions": [{"key": {"device_id": "n", "pane_id": "w1:p1", "session": "d"}, "x": 10.0, "y": 20.0}]}),
        ] {
            let change: OverviewGraphChange = serde_json::from_value(change).unwrap();
            state.overview_graph.apply(change).unwrap();
        }
        save(&path, &state, &PaneTerminalSizes::new()).expect("persist the map's choices");
        let (restored, _sizes, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(restored.overview_graph, state.overview_graph);
        assert_eq!(restored.overview_graph.focus, OverviewGraphFocus::Results);

        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["overview_graph"]["hidden_projects"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"device_id": "n"}));
        let (partial, _sizes, disposition) = decode(&serde_json::to_vec(&stored).unwrap());
        assert_eq!(
            disposition,
            LoadDisposition::Unrecognized(vec![UnrecognizedField::OverviewGraph])
        );
        assert_eq!(partial.overview_graph, state.overview_graph);
        assert_eq!(partial.task_view, TaskView::Project);

        stored["overview_graph"] = serde_json::json!("not a map");
        let (unreadable, _sizes, disposition) = decode(&serde_json::to_vec(&stored).unwrap());
        assert_eq!(
            disposition,
            LoadDisposition::Unrecognized(vec![UnrecognizedField::OverviewGraph])
        );
        assert_eq!(unreadable.overview_graph, Default::default());
        assert_eq!(unreadable.task_view, TaskView::Project);
    }

    /// PRD sidebar-task-lens B3, D-25, D-48, D-50: the sidebar's lens, the
    /// 태스크 view option and the 보기 slot survive a relaunch; a store
    /// written before them opens on the projects, by state, with an empty
    /// slot; a value this build does not know loads the same way and says so.
    #[test]
    fn the_lens_view_option_and_detail_slot_survive_a_relaunch() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        let mut state = UiStateSnapshot {
            sidebar_lens: SidebarLens::Tasks,
            task_view: TaskView::Project,
            sidebar_project: Some(SidebarProject {
                device_id: "mini".to_owned(),
                workspace_id: "remote:mini:project:p".to_owned(),
            }),
            sessions_expanded_agent_pane_ids: vec!["w1:p1".to_owned()],
            ..UiStateSnapshot::default()
        };
        state.detail.open(crate::model::DetailItem::PullRequest {
            workspace_id: "workspace:alpha".to_owned(),
            number: 7,
        });
        save(&path, &state, &PaneTerminalSizes::new()).expect("persist the lens");
        let (restored, _sizes, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(restored.sidebar_lens, SidebarLens::Tasks);
        assert_eq!(restored.task_view, TaskView::Project);
        assert_eq!(restored.sidebar_project, state.sidebar_project);
        assert_eq!(restored.sessions_expanded_agent_pane_ids, ["w1:p1"]);
        assert_eq!(restored.detail, state.detail);

        let older = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#;
        let (older_state, _sizes, older_disposition) = decode(older);
        assert_eq!(older_disposition, LoadDisposition::Loaded);
        assert_eq!(older_state.sidebar_lens, SidebarLens::Projects);
        assert_eq!(older_state.task_view, TaskView::Status);
        assert_eq!(older_state.sidebar_project, None);
        assert_eq!(older_state.detail, DetailSlot::default());

        let unknown = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null,
            "sidebar_lens":"sessions","task_view":"status","detail":{"items":[{"kind":"tab"}],"shown":true}}"#;
        let (unknown_state, _sizes, unknown_disposition) = decode(unknown);
        assert_eq!(
            unknown_disposition,
            LoadDisposition::Unrecognized(vec![
                UnrecognizedField::SidebarLens,
                UnrecognizedField::Detail
            ])
        );
        assert_eq!(unknown_state.sidebar_lens, SidebarLens::Projects);
        assert_eq!(unknown_state.detail, DetailSlot::default());
    }

    /// Practice-mode progress survives a relaunch; a store written before it
    /// loads empty, and an entry this build cannot read is dropped while the
    /// rest of the store, and the other chapters, still load.
    #[test]
    fn practice_progress_survives_a_relaunch_and_a_bad_entry_is_dropped() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        let mut state = UiStateSnapshot {
            task_view: TaskView::Project,
            ..UiStateSnapshot::default()
        };
        state.practice.insert(
            "first-agent".to_owned(),
            PracticeProgress {
                version: 2,
                step: 5,
                finished: false,
            },
        );
        state.practice.insert(
            "tour".to_owned(),
            PracticeProgress {
                version: 1,
                step: 255,
                finished: true,
            },
        );
        state.practice_invite_closed = true;
        save(&path, &state, &PaneTerminalSizes::new()).expect("persist the progress");
        let (restored, _sizes, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(restored.practice, state.practice);
        assert!(restored.practice_invite_closed);

        let older = br#"{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#;
        let (older_state, _sizes, older_disposition) = decode(older);
        assert_eq!(older_disposition, LoadDisposition::Loaded);
        assert!(older_state.practice.is_empty());
        assert!(!older_state.practice_invite_closed);

        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["practice"]["broken"] = serde_json::json!({"version": "two"});
        stored["practice"]["Bad Id"] =
            serde_json::json!({"version": 1, "step": 0, "finished": false});
        stored["practice"]["far"] =
            serde_json::json!({"version": 1, "step": 256, "finished": false});
        stored["practice"]["extra"] =
            serde_json::json!({"version": 1, "step": 0, "finished": false, "later": 1});
        let (partial, _sizes, disposition) = decode(&serde_json::to_vec(&stored).unwrap());
        assert_eq!(
            disposition,
            LoadDisposition::Unrecognized(vec![UnrecognizedField::Practice])
        );
        assert_eq!(partial.practice, state.practice);
        assert_eq!(partial.task_view, TaskView::Project);

        stored["practice"] = serde_json::json!(["not", "a", "map"]);
        let (unreadable, _sizes, disposition) = decode(&serde_json::to_vec(&stored).unwrap());
        assert_eq!(
            disposition,
            LoadDisposition::Unrecognized(vec![UnrecognizedField::Practice])
        );
        assert!(unreadable.practice.is_empty());
        assert_eq!(unreadable.task_view, TaskView::Project);

        let many: serde_json::Map<String, serde_json::Value> = (0..65)
            .map(|index| {
                (
                    format!("c{index:02}"),
                    serde_json::json!({"version": 1, "step": 0, "finished": false}),
                )
            })
            .collect();
        stored["practice"] = serde_json::Value::Object(many);
        let (capped, _sizes, disposition) = decode(&serde_json::to_vec(&stored).unwrap());
        assert_eq!(
            disposition,
            LoadDisposition::Unrecognized(vec![UnrecognizedField::Practice])
        );
        assert_eq!(capped.practice.len(), PRACTICE_CHAPTER_LIMIT);
    }

    /// Settings › Conductor survives a relaunch, and a value this build
    /// cannot read (a later build's field, a cap out of range) loads the
    /// defaults and says so while every other stored choice still loads.
    #[test]
    fn conductor_settings_this_build_cannot_read_load_the_defaults_and_keep_the_rest() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        let mut state = UiStateSnapshot::default();
        state.conductor_settings.concurrency = 6;
        save(&path, &state, &PaneTerminalSizes::new()).expect("persist the settings");
        let (restored, _sizes, disposition) = load(&path);
        assert_eq!(disposition, LoadDisposition::Loaded);
        assert_eq!(restored.conductor_settings.concurrency, 6);

        for settings in [
            r#"{"concurrency":3,"later_field":true}"#,
            r#"{"concurrency":99}"#,
        ] {
            let stored = format!(
                r#"{{"schema_version":1,"expanded_paths":[],"selected_path":null,"selected_pane_id":null,
                "sidebar_lens":"tasks","conductor_settings":{settings}}}"#
            );
            let (state, _sizes, disposition) = decode(stored.as_bytes());
            assert_eq!(
                disposition,
                LoadDisposition::Unrecognized(vec![UnrecognizedField::ConductorSettings]),
                "{settings}"
            );
            assert_eq!(
                state.conductor_settings,
                crate::missions::ConductorSettings::default()
            );
            assert_eq!(state.sidebar_lens, SidebarLens::Tasks);
        }
    }

    #[test]
    fn corrupt_and_unknown_schema_states_fall_back_without_blocking() {
        for source in [
            b"not json".as_slice(),
            br#"{"schema_version":99,"expanded_paths":[],"selected_path":null,"selected_pane_id":null}"#,
        ] {
            let (state, _sizes, disposition) = decode(source);
            assert_eq!(disposition, LoadDisposition::Corrupt);
            assert!(state.expanded_paths.is_empty());
        }
    }
}
