//! What the guardrails read about a path: the checkout that holds it, read
//! from the repository's own files (`hide_project::git`), and the policy
//! project it belongs to. No Git process runs here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Policy, ProjectPolicy};

/// A checkout of a policy project: a Git worktree, or a plain folder
/// project's root (`git` false), whose only protection is the files one.
#[derive(Clone, Debug)]
pub(super) struct Checkout {
    /// The checkout's own root.
    pub root: PathBuf,
    /// The repository's main worktree root; the folder itself for a folder.
    pub main_root: PathBuf,
    pub branch: Option<String>,
    common_dir: Option<PathBuf>,
}

impl Checkout {
    pub fn git(&self) -> bool {
        self.common_dir.is_some()
    }

    /// Whether the repository has a local branch `name`, from its own ref
    /// files: a loose ref, or a line of `packed-refs` (read up to 8 MiB).
    pub fn has_branch(&self, name: &str) -> bool {
        use std::io::Read;
        let Some(common) = &self.common_dir else {
            return false;
        };
        if name.is_empty() || name.split('/').any(|part| part.is_empty() || part == "..") {
            return false;
        }
        if common.join("refs/heads").join(name).is_file() {
            return true;
        }
        let mut packed = String::new();
        let read = std::fs::File::open(common.join("packed-refs"))
            .and_then(|file| file.take(PACKED_REFS_LIMIT).read_to_string(&mut packed));
        let wanted = format!("refs/heads/{name}");
        read.is_ok()
            && packed
                .lines()
                .filter_map(|line| line.split_once(' '))
                .any(|(_, reference)| reference == wanted)
    }

    /// The original checkout: the repository's main worktree.
    pub fn original(&self) -> bool {
        self.git() && self.root == self.main_root
    }
}

#[derive(Clone, Debug)]
pub(super) enum Located {
    /// Under no project of the policy.
    Outside,
    In(Checkout),
    /// Under a policy project's root, but its repository could not be read.
    Unreadable,
}

#[derive(Clone, Debug)]
pub(super) struct Place {
    pub place: Located,
    project: Option<usize>,
    /// Inside a repository nested in the checkout (a submodule or a clone):
    /// a folder of that checkout for its files, while a git command there
    /// works on the nested repository, which no protection guards.
    pub nested: bool,
}

pub(super) struct Project<'a> {
    pub policy: &'a ProjectPolicy,
}

impl Place {
    pub fn project<'a>(&self, facts: &Facts<'a>) -> Option<Project<'a>> {
        self.project.map(|index| Project {
            policy: &facts.policy.projects[index],
        })
    }
}

/// The most of `packed-refs` read for one branch lookup.
const PACKED_REFS_LIMIT: u64 = 8 * 1024 * 1024;

pub(super) struct Facts<'a> {
    policy: &'a Policy,
    pub home: PathBuf,
    /// Each project's root as the filesystem spells it; `None` when it is
    /// gone, so nothing is matched against it.
    roots: Vec<Option<PathBuf>>,
    places: RefCell<HashMap<PathBuf, Place>>,
    /// Whether a folder tells `Name` from `name`, by folder, for this call.
    cases: RefCell<HashMap<PathBuf, bool>>,
    /// Set when a shell call named more words than are checked.
    pub word_limit: Cell<bool>,
}

impl<'a> Facts<'a> {
    pub fn new(policy: &'a Policy, home: &Path) -> Self {
        Self {
            policy,
            home: canonical_prefix(home),
            roots: policy
                .projects
                .iter()
                .map(|project| {
                    project
                        .rules
                        .any()
                        .then(|| {
                            hide_platform::fs::identity::canonical(Path::new(&project.root)).ok()
                        })
                        .flatten()
                })
                .collect(),
            places: RefCell::new(HashMap::new()),
            cases: RefCell::new(HashMap::new()),
            word_limit: Cell::new(false),
        }
    }

    /// The root of the project `place` belongs to, as the filesystem spells it.
    pub fn project_root(&self, place: &Place) -> Option<&Path> {
        self.roots.get(place.project?)?.as_deref()
    }

    /// Whether names under `folder` compare with their case (B24), judged by
    /// what the folder holds so the hook writes nothing there; a folder that
    /// cannot say is taken as ignoring case, which protects more.
    pub fn case_sensitive(&self, folder: &Path) -> bool {
        if let Some(known) = self.cases.borrow().get(folder) {
            return *known;
        }
        let sensitive = hide_platform::fs::identity::case_sensitive_by_entries(folder)
            .ok()
            .flatten()
            .unwrap_or(false);
        self.cases
            .borrow_mut()
            .insert(folder.to_path_buf(), sensitive);
        sensitive
    }

    /// The checkout and project that hold `path`, which need not exist yet.
    pub fn locate(&self, path: &Path) -> Place {
        let (existing, _) = existing_ancestor(path);
        if let Some(place) = self.places.borrow().get(&existing) {
            return place.clone();
        }
        let place = self.read(path, &existing);
        self.places.borrow_mut().insert(existing, place.clone());
        place
    }

    fn read(&self, path: &Path, existing: &Path) -> Place {
        let spelled = canonical_prefix(path);
        let under = self
            .roots
            .iter()
            .position(|root| root.as_ref().is_some_and(|root| spelled.starts_with(root)));
        // A repository nested in a project's checkout, a submodule or a clone,
        // is a folder of the checkout around it; so is one whose `.git` the
        // reader refuses, such as a submodule's, which has no `commondir`.
        let enclosing = |holder: &Path| Place {
            nested: true,
            ..match holder.parent() {
                Some(parent) => self.read(path, parent),
                None => self.folder(under),
            }
        };
        match hide_project::git::discover_checked(existing) {
            Ok(Some(repository)) => {
                let (Ok(root), Ok(main_root)) = (
                    hide_platform::fs::identity::canonical(&repository.root),
                    hide_platform::fs::identity::canonical(&repository.main_root()),
                ) else {
                    return self.unreadable(under);
                };
                let Some(project) = self.root_index(&main_root) else {
                    return enclosing(&repository.root);
                };
                Place {
                    place: Located::In(Checkout {
                        branch: repository.branch(),
                        common_dir: Some(repository.common_dir.clone()),
                        root,
                        main_root,
                    }),
                    project: Some(project),
                    nested: false,
                }
            }
            Ok(None) => self.folder(under),
            Err(_) => match git_holder(existing) {
                Some(holder) if !self.is_root(&holder) => enclosing(&holder),
                _ => self.unreadable(under),
            },
        }
    }

    fn is_root(&self, folder: &Path) -> bool {
        hide_platform::fs::identity::canonical(folder)
            .is_ok_and(|folder| self.root_index(&folder).is_some())
    }

    fn root_index(&self, main_root: &Path) -> Option<usize> {
        self.roots
            .iter()
            .position(|root| root.as_deref() == Some(main_root))
    }

    fn folder(&self, under: Option<usize>) -> Place {
        let Some(index) = under else {
            return Place {
                place: Located::Outside,
                project: None,
                nested: false,
            };
        };
        let root = self.roots[index].clone().unwrap_or_default();
        Place {
            place: Located::In(Checkout {
                main_root: root.clone(),
                root,
                branch: None,
                common_dir: None,
            }),
            project: Some(index),
            nested: false,
        }
    }

    fn unreadable(&self, under: Option<usize>) -> Place {
        match under {
            Some(index) => Place {
                place: Located::Unreadable,
                project: Some(index),
                nested: false,
            },
            None => Place {
                place: Located::Outside,
                project: None,
                nested: false,
            },
        }
    }

    /// The branch the base branch protection guards in `checkout`'s
    /// repository: the operator's choice, else what `origin/HEAD` names,
    /// else, in a repository with no remote, the branch its main worktree
    /// has out, as the worktree reader names the default branch.
    pub fn base(&self, project: &ProjectPolicy, checkout: &Checkout) -> Option<String> {
        if let Some(base) = &project.base {
            return Some(base.clone());
        }
        let common = checkout.common_dir.as_ref()?;
        let origin = std::fs::read_to_string(common.join("refs/remotes/origin/HEAD"))
            .ok()
            .and_then(|text| {
                let branch = text
                    .trim()
                    .strip_prefix("ref:")?
                    .trim()
                    .strip_prefix("refs/remotes/origin/")?
                    .to_owned();
                (!branch.is_empty()).then_some(branch)
            });
        origin.or_else(|| hide_project::git::discover(&checkout.main_root)?.own_default_branch())
    }
}

/// The nearest folder at or above `path` that holds a `.git`.
fn git_holder(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|folder| std::fs::symlink_metadata(folder.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// The deepest ancestor of `path` that exists (itself when it does), and
/// what follows it.
fn existing_ancestor(path: &Path) -> (PathBuf, PathBuf) {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
        let Some(name) = existing.file_name().map(|name| name.to_os_string()) else {
            break;
        };
        rest.push(name);
        if !existing.pop() {
            break;
        }
    }
    let rest = rest.into_iter().rev().collect();
    (existing, rest)
}

/// `path` with its existing part spelled the way the filesystem spells it
/// (links followed, case as stored), so it compares by name with a project
/// root on a filesystem that ignores case too (B24).
pub(super) fn canonical_prefix(path: &Path) -> PathBuf {
    let (existing, rest) = existing_ancestor(path);
    match hide_platform::fs::identity::canonical(&existing) {
        Ok(canonical) if rest.as_os_str().is_empty() => canonical,
        Ok(canonical) => canonical.join(rest),
        Err(_) => path.to_path_buf(),
    }
}
