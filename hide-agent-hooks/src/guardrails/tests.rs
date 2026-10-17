//! The guardrails' decisions against real repositories in a private folder:
//! each protection's refused and allowed forms (PRD agent-guardrails B6-B24).

use std::path::{Path, PathBuf};
use std::process::Command;

use hide_agent_adapter::HookDialect;

use super::payload::{self, ToolCall};
use super::*;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository on `main` with a linked worktree on `feature`, a home with
/// a key, and a policy file naming the repository.
struct Fixture {
    _folder: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    worktree: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let base = hide_platform::fs::identity::canonical(folder.path()).unwrap();
        let home = base.join("home");
        let repo = base.join("repo");
        let worktree = base.join("repo-feature");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
        std::fs::write(home.join("notes.txt"), "notes").unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        for (name, text) in [
            ("a.txt", "a"),
            ("src/lib.rs", "fn main() {}"),
            (".env", "SECRET=1"),
            (".env.local", "SECRET=2"),
            (".env.example", "SECRET="),
            ("server.pem", "pem"),
        ] {
            std::fs::write(repo.join(name), text).unwrap();
        }
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );
        Self {
            _folder: folder,
            home,
            repo,
            worktree,
        }
    }

    fn policy(&self, rules: Rules, base: Option<&str>) {
        write_policy(
            &self.home,
            &Policy::new(vec![ProjectPolicy {
                root: self.repo.to_string_lossy().into_owned(),
                base: base.map(str::to_owned),
                rules,
            }]),
        )
        .unwrap();
    }

    fn decide_in(&self, cwd: &Path, action: Action, dirty: Option<bool>) -> Outcome {
        decide(&self.home, cwd, &action, &move |_, _| dirty)
    }

    fn shell(&self, cwd: &Path, command: &str) -> Option<Rule> {
        self.shell_dirty(cwd, command, Some(false))
    }

    fn shell_dirty(&self, cwd: &Path, command: &str, dirty: Option<bool>) -> Option<Rule> {
        refused(self.decide_in(cwd, Action::Shell(command.to_owned()), dirty))
    }

    fn write(&self, cwd: &Path, path: &Path) -> Option<Rule> {
        refused(self.decide_in(
            cwd,
            Action::Write(vec![path.to_string_lossy().into_owned()]),
            Some(false),
        ))
    }

    fn read(&self, cwd: &Path, path: &str) -> Option<Rule> {
        refused(self.decide_in(cwd, Action::Read(vec![path.to_owned()]), Some(false)))
    }
}

fn refused(outcome: Outcome) -> Option<Rule> {
    match outcome {
        Outcome::Allow => None,
        Outcome::Refuse(refusal) => Some(refusal.rule),
        Outcome::PolicyUnreadable(failure) => panic!("policy unreadable: {failure:?}"),
    }
}

fn rules(edit: impl FnOnce(&mut Rules)) -> Rules {
    let mut rules = Rules::default();
    edit(&mut rules);
    rules
}

#[test]
fn the_original_checkout_keeps_its_branch() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.worktree_folder = true), Some("main"));
    let repo = &fixture.repo;
    for command in [
        "git checkout feature-2",
        "git checkout -b topic",
        "git switch topic",
        "git switch -c topic",
        "git checkout -",
        "gh pr checkout 12",
        "cd src && git switch topic",
        "FOO=1 git -c core.pager=cat switch topic",
    ] {
        assert_eq!(
            fixture.shell(repo, command),
            Some(Rule::WorktreeFolder),
            "{command}"
        );
    }
    // From the worktree, a command aimed at the original checkout is refused,
    // and its own branch changes are not. A path is quoted as a POSIX shell
    // (Git Bash on Windows) needs it: unquoted, `C:\repo`'s backslashes are
    // escapes there, and the hook reads the words as that shell runs them.
    let aimed = format!("git -C '{}' switch topic", repo.display());
    assert_eq!(
        fixture.shell(&fixture.worktree, &aimed),
        Some(Rule::WorktreeFolder)
    );
    for command in [
        "git switch topic",
        "git checkout -b topic",
        "gh pr checkout 12",
    ] {
        assert_eq!(fixture.shell(&fixture.worktree, command), None, "{command}");
    }
    // A path checkout, a status and moving away first are not branch changes.
    for command in [
        "git checkout -- a.txt",
        "git checkout a.txt",
        "git checkout .",
        "git status",
        "git checkout",
        "git switch --help",
    ] {
        assert_eq!(fixture.shell(repo, command), None, "{command}");
    }
    let away = format!("cd '{}' && git switch topic", fixture.worktree.display());
    assert_eq!(fixture.shell(repo, &away), None);
    // A branch named like a folder is the branch, as Git reads it, and a
    // tree's paths after `--` are paths.
    git(repo, &["branch", "src"]);
    assert_eq!(
        fixture.shell(repo, "git checkout src"),
        Some(Rule::WorktreeFolder)
    );
    for command in [
        "git checkout HEAD -- a.txt",
        "git checkout feature -- src/lib.rs",
    ] {
        assert_eq!(fixture.shell(repo, command), None, "{command}");
    }
    // A command named in another script before it is still read.
    assert_eq!(
        fixture.shell(repo, "./스크립트.sh && 日本語 && git switch topic"),
        Some(Rule::WorktreeFolder)
    );
}

#[test]
fn an_agent_writes_only_in_its_own_checkout() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.worktree_folder = true), Some("main"));
    let (repo, worktree) = (&fixture.repo, &fixture.worktree);
    assert_eq!(
        fixture.write(worktree, &repo.join("src/lib.rs")),
        Some(Rule::WorktreeFolder)
    );
    assert_eq!(
        fixture.write(worktree, &repo.join("src/new/file.rs")),
        Some(Rule::WorktreeFolder),
        "a file that does not exist yet"
    );
    assert_eq!(
        fixture.write(repo, &worktree.join("a.txt")),
        Some(Rule::WorktreeFolder)
    );
    assert_eq!(fixture.write(worktree, &worktree.join("src/lib.rs")), None);
    assert_eq!(
        fixture.write(worktree, Path::new("src/lib.rs")),
        None,
        "relative"
    );
    assert_eq!(
        fixture.write(worktree, &fixture.home.join("notes.txt")),
        None
    );
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(fixture.write(worktree, &outside.path().join("x")), None);
    assert_eq!(
        fixture.read(worktree, &repo.join("a.txt").to_string_lossy()),
        None
    );
}

#[test]
fn the_base_branch_takes_no_writes_commits_or_pushes() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.base_branch = true), Some("main"));
    let (repo, worktree) = (&fixture.repo, &fixture.worktree);
    assert_eq!(
        fixture.write(repo, &repo.join("a.txt")),
        Some(Rule::BaseBranch)
    );
    assert_eq!(
        fixture.write(worktree, &repo.join("a.txt")),
        Some(Rule::BaseBranch),
        "from any checkout"
    );
    assert_eq!(fixture.write(worktree, &worktree.join("a.txt")), None);
    for command in [
        "git commit -m x",
        "git commit --amend --no-edit",
        "git merge feature",
        "git cherry-pick abc",
        "git revert abc",
        "git am patch.mbox",
        "git rebase feature",
        "git merge --ff-only feature",
        "git pull",
        "git push",
        "git push origin",
        "git push origin main",
        "git push origin HEAD",
    ] {
        assert_eq!(
            fixture.shell(repo, command),
            Some(Rule::BaseBranch),
            "{command}"
        );
    }
    for command in [
        "git fetch",
        "git pull --ff-only",
        "git merge --abort",
        "git rebase --abort",
        "git status",
        "git push origin feature",
    ] {
        assert_eq!(fixture.shell(repo, command), None, "{command}");
    }
    for command in [
        "git push origin main",
        "git push origin HEAD:main",
        "git push origin feature:refs/heads/main",
        "git push origin +feature:main",
        "git push --all origin",
        "git push --mirror origin",
        "git push origin --delete main",
        "git push origin :main",
        "git push origin refs/heads/*:refs/heads/*",
    ] {
        assert_eq!(
            fixture.shell(worktree, command),
            Some(Rule::BaseBranch),
            "{command}"
        );
    }
    for command in [
        "git commit -m x",
        "git push",
        "git push -u origin feature",
        "git push origin HEAD",
        "git push origin feature:feature",
        "gh pr merge 12",
    ] {
        assert_eq!(fixture.shell(worktree, command), None, "{command}");
    }
}

#[test]
fn a_changed_base_branch_is_the_one_guarded() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.base_branch = true), Some("feature"));
    assert_eq!(
        fixture.write(&fixture.repo, &fixture.repo.join("a.txt")),
        None
    );
    assert_eq!(
        fixture.shell(&fixture.worktree, "git commit -m x"),
        Some(Rule::BaseBranch)
    );
    // Automatic is what origin/HEAD names, read from the repository's files.
    let origin = fixture.repo.join(".git/refs/remotes/origin");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::write(origin.join("HEAD"), "ref: refs/remotes/origin/main\n").unwrap();
    fixture.policy(rules(|rules| rules.base_branch = true), None);
    assert_eq!(
        fixture.shell(&fixture.repo, "git commit -m x"),
        Some(Rule::BaseBranch)
    );
    assert_eq!(fixture.shell(&fixture.worktree, "git commit -m x"), None);
    // With no origin/HEAD and no choice, the base cannot be read: refused.
    std::fs::remove_file(origin.join("HEAD")).unwrap();
    let outcome = fixture.decide_in(
        &fixture.worktree,
        Action::Shell("git commit -m x".to_owned()),
        Some(false),
    );
    let Outcome::Refuse(refusal) = outcome else {
        panic!("{outcome:?}")
    };
    assert!(refusal.unread && refusal.reason().contains("base branch"));
}

#[test]
fn merging_a_pull_request_is_refused_only_with_the_option() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.base_branch = true), Some("main"));
    assert_eq!(
        fixture.shell(&fixture.worktree, "gh pr merge 12 --auto"),
        None
    );
    fixture.policy(
        rules(|rules| {
            rules.base_branch = true;
            rules.agent_merge = true;
        }),
        Some("main"),
    );
    for command in [
        "gh pr merge 12 --auto --squash",
        "gh pr merge",
        "gh pr merge --repo o/r 3",
        "gh pr -R o/r merge 12",
        "gh --repo o/r pr merge 12",
    ] {
        assert_eq!(
            fixture.shell(&fixture.worktree, command),
            Some(Rule::AgentMerge),
            "{command}"
        );
    }
    assert_eq!(fixture.shell(&fixture.worktree, "gh pr view 12"), None);
    // The option alone, without the base protection, is not shown or applied.
    fixture.policy(rules(|rules| rules.agent_merge = true), Some("main"));
    assert_eq!(fixture.shell(&fixture.worktree, "gh pr merge 12"), None);
}

#[test]
fn risky_git_commands_are_refused_and_their_safe_forms_run() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.git_commands = true), Some("main"));
    let worktree = &fixture.worktree;
    for command in [
        "git push --force origin feature",
        "git push -f origin feature",
        "git push -uf origin feature",
        "git push origin +feature",
        "git push --force --force-with-lease origin feature",
        "git push --no-verify",
        "git commit --no-verify -m x",
        "git commit -nm x",
        "git commit -an -m x",
    ] {
        assert_eq!(
            fixture.shell(worktree, command),
            Some(Rule::GitCommands),
            "{command}"
        );
    }
    for command in [
        "git push --force-with-lease origin feature",
        "git push --force-with-lease=feature:abc origin feature",
        "git push origin feature",
        "git commit -m -n",
        "git commit -m 'skip --no-verify'",
        "git commit -am x",
    ] {
        assert_eq!(fixture.shell(worktree, command), None, "{command}");
    }
    // Discarding is refused only when there is something to lose.
    for command in [
        "git reset --hard",
        "git reset --hard HEAD~1",
        "git checkout -- a.txt",
        "git checkout .",
        "git checkout -f feature",
        "git restore a.txt",
        "git restore --worktree --staged a.txt",
        "git clean -fd",
        "git clean --force",
        "git switch --discard-changes topic",
    ] {
        assert_eq!(
            fixture.shell_dirty(worktree, command, Some(true)),
            Some(Rule::GitCommands),
            "{command}"
        );
        assert_eq!(
            fixture.shell_dirty(worktree, command, Some(false)),
            None,
            "{command}"
        );
    }
    for command in [
        "git reset --soft HEAD~1",
        "git reset a.txt",
        "git restore --staged a.txt",
        "git clean -n",
        "git clean -fn",
        "git stash",
    ] {
        assert_eq!(
            fixture.shell_dirty(worktree, command, Some(true)),
            None,
            "{command}"
        );
    }
    // Untracked files are what `git clean` removes; the other commands are
    // asked about tracked changes only.
    let asked = std::cell::RefCell::new(Vec::new());
    for command in ["git reset --hard", "git restore a.txt", "git clean -f"] {
        decide(
            &fixture.home,
            worktree,
            &Action::Shell(command.into()),
            &|_, untracked| {
                asked.borrow_mut().push((command, untracked));
                Some(false)
            },
        );
    }
    assert_eq!(
        asked.into_inner(),
        [
            ("git reset --hard", false),
            ("git restore a.txt", false),
            ("git clean -f", true)
        ]
    );
    // Words past the files protection's limit leave a later command judged.
    let many = (0..700)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        fixture.shell(
            worktree,
            &format!("git add {many} && git push --force origin feature")
        ),
        Some(Rule::GitCommands)
    );
    // Whether the checkout has changes could not be read: refused.
    let outcome = fixture.decide_in(worktree, Action::Shell("git reset --hard".into()), None);
    assert!(matches!(
        outcome,
        Outcome::Refuse(Refusal {
            rule: Rule::GitCommands,
            unread: true,
            ..
        })
    ));
}

#[test]
fn protected_files_are_refused_to_tools_and_shell_words() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.files = true), None);
    let (repo, worktree) = (&fixture.repo, &fixture.worktree);
    for path in [".env", ".env.local", "server.pem", "sub/new.key"] {
        assert_eq!(fixture.read(repo, path), Some(Rule::Files), "{path}");
    }
    assert_eq!(fixture.read(repo, ".env.example"), None);
    assert_eq!(fixture.read(repo, "a.txt"), None);
    assert_eq!(
        fixture.read(worktree, ".env"),
        Some(Rule::Files),
        "a linked worktree too"
    );
    assert_eq!(
        fixture.read(
            repo,
            &fixture.home.join(".ssh/id_ed25519").to_string_lossy()
        ),
        Some(Rule::Files)
    );
    assert_eq!(fixture.read(repo, "~/.ssh/id_ed25519"), Some(Rule::Files));
    assert_eq!(fixture.read(repo, "~/notes.txt"), None);
    assert_eq!(fixture.write(repo, &repo.join(".env")), Some(Rule::Files));
    for command in [
        "cat .env",
        "cat < .env",
        // A new file named with a space is a path, not prose.
        "cp a.txt 'my cert.pem'",
        "touch 'a b.key'",
        "touch -a 'a b.key'",
        "tee -a 'my cert.pem'",
        "touch -m 'a b.key'",
        "touch -am 'a b.key'",
        "curl -c 'jar x.pem' https://example.invalid",
        // A program whose path has a space is a path, and the rest is read.
        "'/opt/My Tools/rg' x; cat .env",
        "rg -e x 'my api.key'",
        "echo x >> .env.local",
        "docker run --env-file=.env image",
        "cd src && cat ../.env",
        "cat ~/.ssh/id_ed25519",
        "git add .env",
    ] {
        assert_eq!(fixture.shell(repo, command), Some(Rule::Files), "{command}");
    }
    for command in [
        "cat .env.example",
        "cat a.txt 2>&1",
        "echo .env-free",
        "ls",
        // Prose that ends like a protected name is not a path.
        "git commit -m 'remove api.key'",
        "gh pr create --title 'rotate server.pem' --body x",
        "echo 'renew the server.pem'",
        "rg 'load the api.key' src",
        "rg -A 3 'load the api.key' src",
        "awk -F : 'renew the server.pem' notes.txt",
        "python3 -c 'print(1) # server.pem'",
        "rg -F 'load the api.key' src",
        "grep -v 'renew the server.pem' notes.txt",
        "awk -v x=1 'renew the server.pem' notes.txt",
        "git commit --message='rotate server.pem'",
        "git commit -am 'rotate server.pem'",
        "gh pr create --title='renew api.key' --body='x'",
    ] {
        assert_eq!(fixture.shell(repo, command), None, "{command}");
    }
    // A quoted path with a space that is there is still a path.
    std::fs::write(repo.join("my api.key"), "x").unwrap();
    assert_eq!(fixture.shell(repo, "cat 'my api.key'"), Some(Rule::Files));
    assert_eq!(
        fixture.shell(repo, "cd && cat .ssh/id_ed25519"),
        Some(Rule::Files),
        "a bare cd goes home"
    );
    // An example the operator lists stays protected beside `.env.*`.
    let mut listed = rules(|rules| rules.files = true);
    listed.paths = vec![".env.example".into(), ".env.*".into()];
    fixture.policy(listed, None);
    assert_eq!(fixture.read(repo, ".env.example"), Some(Rule::Files));
    assert_eq!(fixture.read(repo, ".env.local"), Some(Rule::Files));
    fixture.policy(rules(|rules| rules.files = true), None);
    // A home path is protected only for an agent in a project that lists it.
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(fixture.read(outside.path(), "~/.ssh/id_ed25519"), None);
}

#[test]
fn a_path_is_compared_by_the_filesystems_own_rule_for_case() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.files = true), None);
    let ignores_case = !hide_platform::fs::identity::case_sensitive(&fixture.repo).unwrap();
    // `.ENV` is the protected `.env` where the folder ignores case, and
    // another (missing) file where it does not.
    assert_eq!(
        fixture.read(&fixture.repo, ".ENV"),
        ignores_case.then_some(Rule::Files)
    );
    // A stored name in capitals matches its pattern the same way.
    std::fs::write(fixture.repo.join("Key.PEM"), "x").unwrap();
    assert_eq!(
        fixture.read(&fixture.repo, "Key.PEM"),
        ignores_case.then_some(Rule::Files)
    );
}

#[test]
fn a_repository_that_cannot_be_read_refuses_what_needs_it() {
    let fixture = Fixture::new();
    fixture.policy(
        rules(|rules| {
            rules.worktree_folder = true;
            rules.files = true;
        }),
        Some("main"),
    );
    // A nested `.git` the reader refuses, as a submodule's is, belongs to the
    // checkout around it: no lockout, and that checkout's rules hold.
    let nested = fixture.repo.join("vendor");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join(".git"), "gitdir: ../.git/modules/vendor").unwrap();
    std::fs::write(nested.join(".env"), "x").unwrap();
    assert_eq!(fixture.write(&nested, &nested.join("x.rs")), None);
    assert_eq!(fixture.shell(&nested, "git status"), None);
    assert_eq!(fixture.shell(&nested, "ls"), None);
    assert_eq!(fixture.read(&nested, ".env"), Some(Rule::Files));
    assert_eq!(
        fixture.write(&fixture.worktree, &nested.join("x.rs")),
        Some(Rule::WorktreeFolder),
        "a write from another checkout into the one around it"
    );
    // A git command in a clone nested in the checkout works on that clone,
    // which no protection guards, though its files stay the checkout's.
    let clone = fixture.repo.join("clone");
    std::fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    assert_eq!(fixture.shell(&clone, "git switch -c y"), None);
    assert_eq!(
        fixture.shell(&fixture.repo, "git -C clone switch -c y"),
        None
    );
    assert_eq!(fixture.write(&clone, &clone.join("x.rs")), None);
    std::fs::write(clone.join(".env"), "x").unwrap();
    assert_eq!(fixture.read(&clone, ".env"), Some(Rule::Files));
    // A project whose own repository cannot be read refuses only what a
    // protection needs the repository for.
    let broken = fixture.home.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join(".git"), "not a pointer").unwrap();
    std::fs::write(broken.join(".env"), "x").unwrap();
    write_policy(
        &fixture.home,
        &Policy::new(vec![ProjectPolicy {
            root: broken.to_string_lossy().into_owned(),
            base: Some("main".into()),
            rules: rules(|rules| {
                rules.worktree_folder = true;
                rules.files = true;
            }),
        }]),
    )
    .unwrap();
    let outcome = fixture.decide_in(
        &broken,
        Action::Write(vec![broken.join("x.rs").to_string_lossy().into_owned()]),
        Some(false),
    );
    let Outcome::Refuse(refusal) = outcome else {
        panic!("{outcome:?}")
    };
    assert!(refusal.unread, "{refusal:?}");
    assert_eq!(fixture.shell(&broken, "git status"), None);
    assert_eq!(
        fixture.shell(&broken, "git switch main"),
        Some(Rule::WorktreeFolder)
    );
    assert_eq!(fixture.read(&broken, "/etc/hosts"), None);
    assert_eq!(fixture.read(&broken, ".env"), Some(Rule::Files));
    // Outside every project, the same breakage is not Hide's to judge.
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join(".git"), "not a pointer").unwrap();
    assert_eq!(
        fixture.write(outside.path(), &outside.path().join("x")),
        None
    );
}

#[test]
fn nothing_is_refused_with_every_protection_off_or_outside_a_project() {
    let fixture = Fixture::new();
    fixture.policy(Rules::default(), Some("main"));
    assert_eq!(
        fixture.shell(&fixture.repo, "git push --force origin main"),
        None
    );
    assert_eq!(fixture.read(&fixture.repo, ".env"), None);
    fixture.policy(
        rules(|rules| {
            rules.worktree_folder = true;
            rules.base_branch = true;
            rules.git_commands = true;
            rules.files = true;
        }),
        Some("main"),
    );
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(other.path().join(".env"), "x").unwrap();
    for command in [
        "git switch x",
        "git commit -m x",
        "git push -f origin main",
        "cat .env",
    ] {
        assert_eq!(fixture.shell(other.path(), command), None, "{command}");
    }
}

#[test]
fn a_missing_corrupt_or_newer_policy_lets_the_call_run_and_says_why() {
    let fixture = Fixture::new();
    let action = Action::Shell("git push --force".to_owned());
    let decide = || decide(&fixture.home, &fixture.repo, &action, &|_, _| Some(false));
    assert_eq!(decide(), Outcome::PolicyUnreadable(PolicyFailure::Missing));
    std::fs::create_dir_all(policy_path(&fixture.home).parent().unwrap()).unwrap();
    std::fs::write(policy_path(&fixture.home), "{not json").unwrap();
    assert_eq!(decide(), Outcome::PolicyUnreadable(PolicyFailure::Corrupt));
    std::fs::write(policy_path(&fixture.home), r#"{"version":2,"projects":[]}"#).unwrap();
    assert_eq!(decide(), Outcome::PolicyUnreadable(PolicyFailure::Version));
}

#[test]
fn the_reason_names_the_protection_and_what_to_do() {
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.worktree_folder = true), Some("main"));
    let Outcome::Refuse(refusal) = fixture.decide_in(
        &fixture.repo,
        Action::Shell("git switch topic".into()),
        Some(false),
    ) else {
        panic!("refused")
    };
    let reason = refusal.reason();
    assert!(reason.contains("Guard working folders"), "{reason}");
    assert!(reason.contains("worktree"), "{reason}");
    assert!(reason.contains("Project Settings"), "{reason}");
    assert!(!reason.contains('\u{2014}'), "no em dash");
    // With no base branch to read, the reason says who names one and where.
    fixture.policy(rules(|rules| rules.base_branch = true), None);
    let Outcome::Refuse(refusal) = fixture.decide_in(
        &fixture.repo,
        Action::Shell("git commit -m x".into()),
        Some(false),
    ) else {
        panic!("refused")
    };
    let reason = refusal.reason();
    assert!(
        reason.contains("could not read the base branch"),
        "{reason}"
    );
    assert!(
        reason.contains("choose the base branch in Project Settings"),
        "{reason}"
    );
}

#[test]
fn a_path_list_is_saved_only_within_its_limits() {
    assert_eq!(Rules::default().validate(), Ok(()));
    let with = |paths: Vec<String>| Rules {
        paths,
        ..Rules::default()
    };
    assert_eq!(with(Vec::new()).validate(), Ok(()));
    let many: Vec<String> = (0..PATH_LIMIT).map(|index| format!("f{index}")).collect();
    assert_eq!(with(many.clone()).validate(), Ok(()));
    let mut over = many;
    over.push("one-more".into());
    assert_eq!(with(over).validate(), Err(PathRefusal::TooMany));
    assert_eq!(with(vec!["a".repeat(PATH_LENGTH_LIMIT)]).validate(), Ok(()));
    assert_eq!(
        with(vec!["경".repeat(PATH_LENGTH_LIMIT)]).validate(),
        Ok(()),
        "characters, not bytes"
    );
    assert_eq!(
        with(vec!["a".repeat(PATH_LENGTH_LIMIT + 1)]).validate(),
        Err(PathRefusal::TooLong)
    );
    assert_eq!(
        with(vec![".env".into(), ".env".into()]).validate(),
        Err(PathRefusal::Duplicate)
    );
    for invalid in [
        "", " .env", "~", "~user/x", "~/", "!keep", "# note", "[z-a]", "{a,b", "a\nb",
    ] {
        assert_eq!(
            with(vec![invalid.into()]).validate(),
            Err(PathRefusal::Invalid),
            "{invalid:?}"
        );
    }
    for valid in [
        "secrets/",
        "/config/prod.yml",
        "**/*.p12",
        "~/.aws/credentials",
    ] {
        assert_eq!(with(vec![valid.into()]).validate(), Ok(()), "{valid}");
    }
}

#[test]
fn each_runtimes_payload_reads_as_its_action() {
    let call = |payload: &str, dialect| payload::read_hook(payload.as_bytes(), false, dialect);
    let write = |path: &str| Action::Write(vec![path.to_owned()]);
    let read = |path: &str| Action::Read(vec![path.to_owned()]);
    // Claude Code 2.1.300.
    assert_eq!(
        call(
            r#"{"cwd":"/r","hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/r/a.txt","old_string":"a","new_string":"b"}}"#,
            HookDialect::ClaudeCode
        ),
        Some(ToolCall {
            action: write("/r/a.txt"),
            cwd: Some("/r".into())
        })
    );
    assert_eq!(
        call(
            r#"{"cwd":"/r","tool_name":"NotebookEdit","tool_input":{"notebook_path":"/r/n.ipynb"}}"#,
            HookDialect::ClaudeCode
        )
        .map(|call| call.action),
        Some(write("/r/n.ipynb"))
    );
    assert_eq!(
        call(
            r#"{"cwd":"/r","tool_name":"Read","tool_input":{"file_path":".env"}}"#,
            HookDialect::ClaudeCode
        )
        .map(|call| call.action),
        Some(read(".env"))
    );
    // Codex 0.162.1: the patch names its files.
    assert_eq!(
        call(
            r#"{"cwd":"/r","tool_name":"apply_patch","tool_input":{"command":"*** Begin Patch\n*** Update File: /r/a.txt\n@@\n-a\n+b\n*** Add File: b.txt\n+x\n*** Delete File: c.txt\n*** End Patch"}}"#,
            HookDialect::Codex
        )
        .map(|call| call.action),
        Some(Action::Write(vec!["/r/a.txt".into(), "b.txt".into(), "c.txt".into()]))
    );
    assert_eq!(
        call(
            r#"{"cwd":"/r","tool_name":"Bash","tool_input":{"command":"git status"}}"#,
            HookDialect::Codex
        )
        .map(|call| call.action),
        Some(Action::Shell("git status".into()))
    );
    // Cursor 2026.10.14: an edit arrives as `Write`; a shell call's empty cwd
    // falls back to the workspace.
    assert_eq!(
        call(
            r#"{"hook_event_name":"preToolUse","tool_name":"Write","tool_input":{"file_path":"/r/c.txt","content":"x"},"workspace_roots":["/r"]}"#,
            HookDialect::Cursor
        ),
        Some(ToolCall {
            action: write("/r/c.txt"),
            cwd: Some("/r".into())
        })
    );
    assert_eq!(
        call(
            r#"{"hook_event_name":"preToolUse","tool_name":"Shell","tool_input":{"command":"git status","cwd":""},"cwd":"","workspace_roots":["/r"]}"#,
            HookDialect::Cursor
        ),
        Some(ToolCall {
            action: Action::Shell("git status".into()),
            cwd: Some("/r".into())
        })
    );
    // Grok 1.0.52: camelCase keys.
    assert_eq!(
        call(
            r#"{"cwd":"/r","toolName":"read_file","toolInput":{"path":"src/main.rs"}}"#,
            HookDialect::Grok
        )
        .map(|call| call.action),
        Some(read("src/main.rs"))
    );
    assert_eq!(
        call(
            r#"{"cwd":"/r","toolName":"search_replace","toolInput":{"file_path":"a.txt","old_string":"a","new_string":"b"}}"#,
            HookDialect::Grok
        )
        .map(|call| call.action),
        Some(write("a.txt"))
    );
    // A tool the guardrails do not read, and a payload that is not JSON.
    assert_eq!(
        call(
            r#"{"tool_name":"Grep","tool_input":{"pattern":"x"}}"#,
            HookDialect::ClaudeCode
        ),
        None
    );
    assert_eq!(call("not json", HookDialect::ClaudeCode), None);
}

#[test]
fn the_scripts_tools_read_as_their_actions() {
    let action = |tool: &str, args: serde_json::Value| payload::action(tool, &args);
    // OpenCode 1.18.30.
    assert_eq!(
        action(
            "edit",
            serde_json::json!({"filePath": "/r/a.txt", "oldString": "a", "newString": "b"})
        ),
        Some(Action::Write(vec!["/r/a.txt".into()]))
    );
    assert_eq!(
        action(
            "apply_patch",
            serde_json::json!({"patchText": "*** Begin Patch\n*** Add File: n.txt\n+x\n*** End Patch"})
        ),
        Some(Action::Write(vec!["n.txt".into()]))
    );
    assert_eq!(
        action("read", serde_json::json!({"filePath": ".env"})),
        Some(Action::Read(vec![".env".into()]))
    );
    // OpenCode 2.0.29.
    assert_eq!(
        action(
            "patch",
            serde_json::json!({"patchText": "*** Begin Patch\n*** Update File: /r/a.txt\n@@\n-a\n+b\n*** End Patch"})
        ),
        Some(Action::Write(vec!["/r/a.txt".into()]))
    );
    assert_eq!(
        action(
            "edit",
            serde_json::json!({"path": "/r/a.txt", "oldString": "a", "newString": "b"})
        ),
        Some(Action::Write(vec!["/r/a.txt".into()]))
    );
    assert_eq!(
        action(
            "shell",
            serde_json::json!({"command": "git push", "workdir": "/r", "timeout": 1000})
        ),
        Some(Action::Shell("git push".into()))
    );
    // Pi 1.0.4 and omp 18.7.0.
    assert_eq!(
        action(
            "write",
            serde_json::json!({"path": "b.txt", "content": "x"})
        ),
        Some(Action::Write(vec!["b.txt".into()]))
    );
    assert_eq!(
        action("bash", serde_json::json!({"command": "git push"})),
        Some(Action::Shell("git push".into()))
    );
    assert_eq!(action("grep", serde_json::json!({"pattern": "x"})), None);
}

#[test]
fn an_agy_write_cut_before_its_name_is_refused_where_writes_are_guarded() {
    let content = "x".repeat(4096);
    let payload = format!(
        r#"{{"conversationId":"c","toolCall":{{"args":{{"CodeContent":"{content}","TargetFile":"/r/.env"}},"name":"write_to_file"}},"workspacePaths":["/r"]}}"#
    );
    let cut = &payload.as_bytes()[..payload.len() / 2];
    let call = payload::read_hook(cut, true, HookDialect::Antigravity).unwrap();
    assert_eq!(call.action, Action::UnreadWrite);
    let fixture = Fixture::new();
    fixture.policy(rules(|rules| rules.files = true), None);
    assert_eq!(
        refused(fixture.decide_in(&fixture.repo, call.action.clone(), Some(false))),
        Some(Rule::Files)
    );
    fixture.policy(rules(|_| {}), None);
    assert_eq!(
        refused(fixture.decide_in(&fixture.repo, call.action, Some(false))),
        None
    );
    // A cut call with no write content is not judged.
    let view = format!(
        r#"{{"toolCall":{{"args":{{"AbsolutePath":"/r/{content}"}},"name":"view_file"}}}}"#
    );
    assert_eq!(
        payload::read_hook(&view.as_bytes()[..100], true, HookDialect::Antigravity),
        None
    );
}

#[test]
fn a_payload_cut_at_the_limit_still_names_its_file() {
    let content = "x".repeat(4096);
    let payload = format!(
        r#"{{"cwd":"/r","tool_name":"Write","tool_input":{{"file_path":"/r/.env","content":"{content}"}}}}"#
    );
    let cut = &payload.as_bytes()[..payload.len() / 2];
    assert_eq!(
        payload::read_hook(cut, true, HookDialect::ClaudeCode),
        Some(ToolCall {
            action: Action::Write(vec!["/r/.env".into()]),
            cwd: Some("/r".into())
        })
    );
    // A patch cut inside its text names each file whose line arrived whole.
    let patch = format!(
        r#"{{"cwd":"/r","tool_name":"apply_patch","tool_input":{{"command":"*** Begin Patch\n*** Update File: a.txt\n+{content}\n*** Add File: .env\n+{content}\n*** Add File: b.txt\n+x"}}}}"#
    );
    let cut = &patch.as_bytes()[..patch.len() - 2048];
    assert_eq!(
        payload::read_hook(cut, true, HookDialect::Codex).map(|call| call.action),
        Some(Action::Write(vec![".env".into(), "a.txt".into()]))
    );
    let shell = format!(r#"{{"tool_name":"Bash","tool_input":{{"command":"echo {content}"}}}}"#);
    assert_eq!(
        payload::read_hook(&shell.as_bytes()[..100], true, HookDialect::ClaudeCode),
        None,
        "a cut command is not decided on"
    );
}

#[cfg(unix)]
#[test]
fn the_status_read_runs_no_command_the_repository_configures() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let marker = dir.path().join("ran");
    let monitor = dir.path().join("monitor.sh");
    std::fs::write(
        &monitor,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&monitor, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &repo,
        &["config", "core.fsmonitor", monitor.to_str().unwrap()],
    );
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    assert_eq!(git_dirty(&repo, true), Some(true));
    assert!(!marker.exists(), "the repository's fsmonitor ran");
}

#[cfg(windows)]
#[test]
fn a_git_bash_drive_path_names_the_windows_file() {
    let fixture = Fixture::new();
    let mut rules = rules(|rules| rules.files = true);
    rules.worktree_folder = true;
    fixture.policy(rules, Some("main"));
    // `C:\...\repo` as Git Bash spells it: `/c/.../repo`.
    let native = fixture.repo.to_string_lossy().into_owned();
    let (drive, rest) = native.split_once(":\\").unwrap();
    let mounted = format!(
        "/{}/{}",
        drive.to_ascii_lowercase(),
        rest.replace('\\', "/")
    );
    std::fs::write(fixture.repo.join(".env"), "x").unwrap();
    assert_eq!(
        fixture.shell(&fixture.repo, &format!("cat '{mounted}/.env'")),
        Some(Rule::Files)
    );
    assert_eq!(
        fixture.shell(
            &fixture.worktree,
            &format!("git -C '{mounted}' switch topic")
        ),
        Some(Rule::WorktreeFolder)
    );
}
