//! What paths are on this node, as the core's catalog reads them
//! (`hide_node_link::catalog`). Every fact is read from files: the catalog is
//! rebuilt on the session-sync coordinator, where a process would hold every
//! Herdr event behind it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hide_node_link::catalog::{
    PATH_FACTS_ANSWER_LIMIT, PATH_FACTS_LIMIT, PathFact, PathFacts, RepositoryPlace,
};
use hide_node_link::gh::repository_of_remote;
use hide_platform::fs::identity;
use hide_platform::path;

use crate::error::{ErrorCode, HostError, HostResult};

/// Reads each of `paths`; a path that is not absolute is answered as one
/// that does not exist, spelled as it was asked.
pub fn path_facts(paths: &[String]) -> HostResult<PathFacts> {
    path_facts_within(paths, PATH_FACTS_ANSWER_LIMIT)
}

fn path_facts_within(paths: &[String], answer_limit: usize) -> HostResult<PathFacts> {
    if paths.len() > PATH_FACTS_LIMIT {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            format!(
                "{} paths were asked at once; a node answers at most {PATH_FACTS_LIMIT}",
                paths.len()
            ),
        ));
    }
    let mut facts = PathFacts::default();
    let mut bytes = 0_usize;
    for asked in paths {
        if facts.paths.contains_key(asked) {
            continue;
        }
        let native = Path::new(asked);
        let fact = if native.is_absolute() {
            read(native, &mut facts, &mut bytes)
        } else {
            PathFact {
                comparison: asked.clone(),
                exists: false,
                project: None,
                repository: None,
            }
        };
        bytes = bytes.saturating_add(asked.len() + fact_bytes(&fact));
        if bytes > answer_limit {
            return Err(HostError::new(
                ErrorCode::TooLarge,
                format!(
                    "The facts of {} paths pass the {answer_limit}-byte answer limit",
                    paths.len()
                ),
            ));
        }
        facts.paths.insert(asked.clone(), fact);
    }
    Ok(facts)
}

fn fact_bytes(fact: &PathFact) -> usize {
    fact.comparison.len()
        + fact.project.as_ref().map_or(0, |project| {
            project.root.to_string_lossy().len()
                + project.checkout_root.to_string_lossy().len()
                + project.branch.as_ref().map_or(0, String::len)
        })
        + fact.repository.as_ref().map_or(0, |place| {
            place.root.len()
                + place.main_root.len()
                + place.branch.as_ref().map_or(0, String::len)
                + place.head_oid.as_ref().map_or(0, String::len)
        })
}

fn notes_bytes(
    notes: &Result<BTreeMap<String, hide_node_link::catalog::BranchNote>, String>,
) -> usize {
    match notes {
        Ok(notes) => notes
            .iter()
            .map(|(branch, note)| {
                branch.len()
                    + note.description.as_ref().map_or(0, String::len)
                    + note.issue.as_ref().map_or(0, String::len)
            })
            .sum(),
        Err(error) => error.len(),
    }
}

/// The repositories `gh` could read from these remote URLs, sorted, or
/// `None` when one names a server in a form `repository_of_remote` does not
/// read (`http://`, `git+ssh://`): what `gh` makes of that remote is
/// unknown, so it is asked. A remote that is a path on this machine names no
/// server and is left out.
fn github_remotes(urls: &[String]) -> Option<Vec<String>> {
    let mut remotes = Vec::new();
    for url in urls {
        match repository_of_remote(url) {
            Some(repository) => remotes.push(repository),
            None if names_local_path(url) => {}
            None => return None,
        }
    }
    remotes.sort();
    remotes.dedup();
    Some(remotes)
}

/// Whether Git reads a remote URL as a local path: `file://`, or no
/// `scheme://` and no `host:` before the first `/` (Git's
/// `url_is_local_not_ssh`), a drive letter (`C:\`) being a path too.
fn names_local_path(url: &str) -> bool {
    let url = url.trim();
    if url.starts_with("file://") {
        return true;
    }
    if url.contains("://") {
        return false;
    }
    let Some(colon) = url.find(':') else {
        return true;
    };
    let drive = colon == 1 && url.as_bytes()[0].is_ascii_alphabetic();
    drive || url.find('/').is_some_and(|slash| slash < colon)
}

/// One path's fact; the first path of a repository also reads its config
/// once, for its branch notes and its remotes.
fn read(native: &Path, facts: &mut PathFacts, bytes: &mut usize) -> PathFact {
    // One walk up to `.git` answers both the Project and the repository. A
    // path whose Project cannot be read, or not carried (the wire spells
    // paths in UTF-8), is answered without one; the other paths still are.
    let (project, repository) = match hide_project::facts_and_repository(native) {
        Ok((project, repository)) => (wire_project(native, project), repository),
        Err(hide_project::ResolveError::MissingPath(_)) => (None, None),
        Err(error) => {
            project_unread(native, &error.to_string());
            (None, None)
        }
    };
    let repository = repository.map(|repository| {
        let main_root = comparison(&repository.main_root());
        if !facts.repositories.contains_key(&main_root) {
            let (notes, remote_urls) = match repository.config() {
                Ok(config) => {
                    let urls = config.remote_urls();
                    (Ok(config.notes), urls)
                }
                Err(error) => (Err(error), None),
            };
            *bytes = bytes.saturating_add(main_root.len() + notes_bytes(&notes));
            facts.repositories.insert(main_root.clone(), notes);
            if let Some(remotes) = remote_urls.as_deref().and_then(github_remotes) {
                *bytes = bytes.saturating_add(remotes.iter().map(String::len).sum::<usize>());
                facts.remotes.insert(main_root.clone(), remotes);
            }
        }
        RepositoryPlace {
            root: comparison(&repository.root),
            main_root,
            branch: repository.branch(),
            head_oid: repository.head_oid(),
        }
    });
    PathFact {
        comparison: comparison(native),
        exists: native.exists(),
        project,
        repository,
    }
}

fn wire_project(
    native: &Path,
    project: hide_project::ProjectFacts,
) -> Option<hide_project::ProjectFacts> {
    if project.root.to_str().is_some() && project.checkout_root.to_str().is_some() {
        return Some(project);
    }
    project_unread(native, "project_path_not_utf8");
    None
}

/// One record on the node's error stream for a path asked about whose
/// Project could not be answered.
fn project_unread(native: &Path, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "catalog",
            "kind": "path_facts.project_unread",
            "path": path::to_wire_lossy(native),
            "reason": reason,
        })
    );
}

/// `path` with its links resolved as far as it exists: the whole path when
/// it is there, its parent and last name when only the parent is, and the
/// path as written otherwise.
pub fn comparison(native: &Path) -> String {
    path::to_wire_lossy(&resolved(native))
}

fn resolved(native: &Path) -> PathBuf {
    if let Ok(real) = identity::canonical(native) {
        return real;
    }
    match (native.parent(), native.file_name()) {
        (Some(parent), Some(name)) => identity::canonical(parent)
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| native.to_path_buf()),
        _ => native.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_path_carries_its_place_and_its_notes_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = identity::canonical(temp.path()).unwrap();
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
        std::fs::write(
            git.join("config"),
            "[branch \"topic\"]\n\tdescription = the purpose\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let src = path::to_wire_lossy(&root.join("src"));
        let gone = path::to_wire_lossy(&root.join("gone"));

        let facts = path_facts(&[src.clone(), gone.clone(), "relative".to_owned()]).unwrap();

        let fact = &facts.paths[&src];
        assert!(fact.exists);
        let place = fact.repository.as_ref().expect("in the repository");
        assert_eq!(place.root, path::to_wire_lossy(&root));
        assert_eq!(place.main_root, place.root);
        assert_eq!(place.branch.as_deref(), Some("topic"));
        let notes = facts.repositories[&place.main_root].as_ref().unwrap();
        assert_eq!(notes["topic"].description.as_deref(), Some("the purpose"));
        assert_eq!(facts.repositories.len(), 1);
        assert!(!facts.paths[&gone].exists);
        assert_eq!(facts.paths[&gone].comparison, gone);
        assert!(!facts.paths["relative"].exists);
        assert!(facts.paths["relative"].repository.is_none());
    }

    /// A repository whose config carries more notes than the answer may
    /// hold refuses the request whole, never a partial answer.
    #[test]
    fn an_answer_past_its_byte_limit_is_refused_as_too_large() {
        let temp = tempfile::tempdir().unwrap();
        let root = identity::canonical(temp.path()).unwrap();
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
        let description = "d".repeat(4096);
        std::fs::write(
            git.join("config"),
            format!("[branch \"topic\"]\n\tdescription = {description}\n"),
        )
        .unwrap();
        let asked = vec![path::to_wire_lossy(&root)];

        assert!(path_facts_within(&asked, 8192).is_ok());
        let refused = path_facts_within(&asked, 4096).unwrap_err();
        assert_eq!(refused.code, ErrorCode::TooLarge);
    }

    /// The repository each remote names reaches the core with the path
    /// facts it already asks for; the URL, which can carry a token, does not.
    #[test]
    fn a_repository_carries_the_repositories_its_remotes_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = identity::canonical(temp.path()).unwrap();
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let asked = vec![path::to_wire_lossy(&root)];
        let remotes = |config: &str| {
            std::fs::write(git.join("config"), config).unwrap();
            path_facts(&asked).unwrap().remotes
        };
        let main_root = path::to_wire_lossy(&root);
        assert_eq!(
            remotes("[core]\n\tbare = false\n"),
            [(main_root.clone(), Vec::new())].into(),
            "no remote at all"
        );
        assert_eq!(
            remotes(concat!(
                "[remote \"origin\"]\n\turl = https://token@github.com/acme/app.git\n",
                "[remote \"mirror\"]\n\turl = /srv/git/app.git\n",
            )),
            [(main_root, vec!["acme/app".to_owned()])].into(),
        );
        assert!(
            remotes("[include]\n\tpath = more\n").is_empty(),
            "an include may name a remote: unknown"
        );
        for url in [
            "http://github.com/acme/app.git",
            "git+ssh://git@github.com/acme/app",
        ] {
            assert!(
                remotes(&format!("[remote \"origin\"]\n\turl = {url}\n")).is_empty(),
                "{url}: a server in a form not read here is left to gh"
            );
        }
    }

    #[test]
    fn a_path_whose_project_cannot_be_read_is_answered_beside_the_others() {
        let temp = tempfile::tempdir().unwrap();
        let root = identity::canonical(temp.path()).unwrap();
        let broken = root.join("broken");
        std::fs::create_dir(&broken).unwrap();
        std::fs::write(broken.join(".git"), "gitdir: nowhere-at-all\n").unwrap();
        let plain = root.join("plain");
        std::fs::create_dir(&plain).unwrap();
        let broken = path::to_wire_lossy(&broken);
        let plain_wire = path::to_wire_lossy(&plain);

        let facts = path_facts(&[broken.clone(), plain_wire.clone()]).unwrap();

        let fact = &facts.paths[&broken];
        assert!(fact.exists);
        assert!(fact.project.is_none() && fact.repository.is_none());
        let project = facts.paths[&plain_wire]
            .project
            .as_ref()
            .expect("the other path keeps its Project");
        assert_eq!(project.kind, hide_project::ProjectKind::Folder);
        assert_eq!(project.root, hide_project::facts(&plain).unwrap().root);
    }

    #[cfg(unix)]
    #[test]
    fn a_project_the_wire_cannot_spell_is_left_out_and_the_answer_still_serializes() {
        use std::os::unix::ffi::OsStrExt;
        let unspellable = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff-project"));
        let project = hide_project::ProjectFacts {
            root: unspellable.clone(),
            checkout_root: unspellable,
            kind: hide_project::ProjectKind::Folder,
            branch: None,
            linked_worktree: false,
        };
        assert!(serde_json::to_string(&project).is_err());
        let kept = wire_project(Path::new("/tmp/asked"), project);
        assert!(kept.is_none());
        let mut facts = PathFacts::default();
        facts.paths.insert(
            "/tmp/asked".into(),
            PathFact {
                comparison: "/tmp/asked".into(),
                exists: true,
                project: kept,
                repository: None,
            },
        );
        assert!(serde_json::to_string(&facts).is_ok());
    }

    #[test]
    fn more_paths_than_the_limit_are_refused() {
        let paths = vec!["/a".to_owned(); PATH_FACTS_LIMIT + 1];
        assert!(path_facts(&paths).is_err());
    }
}
