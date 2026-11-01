//! `hide workspace remove <path> [--delete-branch] [--force]`: removes a
//! worktree of the caller's machine through the delete dialog's own path, so
//! its Herdr workspace and panes close with it. The daemon runs it
//! (`herdr_core::worktree_command`).

use herdr_core::worktree_command::WorktreeRemove;
use serde_json::json;

use crate::env::Env;

pub const USAGE: &str = "hide workspace remove <path> [--delete-branch] [--force]";

/// One parsed command: the path as the caller typed it, resolved against
/// the working directory only when it runs.
#[derive(Debug, Eq, PartialEq)]
pub struct RemoveCli {
    pub path: String,
    pub delete_branch: bool,
    pub force: bool,
}

/// `hide workspace remove …`, the words after `remove`: the path, then the
/// switches in any order.
pub fn parse(args: &[String]) -> Result<RemoveCli, String> {
    let mut path = None;
    let mut delete_branch = false;
    let mut force = false;
    for word in args {
        match word.as_str() {
            "--delete-branch" if !delete_branch => delete_branch = true,
            "--force" if !force => force = true,
            word if path.is_none()
                && !word.is_empty()
                && !word.starts_with('-')
                && !word.chars().any(char::is_control) =>
            {
                path = Some(word.to_owned());
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(RemoveCli {
        path: path.ok_or_else(|| USAGE.to_owned())?,
        delete_branch,
        force,
    })
}

/// The checkout `raw` names on this machine, absolute and spelled as the
/// core spells paths: its real path when it exists, so a link or `/tmp`
/// names the folder Git lists, and the absolute path when it is gone, as a
/// removed folder's leftover workspace is named.
fn wire_path(raw: &str) -> Result<String, String> {
    let absolute = std::path::absolute(raw)
        .map_err(|_| format!("{raw}: Hide cannot resolve this path from the working directory"))?;
    let path = hide_platform::fs::identity::canonical(&absolute).unwrap_or(absolute);
    hide_platform::path::to_wire(&path)
        .map_err(|_| format!("{raw}: Hide cannot spell this folder's path; rename it and retry"))
}

/// Runs the command and prints one line, `{"ok":true,"value":…}` or
/// `{"ok":false,"error":{"code","message"}}` with the step to take, and a
/// non-zero exit for a refusal.
pub fn run(env: &Env, cli: RemoveCli) -> Result<(), String> {
    let refused_with = |code: String, message: &str, answer: &serde_json::Value| {
        println!("{}", crate::cli::refusal_line(&code, message, answer));
        Err(code)
    };
    let command = WorktreeRemove {
        path: wire_path(&cli.path)?,
        delete_branch: cli.delete_branch,
        force: cli.force,
    };
    let answer = crate::workspace_cli::Credential::acquire(env).and_then(|mut credential| {
        crate::workspace_cli::request_worktree_remove(&mut credential, &command)
    });
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"value":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("worktree_remove_result_unknown")
                .to_owned();
            let message = answer["next_action"]
                .as_str()
                .or_else(|| herdr_core::worktree_command::next_action(&code))
                .unwrap_or("Retry; the reason is in Hide's diagnostic log")
                .to_owned();
            refused_with(code, &message, &answer)
        }
        Err(code) => {
            // The daemon's own wait is shorter than this command's, so a
            // timeout here is a removal that may still be running.
            let message = if code == "request_timeout" {
                herdr_core::worktree_command::next_action("worktree_remove_pending")
                    .unwrap_or_default()
            } else {
                crate::cli::request_next_action(&code)
            };
            refused_with(code, message, &json!({}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_path_comes_with_its_switches_in_any_order() {
        assert_eq!(
            parse(&words(&["--force", "../wt", "--delete-branch"])).unwrap(),
            RemoveCli {
                path: "../wt".into(),
                delete_branch: true,
                force: true,
            }
        );
        assert_eq!(
            parse(&words(&["/repo-wt"])).unwrap(),
            RemoveCli {
                path: "/repo-wt".into(),
                delete_branch: false,
                force: false,
            }
        );
        for refused in [
            &[][..],
            &["--force"],
            &["/a", "/b"],
            &["/a", "--force", "--force"],
            &["/a", "--discard"],
        ] {
            assert!(parse(&words(refused)).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn a_relative_path_is_sent_absolute() {
        let folder = tempfile::tempdir().unwrap();
        let real = hide_platform::fs::identity::canonical(folder.path()).unwrap();
        let wire = wire_path(folder.path().to_str().unwrap()).unwrap();
        assert_eq!(wire, hide_platform::path::to_wire(&real).unwrap());
        let gone = real.join("gone");
        assert_eq!(
            wire_path(gone.to_str().unwrap()).unwrap(),
            hide_platform::path::to_wire(&gone).unwrap()
        );
        assert!(hide_platform::path::is_wire_absolute(
            &wire_path("relative/child").unwrap()
        ));
    }
}
