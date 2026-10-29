//! A device's start script (PRD system-ssh-device-link D-02, B22-B24): one
//! POSIX `sh` that runs over the same input and output the device's link
//! then uses, so a device signs in once per connection. The `ssh` or
//! `wsl.exe` that reaches the device runs [`WRAPPER`]; the node sends the
//! script as its first line, and then, step by step, waiting for each
//! answer: what the account is ([`probe`]), which of the build's files it
//! already holds ([`install`]), each missing file whole, and the helper's
//! start ([`start`]), after which the same pipes are the helper's link.
//!
//! The connection to a core on another machine runs the same script (PRD
//! system-ssh-device-link B33, `super::super::session`), with steps of its
//! own between the probe and the end: a command whose output comes back
//! counted ([`run`]), a file sent to or read from an absolute path
//! ([`put_file`], [`get_file`]), and the attach role's start in place of the
//! script ([`attach`]).
//!
//! The steps go in lock step on purpose. A file's bytes are read with
//! `head -c <size>`, and macOS's `head` reads through a buffer that would
//! take bytes past the file's end if they were already waiting; the node
//! sends nothing after a file until the device has answered for it, so
//! nothing is ever waiting.
//!
//! A file goes beside its final name in a folder only the account can
//! enter, is checked against its digest and renamed into place, so the final
//! name only ever holds a whole copy, an interrupted install leaves no half
//! file in use, and the next connection sends only what is missing (B23).
//! The install folder and every folder above it must be ones no other
//! account can change (OpenSSH's own rule for key files), checked on the
//! path the folder really is.

use super::*;

/// The remote command an `ssh` runs: reads the script as one line from the
/// link's own input and runs it. It is the same in every login shell (sh,
/// bash, zsh, fish, csh): single quotes, and nothing inside them any of
/// those reads.
pub(crate) const REMOTE_COMMAND: &str = "sh -c 'IFS= read -r s && eval \"$s\"'";

/// [`REMOTE_COMMAND`] as the words of a program run without a shell
/// (`wsl.exe -e`).
pub(crate) const WRAPPER: [&str; 3] = ["sh", "-c", "IFS= read -r s && eval \"$s\""];

/// The script, one statement per line; [`script_line`] joins it into the
/// single line [`WRAPPER`] reads. Every line ends where a space may follow.
const SCRIPT: &str = r#"
set -fu;
T=$(printf '\t');
say() { printf 'HIDE1'; for w in "$@"; do printf '\t%s' "$w"; done; printf '\n'; };
uid=$(id -u) || exit 65;
me=$(id -un) || exit 65;
mid=${HIDE_MACHINE_ID:-};
if [ -z "$mid" ] && [ -r /etc/machine-id ]; then mid=$(cat /etc/machine-id); fi;
if [ -z "$mid" ] && [ -x /usr/sbin/ioreg ]; then mid=$(/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice | sed -n 's/.*"IOPlatformUUID" = "\(.*\)".*/\1/p'); fi;
hs=yes;
if command -v sha256sum >/dev/null 2>&1; then sum() { sha256sum < "$1" | cut -d ' ' -f 1; };
elif command -v shasum >/dev/null 2>&1; then sum() { shasum -a 256 < "$1" | cut -d ' ' -f 1; };
else hs=none; sum() { echo none; }; fi;
safe() { set -- $(ls -ldn -- "$1" 2>/dev/null); case ${1:-} in d*) ;; *) return 3;; esac; [ "${3:-}" = "$uid" ] || [ "${3:-}" = 0 ] || return 1; case $1 in ?????w*|????????w*) ;; *) return 0;; esac; [ "$3" = 0 ] || return 2; case $1 in ?????????[tT]*) return 0;; esac; return 2; };
mine() { set -- $(ls -ldn -- "$1" 2>/dev/null); [ "${3:-}" = "$uid" ] || return 1; case ${1:-} in ?????w*|????????w*) return 1;; esac; return 0; };
store() { { (umask 077; mkdir -p -- "${1%/*}") && cat > "$1"; } || { cat > /dev/null; return 1; }; };
say probe "$me" "${HOME:-}" "$(uname -s)" "$(uname -m)" "${mid:--}" "$hs";
root=;
while IFS=$T read -r cmd a b c d; do
case $cmd in
root) case $a in /*) ;; *) say fail "the install folder $a is not absolute"; continue;; esac;
(umask 077; mkdir -p -- "$a") 2>/dev/null || { say fail "$a could not be made"; continue; };
r=$(cd -P -- "$a" 2>/dev/null && pwd -P) || { say fail "$a could not be opened"; continue; };
bad=; rest=${r#/}; p=;
while [ -n "$rest" ]; do part=${rest%%/*}; case $rest in */*) rest=${rest#*/};; *) rest=;; esac; [ -n "$rest" ] || break; p=$p/$part; safe "$p"; case $? in 0) ;; 1) bad="$p belongs to another account, so the helper was not installed under it; choose another install root"; break;; *) bad="$p can be changed by its group or by other accounts, so the helper was not installed under it; remove that write access on the device (chmod go-w $p) or choose another install root"; break;; esac; done;
[ -z "$bad" ] || { say fail "$bad"; continue; };
chmod 700 -- "$r" 2>/dev/null;
mine "$r" && [ ! -L "$a" ] || { say fail "$r must be a folder owned by the account and not writable by others; the helper was not installed"; continue; };
root=$r; say root "$r";;
held) [ -n "$root" ] || { say fail "no install folder was chosen"; exit 65; }; i=0;
while [ "$i" -lt "$a" ] && IFS=$T read -r rel dig; do i=$((i+1)); f=$root/$rel; if [ -f "$f" ] && [ ! -L "$f" ] && mine "$f" && [ "$(sum "$f")" = "$dig" ]; then say held "$rel"; fi; done;
say end;;
put) [ -n "$root" ] || { say fail "no install folder was chosen"; exit 65; }; f=$root/$a; t=${f%/*}/.upload-${f##*/}-$$;
find "${f%/*}" -maxdepth 1 -name '.upload-*' -mmin +10 -exec rm -f {} + 2>/dev/null;
if head -c "$d" | store "$t" && [ "$(sum "$t")" = "$b" ] && chmod -- "$c" "$t" && mv -f -- "$t" "$f"; then say ok; else rm -f -- "$t"; say fail "$a did not arrive whole"; fi;;
start) [ -n "$root" ] || { say fail "no install folder was chosen"; exit 65; }; h=$root/$a;
case $b in session=*) unset HERDR_SOCKET_PATH; HERDR_SESSION=${b#session=}; export HERDR_SESSION;; socket=*) unset HERDR_SESSION; HERDR_SOCKET_PATH=${b#socket=}; export HERDR_SOCKET_PATH;; esac;
[ -x "$h" ] || { say fail "the helper $h is missing"; exit 65; };
cd -- "$HOME" 2>/dev/null || :;
say start; exec "$h" node serve --own-herdr;;
run) set --; i=0; while [ "$i" -lt "$a" ] && IFS= read -r w; do set -- "$@" "$w"; i=$((i+1)); done;
o=$(mktemp) || { say fail "no temporary file for the command's output"; continue; };
e=$(mktemp) || { rm -f -- "$o"; say fail "no temporary file for the command's output"; continue; };
command "$@" </dev/null >"$o" 2>"$e"; s=$?; ol=$(($(wc -c <"$o"))); el=$(($(wc -c <"$e")));
if [ "$ol" -gt "$b" ] || [ "$el" -gt "$b" ]; then say big "$s" "$ol" "$el"; else say ran "$s" "$ol" "$el"; cat -- "$o" "$e"; fi;
rm -f -- "$o" "$e";;
fput) case $a in /*) ;; *) head -c "$c" >/dev/null; say fail "$a is not absolute"; continue;; esac; t=$a.part;
if head -c "$c" | (umask 077; store "$t") && [ "$(sum "$t")" = "$b" ] && mv -f -- "$t" "$a"; then say ok; else rm -f -- "$t"; say fail "$a did not arrive whole"; fi;;
fget) if [ -f "$a" ] && [ -r "$a" ]; then n=$(($(wc -c <"$a"))); say file "$n" "$(sum "$a")"; head -c "$n" <"$a"; else say fail "$a is not a file Hide can read"; fi;;
attach) [ -x "$a" ] || { say fail "$a is missing"; exit 65; };
cd -- "${HOME:-/}" 2>/dev/null || :;
say attach; case $b in -) exec "$a" attach;; *) exec "$a" attach --state-dir "$b";; esac;;
*) say fail "unknown step $cmd"; exit 64;;
esac;
done
"#;

/// The script as the one line [`WRAPPER`] reads, with its line end.
pub(crate) fn script_line() -> String {
    let mut line = SCRIPT
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    line.push('\n');
    line
}

/// How a start script's lines arrive and its steps are sent: the child's
/// pipes, read by the transport, which also reads the child's error output
/// as it signs in.
pub(crate) trait Pipe {
    fn send(&mut self, bytes: &[u8], until: Instant) -> Result<(), String>;
    /// The next line of the device's output, without its end.
    fn line(&mut self, until: Instant) -> Result<Vec<u8>, Stop>;
    /// At most `max` bytes of the device's output, at least one, for a step
    /// that answers in counted bytes.
    fn bytes(&mut self, max: usize, until: Instant) -> Result<Vec<u8>, Stop>;
}

/// Why no line came.
#[derive(Debug)]
pub(crate) enum Stop {
    /// Nothing within the time given.
    Quiet,
    /// The device's output ended.
    Ended(String),
}

/// The most lines a login file may print before the script answers; they
/// are skipped (a greeting a login file prints).
const MAX_GREETING_LINES: usize = 256;
/// How long one step of the install may take, every file of it together.
pub(crate) const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a step that is not a file has to answer.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// What the account answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Facts {
    pub user: String,
    pub home: String,
    pub os: String,
    pub arch: String,
    /// The machine's own id, lowercased, as `hide_platform::host::machine_id`
    /// reads it there. A consent binds to it (D-09), so a machine that
    /// reports none is refused before anything is installed.
    pub machine_id: String,
}

#[derive(Debug)]
pub(crate) enum ProbeError {
    /// The device never answered.
    Stopped(Stop),
    /// It answered, and it is not a machine this build runs on.
    Unsupported(String),
}

/// Sends the script and reads the account's answer, skipping what a login
/// file printed before it.
pub(crate) fn probe(pipe: &mut dyn Pipe, until: Instant) -> Result<Facts, ProbeError> {
    pipe.send(script_line().as_bytes(), until)
        .map_err(|reason| ProbeError::Stopped(Stop::Ended(reason)))?;
    for _ in 0..MAX_GREETING_LINES {
        let line = pipe.line(until).map_err(ProbeError::Stopped)?;
        let Some(fields) = answer(&line) else {
            continue;
        };
        return read_probe(&fields).map_err(ProbeError::Unsupported);
    }
    Err(ProbeError::Unsupported(format!(
        "The device printed more than {MAX_GREETING_LINES} lines before Hide's start script answered"
    )))
}

/// A line of the script's (`HIDE1` and tab-separated fields), or `None` for
/// anything else.
fn answer(line: &[u8]) -> Option<Vec<String>> {
    let line = std::str::from_utf8(line).ok()?;
    let mut fields = line.split('\t');
    (fields.next()? == "HIDE1").then(|| fields.map(str::to_owned).collect())
}

fn read_probe(fields: &[String]) -> Result<Facts, String> {
    let [step, user, home, system, machine, id, digest] = fields else {
        return Err(format!(
            "The device answered an unreadable probe: {fields:?}"
        ));
    };
    if step != "probe" {
        return Err(format!(
            "The device's start script failed: {}",
            fields.join(" ")
        ));
    }
    if !hide_platform::wsl::valid_user_name(user) {
        return Err(format!(
            "The device's account {user:?} is not a name Hide can run as"
        ));
    }
    if !home.starts_with('/') || home.chars().any(char::is_control) {
        return Err("The device's account has no usable home folder".to_owned());
    }
    if digest != "yes" {
        return Err(
            "The device has neither sha256sum nor shasum, which Hide checks its install with"
                .to_owned(),
        );
    }
    let (os, arch) = platform_of(&format!("{system} {machine}"))?;
    let machine_id = id.trim().to_lowercase();
    if !valid_machine_id(&machine_id) {
        return Err(NO_MACHINE_ID.to_owned());
    }
    Ok(Facts {
        user: user.clone(),
        home: home.clone(),
        os,
        arch,
        machine_id,
    })
}

/// Why a machine that names itself by no usable id gets nothing installed:
/// the consent binds to that id, and without it this machine cannot be told
/// from another that answers at the same address.
pub(crate) const NO_MACHINE_ID: &str = "The device reports no machine id (/etc/machine-id, or a Mac's hardware id), so Hide cannot tell it from another machine at this address and installs nothing; give it one (on Linux, `sudo systemd-machine-id-setup`) and connect again";

/// An id as a system writes it: letters, digits and `-`, `_` or `.`, at most
/// 128 of them; `-` alone is the script's word for none.
fn valid_machine_id(id: &str) -> bool {
    id != "-"
        && (1..=128).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// What the install did: where the helper is and how the files arrived.
pub(crate) struct Installed {
    pub helper_path: String,
    /// The helper's own path under the install folder, for [`start`].
    pub helper_relative: String,
    pub fresh: bool,
    pub upload: Upload,
}

/// Puts `payload` in its version folder under `helper_root` (`~/` is the
/// account's home), sending only the files the device does not already hold
/// whole.
pub(crate) fn install(
    pipe: &mut dyn Pipe,
    helper_root: &str,
    home: &str,
    payload: &Payload,
) -> Result<Installed, EstablishError> {
    let deadline = Instant::now() + INSTALL_TIMEOUT;
    let failed = |reason: String| EstablishError::Install(reason);
    let root = match helper_root.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.trim_end_matches('/')),
        None if helper_root.starts_with('/') => helper_root.to_owned(),
        None => {
            return Err(failed(format!(
                "The helper install root {helper_root:?} must be absolute or start with ~/"
            )));
        }
    };
    if root.split('/').any(|part| part == ".." || part == ".") || !plain(&root) {
        return Err(failed(
            "The helper install root is not a plain path".to_owned(),
        ));
    }
    let resolved = match step(pipe, &format!("root\t{root}\n"), deadline)?.as_slice() {
        [kind, resolved] if kind == "root" && resolved.starts_with('/') => resolved.clone(),
        other => return Err(failed(refusal(other))),
    };
    let version = &payload.version()[..16];
    let mut files: Vec<(String, &Package)> = payload
        .files
        .iter()
        .map(|file| (format!("{version}/{}", file.relative), file))
        .collect();
    if let Some(herdr) = &payload.herdr {
        files.push((
            format!(
                "{}/{}/{}",
                hide_kit::device_herdr::HERDR_DIR,
                herdr_copy_name(herdr),
                herdr.relative
            ),
            herdr,
        ));
    }
    if files.iter().any(|(relative, _)| !plain(relative)) {
        return Err(failed("The build names a file Hide cannot send".to_owned()));
    }
    let mut asked = format!("held\t{}\n", files.len());
    for (relative, file) in &files {
        asked.push_str(&format!("{relative}\t{}\n", file.digest));
    }
    send(pipe, asked.as_bytes(), deadline)?;
    let mut held = std::collections::BTreeSet::new();
    loop {
        match read_step(pipe, deadline)?.as_slice() {
            [kind, relative] if kind == "held" => {
                held.insert(relative.clone());
            }
            [kind] if kind == "end" => break,
            other => return Err(failed(refusal(other))),
        }
    }
    let mut upload = Upload {
        missing: payload.missing.clone(),
        ..Upload::default()
    };
    let mut fresh = false;
    for (index, (relative, file)) in files.iter().enumerate() {
        if held.contains(relative) {
            upload.reused += 1;
            continue;
        }
        let mode = if file.executable { "700" } else { "600" };
        send(
            pipe,
            format!(
                "put\t{relative}\t{}\t{mode}\t{}\n",
                file.digest,
                file.bytes.len()
            )
            .as_bytes(),
            deadline,
        )?;
        send(pipe, &file.bytes, deadline)?;
        match read_step(pipe, deadline)?.as_slice() {
            [kind] if kind == "ok" => {
                upload.sent += 1;
                fresh |= index == 0;
            }
            [kind, reason] if kind == "fail" => {
                // The helper, a chosen Herdr and its pin must arrive before
                // the helper starts; an optional kit part the kit reports.
                if index == 0
                    || file.relative == hide_kit::device_herdr::HERDR_PIN
                    || file.relative == hide_kit::device_herdr::HERDR_PROGRAM
                {
                    return Err(failed(format!("The device could not take {reason}")));
                }
                upload.missing.push(reason.clone());
            }
            other => return Err(failed(refusal(other))),
        }
    }
    let helper = payload
        .files
        .first()
        .ok_or_else(|| failed("The build carries no helper".to_owned()))?;
    let helper_relative = format!("{version}/{}", helper.relative);
    Ok(Installed {
        helper_path: format!("{resolved}/{helper_relative}"),
        helper_relative,
        fresh,
        upload,
    })
}

/// Which Herdr server the helper bridges to the core: the account's own,
/// a named session, or a socket the registration names (B24).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HerdrChoice {
    Account,
    Session(String),
    Socket(String),
}

/// Starts the helper in place of the script; from its answer on, the pipes
/// are the helper's link.
pub(crate) fn start(
    pipe: &mut dyn Pipe,
    helper_relative: &str,
    herdr: &HerdrChoice,
) -> Result<(), EstablishError> {
    let choice = match herdr {
        HerdrChoice::Account => "-".to_owned(),
        HerdrChoice::Session(session) => format!("session={session}"),
        HerdrChoice::Socket(socket) => format!("socket={socket}"),
    };
    if !plain(&choice) || !plain(helper_relative) {
        return Err(EstablishError::Helper(
            "The device's Herdr session or socket is not a plain name".to_owned(),
        ));
    }
    let deadline = Instant::now() + STEP_TIMEOUT;
    match step(
        pipe,
        &format!("start\t{helper_relative}\t{choice}\n"),
        deadline,
    )?
    .as_slice()
    {
        [kind] if kind == "start" => Ok(()),
        other => Err(EstablishError::Helper(refusal(other))),
    }
}

/// Puts this build in its version folder under `root` on a machine that
/// already runs a build of Hide's (a core update, PRD core-host-node-move
/// B10), sending only the files it lacks or holds damaged, without the
/// device's Herdr. Answers where the build's `hided` is.
pub(crate) fn install_build(
    pipe: &mut dyn Pipe,
    root: &str,
    home: &str,
    payload: Payload,
) -> Result<String, EstablishError> {
    let payload = payload.without_herdr();
    if !payload.missing.is_empty() {
        return Err(EstablishError::Install(format!(
            "This build cannot update a core without all of its parts: {}",
            payload.missing.join("; ")
        )));
    }
    install(pipe, root, home, &payload).map(|installed| installed.helper_path)
}

/// Why a step of a connection to a core's machine did not finish. After
/// any but [`StepError::Refused`] the script's input and output are out of
/// step, and the connection is not used again.
#[derive(Debug)]
pub enum StepError {
    /// The connection ended, or answered nothing in time: what the step did
    /// there is unknown.
    Lost(String),
    /// The machine answered that the step failed, and why.
    Refused(String),
    /// A file on this machine could not be read or written.
    Local(String),
}

impl fmt::Display for StepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lost(reason) => write!(formatter, "the connection was lost: {reason}"),
            Self::Refused(reason) => formatter.write_str(reason),
            Self::Local(reason) => write!(formatter, "local: {reason}"),
        }
    }
}

/// What a command run on the machine answered.
#[derive(Debug)]
pub struct Ran {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The most of a counted answer taken from the pipe at once.
const COUNTED_CHUNK: usize = 64 * 1024;
/// How long one chunk of a file may wait to be taken or to arrive: a copy
/// takes as long as its size needs, and one that stops moving ends.
pub(crate) const STALL_TIMEOUT: Duration = Duration::from_secs(30);

fn lost(stop: Stop) -> StepError {
    StepError::Lost(match stop {
        Stop::Quiet => "the machine did not answer in time".to_owned(),
        Stop::Ended(reason) => reason,
    })
}

/// Sends `line` and reads the script's answer to it.
fn ask(pipe: &mut dyn Pipe, line: &str, until: Instant) -> Result<Vec<String>, StepError> {
    pipe.send(line.as_bytes(), until).map_err(StepError::Lost)?;
    let answered = pipe.line(until).map_err(lost)?;
    answer(&answered).ok_or_else(|| {
        StepError::Lost(format!(
            "the machine answered {:?}",
            String::from_utf8_lossy(&answered)
        ))
    })
}

/// Exactly `size` bytes of the pipe, handed to `take` as they arrive, each
/// chunk within `stall` of the one before.
fn counted(
    pipe: &mut dyn Pipe,
    size: u64,
    stall: Duration,
    take: &mut dyn FnMut(&[u8]) -> Result<(), StepError>,
) -> Result<(), StepError> {
    let mut left = size;
    while left > 0 {
        let wanted = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(COUNTED_CHUNK);
        let chunk = pipe.bytes(wanted, Instant::now() + stall).map_err(lost)?;
        take(&chunk)?;
        left -= chunk.len() as u64;
    }
    Ok(())
}

fn number<T: std::str::FromStr>(field: &str) -> Result<T, StepError> {
    field.parse().map_err(|_| {
        StepError::Lost(format!(
            "the machine answered an unreadable count {field:?}"
        ))
    })
}

/// Runs `argv` on the machine with no input, from the account's home, and
/// answers its exit status and output, each at most `cap` bytes. A command
/// still running at `until` loses the connection, and the command is not
/// waited for.
pub(crate) fn run(
    pipe: &mut dyn Pipe,
    argv: &[String],
    cap: usize,
    until: Instant,
) -> Result<Ran, StepError> {
    if argv.is_empty() || !argv.iter().all(|word| plain(word)) {
        return Err(StepError::Refused(
            "The command names a word Hide cannot send".to_owned(),
        ));
    }
    let mut asked = format!("run\t{}\t{cap}\n", argv.len());
    for word in argv {
        asked.push_str(word);
        asked.push('\n');
    }
    match ask(pipe, &asked, until)?.as_slice() {
        [kind, status, out, err] if kind == "ran" => {
            let (status, out, err): (i32, u64, u64) = (number(status)?, number(out)?, number(err)?);
            if out > cap as u64 || err > cap as u64 {
                return Err(StepError::Lost(
                    "the machine sent more output than it was asked for".to_owned(),
                ));
            }
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let left = until.saturating_duration_since(Instant::now());
            counted(pipe, out, left, &mut |chunk| {
                stdout.extend_from_slice(chunk);
                Ok(())
            })?;
            counted(pipe, err, left, &mut |chunk| {
                stderr.extend_from_slice(chunk);
                Ok(())
            })?;
            Ok(Ran {
                status,
                stdout,
                stderr,
            })
        }
        [kind, ..] if kind == "big" => Err(StepError::Refused(format!(
            "the command wrote more than {cap} bytes"
        ))),
        other => Err(StepError::Refused(refusal(other))),
    }
}

/// Sends `size` bytes that `read` gives to the absolute path `remote` on the
/// machine, whose SHA-256 is `digest`. The bytes go to `<remote>.part`
/// beside their final name, in a folder made private where it is missing,
/// are checked against the digest and renamed into place, so the final name
/// only ever holds a whole copy. `sent` hears each chunk's size as it goes.
pub(crate) fn put_file(
    pipe: &mut dyn Pipe,
    remote: &str,
    digest: &str,
    size: u64,
    read: &mut dyn std::io::Read,
    sent: &(dyn Fn(u64) + Sync),
) -> Result<(), StepError> {
    if !remote_path(remote) {
        return Err(StepError::Refused(format!(
            "{remote} is not a plain absolute path"
        )));
    }
    pipe.send(
        format!("fput\t{remote}\t{digest}\t{size}\n").as_bytes(),
        Instant::now() + STALL_TIMEOUT,
    )
    .map_err(StepError::Lost)?;
    let mut buffer = vec![0_u8; COUNTED_CHUNK];
    let mut left = size;
    while left > 0 {
        let wanted = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let filled = match read.read(&mut buffer[..wanted]) {
            Ok(0) => {
                return Err(StepError::Local(format!(
                    "the file for {remote} ended {left} bytes short of its size"
                )));
            }
            Ok(filled) => filled,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(StepError::Local(error.to_string())),
        };
        pipe.send(&buffer[..filled], Instant::now() + STALL_TIMEOUT)
            .map_err(StepError::Lost)?;
        sent(filled as u64);
        left -= filled as u64;
    }
    let answered = pipe.line(Instant::now() + STALL_TIMEOUT).map_err(lost)?;
    match answer(&answered).as_deref() {
        Some([kind]) if kind == "ok" => Ok(()),
        Some(other) => Err(StepError::Refused(refusal(other))),
        None => Err(StepError::Lost(format!(
            "the machine answered {:?}",
            String::from_utf8_lossy(&answered)
        ))),
    }
}

/// Reads the file at the absolute path `remote` on the machine, handing its
/// bytes to `write` as they arrive; `received` hears each chunk's size.
/// Answers its size and the SHA-256 the machine stated for it, for the
/// caller to check what it wrote against.
pub(crate) fn get_file(
    pipe: &mut dyn Pipe,
    remote: &str,
    write: &mut dyn std::io::Write,
    received: &(dyn Fn(u64) + Sync),
) -> Result<(u64, String), StepError> {
    if !remote_path(remote) {
        return Err(StepError::Refused(format!(
            "{remote} is not a plain absolute path"
        )));
    }
    match ask(
        pipe,
        &format!("fget\t{remote}\n"),
        Instant::now() + STALL_TIMEOUT,
    )?
    .as_slice()
    {
        [kind, size, digest] if kind == "file" => {
            let size: u64 = number(size)?;
            counted(pipe, size, STALL_TIMEOUT, &mut |chunk| {
                write
                    .write_all(chunk)
                    .map_err(|error| StepError::Local(error.to_string()))?;
                received(chunk.len() as u64);
                Ok(())
            })?;
            Ok((size, digest.clone()))
        }
        other => Err(StepError::Refused(refusal(other))),
    }
}

/// Starts `program attach` in place of the script, with `--state-dir` when
/// one is named; from its answer on, the pipes are the attach role's.
pub(crate) fn attach(
    pipe: &mut dyn Pipe,
    program: &str,
    state_dir: Option<&str>,
) -> Result<(), StepError> {
    let state_dir = state_dir.unwrap_or("-");
    if !remote_path(program) || (state_dir != "-" && !remote_path(state_dir)) {
        return Err(StepError::Refused(
            "The core's program or state folder is not a plain absolute path".to_owned(),
        ));
    }
    match ask(
        pipe,
        &format!("attach\t{program}\t{state_dir}\n"),
        Instant::now() + STEP_TIMEOUT,
    )?
    .as_slice()
    {
        [kind] if kind == "attach" => Ok(()),
        other => Err(StepError::Refused(refusal(other))),
    }
}

/// An absolute path the script takes as one field: plain, `/`-separated,
/// with no `.` or `..` step.
fn remote_path(path: &str) -> bool {
    plain(path) && path.starts_with('/') && !path.split('/').any(|part| part == "." || part == "..")
}

/// A field the script reads whole: no tab, no line end, no control
/// character, never empty.
fn plain(field: &str) -> bool {
    !field.is_empty() && !field.chars().any(char::is_control)
}

fn send(pipe: &mut dyn Pipe, bytes: &[u8], until: Instant) -> Result<(), EstablishError> {
    pipe.send(bytes, until).map_err(|reason| {
        EstablishError::Install(format!("The device stopped taking the install: {reason}"))
    })
}

fn step(pipe: &mut dyn Pipe, line: &str, until: Instant) -> Result<Vec<String>, EstablishError> {
    send(pipe, line.as_bytes(), until)?;
    read_step(pipe, until)
}

fn read_step(pipe: &mut dyn Pipe, until: Instant) -> Result<Vec<String>, EstablishError> {
    let line = pipe.line(until).map_err(|stop| {
        EstablishError::Install(match stop {
            Stop::Quiet => {
                "The device did not answer the install in time; nothing was started".to_owned()
            }
            Stop::Ended(reason) => format!("The connection ended during the install: {reason}"),
        })
    })?;
    answer(&line).ok_or_else(|| {
        EstablishError::Install(format!(
            "The device answered the install with {:?}",
            String::from_utf8_lossy(&line)
        ))
    })
}

fn refusal(fields: &[String]) -> String {
    match fields {
        [kind, reason] if kind == "fail" => reason.clone(),
        other => format!(
            "The device answered an unexpected step: {}",
            other.join(" ")
        ),
    }
}

#[cfg(all(test, unix))]
#[path = "bootstrap_tests.rs"]
mod tests;
