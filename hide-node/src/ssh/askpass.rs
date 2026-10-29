//! Sign-in questions from a system `ssh` to the device row (PRD
//! system-ssh-device-link D-05, D-06, D-08, B6-B12).
//!
//! Each connection attempt opens a [`Desk`]: a local endpoint only this
//! account can reach, named to the `ssh` it starts with a random token. ssh
//! runs Hide's `hide` program as its askpass (`SSH_ASKPASS_REQUIRE=force`,
//! so never a terminal), which hands the prompt to the desk ([`client`]) and
//! prints the answer the desk sends back. The desk decides what kind of
//! question it is ([`asked`], the one place that reads ssh's sentences) and
//! puts it to the attempt's [`SignIn`], which is the row. An answer goes from
//! the row to ssh through this endpoint and nowhere else: it is never written
//! to a file, a log or a snapshot (B10).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hide_node_link::device::{Answer, Refusal, SignIn, SignInWait};
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream};
use serde::{Deserialize, Serialize};

/// The variables that make `hide` an askpass, and where it reaches its desk.
pub const DESK_VARIABLE: &str = "HIDE_SSH_ASKPASS";
pub const TOKEN_VARIABLE: &str = "HIDE_SSH_ASKPASS_TOKEN";

/// The most a prompt or an answer may be; ssh reads at most 1023 bytes of an
/// askpass answer, and its longest prompt (a new host key) is a few lines.
const MAX_MESSAGE: usize = 16 * 1024;
/// How long an askpass has to say what it asks once connected.
const REQUEST_WITHIN: Duration = Duration::from_secs(10);
/// How long the askpass waits for its answer: past the row's own bound
/// (five minutes, D-05), so the row, not the askpass, decides when it ends.
const ANSWER_WITHIN: Duration = Duration::from_secs(6 * 60);
/// How long a security key waits for its touch (D-05's five minutes): past
/// it the attempt ends as a sign-in nobody answered, as a question does.
const TOUCH_WITHIN: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(5 * 60)
};
/// Askpass connections a desk serves at once. ssh asks one question at a
/// time; anything past this is refused, not queued.
const MAX_ASKING: usize = 4;
/// A desk's folder name: this prefix, the opening process's id, a dash and
/// random hex.
const DESK_PREFIX: &str = "hide-ssh-";

/// Removes the desk folders under `parent` whose opening process is gone: a
/// process that died without closing its desk (killed, or its machine lost
/// power) leaves its folder, which held only the socket of a desk no `ssh`
/// can reach now. A folder of a live process, or another account's, stays.
fn sweep(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name
            .to_str()
            .and_then(|name| name.strip_prefix(DESK_PREFIX))
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if owner != std::process::id()
            && !hide_platform::process::is_alive(owner)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// What ssh asked, as the row shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asked {
    /// A question the operator answers.
    Question(SignInWait),
    /// A notice that ends by itself: a security key waiting for a touch
    /// (`SSH_ASKPASS_PROMPT=none`, B11). ssh ends the askpass when it is done.
    Touch(String),
}

/// Reads ssh's prompt (its askpass argument) and `SSH_ASKPASS_PROMPT`. A
/// sentence this does not know is a secret to type, shown masked: the safe
/// way to be wrong (PRD Risks).
pub fn asked(prompt: &str, hint: Option<&str>) -> Asked {
    let prompt = prompt.trim_end_matches(['\n', '\r']);
    match hint {
        Some("none") => return Asked::Touch(prompt.trim().to_owned()),
        Some("confirm") => {
            return Asked::Question(SignInWait::Confirm {
                prompt: prompt.trim().to_owned(),
            });
        }
        _ => {}
    }
    if prompt.contains("Are you sure you want to continue connecting") {
        let host = between(prompt, "The authenticity of host '", "'")
            .unwrap_or_default()
            .to_owned();
        let fingerprint = prompt
            .split_whitespace()
            .find(|word| word.starts_with("SHA256:") || word.starts_with("MD5:"))
            .map(|word| word.trim_end_matches('.').to_owned())
            .unwrap_or_default();
        return Asked::Question(SignInWait::HostKey { host, fingerprint });
    }
    if prompt.contains("(yes/no)") || prompt.starts_with("Please type 'yes'") {
        return Asked::Question(SignInWait::Confirm {
            prompt: prompt.trim().to_owned(),
        });
    }
    let key_file = prompt
        .starts_with("Enter passphrase for")
        .then(|| between(prompt, "key '", "'"))
        .flatten()
        .map(str::to_owned);
    Asked::Question(SignInWait::Secret {
        prompt: prompt.trim().to_owned(),
        key_file,
    })
}

/// How much of a key's path ssh prints in its passphrase prompt: `%.100s`
/// bytes (OpenSSH `sshconnect2.c`).
const PROMPT_PATH_BYTES: usize = 100;

/// The whole path of the key a passphrase prompt names as `shown`: `shown`
/// itself when ssh printed all of it, else the one key file of the alias it
/// begins. `None` when that is not certain, a relative path or a shortened
/// one no single known file begins, so the row offers no command naming a
/// file that is not the key (B9).
fn key_file(shown: &str, key_files: Option<&[PathBuf]>) -> Option<String> {
    if !Path::new(shown).is_absolute() {
        return None;
    }
    if shown.len() < PROMPT_PATH_BYTES {
        return Some(shown.to_owned());
    }
    let mut whole = key_files?
        .iter()
        .filter_map(|file| file.to_str())
        .filter(|file| file.starts_with(shown));
    let file = whole.next()?;
    whole.next().is_none().then(|| file.to_owned())
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &text[text.find(start)? + start.len()..];
    Some(&rest[..rest.find(end)?])
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
    /// `SSH_ASKPASS_PROMPT` as ssh set it.
    #[serde(default)]
    hint: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Reply {
    /// What to print for ssh; absent when there is no answer.
    #[serde(default)]
    answer: Option<String>,
}

/// One attempt's endpoint for its `ssh`'s askpass.
pub struct Desk {
    path: PathBuf,
    folder: PathBuf,
    token: String,
    closer: ListenerCloser,
    accepting: Option<std::thread::JoinHandle<()>>,
    state: Arc<DeskState>,
}

struct DeskState {
    sign_in: Arc<dyn SignIn>,
    token: String,
    /// The key files ssh tries for the alias, whole; `None` when they are
    /// not all known ([`key_file`]).
    key_files: Option<Vec<PathBuf>>,
    /// The first question the attempt got no answer to, and why. Once set,
    /// that question and every later one are held unanswered until the desk
    /// closes ([`hold`]), and the attempt ends at its next look (B8, B14).
    refusal: Mutex<Option<Refusal>>,
    /// Wakes the questions [`hold`] keeps once the desk closes; it waits on
    /// `refusal`'s lock.
    held: Condvar,
    /// Questions and touches the operator holds now.
    operator: AtomicUsize,
    asking: AtomicUsize,
    closed: AtomicBool,
}

impl Desk {
    /// Opens a desk whose questions go to `sign_in`, in a private folder of
    /// its own under `parent`, and removes the folders desks there left when
    /// the process that opened them died without closing them. `key_files`
    /// are the alias's key files, whole, for a passphrase prompt that names
    /// one shortened.
    pub fn open(
        parent: &Path,
        sign_in: Arc<dyn SignIn>,
        key_files: Option<Vec<PathBuf>>,
    ) -> std::io::Result<Self> {
        sweep(parent);
        let token = random_hex(16)?;
        // Short, because a Unix socket's path is limited to about a hundred
        // bytes; the opening process's id names whose it is.
        let folder = parent.join(format!(
            "{DESK_PREFIX}{}-{}",
            std::process::id(),
            random_hex(6)?
        ));
        hide_platform::fs::private::create_dir_all(&folder)?;
        let path = folder.join("askpass");
        let listener = match LocalListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                return Err(error);
            }
        };
        let closer = listener.closer();
        let state = Arc::new(DeskState {
            sign_in,
            token: token.clone(),
            key_files,
            refusal: Mutex::new(None),
            held: Condvar::new(),
            operator: AtomicUsize::new(0),
            asking: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        });
        let serving = Arc::clone(&state);
        let accepting = std::thread::Builder::new()
            .name("ssh-askpass-desk".into())
            .spawn(move || accept(&listener, &serving));
        let accepting = match accepting {
            Ok(handle) => handle,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                return Err(error);
            }
        };
        Ok(Self {
            path,
            folder,
            token,
            closer,
            accepting: Some(accepting),
            state,
        })
    }

    /// The variables the attempt's `ssh` starts with.
    pub fn environment(&self, askpass: &Path) -> Vec<(&'static str, std::ffi::OsString)> {
        vec![
            ("SSH_ASKPASS", askpass.as_os_str().to_owned()),
            ("SSH_ASKPASS_REQUIRE", "force".into()),
            (DESK_VARIABLE, self.path.as_os_str().to_owned()),
            (TOKEN_VARIABLE, self.token.clone().into()),
        ]
    }

    /// The first question of this attempt that got no answer, and why.
    pub fn refusal(&self) -> Option<Refusal> {
        *lock(&self.state.refusal)
    }

    /// Whether the operator holds a question or a touch now, so the attempt's
    /// own clock waits for them (their bound is the row's).
    pub fn operator_turn(&self) -> bool {
        self.state.operator.load(Ordering::SeqCst) > 0
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        {
            // Under the lock `hold` waits on, so no held question misses it.
            let _refusal = lock(&self.state.refusal);
            self.state.closed.store(true, Ordering::SeqCst);
        }
        self.state.held.notify_all();
        self.closer.close();
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn accept(listener: &LocalListener, state: &Arc<DeskState>) {
    loop {
        let Ok(stream) = listener.accept() else {
            return;
        };
        if state.closed.load(Ordering::SeqCst) {
            return;
        }
        if state.asking.fetch_add(1, Ordering::SeqCst) >= MAX_ASKING {
            state.asking.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let serving = Arc::clone(state);
        // Each question waits for the operator on its own thread, which ends
        // when the row answers, refuses or the attempt ends: never longer
        // than the row's bound.
        let started = std::thread::Builder::new()
            .name("ssh-askpass-question".into())
            .spawn(move || {
                serve(stream, &serving);
                serving.asking.fetch_sub(1, Ordering::SeqCst);
            });
        if started.is_err() {
            state.asking.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn serve(stream: LocalStream, state: &DeskState) {
    if !stream.peer_is_this_account().unwrap_or(false) {
        return;
    }
    let reading = stream.duplicate();
    reading.set_deadline(Instant::now() + REQUEST_WITHIN);
    let mut line = Vec::new();
    if BufReader::new(reading.take(MAX_MESSAGE as u64))
        .read_until(b'\n', &mut line)
        .is_err()
    {
        return;
    }
    let Ok(request) = serde_json::from_slice::<Request>(&line) else {
        return;
    };
    if request.token != state.token {
        return;
    }
    let reply = match asked(&request.prompt, request.hint.as_deref()) {
        Asked::Touch(prompt) => {
            state.operator.fetch_add(1, Ordering::SeqCst);
            state.sign_in.show(Some(SignInWait::Touch { prompt }));
            // ssh ends the askpass once the key was touched; its end is this
            // stream's end. A touch that never comes ends the attempt.
            let mut rest = stream.duplicate();
            let until = Instant::now() + TOUCH_WITHIN;
            rest.set_deadline(until);
            let read = std::io::copy(
                &mut (&mut rest).take(MAX_MESSAGE as u64),
                &mut std::io::sink(),
            );
            state.sign_in.show(None);
            state.operator.fetch_sub(1, Ordering::SeqCst);
            if read.is_err() && Instant::now() >= until {
                lock(&state.refusal).get_or_insert(Refusal::TimedOut);
            }
            return;
        }
        Asked::Question(_) if lock(&state.refusal).is_some() => return hold(state),
        Asked::Question(mut question) => {
            if let SignInWait::Secret { key_file, .. } = &mut question {
                *key_file = key_file
                    .take()
                    .and_then(|shown| self::key_file(&shown, state.key_files.as_deref()));
            }
            state.operator.fetch_add(1, Ordering::SeqCst);
            let answer = state.sign_in.ask(question);
            state.operator.fetch_sub(1, Ordering::SeqCst);
            match answer {
                Ok(Answer::Text(secret)) => Reply {
                    answer: Some(secret.expose().to_owned()),
                },
                Ok(Answer::Accept) => Reply {
                    answer: Some("yes".to_owned()),
                },
                Err(refusal) => {
                    lock(&state.refusal).get_or_insert(refusal);
                    return hold(state);
                }
            }
        }
    };
    let mut writing = stream;
    writing.set_deadline(Instant::now() + REQUEST_WITHIN);
    if let Ok(mut bytes) = serde_json::to_vec(&reply) {
        bytes.push(b'\n');
        let _ = writing.write_all(&bytes);
    }
}

/// Keeps a question the attempt will not answer until the desk closes. An
/// askpass that answered nothing would have ssh send an empty secret, which a
/// server counts as a failed sign-in, or try its next way of signing in; held,
/// ssh waits until the attempt ends it, and its askpass with it, at its next
/// look (B8, B14).
fn hold(state: &DeskState) {
    let mut refusal = lock(&state.refusal);
    while !state.closed.load(Ordering::SeqCst) {
        refusal = state
            .held
            .wait(refusal)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn random_hex(bytes: usize) -> std::io::Result<String> {
    let mut random = vec![0_u8; bytes];
    getrandom::getrandom(&mut random)
        .map_err(|error| std::io::Error::other(format!("no random bytes: {error}")))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// `hide` as ssh's askpass: when [`DESK_VARIABLE`] is set, hands the prompt
/// (the first argument) to the desk and prints its answer for ssh. Answers
/// the exit code: 0 with an answer printed, 1 without one, which ssh reads
/// as a refusal. `None` when this process is not an askpass.
pub fn client(args: &[std::ffi::OsString]) -> Option<i32> {
    let desk = std::env::var_os(DESK_VARIABLE)?;
    let token = std::env::var(TOKEN_VARIABLE).unwrap_or_default();
    let prompt = args
        .get(1)
        .map(|prompt| prompt.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hint = std::env::var("SSH_ASKPASS_PROMPT").ok();
    Some(match ask_desk(Path::new(&desk), token, prompt, hint) {
        Some(answer) => {
            let mut stdout = std::io::stdout().lock();
            if stdout
                .write_all(answer.as_bytes())
                .and_then(|()| stdout.write_all(b"\n"))
                .and_then(|()| stdout.flush())
                .is_ok()
            {
                0
            } else {
                1
            }
        }
        None => 1,
    })
}

fn ask_desk(desk: &Path, token: String, prompt: String, hint: Option<String>) -> Option<String> {
    let mut stream = LocalStream::connect_until(desk, Instant::now() + ANSWER_WITHIN).ok()?;
    let mut bytes = serde_json::to_vec(&Request {
        token,
        prompt,
        hint,
    })
    .ok()?;
    bytes.push(b'\n');
    stream.write_all(&bytes).ok()?;
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_MESSAGE as u64))
        .read_until(b'\n', &mut line)
        .ok()?;
    serde_json::from_slice::<Reply>(&line).ok()?.answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(prompt: &str, key_file: Option<&str>) -> Asked {
        Asked::Question(SignInWait::Secret {
            prompt: prompt.to_owned(),
            key_file: key_file.map(str::to_owned),
        })
    }

    /// The prompts OpenSSH 8.x and 9.x hand an askpass, as `readpass.c`,
    /// `sshconnect.c` and `sshconnect2.c` word them.
    #[test]
    fn each_prompt_ssh_hands_askpass_is_its_kind_of_question() {
        assert_eq!(
            asked("al@studio.example.test's password: ", None),
            secret("al@studio.example.test's password:", None)
        );
        assert_eq!(
            asked(
                "Enter passphrase for key '/home/al/.ssh/id_ed25519': ",
                Some("")
            ),
            secret(
                "Enter passphrase for key '/home/al/.ssh/id_ed25519':",
                Some("/home/al/.ssh/id_ed25519")
            )
        );
        assert_eq!(
            asked("(al@studio) Verification code: ", None),
            secret("(al@studio) Verification code:", None)
        );
        assert_eq!(
            asked(
                "Enter PIN for ED25519-SK key /home/al/.ssh/id_ed25519_sk: ",
                None
            ),
            secret(
                "Enter PIN for ED25519-SK key /home/al/.ssh/id_ed25519_sk:",
                None
            )
        );
        let new_host = "The authenticity of host '[127.0.0.1]:2222 ([127.0.0.1]:2222)' can't be established.\nED25519 key fingerprint is SHA256:1f2c4d3RkV0Wz8gq+GmxbLX0XsW5yq9bZ7lHn1Q2c3E.\nThis key is not known by any other names.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert_eq!(
            asked(new_host, None),
            Asked::Question(SignInWait::HostKey {
                host: "[127.0.0.1]:2222 ([127.0.0.1]:2222)".to_owned(),
                fingerprint: "SHA256:1f2c4d3RkV0Wz8gq+GmxbLX0XsW5yq9bZ7lHn1Q2c3E".to_owned(),
            })
        );
        assert_eq!(
            asked("Accept updated hostkeys? (yes/no): ", None),
            Asked::Question(SignInWait::Confirm {
                prompt: "Accept updated hostkeys? (yes/no):".to_owned()
            })
        );
        assert_eq!(
            asked(
                "Allow use of key /home/al/.ssh/id_ed25519?",
                Some("confirm")
            ),
            Asked::Question(SignInWait::Confirm {
                prompt: "Allow use of key /home/al/.ssh/id_ed25519?".to_owned()
            })
        );
        assert_eq!(
            asked(
                "Confirm user presence for key ED25519-SK SHA256:abc",
                Some("none")
            ),
            Asked::Touch("Confirm user presence for key ED25519-SK SHA256:abc".to_owned())
        );
        // A sentence nobody wrote this for is typed, masked.
        assert_eq!(
            asked("Something else entirely: ", None),
            secret("Something else entirely:", None)
        );
    }

    /// A row that answers what the test chose, and records what it saw.
    struct Row {
        answer: Mutex<Option<Result<String, Refusal>>>,
        seen: Mutex<Vec<String>>,
    }

    impl SignIn for Row {
        fn show(&self, wait: Option<SignInWait>) {
            lock(&self.seen).push(match wait {
                Some(wait) => wait.kind().to_owned(),
                None => "cleared".to_owned(),
            });
        }

        fn ask(&self, question: SignInWait) -> Result<Answer, Refusal> {
            lock(&self.seen).push(question.kind().to_owned());
            match lock(&self.answer).take().expect("one question") {
                Ok(text) if text == "yes" => Ok(Answer::Accept),
                Ok(text) => Ok(Answer::Text(hide_node_link::device::Secret::new(text))),
                Err(refusal) => Err(refusal),
            }
        }
    }

    fn desk_with(answer: Result<String, Refusal>) -> (Desk, Arc<Row>, tempfile::TempDir) {
        let parent = tempfile::tempdir().unwrap();
        let row = Arc::new(Row {
            answer: Mutex::new(Some(answer)),
            seen: Mutex::new(Vec::new()),
        });
        let desk = Desk::open(parent.path(), Arc::clone(&row) as Arc<dyn SignIn>, None).unwrap();
        (desk, row, parent)
    }

    #[test]
    fn an_answer_from_the_row_reaches_the_askpass_and_nothing_else_does() {
        let (desk, row, _parent) = desk_with(Ok("hunter2".to_owned()));
        let answer = ask_desk(
            &desk.path,
            desk.token.clone(),
            "al@studio's password: ".to_owned(),
            None,
        );
        assert_eq!(answer.as_deref(), Some("hunter2"));
        assert_eq!(*lock(&row.seen), ["secret"]);
        assert_eq!(desk.refusal(), None);
        // The wrong token gets nothing and asks the row nothing.
        let forged = ask_desk(&desk.path, "0".repeat(32), "password: ".to_owned(), None);
        assert_eq!(forged, None);
        assert_eq!(lock(&row.seen).len(), 1);
    }

    /// A refused question is held, not answered empty, so ssh sends the
    /// server nothing it would count as a failed sign-in; it and any later
    /// question end with the desk, and the row is asked once.
    #[test]
    #[allow(clippy::disallowed_methods)] // a window in which a held question must not end: no state reports an answer that was never sent
    fn a_refused_question_waits_unanswered_until_the_attempt_ends() {
        let (desk, row, _parent) = desk_with(Err(Refusal::Cancelled));
        let ask = |desk: &Desk| {
            let (path, token) = (desk.path.clone(), desk.token.clone());
            std::thread::spawn(move || ask_desk(&path, token, "password: ".to_owned(), None))
        };
        let first = ask(&desk);
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while desk.refusal().is_none() {
            assert!(Instant::now() < deadline, "the row was never asked");
            std::thread::yield_now();
        }
        assert_eq!(desk.refusal(), Some(Refusal::Cancelled));
        let again = ask(&desk);
        while desk.state.asking.load(Ordering::SeqCst) < 2 {
            assert!(
                Instant::now() < deadline,
                "the second question never arrived"
            );
            std::thread::yield_now();
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!first.is_finished() && !again.is_finished());
        drop(desk);
        assert_eq!(first.join().unwrap(), None);
        assert_eq!(again.join().unwrap(), None);
        assert_eq!(*lock(&row.seen), ["secret"]);
    }

    /// D-05, B11: a security key's touch is shown and cleared, and one that
    /// never comes ends the attempt as a sign-in nobody answered once its
    /// bound passes, as an unanswered question does.
    #[test]
    fn a_touch_that_never_comes_ends_the_attempt_at_its_bound() {
        let (desk, row, _parent) = desk_with(Err(Refusal::Cancelled));
        let (path, token) = (desk.path.clone(), desk.token.clone());
        let touching = std::thread::spawn(move || {
            ask_desk(
                &path,
                token,
                "Confirm user presence for key ED25519-SK SHA256:abc".to_owned(),
                Some("none".to_owned()),
            )
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while desk.refusal().is_none() {
            assert!(Instant::now() < deadline, "the touch never timed out");
            std::thread::yield_now();
        }
        assert_eq!(desk.refusal(), Some(Refusal::TimedOut));
        assert_eq!(*lock(&row.seen), ["touch", "cleared"]);
        drop(desk);
        assert_eq!(touching.join().unwrap(), None);
    }

    /// A desk whose process died leaves its folder; the next desk opened
    /// beside it removes that one and no other.
    #[test]
    fn a_dead_process_s_desk_is_removed_by_the_next_one() {
        let parent = tempfile::tempdir().unwrap();
        let gone = 0x7fff_fff0_u32;
        assert!(!hide_platform::process::is_alive(gone));
        let dead = parent.path().join(format!("{DESK_PREFIX}{gone}-aaaa"));
        let own = parent
            .path()
            .join(format!("{DESK_PREFIX}{}-bbbb", std::process::id()));
        let other = parent.path().join("hide-ssh-notes");
        for folder in [&dead, &own, &other] {
            std::fs::create_dir(folder).unwrap();
        }
        let row = Arc::new(Row {
            answer: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        });
        let desk = Desk::open(parent.path(), row, None).unwrap();
        assert!(!dead.exists());
        assert!(own.exists() && other.exists());
        drop(desk);
    }

    #[test]
    fn a_trusted_server_is_answered_yes() {
        let (desk, row, _parent) = desk_with(Ok("yes".to_owned()));
        let answer = ask_desk(
            &desk.path,
            desk.token.clone(),
            "The authenticity of host 'h' can't be established.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ".to_owned(),
            None,
        );
        assert_eq!(answer.as_deref(), Some("yes"));
        assert_eq!(*lock(&row.seen), ["host_key"]);
    }

    /// ssh prints at most 100 bytes of a key's path; the row's command names
    /// the whole file, or none when no single known file is it (B9).
    #[test]
    fn a_shortened_key_path_is_the_one_whole_file_it_begins_or_none() {
        let folder = std::env::temp_dir().join("k".repeat(120));
        let whole = folder.join("id_ed25519").to_str().unwrap().to_owned();
        let other = folder.join("id_rsa").to_str().unwrap().to_owned();
        let shown = &whole[..PROMPT_PATH_BYTES];
        let short = std::env::temp_dir().join("id_ed25519");
        let short = short.to_str().unwrap();
        assert_eq!(key_file(short, None).as_deref(), Some(short));
        assert_eq!(key_file(".ssh/id_ed25519", None), None);
        let one = [PathBuf::from(&whole)];
        assert_eq!(key_file(shown, Some(&one)).as_deref(), Some(whole.as_str()));
        let two = [PathBuf::from(&whole), PathBuf::from(&other)];
        assert_eq!(key_file(shown, Some(&two)), None);
        assert_eq!(key_file(shown, Some(&[])), None);
        assert_eq!(key_file(shown, None), None);
    }

    #[test]
    fn a_desk_leaves_nothing_behind() {
        let (desk, _row, parent) = desk_with(Ok("x".to_owned()));
        let folder = desk.folder.clone();
        assert!(folder.starts_with(parent.path()));
        drop(desk);
        assert!(!folder.exists());
    }
}
