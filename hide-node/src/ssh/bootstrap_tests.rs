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
        device
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
            .env_remove(hide_platform::host::MACHINE_ID_VARIABLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
        self.pid.set(child.id());
        UnaskedStart::new(ChildLink::start("device:test", child).unwrap())
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
    assert_eq!(facts.machine_id.as_deref(), Some("ab-12"));
    assert_eq!(
        (facts.os.as_str(), facts.arch.as_str()),
        ("linux", "aarch64")
    );
    let none = read_probe(&fields("probe\talice\t/home/alice\tLinux\tx86_64\t-\tyes")).unwrap();
    assert_eq!(none.machine_id, None);
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
