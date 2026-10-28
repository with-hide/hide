//! The WSL transport against a stand-in `wsl.exe`: a script that runs the
//! command it is given on this machine, as the distribution's account would,
//! in a home of the test's, with `uname` answering Linux. Each call it gets
//! is written to a log, so a test sees exactly which `wsl.exe` ran.

use std::os::unix::fs::PermissionsExt;

use super::*;

struct Fixture {
    dir: tempfile::TempDir,
    access: WslAccess,
    found: Arc<Mutex<Vec<Distribution>>>,
}

const FAKE_WSL: &str = r#"#!/bin/sh
here=$(dirname "$0")
printf '%s\n' "$*" >> "$here/calls.log"
if [ "$1" = "--list" ]; then
    if [ -e "$here/list-refused" ]; then
        echo 'Access is denied.' >&2
        exit 1
    fi
    cat "$here/running" 2>/dev/null
    exit 0
fi
while [ $# -gt 0 ]; do
    case $1 in
        -d|-u) shift 2 ;;
        --cd) shift 2 ;;
        -e) shift; break ;;
        *) echo "unexpected $1" >&2; exit 64 ;;
    esac
done
HOME=$here/home
PATH=$here/shims:$PATH
export HOME PATH
cd "$HOME" || exit 1
exec "$@"
"#;

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("wsl.exe");
    std::fs::write(&program, FAKE_WSL).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let shims = dir.path().join("shims");
    std::fs::create_dir(&shims).unwrap();
    let uname = shims.join("uname");
    std::fs::write(&uname, "#!/bin/sh\necho Linux x86_64\n").unwrap();
    std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir(dir.path().join("home")).unwrap();
    let found = Arc::new(Mutex::new(vec![distribution()]));
    let listed = Arc::clone(&found);
    let access = WslAccess::with(program, Arc::new(move || Ok(lock_recover(&listed).clone())));
    Fixture { dir, access, found }
}

fn distribution() -> Distribution {
    Distribution {
        name: "Ubuntu".to_owned(),
        id: "25622596-db24-4e3a-9a3e-6b3f0d2e1c11".to_owned(),
        wsl_version: 2,
        installed: true,
        first_run_pending: false,
        default: true,
    }
}

impl Fixture {
    fn device(&self, packages: HelperPackages) -> WslDevice {
        WslDevice::new("Ubuntu", self.access.clone(), packages, None, None).unwrap()
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn running(&self, names: &str) {
        std::fs::write(self.dir.path().join("running"), names).unwrap();
    }

    /// A build folder holding a helper, a `hide` and the hooks for Linux
    /// x86_64, as a Windows package carries them.
    fn packages(&self) -> HelperPackages {
        let builds = self.dir.path().join("builds");
        std::fs::create_dir_all(&builds).unwrap();
        for (name, bytes) in [
            ("hided", "helper build 1"),
            ("hide", "command build 1"),
            ("hide-agent-hooks", "hooks build 1"),
        ] {
            let path = builds.join(format!("{name}-linux-x86_64"));
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        HelperPackages::new(Some(builds))
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }
}

fn consent(identity: Option<HostIdentity>) -> HostConsent {
    HostConsent {
        contract: hide_node_link::device::HOST_CONSENT_CONTRACT,
        helper_root: "~/.hide/helper".to_owned(),
        cli_dir: Some("~/.local/bin".to_owned()),
        granted_at_unix_ms: 1,
        identity,
    }
}

fn establish(device: &WslDevice, consent: &HostConsent, wake: Wake) -> EstablishError {
    match device.establish(consent, &[], wake, Box::new(|_| {})) {
        Ok(_) => panic!("the stand-in has no helper to start"),
        Err(error) => error,
    }
}

/// B24, D-11: an attempt that may not wake a distribution that is off ends
/// as stopped after the one listing that reads which run, and nothing else
/// runs inside it.
#[test]
fn an_attempt_that_may_not_wake_a_stopped_distribution_runs_nothing_in_it() {
    let fixture = fixture();
    fixture.running("Debian\n");
    let device = fixture.device(fixture.packages());
    let error = establish(&device, &consent(None), Wake::NotAllowed);
    assert!(matches!(error, EstablishError::Stopped(_)), "{error}");
    assert_eq!(fixture.calls(), ["--list --running --quiet"]);
}

/// D-15, B24: a listing that fails says nothing about which distributions
/// run: the attempt is a failure, not stopped, and still runs nothing in
/// the distribution.
#[test]
fn a_failing_listing_is_no_answer_and_wakes_nothing() {
    let fixture = fixture();
    std::fs::write(fixture.dir.path().join("list-refused"), "").unwrap();
    let device = fixture.device(fixture.packages());
    let error = establish(&device, &consent(None), Wake::NotAllowed);
    assert!(matches!(error, EstablishError::Helper(_)), "{error}");
    assert_eq!(fixture.calls(), ["--list --running --quiet"]);
}

/// B3, B23: the same attempt on a distribution that runs, or one that may
/// wake it, goes on to the account probe.
#[test]
fn a_running_distribution_or_an_attempt_that_may_wake_it_is_probed() {
    let fixture = fixture();
    fixture.running("Ubuntu\n");
    let device = fixture.device(HelperPackages::new(None));
    establish(&device, &consent(None), Wake::NotAllowed);
    let calls = fixture.calls();
    assert_eq!(calls[0], "--list --running --quiet");
    assert!(
        calls[1].starts_with("-d Ubuntu --cd ~ -e sh -c id -un;"),
        "{calls:?}"
    );

    let fixture = self::fixture();
    let device = fixture.device(HelperPackages::new(None));
    establish(&device, &consent(None), Wake::Allowed);
    let calls = fixture.calls();
    assert!(
        calls[0].starts_with("-d Ubuntu --cd ~ -e sh -c id -un;"),
        "{calls:?}"
    );
}

/// B28: a distribution whose first-run setup has not finished, or one on
/// WSL 1, is refused from the registry alone, before any `wsl.exe`.
#[test]
fn an_unfinished_or_wsl1_distribution_is_refused_before_any_wsl_call() {
    let fixture = fixture();
    let device = fixture.device(fixture.packages());
    lock_recover(&fixture.found)[0].first_run_pending = true;
    let error = establish(&device, &consent(None), Wake::Allowed);
    assert!(
        matches!(&error, EstablishError::Unsupported(reason) if reason.contains("Finish setting up")),
        "{error}"
    );
    lock_recover(&fixture.found)[0] = Distribution {
        wsl_version: 1,
        ..distribution()
    };
    let error = establish(&device, &consent(None), Wake::Allowed);
    assert!(
        matches!(&error, EstablishError::Unsupported(reason) if reason.contains("wsl --set-version Ubuntu 2")),
        "{error}"
    );
    assert!(fixture.calls().is_empty(), "{:?}", fixture.calls());
}

/// B28: a reinstalled distribution, or one whose default user changed,
/// answers with another identity than the consent was bound to, and the
/// attempt stops there, before anything is installed.
#[test]
fn a_reinstalled_distribution_or_another_default_user_asks_consent_again() {
    let fixture = fixture();
    let device = fixture.device(fixture.packages());
    let user = String::from_utf8(
        std::process::Command::new("id")
            .arg("-un")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let bound = identity_of(&distribution(), user.trim());
    let reinstalled = HostIdentity {
        host_key_sha256: "wsl:00000000-0000-0000-0000-000000000000".to_owned(),
        ..bound.clone()
    };
    let error = establish(&device, &consent(Some(reinstalled)), Wake::Allowed);
    assert!(
        matches!(error, EstablishError::IdentityChanged { .. }),
        "{error}"
    );
    let other_user = HostIdentity {
        user: "someone-else".to_owned(),
        ..bound
    };
    let error = establish(&device, &consent(Some(other_user)), Wake::Allowed);
    assert!(
        matches!(error, EstablishError::IdentityChanged { .. }),
        "{error}"
    );
    assert!(!fixture.home().join(".hide").exists());
}

/// B26: the install sends each file once, reuses what is there whole, and
/// sends again only what an interrupted or damaged install left wrong.
#[test]
fn an_install_resumes_with_only_what_is_missing_or_damaged() {
    let fixture = fixture();
    let packages = fixture.packages();
    let device = fixture.device(packages.clone());
    let payload = packages.payload("linux", "x86_64").unwrap();
    let user = String::from_utf8(
        std::process::Command::new("id")
            .arg("-un")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let root = fixture.home().join(".hide/helper");
    let root_text = root.to_str().unwrap();

    let (helper, fresh, upload) = device.install(user.trim(), root_text, &payload).unwrap();
    assert!(fresh);
    assert_eq!((upload.sent, upload.reused), (3, 0));
    assert_eq!(std::fs::read(&helper).unwrap(), b"helper build 1");
    let mode = std::fs::metadata(&helper).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let root_mode = std::fs::metadata(&root).unwrap().permissions().mode();
    assert_eq!(
        root_mode & 0o077,
        0,
        "the helper root is the account's alone"
    );

    let (_, fresh, upload) = device.install(user.trim(), root_text, &payload).unwrap();
    assert!(!fresh);
    assert_eq!((upload.sent, upload.reused), (0, 3));

    // An upload cut off before its rename leaves its staging name and the
    // final one untouched; a damaged final copy is replaced.
    let folder = std::path::Path::new(&helper).parent().unwrap();
    std::fs::write(folder.join(".upload-hide-4242"), b"half").unwrap();
    std::fs::write(folder.join("hide"), b"damaged").unwrap();
    let (_, fresh, upload) = device.install(user.trim(), root_text, &payload).unwrap();
    assert!(!fresh);
    assert_eq!((upload.sent, upload.reused), (1, 2));
    assert_eq!(
        std::fs::read(folder.join("hide")).unwrap(),
        b"command build 1"
    );
}

/// B27: the link owns its `wsl.exe`: closing the link ends the child, and
/// the child ending ends the link.
#[test]
fn a_child_link_ends_its_child_and_ends_with_it() {
    let marker = tempfile::tempdir().unwrap();
    let ended = marker.path().join("ended");
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", &format!("cat >/dev/null; touch {}", ended.display())])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let pid = child.id();
    let link = super::super::over_child("wsl:test", child, None, Box::new(|_| {})).unwrap();
    link.close("the test is done");
    let deadline = Instant::now() + Duration::from_secs(10);
    while hide_platform::process::is_alive(pid) {
        assert!(Instant::now() < deadline, "the child outlived its link");
        std::thread::yield_now();
    }
    assert!(
        ended.exists(),
        "the child read the end of its input and exited by itself"
    );

    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", "exit 0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let (heard, closed) = std::sync::mpsc::channel();
    let link = super::super::over_child(
        "wsl:test",
        child,
        None,
        Box::new(move |reason| {
            let _ = heard.send(reason);
        }),
    )
    .unwrap();
    let reason = closed.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(reason.contains("output closed"), "{reason}");
    assert!(link.closed_reason().is_some());
}

#[test]
fn the_probe_reads_the_account_and_refuses_what_it_cannot_run_as() {
    let probe = Probe::read("alice\n1000\n/srv/alice\nLinux x86_64\nsha256sum\n").unwrap();
    assert_eq!(
        probe,
        Probe {
            user: "alice".to_owned(),
            home: "/srv/alice".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        }
    );
    assert!(Probe::read("alice\n1000\n/srv/alice\nLinux x86_64\nnone\n").is_err());
    assert!(Probe::read("Alice Smith\n1000\n/srv/alice\nLinux x86_64\nsha256sum\n").is_err());
    assert!(Probe::read("alice\n1000\n/srv/alice\nLinux riscv64\nsha256sum\n").is_err());
}

#[test]
fn a_call_past_the_cap_waits_until_its_deadline_and_one_that_ends_frees_a_slot() {
    let gate: &'static CallGate = Box::leak(Box::new(CallGate {
        running: Mutex::new(0),
        freed: std::sync::Condvar::new(),
    }));
    let mut held: Vec<_> = (0..MAX_CALLS)
        .map(|_| gate.admit(Instant::now()).expect("a free slot"))
        .collect();
    let asked = Instant::now();
    assert!(
        gate.admit(asked + Duration::from_millis(50)).is_none(),
        "a call past the cap was admitted"
    );
    assert!(asked.elapsed() >= Duration::from_millis(50));
    held.pop();
    assert!(gate.admit(Instant::now()).is_some());
}

#[test]
fn a_write_blocked_on_a_child_that_stopped_reading_does_not_hold_its_end() {
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", "exec sleep 30"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let pid = child.id();
    let stdin = child.take_stdin().unwrap();
    let input: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
        Arc::new(Mutex::new(Some(Box::new(stdin))));
    let (holding, held) = std::sync::mpsc::channel();
    let writer = {
        let input = Arc::clone(&input);
        std::thread::spawn(move || {
            let mut stream = input.lock().unwrap();
            holding.send(()).unwrap();
            // More than any pipe holds, to a child that never reads.
            std::io::Write::write_all(stream.as_mut().unwrap(), &vec![0u8; 8 * 1024 * 1024])
        })
    };
    held.recv().unwrap();
    let told = Instant::now();
    super::super::ChildEnding::start("wsl:test", child, input)
        .unwrap()
        .end();
    let deadline = told + super::super::CHILD_EXIT_GRACE + Duration::from_secs(5);
    while hide_platform::process::is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the child outlived its link's end"
        );
        std::thread::yield_now();
    }
    assert!(
        writer.join().unwrap().is_err(),
        "the blocked write ends with the child"
    );
}
