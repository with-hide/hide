//! The start script against a real `sh` on this machine, through the same
//! child link a device connection uses: the device side is the script
//! itself, so what it answers is what a device answers.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use super::*;

struct Device {
    dir: tempfile::TempDir,
    /// The process of the latest connection.
    pid: std::cell::Cell<u32>,
}

impl Device {
    fn new() -> Self {
        let device = Self {
            dir: tempfile::tempdir().unwrap(),
            pid: std::cell::Cell::new(0),
        };
        std::fs::create_dir(device.home()).unwrap();
        std::fs::create_dir(device.tmp()).unwrap();
        device
    }

    /// The account's temporary folder, where a run's output waits.
    fn tmp(&self) -> PathBuf {
        self.dir.path().join("tmp")
    }

    fn tmp_is_empty(&self) -> bool {
        std::fs::read_dir(self.tmp()).unwrap().next().is_none()
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// One connection: `sh` running the wrapper in the account's home, as
    /// the remote command of an `ssh` runs it; `greeting` is what a login
    /// file prints before it.
    fn connect(&self, greeting: &str) -> UnaskedStart {
        let mut command = std::process::Command::new("sh");
        command
            .args(["-c", &format!("{greeting}{}", WRAPPER[2])])
            .current_dir(self.home())
            .env("HOME", self.home())
            .env("TMPDIR", self.tmp())
            .env_remove(hide_platform::host::MACHINE_ID_VARIABLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
        self.pid.set(child.id());
        UnaskedStart::new(
            ChildLink::start("device:test", child, None).unwrap(),
            hide_node_link::device::Cancel::default(),
        )
    }

    /// A build for this machine whose helper is `helper`.
    fn packages(&self, os: &str, arch: &str, helper: &str) -> HelperPackages {
        let builds = self.dir.path().join("builds");
        std::fs::create_dir_all(&builds).unwrap();
        for (name, bytes) in [
            ("hided", helper),
            ("hide", "command build 1"),
            ("hide-agent-hooks", "hooks build 1"),
        ] {
            let path = builds.join(format!("{name}-{os}-{arch}"));
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        HelperPackages::new(Some(builds))
    }
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

fn own_user() -> String {
    let output = std::process::Command::new("id")
        .arg("-un")
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// B22: the probe answers the account, its home and its platform, past
/// whatever a login file printed first.
#[test]
fn the_probe_reads_the_account_past_a_login_greeting() {
    let device = Device::new();
    let mut pipe = device.connect("echo 'Welcome to the build host'; echo; ");
    let facts = probe(&mut pipe, soon()).unwrap();
    assert_eq!(facts.user, own_user());
    assert_eq!(Path::new(&facts.home), device.home());
    assert_eq!(facts.os, std::env::consts::OS);
    assert_eq!(facts.arch, std::env::consts::ARCH);
}

#[test]
fn a_probe_answer_names_only_a_machine_hide_runs_on() {
    let fields = |line: &str| line.split('\t').map(str::to_owned).collect::<Vec<_>>();
    let facts = read_probe(&fields(
        "probe\talice\t/home/alice\tLinux\taarch64\tAB-12\tyes",
    ))
    .unwrap();
    assert_eq!(facts.machine_id, "ab-12");
    assert_eq!(
        (facts.os.as_str(), facts.arch.as_str()),
        ("linux", "aarch64")
    );
    // P1-5: a machine with no usable id is refused, never bound to nothing.
    for id in ["-", "", "a b", "ab\u{7}c", &"a".repeat(129)] {
        assert_eq!(
            read_probe(&fields(&format!(
                "probe\talice\t/home/alice\tLinux\tx86_64\t{id}\tyes"
            ))),
            Err(super::NO_MACHINE_ID.to_owned()),
            "{id:?}"
        );
    }
    for refused in [
        "probe\talice\t/home/alice\tLinux\tx86_64\t-\tnone",
        "probe\tAlice Smith\t/home/alice\tLinux\tx86_64\t-\tyes",
        "probe\talice\trelative\tLinux\tx86_64\t-\tyes",
        "probe\talice\t/home/alice\tFreeBSD\tx86_64\t-\tyes",
        "probe\talice\t/home/alice\tLinux\triscv64\t-\tyes",
        "fail\tsomething",
    ] {
        assert!(read_probe(&fields(refused)).is_err(), "{refused}");
    }
}

/// B23: the install sends each file once, reuses what the device holds
/// whole, and sends again only what is missing or damaged.
#[test]
fn an_install_sends_only_what_the_device_does_not_hold_whole() {
    let device = Device::new();
    let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, "helper 1");
    let install_once = || {
        let mut pipe = device.connect("");
        let facts = probe(&mut pipe, soon()).unwrap();
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        install(&mut pipe, "~/.hide/helper", &facts.home, &payload).unwrap()
    };

    let first = install_once();
    assert!(first.fresh);
    assert_eq!((first.upload.sent, first.upload.reused), (3, 0));
    let helper = PathBuf::from(&first.helper_path);
    assert_eq!(std::fs::read(&helper).unwrap(), b"helper 1");
    assert_eq!(mode(&helper), 0o700);
    let folder = helper.parent().unwrap().to_owned();
    assert_eq!(mode(&folder.join("hide")), 0o700);
    let root = device.home().join(".hide/helper");
    assert_eq!(
        mode(&root) & 0o077,
        0,
        "the install folder is the account's alone"
    );
    assert!(first.helper_path.ends_with(&first.helper_relative));

    let again = install_once();
    assert!(!again.fresh);
    assert_eq!((again.upload.sent, again.upload.reused), (0, 3));

    std::fs::write(folder.join("hide"), b"damaged").unwrap();
    let repaired = install_once();
    assert!(!repaired.fresh);
    assert_eq!((repaired.upload.sent, repaired.upload.reused), (1, 2));
    assert_eq!(
        std::fs::read(folder.join("hide")).unwrap(),
        b"command build 1"
    );
}

/// B23: a connection that ends part way through a file leaves the final
/// name as it was, so nothing half sent is ever started.
#[test]
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn an_install_cut_off_mid_file_leaves_no_half_file_in_use() {
    let device = Device::new();
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let root = format!("{}/.hide/helper", facts.home);
    let [kind, _] = &step(&mut pipe, &format!("root\t{root}\n"), soon()).unwrap()[..] else {
        panic!("the root step did not answer");
    };
    assert_eq!(kind, "root");
    pipe.send(b"put\tv1/hided\t00\t700\t100\nonly ten b", soon())
        .unwrap();
    drop(pipe);
    let deadline = Instant::now() + Duration::from_secs(10);
    while hide_platform::process::is_alive(device.pid.get()) {
        assert!(
            Instant::now() < deadline,
            "the script outlived its connection"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let folder = device.home().join(".hide/helper/v1");
    assert!(
        !folder.join("hided").exists(),
        "a half file took the final name"
    );
}

/// The install folder and every folder above it must be ones no other
/// account can change.
#[test]
fn an_install_under_a_folder_others_can_change_is_refused() {
    let device = Device::new();
    let open = device.home().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, "helper 1");
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let payload = packages.payload(&facts.os, &facts.arch).unwrap();
    let Err(EstablishError::Install(reason)) =
        install(&mut pipe, "~/open/helper", &facts.home, &payload)
    else {
        panic!("an install under a folder anyone can change was accepted");
    };
    assert!(reason.contains("chmod go-w"), "{reason}");
    let mut placed = std::fs::read_dir(open.join("helper")).unwrap();
    assert!(placed.next().is_none(), "a file was installed under it");
}

/// B24: the helper starts in place of the script, on the same pipes, with
/// the Herdr the registration names.
#[test]
fn the_helper_starts_on_the_same_pipes_with_the_chosen_herdr() {
    let device = Device::new();
    let helper = "#!/bin/sh\necho \"args:$*\"\necho \"session:${HERDR_SESSION:-} socket:${HERDR_SOCKET_PATH:-}\"\n";
    let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, helper);
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let payload = packages.payload(&facts.os, &facts.arch).unwrap();
    let installed = install(&mut pipe, "~/.hide/helper", &facts.home, &payload).unwrap();
    start(
        &mut pipe,
        &installed.helper_relative,
        &HerdrChoice::Session("work".to_owned()),
    )
    .unwrap();
    assert_eq!(pipe.line(soon()).unwrap(), b"args:node serve --own-herdr");
    assert_eq!(pipe.line(soon()).unwrap(), b"session:work socket:");
}

#[test]
fn a_step_naming_a_line_end_is_never_sent() {
    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    assert!(
        start(
            &mut pipe,
            "v1/hided",
            &HerdrChoice::Socket("/tmp/a\nstart\tx".to_owned())
        )
        .is_err()
    );
}

fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// A core move's command answers its status and both outputs, its words
/// arrive whole, and the script takes the next step after it; output past
/// the cap is refused with the connection still in step.
#[test]
fn a_command_answers_its_status_and_output_and_the_next_step_follows() {
    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    let ran = run(
        &mut pipe,
        &words(&["sh", "-c", "printf 'out\\n'; printf err >&2; exit 3"]),
        1024,
        soon(),
    )
    .unwrap();
    assert_eq!(
        (ran.status, ran.stdout.as_slice(), ran.stderr.as_slice()),
        (3, &b"out\n"[..], &b"err"[..])
    );
    let ran = run(
        &mut pipe,
        &words(&["printf", "%s|", "a b", "'c\"", "$HOME", "*"]),
        1024,
        soon(),
    )
    .unwrap();
    assert_eq!(ran.stdout, b"a b|'c\"|$HOME|*|");
    assert!(matches!(
        run(
            &mut pipe,
            &words(&["head", "-c", "2000", "/dev/zero"]),
            1024,
            soon()
        ),
        Err(StepError::Refused(_))
    ));
    // The script's own helpers are not commands a step can run.
    assert_eq!(
        run(&mut pipe, &words(&["store", "x"]), 1024, soon())
            .unwrap()
            .status,
        127
    );
    assert!(run(&mut pipe, &words(&["a\nb"]), 1024, soon()).is_err());
    assert_eq!(
        run(&mut pipe, &words(&["true"]), 16, soon())
            .unwrap()
            .status,
        0
    );
}

/// A copied file lands whole under a private folder, a damaged one leaves
/// the file already there alone, and the same file reads back with the
/// digest the machine states for it.
#[test]
fn a_file_goes_whole_into_a_private_folder_and_comes_back_whole() {
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    let bytes: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let remote = device.home().join("moved/deep/file");
    let spelled = remote.to_str().unwrap();
    let sent = AtomicU64::new(0);
    put_file(
        &mut pipe,
        spelled,
        &digest,
        bytes.len() as u64,
        &mut bytes.as_slice(),
        &|chunk| {
            sent.fetch_add(chunk, Ordering::SeqCst);
        },
    )
    .unwrap();
    assert_eq!(sent.load(Ordering::SeqCst), bytes.len() as u64);
    assert_eq!(std::fs::read(&remote).unwrap(), bytes);
    assert_eq!(mode(&device.home().join("moved")), 0o700);
    assert_eq!(mode(&remote) & 0o077, 0);

    assert!(matches!(
        put_file(
            &mut pipe,
            spelled,
            &"0".repeat(64),
            5,
            &mut &b"other"[..],
            &|_| {}
        ),
        Err(StepError::Refused(_))
    ));
    assert_eq!(std::fs::read(&remote).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(remote.parent().unwrap()).unwrap().count(),
        1
    );

    let mut back = Vec::new();
    let (size, stated) = get_file(&mut pipe, spelled, &mut back, &|_| {}).unwrap();
    assert_eq!(
        (size, stated.as_str()),
        (bytes.len() as u64, digest.as_str())
    );
    assert_eq!(back, bytes);
    assert!(matches!(
        get_file(&mut pipe, "/no/such/file", &mut Vec::new(), &|_| {}),
        Err(StepError::Refused(_))
    ));
    for refused in ["relative/file", "/a/../b"] {
        assert!(matches!(
            put_file(&mut pipe, refused, &digest, 0, &mut &b""[..], &|_| {}),
            Err(StepError::Refused(_))
        ));
    }
    assert_eq!(
        run(&mut pipe, &words(&["true"]), 16, soon())
            .unwrap()
            .status,
        0
    );
}

/// The attach step hands the connection's pipes to the core's program, with
/// its state folder when one is named.
#[test]
fn attach_hands_the_pipes_to_the_core_program() {
    let device = Device::new();
    let program = device.dir.path().join("hided");
    std::fs::write(&program, "#!/bin/sh\necho \"args:$*\"\nexec cat\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spelled = program.to_str().unwrap();

    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    attach(&mut pipe, spelled, Some("/state/dir")).unwrap();
    assert_eq!(
        pipe.line(soon()).unwrap(),
        b"args:attach --state-dir /state/dir"
    );
    pipe.send(b"carried\n", soon()).unwrap();
    assert_eq!(pipe.line(soon()).unwrap(), b"carried");

    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    attach(&mut pipe, spelled, None).unwrap();
    assert_eq!(pipe.line(soon()).unwrap(), b"args:attach");

    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    assert!(matches!(
        attach(&mut pipe, "/no/such/hided", None),
        Err(StepError::Refused(_))
    ));
    assert!(attach(&mut pipe, "hided", None).is_err());
}

/// A command that writes past the cap is refused while it writes, not
/// after: one that never stops writing ends, the connection stays in step,
/// and nothing is left in the temporary folder.
#[test]
fn a_command_writing_past_its_cap_is_refused_while_it_writes() {
    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    for endless in [&["yes"][..], &["sh", "-c", "yes >&2"]] {
        assert!(matches!(
            run(&mut pipe, &words(endless), 1024, soon()),
            Err(StepError::Refused(_))
        ));
    }
    assert_eq!(
        run(&mut pipe, &words(&["true"]), 16, soon())
            .unwrap()
            .status,
        0
    );
    // The script removes a run's files after it answers; the next answer
    // comes after that removal.
    let missing = device.home().join("missing");
    assert!(matches!(
        get_file(
            &mut pipe,
            missing.to_str().unwrap(),
            &mut Vec::new(),
            &|_| {}
        ),
        Err(StepError::Refused(_))
    ));
    assert!(device.tmp_is_empty());
}

/// A run the caller gave up on leaves nothing in the temporary folder once
/// the script ends.
#[test]
fn a_run_given_up_leaves_no_temporary_file() {
    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    let given_up = run(
        &mut pipe,
        &words(&["sh", "-c", "sleep 1; echo late"]),
        1024,
        Instant::now() + Duration::from_millis(200),
    );
    assert!(matches!(given_up, Err(StepError::Lost(_))));
    assert!(!device.tmp_is_empty());
    drop(pipe);
    let deadline = soon();
    while !device.tmp_is_empty() {
        assert!(Instant::now() < deadline, "the run's files outlived it");
        std::thread::yield_now();
    }
}
