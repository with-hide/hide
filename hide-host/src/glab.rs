//! The node's `glab`: the one bounded, non-interactive way Hide runs
//! GitLab's CLI with the operator's gitlab.com login, for the commands
//! `hide_node_link::glab::allowed` names. The core parses what it answers.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_node_link::gh::GithubFailureCategory;
use hide_node_link::glab::{ANSWER_BYTE_LIMIT, GlabAnswer, allowed};
use hide_platform::process::OwnedChild;

/// How long one `glab` command may run, as one `gh` command may.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
struct GlabFailure {
    category: GithubFailureCategory,
    reason: String,
}

impl GlabFailure {
    fn other(reason: String) -> Self {
        Self {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason,
        }
    }
}

/// What a finished command printed: its output up to the cap, and whether
/// it printed more.
#[derive(Debug)]
struct Printed {
    stdout: Vec<u8>,
    cut: bool,
}

// glab exposes these failures only as human-readable stderr. Keep the
// classifier at this external boundary and keep unknown errors verbatim as
// network failures, as `gh`'s does.
fn classify_failure(reason: String) -> GlabFailure {
    let lower = reason.to_ascii_lowercase();
    let category = if lower.contains("401")
        || lower.contains("invalid token")
        || lower.contains("no token found")
        || lower.contains("glab auth login")
        || lower.contains("not authenticated")
    {
        GithubFailureCategory::NotLoggedIn
    } else {
        GithubFailureCategory::NetworkOrRateLimit
    };
    GlabFailure { category, reason }
}

/// Runs one allowed `glab` command for the account this node runs as, with
/// no prompt and a bounded wait; any other command line is refused unrun.
pub fn run(arguments: &[&str]) -> GlabAnswer {
    // Found and run on the account's own search: a core a login item runs
    // has the system's folders alone on its `PATH`.
    let glab = hide_platform::programs::command("glab");
    match run_glab_for(glab, arguments, COMMAND_TIMEOUT, ANSWER_BYTE_LIMIT) {
        Ok(printed) => match String::from_utf8(printed.stdout) {
            Ok(stdout) => GlabAnswer::Output {
                stdout,
                cut: printed.cut,
            },
            // A cut can fall inside a character; the answer is cut anyway.
            Err(error) if printed.cut => GlabAnswer::Output {
                stdout: String::from_utf8_lossy(error.as_bytes()).into_owned(),
                cut: true,
            },
            Err(error) => GlabAnswer::Failed {
                category: GithubFailureCategory::NetworkOrRateLimit,
                reason: format!("glab output was not UTF-8: {error}"),
            },
        },
        Err(failure) => GlabAnswer::Failed {
            category: failure.category,
            reason: failure.reason,
        },
    }
}

/// Reads all of `reader`, keeping its first `limit` bytes: the rest is read
/// and dropped so the child never blocks on a full pipe.
fn read_capped(mut reader: impl Read, limit: usize) -> std::io::Result<Printed> {
    let mut stdout = Vec::new();
    let mut cut = false;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(Printed { stdout, cut });
        }
        let room = limit.saturating_sub(stdout.len());
        if read > room {
            cut = true;
        }
        stdout.extend_from_slice(&buffer[..read.min(room)]);
    }
}

/// Runs one allowed `glab` command, keeping at most `limit` bytes of its
/// output. It runs in no folder of the operator's: every line names its
/// server and project, and `glab` would otherwise read a folder's remote.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn run_glab_for(
    mut command: Command,
    arguments: &[&str],
    timeout: Duration,
    limit: usize,
) -> Result<Printed, GlabFailure> {
    if !allowed(arguments) {
        return Err(GlabFailure::other("Unsupported glab command".to_owned()));
    }
    command
        .args(arguments)
        .current_dir(std::env::temp_dir())
        // No prompt, no pager, no colour, no update check and no usage
        // report: the call answers its question and nothing else.
        .env("GLAB_NO_PROMPT", "true")
        .env("GLAB_CHECK_UPDATE", "false")
        .env("GLAB_SEND_TELEMETRY", "false")
        .env("GLAB_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = OwnedChild::spawn(&mut command).map_err(|error| GlabFailure {
        category: if error.kind() == std::io::ErrorKind::NotFound {
            GithubFailureCategory::NotInstalled
        } else {
            GithubFailureCategory::NetworkOrRateLimit
        },
        reason: format!("glab could not be run: {error}"),
    })?;
    // Drain both pipes while waiting, otherwise a large answer fills stdout
    // and the child cannot exit before the timeout.
    let stdout = child.take_stdout().expect("piped stdout");
    let stderr = child.take_stderr().expect("piped stderr");
    let out = std::thread::spawn(move || read_capped(stdout, limit));
    let err = std::thread::spawn(move || read_capped(stderr, 64 * 1024));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            outcome => {
                let reason = match outcome {
                    Err(error) => format!("glab wait failed: {error}"),
                    _ => format!("glab timed out after {} ms", timeout.as_millis()),
                };
                // Kill only the tree this invocation created, so draining
                // cannot outlive the deadline.
                let _ = child.kill_tree();
                let _ = child.wait();
                break Err(GlabFailure::other(reason));
            }
        }
    };
    let printed = out
        .join()
        .map_err(|_| GlabFailure::other("glab stdout reader failed".to_owned()))?
        .map_err(|error| GlabFailure::other(format!("glab stdout: {error}")))?;
    let stderr = err
        .join()
        .map_err(|_| GlabFailure::other("glab stderr reader failed".to_owned()))?
        .map_err(|error| GlabFailure::other(format!("glab stderr: {error}")))?;
    let status = status?;
    if !status.success() && !answered_with_errors(arguments, &printed.stdout) {
        let reason = String::from_utf8_lossy(&stderr.stdout).trim().to_owned();
        let stdout = String::from_utf8_lossy(&printed.stdout).trim().to_owned();
        return Err(classify_failure(
            match (reason.is_empty(), stdout.is_empty()) {
                (false, _) => reason,
                // `glab api` prints GitLab's error answer on stdout.
                (true, false) => stdout,
                (true, true) => format!("glab exited with {status}"),
            },
        ));
    }
    Ok(printed)
}

/// Whether a `glab api graphql` that exited non-zero printed GitLab's
/// answer: an answer with `data` beside `errors` is GitLab answering part of
/// the query, which the core reads; an answer with no `data` (a refused
/// token, a query over the complexity limit) stays a failure.
fn answered_with_errors(arguments: &[&str], stdout: &[u8]) -> bool {
    arguments.contains(&"graphql")
        && serde_json::from_slice::<serde_json::Value>(stdout)
            .is_ok_and(|answer| answer.get("data").is_some_and(serde_json::Value::is_object))
}

// Every case drives a fake `glab` shell script.
#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use hide_node_link::glab::{AUTH_STATUS, VERSION, merge_requests_arguments};

    use super::*;

    const FIXTURE_DEADLINE: Duration = Duration::from_secs(10);

    struct GlabFixture {
        root: PathBuf,
        binary: PathBuf,
    }

    impl GlabFixture {
        fn new(body: &str) -> Self {
            let root = tempfile::tempdir().unwrap().keep();
            let binary = root.join("glab");
            crate::executable_fixture::write_executable(&binary, &format!("#!/bin/sh\n{body}\n"));
            Self { root, binary }
        }

        fn run(&self, arguments: &[&str], limit: usize) -> Result<Printed, GlabFailure> {
            run_glab_for(
                Command::new(&self.binary),
                arguments,
                FIXTURE_DEADLINE,
                limit,
            )
        }
    }

    impl Drop for GlabFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn list() -> Vec<String> {
        merge_requests_arguments("group/sub/project").unwrap()
    }

    #[test]
    fn glab_runs_without_prompts_outside_any_folder_and_refuses_other_lines() {
        let fixture = GlabFixture::new(
            r#"
[ "$GLAB_NO_PROMPT" = true ] && [ "$GLAB_CHECK_UPDATE" = false ] && [ "$GLAB_SEND_TELEMETRY" = false ] || exit 90
[ -d .git ] && exit 93
case "$1" in
  api) printf '{"data":{}}';;
  auth) printf 'gitlab.com\n  x gitlab.com: API call failed: GET https://gitlab.com/api/v4/user: 401 {message: 401 Unauthorized}\n' >&2; exit 1;;
  --version) printf 'glab 1.78.0 (fb43432d)\n';;
  *) touch "$(dirname "$0")/forbidden"; exit 91;;
esac"#,
        );
        let list = list();
        let words: Vec<&str> = list.iter().map(String::as_str).collect();
        let printed = fixture.run(&words, ANSWER_BYTE_LIMIT).unwrap();
        assert_eq!(printed.stdout, br#"{"data":{}}"#);
        assert!(!printed.cut);
        assert_eq!(
            fixture.run(&VERSION, ANSWER_BYTE_LIMIT).unwrap().stdout,
            b"glab 1.78.0 (fb43432d)\n"
        );
        let failure = fixture.run(&AUTH_STATUS, ANSWER_BYTE_LIMIT).unwrap_err();
        assert_eq!(failure.category, GithubFailureCategory::NotLoggedIn);
        for refused in [
            &["mr", "merge", "12", "--yes"][..],
            &["api", "user"],
            &["auth", "login"],
            &[
                "api",
                "--hostname",
                "gitlab.com",
                "-X",
                "PUT",
                "projects/1/merge_requests/2/merge",
            ],
        ] {
            assert!(
                fixture.run(refused, ANSWER_BYTE_LIMIT).is_err(),
                "{refused:?}"
            );
        }
        assert!(!fixture.root.join("forbidden").exists());
    }

    #[test]
    fn an_answer_past_the_cap_is_cut_and_says_so() {
        let fixture = GlabFixture::new("head -c 300000 /dev/zero | tr '\\0' a");
        let list = list();
        let words: Vec<&str> = list.iter().map(String::as_str).collect();
        let printed = fixture.run(&words, 1000).unwrap();
        assert_eq!(printed.stdout.len(), 1000);
        assert!(printed.cut);
        let whole = fixture.run(&words, 300_000).unwrap();
        assert_eq!(whole.stdout.len(), 300_000);
        assert!(!whole.cut);
    }

    #[test]
    fn failures_keep_their_categories() {
        let missing = run_glab_for(
            Command::new("/nonexistent/glab"),
            &VERSION,
            FIXTURE_DEADLINE,
            ANSWER_BYTE_LIMIT,
        )
        .unwrap_err();
        assert_eq!(missing.category, GithubFailureCategory::NotInstalled);
        for (body, expected) in [
            (
                r#"printf '{"errors":[{"message":"Invalid token"}]}'; echo 'glab: Invalid token' >&2; exit 1"#,
                GithubFailureCategory::NotLoggedIn,
            ),
            (
                r#"printf '{"errors":[{"message":"Query has complexity of 271, which exceeds max complexity of 250"}]}'; exit 1"#,
                GithubFailureCategory::NetworkOrRateLimit,
            ),
            (
                "echo 'proxyconnect tcp: dial tcp 127.0.0.1:9: connect: connection refused' >&2; exit 1",
                GithubFailureCategory::NetworkOrRateLimit,
            ),
        ] {
            let fixture = GlabFixture::new(body);
            let list = list();
            let words: Vec<&str> = list.iter().map(String::as_str).collect();
            assert_eq!(
                fixture.run(&words, ANSWER_BYTE_LIMIT).unwrap_err().category,
                expected,
                "{body}"
            );
        }
        // GitLab answered part of the query: the answer is the output.
        let fixture = GlabFixture::new(
            r#"printf '{"data":{"project":null},"errors":[{"message":"x"}]}'; exit 1"#,
        );
        let list = list();
        let words: Vec<&str> = list.iter().map(String::as_str).collect();
        assert!(fixture.run(&words, ANSWER_BYTE_LIMIT).is_ok());
    }

    #[test]
    fn a_command_past_its_deadline_is_ended() {
        let fixture = GlabFixture::new("sleep 30");
        let failure = run_glab_for(
            Command::new(&fixture.binary),
            &VERSION,
            Duration::from_millis(200),
            ANSWER_BYTE_LIMIT,
        )
        .unwrap_err();
        assert!(failure.reason.contains("timed out"), "{}", failure.reason);
    }

    /// Every command line the node runs `glab` with, as the core builds it,
    /// against the oldest release Hide supports
    /// ([`hide_node_link::glab::MINIMUM_VERSION`]). `glab` checks a
    /// command's flags before it asks GitLab anything, so each line must get
    /// as far as the network, which a proxy that refuses every connection
    /// stops there: no line reaches GitLab, and none needs a login.
    /// Ignored by default because it needs that release: the rust lane
    /// downloads it (`scripts/fetch-glab.sh`) and names it in
    /// `HIDE_TEST_GLAB_BIN`. A new shape in `allowed` gets a line here.
    #[test]
    #[ignore = "needs the minimum glab release named in HIDE_TEST_GLAB_BIN"]
    fn every_command_line_parses_on_the_oldest_glab_hide_supports() {
        use hide_node_link::glab::{
            GlabVersion, MINIMUM_VERSION, card_arguments, commit_checks_arguments, merge_arguments,
            merge_base_arguments, merge_request_arguments, merged_arguments,
        };
        const PROJECT: &str = "acme/sub/app";
        const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
        let glab = PathBuf::from(
            std::env::var_os("HIDE_TEST_GLAB_BIN")
                .expect("HIDE_TEST_GLAB_BIN names the minimum glab release"),
        );
        let home = tempfile::tempdir().unwrap();
        let run = |arguments: &[&str]| {
            let mut command = Command::new(&glab);
            command
                .env("HOME", home.path())
                .env("GLAB_CONFIG_DIR", home.path())
                .env("GITLAB_TOKEN", "not-a-token")
                .env("HTTPS_PROXY", "http://127.0.0.1:9")
                .env("https_proxy", "http://127.0.0.1:9");
            for name in [
                "GITLAB_HOST",
                "GITLAB_URI",
                "GITLAB_URL",
                "GITLAB_ACCESS_TOKEN",
                "OAUTH_TOKEN",
                "NO_PROXY",
                "no_proxy",
            ] {
                command.env_remove(name);
            }
            run_glab_for(
                command,
                arguments,
                Duration::from_secs(30),
                ANSWER_BYTE_LIMIT,
            )
        };
        let version = run(&VERSION).expect("glab --version runs");
        let version = String::from_utf8(version.stdout).unwrap();
        assert_eq!(
            GlabVersion::of_output(&version),
            Some(MINIMUM_VERSION),
            "HIDE_TEST_GLAB_BIN is not the minimum release: {version}"
        );
        let owned = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect();
        let mut lines: Vec<Vec<String>> = vec![owned(&AUTH_STATUS)];
        let built = [
            merge_requests_arguments(PROJECT),
            card_arguments(PROJECT, 12),
            commit_checks_arguments(PROJECT, &[HEAD.to_owned()]),
            commit_checks_arguments(PROJECT, &[HEAD.to_owned(), "f".repeat(40)]),
            merge_request_arguments(PROJECT, 12),
            merged_arguments(PROJECT, "topic/one"),
            merge_base_arguments(PROJECT, HEAD, &"f".repeat(40)),
            // The write reads the merge request first, which the proxy
            // stops: nothing is merged.
            merge_arguments(PROJECT, 12, HEAD),
        ];
        for (index, line) in built.into_iter().enumerate() {
            lines.push(line.unwrap_or_else(|| panic!("built line {index} is not a valid line")));
        }
        let refused: Vec<String> = lines
            .iter()
            .filter_map(|line| {
                let words: Vec<&str> = line.iter().map(String::as_str).collect();
                let reason = match run(&words) {
                    Ok(printed) => {
                        return Some(format!(
                            "{}: answered without asking GitLab: {}",
                            words.join(" "),
                            String::from_utf8_lossy(&printed.stdout)
                        ));
                    }
                    Err(failure) => failure.reason,
                };
                (!reason.contains("proxyconnect")).then(|| {
                    format!(
                        "{}: {}",
                        words.join(" "),
                        reason.lines().next().unwrap_or_default()
                    )
                })
            })
            .collect();
        assert!(
            refused.is_empty(),
            "glab {MINIMUM_VERSION} refused before asking GitLab:\n{}",
            refused.join("\n")
        );
    }
}
