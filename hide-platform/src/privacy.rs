//! Whose name the operating system asks the user's consent under.
//!
//! macOS asks before a program reads the Documents, Desktop and Downloads
//! folders, iCloud Drive and the like, and remembers the answer for the
//! program that is *responsible* for the one that asked: the program that
//! started the chain. It names that program by its bundle identifier when a
//! bundle holds it, and by its path otherwise, so a program Hide copies into
//! a new folder for every build is asked about again for every build, and
//! the read waits for as long as nobody answers.
//!
//! Two things here let a background program answer under the app's name
//! instead, so one answer serves every build:
//!
//! - [`give`] puts an identity file beside a program, which makes its folder
//!   a bundle of the app's identifier. It does so only for a program that
//!   meets the app's own code requirement, because a program that claimed
//!   the name without meeting it would, on the user's next answer, replace
//!   the record the app's consent is kept under.
//! - [`answer_for_itself`] starts this program again in its own process as
//!   the program responsible for itself, so what it starts is asked about
//!   under its name and not under the name of whatever started it.
//!
//! Windows and Linux keep no consent per program: both answer
//! [`Refusal::NotMacos`] and do nothing.
//!
//! How macOS names a program was measured, not read: no document states it.
//! `docs/ARCHITECTURE.md`, The platform layer, records what was measured on
//! which versions.

use std::ffi::OsString;
use std::path::Path;

/// The file beside a program that makes its folder a bundle.
pub const IDENTITY_FILE: &str = "Info.plist";

/// The oldest macOS on which a program with an identity file beside it was
/// seen to be named by that file's identifier (15.1, 15.8.2, 26.4, 26.7.2).
/// No macOS 14 was at hand, so 14 keeps the path name it has today. Only this
/// bound is compared: macOS 26 answers some programs with 16.
pub const VERIFIED_FROM_MACOS: u32 = 15;

/// The app a program answers as.
#[derive(Clone, Copy, Debug)]
pub struct Identity<'a> {
    /// The app's bundle identifier.
    pub identifier: &'a str,
    /// The name a consent window shows.
    pub name: &'a str,
    /// The code requirement the app's consent is kept under, whole: a program
    /// is given the identity only when the system says it meets this.
    pub requirement: &'a str,
    /// Each `NS…UsageDescription` of the app with its sentence. macOS reads
    /// them from the identity file of the responsible program, and refuses
    /// an access whose sentence is missing without asking.
    pub usage: &'a [(&'a str, &'a str)],
}

/// Why a program keeps its own name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// This system keeps no consent per program.
    NotMacos,
    /// A macOS older than [`VERIFIED_FROM_MACOS`], or one whose version
    /// could not be read; the text is what was read.
    OsUnverified(String),
    /// The program sits inside an app bundle, which already names it.
    InsideApp,
    /// The system says the program does not meet the app's requirement: a
    /// development or locally signed build.
    RequirementNotMet,
    /// Another identity file is beside the program.
    AnotherIdentity,
    /// The requirement could not be checked, or the file not written.
    Failed(String),
}

impl Refusal {
    /// A fixed word for a log line; [`Refusal::detail`] carries the rest.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotMacos => "not_macos",
            Self::OsUnverified(_) => "os_unverified",
            Self::InsideApp => "inside_app",
            Self::RequirementNotMet => "requirement_not_met",
            Self::AnotherIdentity => "another_identity",
            Self::Failed(_) => "failed",
        }
    }

    /// What the system said, where it said something.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::OsUnverified(detail) | Self::Failed(detail) => Some(detail),
            _ => None,
        }
    }
}

/// Whose name a program answers under after [`give`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Given {
    /// The app's: the identity file is beside the program.
    Shared,
    /// Its own, as before, and why.
    Own(Refusal),
}

impl Given {
    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Shared)
    }
}

/// Whether [`give`] left an identity file beside `program`. One look at the
/// folder; it says nothing about whose identity the file is.
pub fn has_identity_file(program: &Path) -> bool {
    program
        .parent()
        .is_some_and(|dir| dir.join(IDENTITY_FILE).is_file())
}

/// Puts `identity`'s file beside `program` when the system says the program
/// meets the app's requirement, and answers whose name the program answers
/// under. The file already there is left as it is; asked again, the answer is
/// the same. A refusal writes nothing.
///
/// The requirement is checked by the system's own `codesign`, once per
/// program in this process, and before the file is written: with the file in
/// place `codesign` reads the folder as a bundle whose seal is missing.
pub fn give(program: &Path, identity: &Identity<'_>) -> Given {
    imp::give(program, identity)
}

/// Why this program goes on as it is after [`answer_for_itself`].
#[derive(Debug)]
pub enum NotAnswering {
    /// This system keeps no consent per program.
    NotMacos,
    /// The system has no way to say it; the function that does is not a
    /// documented one, and a macOS without it leaves things as they were.
    NoWay,
    /// The program could not be started again.
    Start(std::io::Error),
}

impl NotAnswering {
    /// A fixed word for a log line.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotMacos => "not_macos",
            Self::NoWay => "no_symbol",
            Self::Start(_) => "spawn_failed",
        }
    }
}

/// Starts `program` again with `arguments` in this process, as the program
/// responsible for itself: the same pid, environment, working folder and
/// open files that are not marked to close. It returns only when it did not
/// happen, with why; the caller goes on as the process it was.
///
/// A caller changes an argument between the two starts so that the second
/// does not ask again, and calls this before it opens or marks anything: a
/// file marked close-on-exec does not reach the second start.
pub fn answer_for_itself(program: &Path, arguments: &[OsString]) -> NotAnswering {
    imp::answer_for_itself(program, arguments)
}

/// The identity file's bytes for `program` under `identity`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn identity_file(program: &Path, identity: &Identity<'_>) -> Vec<u8> {
    let mut entries = vec![
        ("CFBundleIdentifier", identity.identifier.to_owned()),
        ("CFBundleName", identity.name.to_owned()),
        (
            "CFBundleExecutable",
            program
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        ("CFBundlePackageType", "BNDL".to_owned()),
        ("CFBundleInfoDictionaryVersion", "6.0".to_owned()),
    ];
    entries.extend(
        identity
            .usage
            .iter()
            .map(|(key, sentence)| (*key, (*sentence).to_owned())),
    );
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries {
        text.push_str(&format!(
            "  <key>{}</key>\n  <string>{}</string>\n",
            escaped(key),
            escaped(&value)
        ));
    }
    text.push_str("</dict>\n</plist>\n");
    text.into_bytes()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Whether a macOS of this major version is one the naming was measured on.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn verified(major: u32) -> bool {
    major >= VERIFIED_FROM_MACOS
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub(super) fn give(_program: &Path, _identity: &Identity<'_>) -> Given {
        Given::Own(Refusal::NotMacos)
    }

    pub(super) fn answer_for_itself(_program: &Path, _arguments: &[OsString]) -> NotAnswering {
        NotAnswering::NotMacos
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::HashMap;
    use std::ffi::{CStr, CString};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::sync::{LazyLock, Mutex, PoisonError};
    use std::time::Duration;

    use super::*;

    const CODESIGN: &str = "/usr/bin/codesign";

    /// How long the requirement check may take: `codesign` hashes the whole
    /// program, about two seconds for the largest one on a quiet machine.
    const CHECK_LIMIT: Duration = Duration::from_secs(60);

    /// The most answers kept. A process asks about its own program and one
    /// holder program per build, so the cap is reached only by a caller that
    /// asks about ever new paths; the answers are dropped whole then.
    const JUDGED_CAP: usize = 64;

    type Judged = HashMap<(PathBuf, String), Result<bool, String>>;

    /// What `codesign` said of each program against each requirement, so a
    /// build it refuses is not asked about again for every pane.
    static JUDGED: LazyLock<Mutex<Judged>> = LazyLock::new(Mutex::default);

    pub(super) fn give(program: &Path, identity: &Identity<'_>) -> Given {
        match macos_major() {
            Ok(major) if verified(major) => {}
            Ok(major) => return Given::Own(Refusal::OsUnverified(format!("macOS {major}"))),
            Err(error) => return Given::Own(Refusal::OsUnverified(error.to_string())),
        }
        if program
            .ancestors()
            .skip(1)
            .any(|dir| dir.extension().is_some_and(|extension| extension == "app"))
        {
            return Given::Own(Refusal::InsideApp);
        }
        let Some(dir) = program.parent() else {
            return Given::Own(Refusal::Failed(format!(
                "{} has no folder",
                program.display()
            )));
        };
        let file = dir.join(IDENTITY_FILE);
        let wanted = identity_file(program, identity);
        match std::fs::read(&file) {
            Ok(found) if found == wanted => return Given::Shared,
            Ok(_) => return Given::Own(Refusal::AnotherIdentity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Given::Own(Refusal::Failed(format!("{}: {error}", file.display())));
            }
        }
        match meets(program, identity.requirement) {
            Ok(true) => {}
            Ok(false) => return Given::Own(Refusal::RequirementNotMet),
            Err(error) => return Given::Own(Refusal::Failed(error)),
        }
        match crate::fs::atomic::write_file(&file, &wanted, crate::fs::Access::Private) {
            Ok(_) => Given::Shared,
            Err(error) => Given::Own(Refusal::Failed(format!("{}: {error}", file.display()))),
        }
    }

    /// Whether the system says `program` meets `requirement`, asked once per
    /// program and requirement in this process.
    fn meets(program: &Path, requirement: &str) -> Result<bool, String> {
        let key = (program.to_path_buf(), requirement.to_owned());
        if let Some(known) = JUDGED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return known.clone();
        }
        let answer = ask_codesign(program, requirement);
        let mut judged = JUDGED.lock().unwrap_or_else(PoisonError::into_inner);
        if judged.len() >= JUDGED_CAP {
            judged.clear();
        }
        judged.insert(key, answer.clone());
        answer
    }

    fn ask_codesign(program: &Path, requirement: &str) -> Result<bool, String> {
        let mut command = Command::new(CODESIGN);
        command
            .arg("--verify")
            .arg("-R")
            .arg(format!("={requirement}"))
            .arg(program);
        crate::process::restrict_to_login_environment(&mut command);
        let finished =
            crate::process::run_to_end(&mut command, CHECK_LIMIT, &AtomicBool::new(false))
                .map_err(|error| format!("{CODESIGN} could not be run: {error:?}"))?;
        // `codesign` exits 0 for a program that meets it, 3 for one that is
        // signed and does not, and 1 for anything else, an unsigned or
        // damaged program among them: neither of those is the app's.
        Ok(finished.succeeded())
    }

    /// The major version of this macOS, as the kernel names it.
    fn macos_major() -> io::Result<u32> {
        let mut buffer = [0_u8; 32];
        let mut size = buffer.len();
        // SAFETY: the name is a nul-terminated string and `buffer` holds
        // `size` bytes; the kernel writes a nul-terminated string no longer
        // than that and says how long in `size`.
        if unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let text = CStr::from_bytes_until_nul(&buffer)
            .map_err(io::Error::other)?
            .to_string_lossy();
        text.split('.')
            .next()
            .and_then(|major| major.parse().ok())
            .ok_or_else(|| io::Error::other(format!("macOS names its version {text:?}")))
    }

    /// The function that marks a program started with these attributes as
    /// responsible for itself. It is exported by the system library and
    /// declared in no header, so it is looked up by name when asked for.
    const DISCLAIM: &CStr = c"responsibility_spawnattrs_setdisclaim";

    pub(super) fn answer_for_itself(program: &Path, arguments: &[OsString]) -> NotAnswering {
        start_again(DISCLAIM, program, arguments)
    }

    pub(super) fn start_again(
        disclaim: &CStr,
        program: &Path,
        arguments: &[OsString],
    ) -> NotAnswering {
        // SAFETY: a lookup by name in the images this process already loaded.
        let found = unsafe { libc::dlsym(libc::RTLD_DEFAULT, disclaim.as_ptr()) };
        if found.is_null() {
            return NotAnswering::NoWay;
        }
        let invalid = |error: std::ffi::NulError| {
            NotAnswering::Start(io::Error::new(io::ErrorKind::InvalidInput, error))
        };
        let path = match CString::new(program.as_os_str().as_bytes()) {
            Ok(path) => path,
            Err(error) => return invalid(error),
        };
        let mut owned = vec![path.clone()];
        for argument in arguments {
            match CString::new(argument.as_bytes()) {
                Ok(argument) => owned.push(argument),
                Err(error) => return invalid(error),
            }
        }
        let mut argv: Vec<*mut libc::c_char> = owned
            .iter()
            .map(|argument| argument.as_ptr().cast_mut())
            .collect();
        argv.push(std::ptr::null_mut());

        unsafe extern "C" {
            static environ: *const *mut libc::c_char;
        }
        // SAFETY: `found` is the address of
        // `int responsibility_spawnattrs_setdisclaim(posix_spawnattr_t *, int)`,
        // the signature every caller of it in the open (Chromium, LLDB,
        // Deno, .NET) uses.
        let set_disclaim: unsafe extern "C" fn(
            *mut libc::posix_spawnattr_t,
            libc::c_int,
        ) -> libc::c_int = unsafe { std::mem::transmute(found) };
        let mut attributes = std::mem::MaybeUninit::<libc::posix_spawnattr_t>::uninit();
        // SAFETY: `attributes` is initialized by `posix_spawnattr_init`
        // before any other use and destroyed on every path that returns;
        // `path`, `argv` and `environ` are nul-terminated and outlive the
        // call. With `POSIX_SPAWN_SETEXEC` a successful `posix_spawn` does
        // not return: it replaces this process's image.
        let code = unsafe {
            let code = libc::posix_spawnattr_init(attributes.as_mut_ptr());
            if code != 0 {
                return NotAnswering::Start(io::Error::from_raw_os_error(code));
            }
            let mut code = libc::posix_spawnattr_setflags(
                attributes.as_mut_ptr(),
                libc::POSIX_SPAWN_SETEXEC as libc::c_short,
            );
            if code == 0 {
                code = set_disclaim(attributes.as_mut_ptr(), 1);
            }
            if code == 0 {
                code = libc::posix_spawn(
                    std::ptr::null_mut(),
                    path.as_ptr(),
                    std::ptr::null(),
                    attributes.as_ptr(),
                    argv.as_ptr(),
                    environ,
                );
            }
            libc::posix_spawnattr_destroy(attributes.as_mut_ptr());
            code
        };
        NotAnswering::Start(io::Error::from_raw_os_error(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_macos_the_naming_was_measured_on_is_verified() {
        assert!(!verified(13));
        assert!(!verified(14));
        assert!(verified(15));
        assert!(verified(16));
        assert!(verified(26));
    }

    #[test]
    fn the_identity_file_names_the_program_and_carries_every_usage_sentence() {
        let identity = Identity {
            identifier: "dev.withhide.test.identity",
            name: "hide & <test>",
            requirement: "",
            usage: &[
                ("NSMicrophoneUsageDescription", "a microphone"),
                ("NSCameraUsageDescription", "a camera"),
            ],
        };
        let text =
            String::from_utf8(identity_file(Path::new("/x/hide-holder"), &identity)).unwrap();
        for expected in [
            "<key>CFBundleIdentifier</key>\n  <string>dev.withhide.test.identity</string>",
            "<key>CFBundleName</key>\n  <string>hide &amp; &lt;test&gt;</string>",
            "<key>CFBundleExecutable</key>\n  <string>hide-holder</string>",
            "<key>NSMicrophoneUsageDescription</key>\n  <string>a microphone</string>",
            "<key>NSCameraUsageDescription</key>\n  <string>a camera</string>",
        ] {
            assert!(text.contains(expected), "{expected} is missing from {text}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_system_without_the_function_leaves_the_program_as_it_is() {
        let refused = imp::start_again(
            c"hide_platform_no_such_function",
            &std::env::current_exe().unwrap(),
            &[],
        );
        assert!(matches!(refused, NotAnswering::NoWay), "{refused:?}");
    }
}
