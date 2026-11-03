//! The PowerShell start script against a real `powershell.exe` on this
//! machine, through the same child link a device connection uses: the device
//! side is the script itself, started as Windows OpenSSH starts the remote
//! command in each login shell it may run (`<shell> -c <command>`, `/c` for
//! `cmd.exe`), so what it answers is what a Windows device answers.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use super::*;

/// A Windows sshd's `DefaultShell`.
#[derive(Clone, Copy, Debug)]
enum Shell {
    /// Its default.
    Cmd,
    /// Windows PowerShell 5.1.
    WindowsPowerShell,
    /// PowerShell 7.
    Pwsh,
    /// Git for Windows' bash, an MSYS shell.
    GitBash,
}

const SHELLS: [Shell; 4] = [
    Shell::Cmd,
    Shell::WindowsPowerShell,
    Shell::Pwsh,
    Shell::GitBash,
];

fn system32(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot names Windows' folder"))
        .join("System32")
        .join(name)
}

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

    fn wire(path: &Path) -> String {
        hide_platform::path::to_wire(path).unwrap()
    }

    fn connect(&self) -> UnaskedStart {
        self.connect_through(Shell::Cmd)
    }

    /// One connection: `shell` running the remote command in the account's
    /// home, as Windows OpenSSH starts it.
    fn connect_through(&self, shell: Shell) -> UnaskedStart {
        let command_line = remote_command();
        let mut command = match shell {
            Shell::Cmd => {
                let mut command = std::process::Command::new(system32("cmd.exe"));
                command.raw_arg("/c").raw_arg(&command_line);
                command
            }
            Shell::WindowsPowerShell | Shell::Pwsh => {
                let program = match shell {
                    Shell::Pwsh => PathBuf::from("pwsh.exe"),
                    _ => system32(r"WindowsPowerShell\v1.0\powershell.exe"),
                };
                let mut command = std::process::Command::new(program);
                command.raw_arg("-c").raw_arg(&command_line);
                command
            }
            Shell::GitBash => {
                let program_files = std::env::var_os("ProgramFiles").expect("ProgramFiles is set");
                let mut command = std::process::Command::new(
                    PathBuf::from(program_files).join(r"Git\bin\bash.exe"),
                );
                command.arg("-c").arg(&command_line);
                command
            }
        };
        command
            .current_dir(self.home())
            .env("USERPROFILE", self.home())
            .env_remove(hide_platform::host::MACHINE_ID_VARIABLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command)
            .unwrap_or_else(|error| panic!("{shell:?} did not start: {error}"));
        self.pid.set(child.id());
        UnaskedStart::new(
            ChildLink::start("device:test", child, None).unwrap(),
            hide_node_link::device::Cancel::default(),
        )
    }

    /// A Windows build whose files say what they are.
    fn packages(&self) -> HelperPackages {
        let builds = self.dir.path().join("builds");
        std::fs::create_dir_all(&builds).unwrap();
        for (name, bytes) in [
            ("hided", "helper build 1"),
            ("hide", "command build 1"),
            ("hide-agent-hooks", "hooks build 1"),
        ] {
            std::fs::write(builds.join(format!("{name}-windows-x86_64")), bytes).unwrap();
        }
        HelperPackages::new(Some(builds))
    }

    /// A batch file the steps can start in place of a program: it says what
    /// it was started with and where, then repeats one line of its input.
    fn echoing_program(&self, path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            "@echo off\r\necho args:%*\r\necho cwd:%CD%\r\nset /p line=\r\necho carried:%line%\r\n",
        )
        .unwrap();
    }
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A line of the program the link was handed to, without its line end.
fn line(pipe: &mut UnaskedStart) -> String {
    let line = pipe.line(soon()).unwrap();
    String::from_utf8(line)
        .unwrap()
        .trim_end_matches('\r')
        .to_owned()
}

fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// Every login shell a Windows sshd may run reaches the PowerShell script
/// with one connection: `cmd.exe` through the `&` its single quotes do not
/// hide, both PowerShells by the statement after the `exec` they do not
/// have, and Git Bash by the `sh` script's hand-over. The probe names the
/// account, its home in the wire spelling and the id this machine's own
/// node reads, and the script then answers its steps.
#[test]
fn every_login_shell_of_a_windows_sshd_reaches_the_powershell_script() {
    let machine_id = hide_platform::host::machine_id().unwrap();
    for shell in SHELLS {
        let device = Device::new();
        let mut pipe = device.connect_through(shell);
        let facts = probe(&mut pipe, soon()).unwrap_or_else(|error| panic!("{shell:?}: {error:?}"));
        assert_eq!(facts.home, Device::wire(&device.home()), "{shell:?}");
        assert_eq!(
            (facts.os.as_str(), facts.arch.as_str()),
            ("windows", std::env::consts::ARCH),
            "{shell:?}"
        );
        assert_eq!(facts.machine_id, machine_id, "{shell:?}");
        assert_eq!(facts.user, std::env::var("USERNAME").unwrap(), "{shell:?}");
        let ran = run(
            &mut pipe,
            &words(&["cmd.exe", "/c", "echo ready"]),
            1024,
            soon(),
        )
        .unwrap_or_else(|error| panic!("{shell:?}: {error}"));
        assert_eq!(
            (ran.status, ran.stdout.as_slice()),
            (0, &b"ready\r\n"[..]),
            "{shell:?}"
        );
    }
}

/// B23: the install sends each file once into a folder only the account can
/// use, reuses what the device holds whole, and sends again only what is
/// missing or damaged.
#[test]
fn an_install_sends_only_what_the_device_does_not_hold_whole() {
    let device = Device::new();
    let packages = device.packages();
    let install_once = || {
        let mut pipe = device.connect();
        let facts = probe(&mut pipe, soon()).unwrap();
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        install(&mut pipe, "~/.hide/helper", &facts.home, &payload).unwrap()
    };

    let first = install_once();
    assert!(first.fresh);
    assert_eq!((first.upload.sent, first.upload.reused), (3, 0));
    assert!(
        first.helper_path.ends_with("/hided.exe"),
        "{}",
        first.helper_path
    );
    let helper = hide_platform::path::from_wire(&first.helper_path).unwrap();
    assert_eq!(std::fs::read(&helper).unwrap(), b"helper build 1");
    let root = device.home().join(r".hide\helper");
    assert!(
        hide_platform::fs::private::is_private(&root).unwrap(),
        "the install folder is the account's alone"
    );
    assert!(hide_platform::fs::private::is_private(&helper).unwrap());

    let again = install_once();
    assert!(!again.fresh);
    assert_eq!((again.upload.sent, again.upload.reused), (0, 3));

    let folder = helper.parent().unwrap().to_owned();
    std::fs::write(folder.join("hide.exe"), b"damaged").unwrap();
    let repaired = install_once();
    assert_eq!((repaired.upload.sent, repaired.upload.reused), (1, 2));
    assert_eq!(
        std::fs::read(folder.join("hide.exe")).unwrap(),
        b"command build 1"
    );
}

/// B23: a connection that ends part way through a file leaves the final
/// name as it was and no upload behind it.
#[test]
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn an_install_cut_off_mid_file_leaves_no_half_file_in_use() {
    let device = Device::new();
    let mut pipe = device.connect();
    let facts = probe(&mut pipe, soon()).unwrap();
    let root = format!("{}/.hide/helper", facts.home);
    let [kind, _] = &step(&mut pipe, &format!("root\t{root}\n"), soon()).unwrap()[..] else {
        panic!("the root step did not answer");
    };
    assert_eq!(kind, "root");
    pipe.send(b"put\tv1/hided.exe\t00\t700\t100\nonly ten b", soon())
        .unwrap();
    drop(pipe);
    let deadline = Instant::now() + Duration::from_secs(20);
    while hide_platform::process::is_alive(device.pid.get()) {
        assert!(
            Instant::now() < deadline,
            "the script outlived its connection"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let folder = device.home().join(r".hide\helper\v1");
    assert!(
        !folder.join("hided.exe").exists(),
        "a half file took the final name"
    );
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        0,
        "the upload was left behind"
    );
}

/// The install folder and every folder above it must be ones no other
/// account can change, and real folders rather than links.
#[test]
fn an_install_under_a_folder_others_can_change_or_a_link_is_refused() {
    let device = Device::new();
    let open = device.home().join("open");
    std::fs::create_dir(&open).unwrap();
    // Everyone may change it and what it holds.
    let granted = std::process::Command::new(system32("icacls.exe"))
        .arg(&open)
        .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
        .output()
        .unwrap();
    assert!(granted.status.success(), "{granted:?}");
    let real = device.home().join("real");
    std::fs::create_dir(&real).unwrap();
    let linked = std::process::Command::new(system32("cmd.exe"))
        .args(["/c", "mklink", "/J"])
        .arg(device.home().join("link"))
        .arg(&real)
        .output()
        .unwrap();
    assert!(linked.status.success(), "{linked:?}");

    let packages = device.packages();
    for (root, refusal) in [
        ("~/open/helper", "can be changed by other accounts"),
        ("~/link/helper", "is a link"),
    ] {
        let mut pipe = device.connect();
        let facts = probe(&mut pipe, soon()).unwrap();
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        let Err(EstablishError::Install(reason)) = install(&mut pipe, root, &facts.home, &payload)
        else {
            panic!("an install under {root} was accepted");
        };
        assert!(reason.contains(refusal), "{root}: {reason}");
    }
    assert_eq!(std::fs::read_dir(open.join("helper")).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(real.join("helper")).unwrap().count(), 0);
}

/// B24: the helper starts on the same pipes, from the account's home, and a
/// Windows device takes no Herdr session or socket.
#[test]
fn the_helper_starts_on_the_same_pipes_and_takes_no_herdr() {
    let device = Device::new();
    let script = device.dir.path().join("helper.cmd");
    device.echoing_program(&script);
    let bytes = std::fs::read(&script).unwrap();
    let mut pipe = device.connect();
    let facts = probe(&mut pipe, soon()).unwrap();
    let [kind, _] = &step(
        &mut pipe,
        &format!("root\t{}/.hide/helper\n", facts.home),
        soon(),
    )
    .unwrap()[..] else {
        panic!("the root step did not answer");
    };
    assert_eq!(kind, "root");
    pipe.send(
        format!(
            "put\tv1/helper.cmd\t{}\t700\t{}\n",
            digest(&bytes),
            bytes.len()
        )
        .as_bytes(),
        soon(),
    )
    .unwrap();
    pipe.send(&bytes, soon()).unwrap();
    assert_eq!(read_step(&mut pipe, soon()).unwrap(), ["ok"]);
    assert!(
        start(
            &mut pipe,
            "v1/helper.cmd",
            &HerdrChoice::Session("work".to_owned())
        )
        .is_err()
    );

    let mut pipe = device.connect();
    let facts = probe(&mut pipe, soon()).unwrap();
    let mut asked = format!("root\t{}/.hide/helper\n", facts.home);
    asked.push_str(&format!("held\t1\nv1/helper.cmd\t{}\n", digest(&bytes)));
    pipe.send(asked.as_bytes(), soon()).unwrap();
    assert_eq!(read_step(&mut pipe, soon()).unwrap()[0], "root");
    assert_eq!(
        read_step(&mut pipe, soon()).unwrap(),
        ["held", "v1/helper.cmd"]
    );
    assert_eq!(read_step(&mut pipe, soon()).unwrap(), ["end"]);
    start(&mut pipe, "v1/helper.cmd", &HerdrChoice::Account).unwrap();
    assert_eq!(line(&mut pipe), "args:node serve --own-herdr");
    assert_eq!(
        line(&mut pipe).to_lowercase(),
        format!("cwd:{}", device.home().display()).to_lowercase()
    );
    pipe.send(b"through the link\r\n", soon()).unwrap();
    assert_eq!(line(&mut pipe), "carried:through the link");
}

/// A command answers its status and both outputs, its words arrive whole,
/// one that writes past the cap is ended and refused with the connection
/// still in step, and a program that does not exist answers 127.
#[test]
fn a_command_answers_its_status_and_output_and_the_next_step_follows() {
    let device = Device::new();
    let mut pipe = device.connect();
    probe(&mut pipe, soon()).unwrap();
    let ran = run(
        &mut pipe,
        &words(&["cmd.exe", "/c", "echo out&(echo err)1>&2&exit /b 3"]),
        1024,
        soon(),
    )
    .unwrap();
    assert_eq!(
        (ran.status, ran.stdout.as_slice(), ran.stderr.as_slice()),
        (3, &b"out\r\n"[..], &b"err\r\n"[..])
    );
    let echo = device.home().join("words.ps1");
    std::fs::write(&echo, "[Console]::Out.Write(($args -join '|'))").unwrap();
    let ran = run(
        &mut pipe,
        &words(&[
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-File",
            echo.to_str().unwrap(),
            "a b",
            "c\"d",
            "$HOME",
            "*",
            r"tail\",
        ]),
        1024,
        soon(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(ran.stdout).unwrap(),
        "a b|c\"d|$HOME|*|tail\\"
    );
    assert!(matches!(
        run(
            &mut pipe,
            &words(&["cmd.exe", "/c", "for /l %i in () do @echo y"]),
            1024,
            soon()
        ),
        Err(StepError::Refused(_))
    ));
    assert_eq!(
        run(&mut pipe, &words(&["hide-no-such-program"]), 1024, soon())
            .unwrap()
            .status,
        127
    );
    assert_eq!(
        run(&mut pipe, &words(&["cmd.exe", "/c", "exit 0"]), 16, soon())
            .unwrap()
            .status,
        0
    );
}

/// A copied file lands whole and private, a damaged one leaves the file
/// already there alone, and the same file reads back with the digest the
/// machine states for it.
#[test]
fn a_file_goes_whole_into_a_private_folder_and_comes_back_whole() {
    let device = Device::new();
    let mut pipe = device.connect();
    probe(&mut pipe, soon()).unwrap();
    let bytes: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
    let remote = device.home().join(r"moved\deep\file");
    let spelled = Device::wire(&remote);
    put_file(
        &mut pipe,
        &spelled,
        &digest(&bytes),
        bytes.len() as u64,
        &mut bytes.as_slice(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(std::fs::read(&remote).unwrap(), bytes);
    assert!(hide_platform::fs::private::is_private(&device.home().join("moved")).unwrap());
    assert!(hide_platform::fs::private::is_private(&remote).unwrap());

    assert!(matches!(
        put_file(
            &mut pipe,
            &spelled,
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
    let (size, stated) = get_file(&mut pipe, &spelled, &mut back, &|_| {}).unwrap();
    assert_eq!((size, stated), (bytes.len() as u64, digest(&bytes)));
    assert_eq!(back, bytes);
    assert!(matches!(
        get_file(&mut pipe, "C:/no/such/file", &mut Vec::new(), &|_| {}),
        Err(StepError::Refused(_))
    ));
    assert_eq!(
        run(&mut pipe, &words(&["cmd.exe", "/c", "exit 0"]), 16, soon())
            .unwrap()
            .status,
        0
    );
}

/// The attach step hands the connection's pipes to the core's program, with
/// its state folder in this system's spelling when one is named.
#[test]
fn attach_hands_the_pipes_to_the_core_program() {
    let device = Device::new();
    let program = device.dir.path().join("hided.cmd");
    device.echoing_program(&program);
    let state = device.dir.path().join("state");

    let mut pipe = device.connect();
    probe(&mut pipe, soon()).unwrap();
    attach(
        &mut pipe,
        &Device::wire(&program),
        Some(&Device::wire(&state)),
    )
    .unwrap();
    assert_eq!(
        line(&mut pipe),
        format!("args:attach --state-dir {}", state.display())
    );

    let mut pipe = device.connect();
    probe(&mut pipe, soon()).unwrap();
    assert!(matches!(
        attach(&mut pipe, "C:/no/such/hided.exe", None),
        Err(StepError::Refused(_))
    ));
}
