use serde::{Deserialize, Serialize};

pub const FIXTURE_PREFIX: &str = "hide-verify-";

/// An account shell for FakeHerdr journeys. Unix owns an executable file;
/// Windows uses Herdr's PowerShell recipe. The peer never runs the shell.
#[cfg(test)]
pub(crate) fn integrated_shell(home: &std::path::Path) -> hide_host::shell_integration::UserShell {
    #[cfg(unix)]
    let shell = {
        use std::os::unix::fs::PermissionsExt;

        let shell = home.join("bin/zsh");
        std::fs::create_dir_all(shell.parent().unwrap()).unwrap();
        std::fs::write(&shell, "#!/bin/sh\nexit 97\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        shell.to_str().unwrap().to_owned()
    };
    #[cfg(windows)]
    let shell = {
        let _ = home;
        "powershell.exe".to_owned()
    };
    hide_host::shell_integration::UserShell {
        shell: Some(shell),
        ..Default::default()
    }
}

/// Synthetic fixtures live below the canonical OS temp directory. These
/// bucket spellings come from Pi 1.0.4 and omp 18.7.0, independently of the
/// product's locator, so a wrong locator cannot move the expected fixture.
#[cfg(test)]
pub(crate) fn native_session_folder(
    home: &std::path::Path,
    kind: &str,
    cwd: &std::path::Path,
) -> std::path::PathBuf {
    let encode = |path: &std::path::Path| {
        path.to_string_lossy()
            .trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    };
    let bucket = match kind {
        "pi" => format!("--{}--", encode(cwd)),
        "omp" => {
            let temporary = std::env::temp_dir().canonicalize().unwrap();
            let relative = cwd.strip_prefix(temporary).unwrap();
            if relative.as_os_str().is_empty() {
                "-tmp".into()
            } else {
                format!("-tmp-{}", encode(relative))
            }
        }
        _ => panic!("not a native-file fixture"),
    };
    home.join(format!(".{kind}/agent/sessions")).join(bucket)
}

/// A Grok 1.0.46 session folder with its id-owning summary and an empty
/// conversation, at the group Grok names by URL-encoding the cwd.
#[cfg(test)]
pub(crate) fn grok_session(
    home: &std::path::Path,
    cwd: &std::path::Path,
    id: &str,
) -> std::path::PathBuf {
    let group: String = cwd
        .to_str()
        .unwrap()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let folder = home.join(".grok/sessions").join(group).join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("summary.json"),
        serde_json::json!({"info": {"id": id, "cwd": cwd}, "session_summary": "",
            "created_at": "2026-10-03T01:00:00Z", "updated_at": "2026-10-03T01:00:00Z",
            "num_messages": 0, "current_model_id": "grok-build"})
        .to_string(),
    )
    .unwrap();
    let conversation = folder.join("updates.jsonl");
    std::fs::write(&conversation, "").unwrap();
    conversation
}

/// One root conversation row in agy 1.4.0's index at `step_count` steps,
/// written at `modified` (SQLite time text; `None` is now), with the columns its readers
/// look at; the index is made on the first call.
#[cfg(test)]
pub(crate) fn antigravity_conversation(
    home: &std::path::Path,
    cwd: &std::path::Path,
    id: &str,
    step_count: i64,
    modified: Option<&str>,
) {
    let folder = home.join(".gemini/antigravity-cli");
    std::fs::create_dir_all(folder.join("conversations")).unwrap();
    std::fs::write(folder.join("conversations").join(format!("{id}.db")), b"").unwrap();
    let index = rusqlite::Connection::open(folder.join("conversation_summaries.db")).unwrap();
    index
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS conversation_summaries (conversation_id text PRIMARY KEY, \
             title text NOT NULL DEFAULT '', step_count integer NOT NULL DEFAULT 0, \
             last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, \
             parent_conversation_id text NOT NULL DEFAULT '', \
             app_data_dir text NOT NULL DEFAULT '')",
        )
        .unwrap();
    index
        .execute(
            "INSERT INTO conversation_summaries (conversation_id, title, step_count, \
             last_modified_time, workspace_uris, app_data_dir) \
             VALUES (?1, 'fork fixture', ?2, \
             coalesce(?3, strftime('%Y-%m-%d %H:%M:%S+00:00', 'now')), ?4, 'antigravity-cli')",
            rusqlite::params![
                id,
                step_count,
                modified,
                serde_json::json!([antigravity_uri(cwd)]).to_string()
            ],
        )
        .unwrap();
}

/// How agy names a workspace: `file:///Users/me/repo`, or on Windows
/// `file:///C:/Users/me/repo`.
#[cfg(test)]
fn antigravity_uri(cwd: &std::path::Path) -> String {
    let wire = hide_platform::path::to_wire(cwd).unwrap();
    let slash = if wire.starts_with('/') { "" } else { "/" };
    format!("file://{slash}{}", wire.replace(' ', "%20"))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FixturePlan {
    pub workspace_name: String,
    pub branch_name: String,
    pub pane_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ScalePlan {
    pub schema_version: u32,
    pub ownership_prefix: String,
    pub target: String,
    pub cwd: String,
    pub workspace_names: Vec<String>,
    pub requested_workspace_count: usize,
    pub requested_pane_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FixtureWorkspaceRecord {
    pub workspace_name: String,
    pub workspace_id: String,
    pub pane_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FixtureManifest {
    pub schema_version: u32,
    pub ownership_prefix: String,
    pub target: String,
    pub cwd: String,
    pub workspaces: Vec<FixtureWorkspaceRecord>,
}

pub fn plan(name: &str) -> Result<FixturePlan, String> {
    validate_owned_name(name)?;
    Ok(FixturePlan {
        workspace_name: name.to_owned(),
        branch_name: format!("verify/{name}"),
        pane_label: format!("{name}-long-running"),
    })
}

pub fn plan_scale(
    name: &str,
    target: &str,
    cwd: &str,
    workspace_count: usize,
    pane_count: usize,
) -> Result<ScalePlan, String> {
    validate_owned_name(name)?;
    if !matches!(target, "local" | "mini") {
        return Err("Fixture target must be local or mini".to_owned());
    }
    validate_fixture_cwd(cwd)?;
    if cwd != format!("/tmp/{name}") {
        return Err("Fixture cwd must exactly match the fixture name under /tmp".to_owned());
    }
    if workspace_count == 0 || workspace_count > 32 {
        return Err("Fixture workspace count must be between 1 and 32".to_owned());
    }
    if pane_count < workspace_count || pane_count > 64 {
        return Err(
            "Fixture pane count must be at least the workspace count and at most 64".to_owned(),
        );
    }
    let workspace_names = (1..=workspace_count)
        .map(|index| format!("{name}-w{index:02}"))
        .collect();
    Ok(ScalePlan {
        schema_version: 1,
        ownership_prefix: FIXTURE_PREFIX.to_owned(),
        target: target.to_owned(),
        cwd: cwd.to_owned(),
        workspace_names,
        requested_workspace_count: workspace_count,
        requested_pane_count: pane_count,
    })
}

pub fn validate_manifest(manifest: &FixtureManifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.ownership_prefix != FIXTURE_PREFIX {
        return Err("Fixture manifest ownership contract does not match".to_owned());
    }
    if !matches!(manifest.target.as_str(), "local" | "mini") {
        return Err("Fixture manifest target must be local or mini".to_owned());
    }
    validate_fixture_cwd(&manifest.cwd)?;
    if manifest.workspaces.is_empty() {
        return Err("Fixture manifest must contain at least one workspace".to_owned());
    }
    for workspace in &manifest.workspaces {
        validate_owned_name(&workspace.workspace_name)?;
        if workspace.workspace_id.trim().is_empty() || workspace.pane_ids.is_empty() {
            return Err("Fixture workspace records require ids and panes".to_owned());
        }
    }
    Ok(())
}

pub fn validate_owned_name(name: &str) -> Result<(), String> {
    if !name.starts_with(FIXTURE_PREFIX) {
        return Err(format!("Fixture names must begin with {FIXTURE_PREFIX}"));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(
            "Fixture names may contain lowercase ASCII, digits, and hyphens only".to_owned(),
        );
    }
    Ok(())
}

fn validate_fixture_cwd(cwd: &str) -> Result<(), String> {
    let Some(suffix) = cwd.strip_prefix("/tmp/hide-verify-") else {
        return Err("Fixture cwd must be an owned path under /tmp".to_owned());
    };
    if suffix.is_empty()
        || !suffix.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err("Fixture cwd may contain lowercase ASCII, digits, and hyphens only".to_owned());
    }
    Ok(())
}

/// An OpenCode 1.18.30 database in `home` holding `sessions` as
/// `(id, parent_id, directory)`, each with one operator message: the shape
/// its own `session`, `message` and `part` tables have.
#[cfg(test)]
pub(crate) fn opencode_database(home: &std::path::Path, sessions: &[(&str, Option<&str>, &str)]) {
    let folder = home.join(".local/share/opencode");
    std::fs::create_dir_all(&folder).unwrap();
    let database = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
    database
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS session (id text PRIMARY KEY, project_id text NOT NULL, \
             parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, \
             version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
             CREATE TABLE IF NOT EXISTS message (id text PRIMARY KEY, session_id text NOT NULL, \
             time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
             CREATE TABLE IF NOT EXISTS part (id text PRIMARY KEY, message_id text NOT NULL, \
             session_id text NOT NULL, time_created integer NOT NULL, \
             time_updated integer NOT NULL, data text NOT NULL);",
        )
        .unwrap();
    for (index, (id, parent, directory)) in sessions.iter().enumerate() {
        let at = 1_790_989_200_000_i64 + index as i64;
        database
            .execute(
                "INSERT INTO session VALUES (?1, 'prj_1', ?2, 'slug', ?3, 'OpenCode title', \
                 '1.18.30', ?4, ?4)",
                rusqlite::params![id, parent, directory, at],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![
                    format!("msg_{id}"),
                    id,
                    at,
                    format!(r#"{{"role":"user","time":{{"created":{at}}}}}"#)
                ],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![
                    format!("prt_{id}"),
                    format!("msg_{id}"),
                    id,
                    at,
                    r#"{"type":"text","text":"OpenCode request"}"#
                ],
            )
            .unwrap();
    }
}

/// An OpenCode 2.0.29 database in `home` holding `sessions` as
/// `(id, parent_id, directory)`, each with one operator message: the shape
/// its own `session_v2` and `session_message` tables have.
#[cfg(test)]
pub(crate) fn opencode_2_database(home: &std::path::Path, sessions: &[(&str, Option<&str>, &str)]) {
    let folder = home.join(".local/share/opencode");
    std::fs::create_dir_all(&folder).unwrap();
    let database = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
    database
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS session_v2 (id text PRIMARY KEY, project_id text NOT NULL, \
             parent_id text, slug text NOT NULL, directory text NOT NULL, title text, \
             version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
             CREATE TABLE IF NOT EXISTS session_message (id text PRIMARY KEY, session_id text NOT NULL, \
             type text NOT NULL, seq integer NOT NULL, time_created integer NOT NULL, \
             time_updated integer NOT NULL, data text NOT NULL);",
        )
        .unwrap();
    for (index, (id, parent, directory)) in sessions.iter().enumerate() {
        let at = 1_790_989_200_000_i64 + index as i64;
        database
            .execute(
                "INSERT INTO session_v2 VALUES (?1, 'prj_1', ?2, 'slug', ?3, 'OpenCode title', \
                 '2.0.29', ?4, ?4)",
                rusqlite::params![id, parent, directory, at],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO session_message VALUES (?1, ?2, 'user', 1, ?3, ?3, ?4)",
                rusqlite::params![
                    format!("msg_{id}"),
                    id,
                    at,
                    format!(r#"{{"time":{{"created":{at}}},"text":"OpenCode request"}}"#)
                ],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_fixture_plan_is_deterministic_and_idempotent() {
        let first = plan("hide-verify-shell").unwrap();
        let second = plan("hide-verify-shell").unwrap();
        assert_eq!(first, second);
        assert!(first.branch_name.starts_with("verify/hide-verify-"));
    }

    #[test]
    fn non_prefixed_targets_are_rejected_before_any_runtime_operation() {
        assert!(plan("user-workspace").is_err());
    }

    #[test]
    fn scale_plan_has_exact_requested_counts_and_prefixes() {
        let planned = plan_scale(
            "hide-verify-scale",
            "local",
            "/tmp/hide-verify-scale",
            7,
            11,
        )
        .unwrap();
        assert_eq!(planned.workspace_names.len(), 7);
        assert_eq!(planned.requested_pane_count, 11);
        assert!(
            planned
                .workspace_names
                .iter()
                .all(|name| name.starts_with(FIXTURE_PREFIX))
        );
    }

    #[test]
    fn manifest_validation_fails_closed_before_cleanup_for_unowned_names() {
        let manifest = FixtureManifest {
            schema_version: 1,
            ownership_prefix: FIXTURE_PREFIX.to_owned(),
            target: "local".to_owned(),
            cwd: "/tmp/fixture".to_owned(),
            workspaces: vec![FixtureWorkspaceRecord {
                workspace_name: "user-workspace".to_owned(),
                workspace_id: "w1".to_owned(),
                pane_ids: vec!["w1:p1".to_owned()],
            }],
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn remote_fixture_paths_reject_shell_syntax_before_process_launch() {
        assert!(
            plan_scale(
                "hide-verify-safe",
                "mini",
                "/tmp/hide-verify-safe;touch-pwned",
                1,
                1,
            )
            .is_err()
        );
        assert!(plan_scale("hide-verify-safe", "mini", "/tmp/hide-verify-safe", 1, 1,).is_ok());
    }
}
