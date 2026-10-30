//! The node's `gh`: the one bounded, non-interactive way Hide runs GitHub's
//! CLI with the operator's login, for the commands
//! `hide_node_link::gh::allowed` names. The core parses what it answers.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_node_link::gh::{GhAnswer, GithubFailureCategory, allowed, valid_repository};
use hide_platform::process::OwnedChild;

/// How long one `gh` command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
struct GhFailure {
    category: GithubFailureCategory,
    reason: String,
}

impl GhFailure {
    fn network(reason: String) -> Self {
        Self {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason,
        }
    }
}

// gh exposes these failures only as human-readable stderr. Keep the classifier
// at this external boundary and preserve unknown errors verbatim as network failures.
fn classify_failure(reason: String, exit_code: Option<i32>) -> GhFailure {
    let lower = reason.to_ascii_lowercase();
    // The no-remote answer is read first: it ends by naming `gh auth login`
    // as the way to add a host (gh 2.76), and read as a login failure it
    // would be asked again on every read.
    let category = if lower.contains("no git remotes")
        || lower.contains("none of the git remotes")
        || lower.contains("not a github repository")
        || lower.contains("no github remote")
    {
        GithubFailureCategory::NoGithubRemote
    } else if exit_code == Some(4)
        || lower.contains("not logged")
        || lower.contains("gh auth login")
        || lower.contains("token") && lower.contains("invalid")
    {
        GithubFailureCategory::NotLoggedIn
    } else {
        GithubFailureCategory::NetworkOrRateLimit
    };
    GhFailure { category, reason }
}

/// Runs one allowed `gh` command for the account this node runs as, with no
/// prompt and a bounded wait; any other command line is refused unrun.
pub fn run(cwd: Option<&Path>, repository: Option<&str>, arguments: &[&str]) -> GhAnswer {
    if repository.is_some_and(|repository| !valid_repository(repository)) {
        return GhAnswer::Failed {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: "Unsupported gh repository".to_owned(),
        };
    }
    // Found and run on the account's own search: a core a login item runs
    // has the system's folders alone on its `PATH`.
    let gh = hide_platform::programs::command("gh");
    match run_gh_for(gh, cwd, repository, arguments, COMMAND_TIMEOUT) {
        Ok(stdout) => GhAnswer::Output {
            stdout: hide_node_link::gh::answered_output(arguments, stdout),
        },
        Err(failure) => GhAnswer::Failed {
            category: failure.category,
            reason: failure.reason,
        },
    }
}

#[cfg(test)]
fn run_gh(
    binary: &Path,
    cwd: Option<&Path>,
    arguments: &[&str],
    timeout: Duration,
) -> Result<String, GhFailure> {
    run_gh_for(Command::new(binary), cwd, None, arguments, timeout)
}

/// Runs one allowed `gh` command; `repository` is the repository a command
/// that reads it from its folder works on (`GH_REPO`).
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn run_gh_for(
    mut command: Command,
    cwd: Option<&Path>,
    repository: Option<&str>,
    arguments: &[&str],
    timeout: Duration,
) -> Result<String, GhFailure> {
    if !allowed(arguments) {
        return Err(GhFailure::network("Unsupported gh command".to_owned()));
    }
    command
        .args(arguments)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if let Some(repository) = repository {
        command.env("GH_REPO", repository);
    }
    let mut child = OwnedChild::spawn(&mut command).map_err(|error| GhFailure {
        category: if error.kind() == std::io::ErrorKind::NotFound {
            GithubFailureCategory::NotInstalled
        } else {
            GithubFailureCategory::NetworkOrRateLimit
        },
        reason: format!("gh could not be run: {error}"),
    })?;
    // Drain both pipes while waiting, otherwise a large PR list fills stdout
    // and the child cannot exit before the timeout.
    let mut stdout = child.take_stdout().expect("piped stdout");
    let mut stderr = child.take_stderr().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            outcome => {
                let reason = match outcome {
                    Err(error) => format!("gh wait failed: {error}"),
                    _ => format!("gh timed out after {} ms", timeout.as_millis()),
                };
                // Kill only the tree this invocation created, including helpers
                // retaining the pipe handles, so draining cannot outlive the deadline.
                let _ = child.kill_tree();
                let _ = child.wait();
                break Err(GhFailure::network(reason));
            }
        }
    };
    let stdout = out
        .join()
        .map_err(|_| GhFailure::network("gh stdout reader failed".to_owned()))?
        .map_err(|error| GhFailure::network(format!("gh stdout: {error}")))?;
    let stderr = err
        .join()
        .map_err(|_| GhFailure::network("gh stderr reader failed".to_owned()))?
        .map_err(|error| GhFailure::network(format!("gh stderr: {error}")))?;
    let status = status?;
    if !status.success() && !answered_with_errors(arguments, &stdout) {
        let reason = String::from_utf8_lossy(&stderr).trim().to_owned();
        return Err(classify_failure(
            if reason.is_empty() {
                format!("gh exited with {status}")
            } else {
                reason
            },
            status.code(),
        ));
    }
    String::from_utf8(stdout)
        .map_err(|error| GhFailure::network(format!("gh output was not UTF-8: {error}")))
}

/// Whether a `gh api graphql` that exited non-zero printed GitHub's answer:
/// `gh` exits 1 whenever the answer carries `errors`, though GitHub answered
/// the rest. A repository or issue one alias names that does not resolve is
/// `null` in `data` with a `NOT_FOUND` error beside it (gh 2.76, 2026-10-14),
/// so the answer is the command's output and the core reads its `errors`
/// alias by alias. An answer with no `data` (bad credentials, a refused
/// query) stays a failure.
fn answered_with_errors(arguments: &[&str], stdout: &[u8]) -> bool {
    arguments.starts_with(&["api", "graphql"])
        && serde_json::from_slice::<serde_json::Value>(stdout)
            .is_ok_and(|answer| answer.get("data").is_some_and(serde_json::Value::is_object))
}

// Every case drives a fake `gh` shell script.
#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use hide_node_link::gh::{
        ISSUE_DETAIL_FIELDS, ISSUE_LIST_FIELDS, ISSUE_LIST_LIMIT, ISSUE_LIST_SEARCH,
        OPEN_CHECK_FIELDS, PR_FEEDBACK_FIELDS, PULL_REQUEST_LIMIT,
    };

    use super::*;

    const FIXTURE_DEADLINE: Duration = Duration::from_secs(10);

    /// The two list reads the core makes, as the node allows them.
    const PR_LIST: [&str; 8] = [
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        PULL_REQUEST_LIMIT,
        "--json",
        OPEN_CHECK_FIELDS,
    ];
    const ISSUE_LIST: [&str; 10] = [
        "issue",
        "list",
        "--state",
        "open",
        "--limit",
        ISSUE_LIST_LIMIT,
        "--search",
        ISSUE_LIST_SEARCH,
        "--json",
        ISSUE_LIST_FIELDS,
    ];

    #[test]
    fn gh_boundary_is_read_only_noninteractive_and_preserves_failure_categories() {
        let fixture = GhFixture::new(
            r#"
[ "$GH_PROMPT_DISABLED" = 1 ] && [ "$GIT_TERMINAL_PROMPT" = 0 ] || exit 90
case "$1 $2" in
  "pr list"|"issue list") printf '[]';;
  "auth status") printf 'not logged into any GitHub hosts' >&2; exit 1;;
  *) touch forbidden; exit 91;;
esac"#,
        );
        assert_eq!(
            run_gh(
                &fixture.binary,
                Some(&fixture.root),
                &PR_LIST,
                FIXTURE_DEADLINE
            )
            .unwrap(),
            "[]"
        );
        assert_eq!(
            run_gh(
                &fixture.binary,
                Some(&fixture.root),
                &ISSUE_LIST,
                FIXTURE_DEADLINE
            )
            .unwrap(),
            "[]"
        );
        for write in [
            ["issue", "create"],
            ["issue", "edit"],
            ["issue", "comment"],
            ["issue", "close"],
        ] {
            assert!(
                run_gh(
                    &fixture.binary,
                    Some(&fixture.root),
                    &write,
                    FIXTURE_DEADLINE
                )
                .is_err()
            );
        }
        let failure = run_gh(
            &fixture.binary,
            Some(&fixture.root),
            &["auth", "status"],
            FIXTURE_DEADLINE,
        )
        .unwrap_err();
        assert_eq!(failure.category, GithubFailureCategory::NotLoggedIn);
        assert_eq!(failure.reason, "not logged into any GitHub hosts");
        assert!(
            run_gh(
                &fixture.binary,
                Some(&fixture.root),
                &["auth", "login"],
                FIXTURE_DEADLINE
            )
            .is_err()
        );
        assert!(!fixture.root.join("forbidden").exists());
        assert_eq!(
            std::fs::read_dir(&fixture.root).unwrap().count(),
            1,
            "no token or configuration was written"
        );
        assert_eq!(
            run_gh(
                &fixture.root.join("missing"),
                None,
                &PR_LIST,
                FIXTURE_DEADLINE
            )
            .unwrap_err()
            .category,
            GithubFailureCategory::NotInstalled
        );
        for (stderr, expected) in [
            (
                "None of the git remotes configured for this repository point to a known GitHub host. To tell gh about a new GitHub host, please use `gh auth login`",
                GithubFailureCategory::NoGithubRemote,
            ),
            (
                "HTTP 429 rate limit exceeded",
                GithubFailureCategory::NetworkOrRateLimit,
            ),
        ] {
            let fixture = GhFixture::new(&format!("printf '%s' '{stderr}' >&2; exit 1"));
            let failure = run_gh(&fixture.binary, None, &PR_LIST, FIXTURE_DEADLINE).unwrap_err();
            assert_eq!(failure.category, expected);
            assert_eq!(failure.reason, stderr);
        }
    }

    /// `gh api graphql` exits 1 when one alias does not resolve, with the
    /// others answered on stdout (gh 2.76): that answer reaches the core.
    #[test]
    fn a_graphql_answer_with_an_unresolved_alias_is_output_and_one_without_data_fails() {
        const PARTIAL: &str = r#"{"data":{"r0":{"issue":{"number":7}},"r1":null},"errors":[{"type":"NOT_FOUND","path":["r1"],"message":"Could not resolve to a Repository with the name acme/gone."}]}"#;
        let query = "query=query HideLinkedIssues {}";
        let fixture = GhFixture::new(&format!(
            r#"
case "$1 $2" in
  "api graphql") printf '%s' '{PARTIAL}'; printf 'gh: Could not resolve' >&2; exit 1;;
  "pr list") printf '{{"data":{{}}}}'; printf 'HTTP 502' >&2; exit 1;;
esac"#
        ));
        let answer = run_gh(
            &fixture.binary,
            None,
            &["api", "graphql", "-f", query],
            FIXTURE_DEADLINE,
        )
        .expect("the answered aliases are output");
        assert_eq!(answer, PARTIAL);
        // Only a GraphQL answer is taken from a failed command.
        let failure = run_gh(&fixture.binary, None, &PR_LIST, FIXTURE_DEADLINE).unwrap_err();
        assert_eq!(failure.reason, "HTTP 502");
        let fixture = GhFixture::new(
            r#"printf '{"message":"Bad credentials"}'; printf 'gh: Bad credentials (HTTP 401)' >&2; exit 1"#,
        );
        let failure = run_gh(
            &fixture.binary,
            None,
            &["api", "graphql", "-f", query],
            FIXTURE_DEADLINE,
        )
        .unwrap_err();
        assert_eq!(failure.reason, "gh: Bad credentials (HTTP 401)");
    }

    #[test]
    fn timed_out_gh_and_its_pipe_holding_helper_are_terminated() {
        // The helper holds gh's stdout and stderr for its whole minute.
        let helper_lifetime = Duration::from_secs(60);
        let fixture = GhFixture::new("echo $$ > pid; sleep 60");
        let started = Instant::now();
        let failure = run_gh(
            &fixture.binary,
            Some(&fixture.root),
            &PR_LIST,
            Duration::from_secs(3),
        )
        .unwrap_err();
        assert_eq!(failure.category, GithubFailureCategory::NetworkOrRateLimit);
        assert!(failure.reason.contains("timed out"));
        // `run_gh` answers only once both pipes reach end of file, and gh and
        // every helper it started hold them until they end, so an answer
        // before the helper's own end proves the whole tree was killed. That
        // holds whether or not a loaded machine let the shell reach its first
        // line before the deadline.
        assert!(
            started.elapsed() < helper_lifetime,
            "gh or its helper outlived the deadline holding the pipes"
        );
        // A shell that got as far as writing its PID is gone itself too; one
        // killed before that line wrote nothing or an empty file.
        let pid = match std::fs::read_to_string(fixture.root.join("pid")) {
            Ok(text) => text.trim().to_owned(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("reading the gh PID: {error}"),
        };
        if !pid.is_empty() {
            let pid: u32 = pid.parse().unwrap();
            assert!(
                !hide_platform::process::is_alive(pid),
                "the gh child has ended"
            );
        }
    }

    #[test]
    fn an_issue_panel_reads_only_its_fields_and_nothing_else_passes_issue_view() {
        let fixture = GhFixture::new(
            r#"
case "$1 $2 $6 $7" in
  "issue view --json body,labels,author,assignees,comments,createdAt") printf '{"body":"b","comments":[]}';;
  *) touch forbidden; exit 91;;
esac"#,
        );
        let viewed = |fields: &str| {
            run_gh(
                &fixture.binary,
                Some(&fixture.root),
                &["issue", "view", "7", "--repo", "acme/app", "--json", fields],
                FIXTURE_DEADLINE,
            )
        };
        assert!(viewed(ISSUE_DETAIL_FIELDS).is_ok());
        assert!(viewed("body,title").is_err());
        assert!(!fixture.root.join("forbidden").exists());
    }

    #[test]
    fn a_pull_request_takes_only_its_body_write_its_two_reads_and_the_two_conditioned_writes() {
        let fixture = GhFixture::new(r#"printf 'ok'"#);
        let run = |arguments: &[&str]| {
            run_gh(
                &fixture.binary,
                Some(&fixture.root),
                arguments,
                Duration::from_secs(5),
            )
        };
        for allowed in [
            &["pr", "view", "12", "--json", "body"][..],
            &["pr", "view", "12", "--json", PR_FEEDBACK_FIELDS],
            &["pr", "edit", "12", "--body", "Closes #7"],
            &[
                "pr",
                "merge",
                "12",
                "--squash",
                "--match-head-commit",
                "0123456789abcdef0123456789abcdef01234567",
            ],
            &[
                "api",
                "--method",
                "PUT",
                "repos/acme/app/pulls/12/update-branch",
                "-f",
                "expected_head_sha=0123456789abcdef0123456789abcdef01234567",
            ],
        ] {
            if let Err(failure) = run(allowed) {
                panic!("{allowed:?} must run: {}", failure.reason);
            }
        }
        for refused in [
            &["pr", "edit", "12", "--title", "x"][..],
            &["pr", "edit", "12", "--add-label", "x"],
            &["pr", "edit", "x12", "--body", "y"],
            &["pr", "edit", "12", "--body", "y", "--title", "z"],
            &["pr", "view", "12", "--json", "body,title"],
            &["pr", "merge", "12"],
            &[
                "pr",
                "merge",
                "12",
                "--admin",
                "--match-head-commit",
                "0123456789abcdef0123456789abcdef01234567",
            ],
            &["pr", "merge", "12", "--squash"],
            &["pr", "comment", "12", "--body", "y"],
            &["pr", "review", "12", "--approve"],
            &["pr", "close", "12"],
        ] {
            assert!(run(refused).is_err(), "{refused:?} must be refused");
        }
    }

    struct GhFixture {
        root: PathBuf,
        binary: PathBuf,
    }

    impl GhFixture {
        fn new(body: &str) -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "hide-gh-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            let binary = root.join("gh");
            crate::executable_fixture::write_executable(&binary, &format!("#!/bin/sh\n{body}\n"));
            Self { root, binary }
        }
    }

    impl Drop for GhFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
