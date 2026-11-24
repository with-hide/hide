//! `privacy`'s contract: who is given the app's name, and that a program
//! started again as responsible for itself is the same process.
//!
//! No test here uses the app's identifier or its requirement: a program that
//! claimed them on a developer's Mac would be read against the record the
//! installed app's consent is kept under. They use an identifier of their
//! own, and nothing here asks macOS for any consent.

use std::path::Path;

use hide_platform::privacy::{self, Given, Identity, Refusal};

const IDENTIFIER: &str = "dev.withhide.test.identity";

/// A requirement an ad-hoc program signed under [`IDENTIFIER`] meets.
#[cfg(target_os = "macos")]
const MET: &str = "identifier \"dev.withhide.test.identity\"";

/// One it does not: it names another identifier.
#[cfg(target_os = "macos")]
const UNMET: &str = "identifier \"dev.withhide.test.another\"";

fn identity(requirement: &str) -> Identity<'_> {
    Identity {
        identifier: IDENTIFIER,
        name: "hide test",
        requirement,
        usage: &[("NSMicrophoneUsageDescription", "a test's microphone")],
    }
}

/// The files of `dir`, by name.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[cfg(not(target_os = "macos"))]
#[test]
fn a_system_without_consent_per_program_gives_nothing_and_starts_nothing_again() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program");
    std::fs::write(&program, "x").unwrap();
    assert_eq!(
        privacy::give(&program, &identity("anything")),
        Given::Own(Refusal::NotMacos)
    );
    assert_eq!(names(dir.path()), ["program"]);
    assert!(!privacy::has_identity_file(&program));
    let refused = privacy::answer_for_itself(&program, &[]);
    assert!(
        matches!(refused, privacy::NotAnswering::NotMacos),
        "{refused:?}"
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io::{BufRead, BufReader, Read};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::{Command, Stdio};

    use super::*;

    const ROLE: &str = "HIDE_PLATFORM_PRIVACY_ROLE";
    const LENT: &str = "HIDE_PLATFORM_PRIVACY_LENT";
    const KEPT: &str = "HIDE_PLATFORM_PRIVACY_KEPT";
    /// The argument the second start carries and the first does not. It is
    /// a filter libtest reads and that matches no test.
    const SECOND: &str = "privacy-second-start";

    /// A signed native program at `at`: a copy of a system program signed
    /// ad hoc under [`IDENTIFIER`]. It is asked about, never run.
    fn signed_program(at: &Path) {
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::copy("/usr/bin/true", at).unwrap();
        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--identifier", IDENTIFIER])
            .arg(at)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            signed.status.success(),
            "{}",
            String::from_utf8_lossy(&signed.stderr)
        );
    }

    #[test]
    fn a_program_that_meets_the_requirement_gets_the_identity_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("hide-holder");
        signed_program(&program);
        assert!(!privacy::has_identity_file(&program));

        assert_eq!(privacy::give(&program, &identity(MET)), Given::Shared);
        assert_eq!(names(dir.path()), ["Info.plist", "hide-holder"]);
        assert!(privacy::has_identity_file(&program));
        let file = dir.path().join(privacy::IDENTITY_FILE);
        let written = std::fs::read_to_string(&file).unwrap();
        for expected in [
            "<string>dev.withhide.test.identity</string>",
            "<key>CFBundleExecutable</key>\n  <string>hide-holder</string>",
            "<key>NSMicrophoneUsageDescription</key>\n  <string>a test's microphone</string>",
        ] {
            assert!(written.contains(expected), "{expected} is not in {written}");
        }
        // macOS reads it as a property list.
        let read = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw"])
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&read.stdout).trim(), IDENTIFIER);

        // Asked again, with the file in place: the same answer and file.
        assert_eq!(privacy::give(&program, &identity(MET)), Given::Shared);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    }

    #[test]
    fn a_program_that_does_not_meet_the_requirement_keeps_its_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("hide-holder");
        signed_program(&program);
        assert_eq!(
            privacy::give(&program, &identity(UNMET)),
            Given::Own(Refusal::RequirementNotMet)
        );
        assert_eq!(names(dir.path()), ["hide-holder"]);
        // And an unsigned file, which is no program of the app's either.
        let unsigned = dir.path().join("plain").join("hided");
        std::fs::create_dir(unsigned.parent().unwrap()).unwrap();
        std::fs::write(&unsigned, "not a program").unwrap();
        assert_eq!(
            privacy::give(&unsigned, &identity(MET)),
            Given::Own(Refusal::RequirementNotMet)
        );
        assert_eq!(names(unsigned.parent().unwrap()), ["hided"]);
    }

    #[test]
    fn a_program_inside_an_app_is_left_to_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir
            .path()
            .join("hide.app")
            .join("Contents")
            .join("Resources")
            .join("hided");
        signed_program(&program);
        assert_eq!(
            privacy::give(&program, &identity(MET)),
            Given::Own(Refusal::InsideApp)
        );
        assert_eq!(names(program.parent().unwrap()), ["hided"]);
    }

    #[test]
    fn another_identity_file_beside_the_program_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("hided");
        signed_program(&program);
        let file = dir.path().join(privacy::IDENTITY_FILE);
        std::fs::write(&file, "someone else's").unwrap();
        assert_eq!(
            privacy::give(&program, &identity(MET)),
            Given::Own(Refusal::AnotherIdentity)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "someone else's");
    }

    #[test]
    fn a_program_that_cannot_be_started_again_goes_on_as_it_is() {
        let refused = privacy::answer_for_itself(Path::new("/nonexistent/hide-holder"), &[]);
        match refused {
            privacy::NotAnswering::Start(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// The pid macOS holds responsible for this process. Read through the
    /// system's own function, in this test only: the product asks no such
    /// question.
    fn responsible() -> i32 {
        // SAFETY: a lookup by name, then a call of
        // `pid_t responsibility_get_pid_responsible_for_pid(pid_t)`.
        unsafe {
            let found = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"responsibility_get_pid_responsible_for_pid".as_ptr(),
            );
            assert!(!found.is_null());
            let ask: unsafe extern "C" fn(libc::pid_t) -> libc::pid_t = std::mem::transmute(found);
            ask(libc::getpid())
        }
    }

    /// What a child of this test binary does: the first start says who it is
    /// and starts itself again, the second says who it is now and writes to
    /// the descriptor it was lent.
    #[test]
    fn child_role() {
        if std::env::var_os(ROLE).is_none() {
            return;
        }
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        let pid = std::process::id();
        if arguments.iter().any(|argument| argument == SECOND) {
            println!(
                "SECOND {pid} {} {}",
                responsible(),
                std::env::var(KEPT).unwrap()
            );
            let lent: i32 = std::env::var(LENT).unwrap().parse().unwrap();
            // SAFETY: the descriptor the test lent this process, used once.
            let mut lent = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(lent) });
            std::io::Write::write_all(&mut lent, b"LENT\n").unwrap();
            return;
        }
        println!("FIRST {pid}");
        let mut again = arguments;
        again.extend(["--skip".into(), SECOND.into()]);
        let refused = privacy::answer_for_itself(&std::env::current_exe().unwrap(), &again);
        println!("REFUSED {}", refused.reason());
    }

    #[test]
    fn a_program_started_again_is_the_same_process_and_responsible_for_itself() {
        let mut pipe = [0; 2];
        // SAFETY: `pipe` fills two descriptors, which are not marked to
        // close, so the child and its second start inherit them.
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        // SAFETY: both were just made and are owned here alone.
        let (read, write) =
            unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "privacy::macos::child_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ROLE, "answer")
            .env(LENT, write.as_raw_fd().to_string())
            .env(KEPT, "kept")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(write);
        let said: Vec<String> = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| {
                ["FIRST ", "SECOND ", "REFUSED "]
                    .iter()
                    .find_map(|word| line.find(word).map(|at| line[at..].to_owned()))
            })
            .collect();
        assert!(child.wait().unwrap().success(), "{said:?}");
        let pid = child.id();
        let [first, second] = said.as_slice() else {
            panic!("the child said {said:?}");
        };
        let first: Vec<&str> = first.split(' ').collect();
        let second: Vec<&str> = second.split(' ').collect();
        // The first start is this test's child. Who answers for it is not
        // asserted: started from an app it is the app, and from a shell with
        // no app above it, as on a runner, it is already the child itself.
        assert_eq!(first[1], pid.to_string());
        // The second is the same process, responsible for itself, with the
        // environment it had.
        assert_eq!(
            second,
            ["SECOND", &pid.to_string(), &pid.to_string(), "kept"]
        );
        let mut lent = String::new();
        std::fs::File::from(read).read_to_string(&mut lent).unwrap();
        assert_eq!(lent, "LENT\n");
    }
}
