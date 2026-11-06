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
    /// When the latest connection started.
    connected: std::cell::Cell<Option<Instant>>,
}

impl Device {
    fn new() -> Self {
        let device = Self {
            dir: tempfile::tempdir().unwrap(),
            pid: std::cell::Cell::new(0),
            connected: std::cell::Cell::new(None),
        };
        std::fs::create_dir(device.home()).unwrap();
        device
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// A home of the node's own (`HIDE_DEV_DEVICE_HOME`).
    fn own_home(&self) -> PathBuf {
        self.home().join(r"candidate\home")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
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
            // What the echoing program compares its folders with, since a
            // path it printed would be in the console's code page.
            .env("HIDE_TEST_HOME", self.home())
            .env("HIDE_TEST_OWN_HOME", self.own_home())
            .env("HIDE_TEST_STATE", self.state())
            // Folders of the account's own named apart from its home, which
            // a home of the node's own replaces with its defaults under it
            // or, for an agent's home, its folder there.
            .envs(
                HOME_FOLDER_VARIABLES
                    .iter()
                    .copied()
                    .chain(agent_home_variables().into_iter().map(|(name, _)| name))
                    .map(|name| (name, self.home().join("account").join(name))),
            )
            .env_remove(hide_platform::host::MACHINE_ID_VARIABLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command)
            .unwrap_or_else(|error| panic!("{shell:?} did not start: {error}"));
        self.pid.set(child.id());
        self.connected.set(Some(Instant::now()));
        UnaskedStart::new(
            ChildLink::start("device:test", child, None).unwrap(),
            hide_node_link::device::Cancel::default(),
        )
    }

    /// The latest connection's probe, which fails the test with what the
    /// connection's processes were doing when it found no answer in time.
    fn probe(&self, pipe: &mut UnaskedStart) -> Facts {
        probe(pipe, soon()).unwrap_or_else(|error| {
            panic!(
                "{error:?}; it last said {:?}; {}",
                pipe.last_error(),
                self.stalled()
            )
        })
    }

    /// Each process of the latest connection, the login shell first, with
    /// the processor time it used against the time the connection ran, so a
    /// PowerShell building its module analysis cache (`POWERSHELL_WRAPPER`)
    /// reads apart from one that waits, and the cache path it was given.
    fn stalled(&self) -> String {
        let shell = self.pid.get();
        let ran = self.connected.get().map(|at| at.elapsed());
        let mut pids = vec![shell];
        pids.extend(hide_platform::process::descendants(shell).unwrap_or_default());
        let processes: Vec<String> = pids
            .into_iter()
            .map(|pid| {
                let name = hide_platform::process::name_of(pid)
                    .unwrap_or_else(|error| format!("unnamed ({error})"));
                let worked = hide_platform::process::cpu_time(pid).map_or_else(
                    |error| format!("unknown ({error})"),
                    |time| format!("{time:.1?}"),
                );
                format!("{name} {pid} used {worked} of processor")
            })
            .collect();
        format!(
            "{} in {ran:.1?} since the connection started; PSModuleAnalysisCachePath={:?}",
            processes.join(", "),
            std::env::var_os("PSModuleAnalysisCachePath")
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
        HelperPackages::new(Some(builds), UNPUBLISHED)
    }

    /// A batch file the steps can start in place of a program: it says what
    /// it was started with, in which of the device's folders (`home`, `own`
    /// or `other`), whether its per-user folders all follow the own home,
    /// which folder variables it was started with, whether every agent's
    /// home is its folder under the own home, then repeats one line of its
    /// input.
    fn echoing_program(&self, path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let moved = HOME_FOLDER_VARIABLES
            .iter()
            .map(|name| format!("if defined {name} set moved=%moved% {name}"));
        let agents = agent_home_variables().into_iter().map(|(name, folder)| {
            format!(
                "if /i not \"%{name}%\"==\"%HIDE_TEST_OWN_HOME%\\{}\" set agents=other",
                folder.replace('/', "\\")
            )
        });
        std::fs::write(
            path,
            [
                "@echo off",
                "echo args:%1 %2",
                "if /i \"%~3\"==\"%HIDE_TEST_STATE%\" (echo third:state) else (echo third:%~3)",
                "set where=other",
                "if /i \"%CD%\"==\"%HIDE_TEST_HOME%\" set where=home",
                "if /i \"%CD%\"==\"%HIDE_TEST_OWN_HOME%\" set where=own",
                "echo cwd:%where%",
                "set where=other",
                "if /i \"%USERPROFILE%\"==\"%HIDE_TEST_HOME%\" set where=home",
                "if /i \"%USERPROFILE%\"==\"%HIDE_TEST_OWN_HOME%\" set where=own",
                "echo home:%where%",
                "set folders=other",
                "if /i \"%HOME%\"==\"%HIDE_TEST_OWN_HOME%\" if /i \"%APPDATA%\"==\"%HIDE_TEST_OWN_HOME%\\AppData\\Roaming\" if /i \"%LOCALAPPDATA%\"==\"%HIDE_TEST_OWN_HOME%\\AppData\\Local\" if /i \"%TEMP%\"==\"%HIDE_TEST_OWN_HOME%\\AppData\\Local\\Temp\" if /i \"%TMP%\"==\"%TEMP%\" set folders=own",
                "echo folders:%folders%",
                "set moved=",
            ]
            .into_iter()
            .map(str::to_owned)
            .chain(moved)
            .chain(["echo moved:%moved%", "set agents=own"].map(str::to_owned))
            .chain(agents)
            .chain(
                ["echo agents:%agents%", "set /p line=", "echo carried:%line%", ""]
                    .map(str::to_owned),
            )
            .collect::<Vec<_>>()
            .join("\r\n"),
        )
        .unwrap();
    }
}

/// A step's deadline: the one the node gives a device to sign in and answer
/// its probe, since a PowerShell login shell that starts Windows PowerShell
/// takes seconds of its own on a busy machine.
fn soon() -> Instant {
    Instant::now() + crate::ssh::session::SIGN_IN_TIMEOUT
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

/// Whether the process `pid` is gone within a short bound. A process its job
/// has ended still counts as alive while Windows closes the handles it held
/// (`hide_platform::process::is_alive`), a moment after the job counts no
/// active process; one that left the job runs on past the bound.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn gone_soon(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while hide_platform::process::is_alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn words(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The wrapper in Windows PowerShell with its own module analysis cache at
/// `cache`, handed `script` (a byte count line and the text) and then the end
/// of its input, so the script ends by itself and PowerShell writes any cache
/// it built as it exits.
fn wrapper_reads(script: &str, cache: &Path) -> hide_platform::process::CapturedOutput {
    use std::io::Write;
    let mut command =
        std::process::Command::new(system32(r"WindowsPowerShell\v1.0\powershell.exe"));
    command
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(powershell_wrapper())
        .env("PSModuleAnalysisCachePath", cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let started = Instant::now();
    let mut input = child.take_stdin().unwrap();
    input.write_all(script.as_bytes()).unwrap();
    drop(input);
    let pid = child.id();
    child
        .capture_until(soon(), 64 * 1024)
        .unwrap_or_else(|error| {
            panic!(
                "the wrapper did not end: {error:?}; pid {pid} used {:?} of processor in {:?}",
                hide_platform::process::cpu_time(pid),
                started.elapsed()
            )
        })
}

/// The whole start script runs under the wrapper with a module analysis
/// cache PowerShell has never filled, and PowerShell writes none: it found
/// every cmdlet the wrapper and the script use in the one module the wrapper
/// imports and searched the module path for none, the search that took a
/// fresh runner 12 to 38 s (with-hide/hide-private#580).
#[test]
fn the_start_script_runs_under_an_empty_module_cache_without_building_one() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("ModuleAnalysisCache");
    let output = wrapper_reads(&powershell_script(), &cache);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("HIDE1\tscript\tpowershell") && stdout.contains("HIDE1\tprobe\t"),
        "{stdout}\n{stderr}"
    );
    assert!(
        !cache.exists(),
        "PowerShell searched the module path and built a module analysis cache"
    );
}

/// A cmdlet outside the module the wrapper imports fails at once by name and
/// builds no module analysis cache: with auto-loading on, PowerShell would
/// search the whole module path for it instead, silently.
#[test]
fn a_cmdlet_outside_the_wrappers_import_fails_by_name_and_starts_no_search() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("ModuleAnalysisCache");
    let text = "$null = Get-Item -LiteralPath $env:SystemRoot";
    let output = wrapper_reads(&format!("{}\n{text}", text.len()), &cache);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Get-Item") && stderr.contains("is not recognized"),
        "{:?}: {stderr}",
        output.status
    );
    assert!(
        !cache.exists(),
        "PowerShell searched the module path and built a module analysis cache"
    );
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
        let facts = device.probe(&mut pipe);
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
        let facts = device.probe(&mut pipe);
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
    let facts = device.probe(&mut pipe);
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
        let facts = device.probe(&mut pipe);
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

/// Every folder below the install folder must be a real folder of the
/// account's that no other account can change, as the install folder is: a
/// junction there would take the files somewhere else, and a folder another
/// account can change would let it swap what runs. Neither is used or
/// written through, and the folder outside stays as it was.
#[test]
fn a_link_or_an_open_folder_below_the_install_folder_is_refused() {
    let device = Device::new();
    let packages = device.packages();
    let install_once = || {
        let mut pipe = device.connect();
        let facts = device.probe(&mut pipe);
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        install(&mut pipe, "~/.hide/helper", &facts.home, &payload)
    };
    let first = install_once().unwrap();
    let (version, _) = first.helper_relative.split_once('/').unwrap();
    let version = device.home().join(r".hide\helper").join(version);
    // A copy of the build elsewhere, beside a file of someone else's.
    let outside = device.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    for entry in std::fs::read_dir(&version).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), outside.join(entry.file_name())).unwrap();
    }
    std::fs::write(outside.join("sentinel"), b"untouched").unwrap();
    let contents = || {
        let mut files: Vec<(std::ffi::OsString, Vec<u8>)> = std::fs::read_dir(&outside)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    };
    let before = contents();

    std::fs::remove_dir_all(&version).unwrap();
    let linked = std::process::Command::new(system32("cmd.exe"))
        .args(["/c", "mklink", "/J"])
        .arg(&version)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(linked.status.success(), "{linked:?}");
    let Err(EstablishError::Install(reason)) = install_once() else {
        panic!("an install through a junction below the install folder was accepted");
    };
    assert!(reason.contains("is a link"), "{reason}");
    assert_eq!(contents(), before);

    std::fs::remove_dir(&version).unwrap();
    std::fs::create_dir(&version).unwrap();
    // Everyone may change it and what it holds.
    let granted = std::process::Command::new(system32("icacls.exe"))
        .arg(&version)
        .args(["/grant", "*S-1-1-0:(OI)(CI)M"])
        .output()
        .unwrap();
    assert!(granted.status.success(), "{granted:?}");
    let Err(EstablishError::Install(reason)) = install_once() else {
        panic!("an install into a folder others can change was accepted");
    };
    assert!(reason.contains("can be changed"), "{reason}");
    assert_eq!(std::fs::read_dir(&version).unwrap().count(), 0);
}

/// B24: the helper starts on the same pipes, from the account's home.
#[test]
fn the_helper_starts_on_the_same_pipes() {
    let device = Device::new();
    let script = device.dir.path().join("helper.cmd");
    device.echoing_program(&script);
    let bytes = std::fs::read(&script).unwrap();
    let mut pipe = device.connect();
    let facts = device.probe(&mut pipe);
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

    let mut pipe = device.connect();
    let facts = device.probe(&mut pipe);
    let mut asked = format!("root\t{}/.hide/helper\n", facts.home);
    asked.push_str(&format!("held\t1\nv1/helper.cmd\t{}\n", digest(&bytes)));
    pipe.send(asked.as_bytes(), soon()).unwrap();
    assert_eq!(read_step(&mut pipe, soon()).unwrap()[0], "root");
    assert_eq!(
        read_step(&mut pipe, soon()).unwrap(),
        ["held", "v1/helper.cmd"]
    );
    assert_eq!(read_step(&mut pipe, soon()).unwrap(), ["end"]);
    start(&mut pipe, "v1/helper.cmd", None).unwrap();
    assert_eq!(line(&mut pipe), "args:node serve");
    assert_eq!(line(&mut pipe), "third:");
    assert_eq!(line(&mut pipe), "cwd:home");
    assert_eq!(line(&mut pipe), "home:home");
    assert_eq!(line(&mut pipe), "folders:other");
    let all: String = HOME_FOLDER_VARIABLES
        .iter()
        .map(|name| format!(" {name}"))
        .collect();
    assert_eq!(line(&mut pipe), format!("moved:{all}"));
    assert_eq!(line(&mut pipe), "agents:other");
    pipe.send(b"through the link\r\n", soon()).unwrap();
    assert_eq!(line(&mut pipe), "carried:through the link");

    // A home of the node's own (`HIDE_DEV_DEVICE_HOME`) takes the account's
    // place, made private where it was missing, and so do the per-user
    // folders beside it, and none of the account's folders named apart from
    // its home reaches the helper, and each agent's home is its folder under
    // the own home, made private, so neither the helper's kit and state nor
    // a program in one of its panes (Codex, which reads the account's profile
    // folder from Windows) writes the account's own.
    let own = device.own_home();
    let mut pipe = device.connect();
    let facts = device.probe(&mut pipe);
    let mut asked = format!("root\t{}/.hide/helper\n", facts.home);
    asked.push_str(&format!("held\t1\nv1/helper.cmd\t{}\n", digest(&bytes)));
    pipe.send(asked.as_bytes(), soon()).unwrap();
    for _ in 0..3 {
        read_step(&mut pipe, soon()).unwrap();
    }
    start(&mut pipe, "v1/helper.cmd", Some(&Device::wire(&own))).unwrap();
    line(&mut pipe);
    line(&mut pipe);
    assert_eq!(line(&mut pipe), "cwd:own");
    assert_eq!(line(&mut pipe), "home:own");
    assert_eq!(line(&mut pipe), "folders:own");
    assert_eq!(line(&mut pipe), "moved:");
    assert_eq!(line(&mut pipe), "agents:own");
    for folder in [
        own.clone(),
        own.join(r"AppData\Roaming"),
        own.join(r"AppData\Local\Temp"),
    ]
    .into_iter()
    .chain(
        agent_home_variables()
            .into_iter()
            .map(|(_, folder)| own.join(folder.replace('/', "\\"))),
    ) {
        assert!(
            hide_platform::fs::private::is_private(&folder).unwrap(),
            "{}",
            folder.display()
        );
    }
}

/// A command answers its status and both outputs, its words arrive whole,
/// one that writes past the cap is ended and refused with the connection
/// still in step, and a program that does not exist answers 127.
#[test]
fn a_command_answers_its_status_and_output_and_the_next_step_follows() {
    let device = Device::new();
    let mut pipe = device.connect();
    device.probe(&mut pipe);
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
            "-ExecutionPolicy",
            "Bypass",
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

/// A command that exits while a program it started keeps its outputs and
/// writes past the cap is ended with that program too, so nothing it
/// started outlives the refusal, and the connection stays in step.
#[test]
fn a_command_past_its_cap_is_ended_with_everything_it_started() {
    let device = Device::new();
    let writer = device.home().join("writer.ps1");
    std::fs::write(
        &writer,
        "Set-Content -Encoding ascii -LiteralPath $args[0] -Value $PID; while ($true) { [Console]::Out.Write('y' * 4096) }",
    )
    .unwrap();
    let pid_file = device.home().join("writer.pid");
    let powershell = system32(r"WindowsPowerShell\v1.0\powershell.exe");
    let mut pipe = device.connect();
    device.probe(&mut pipe);
    // `cmd` starts the writer in the background and ends at once; its words
    // travel in the command line rather than a batch file, which `cmd`
    // would read in the console's code page.
    assert!(matches!(
        run(
            &mut pipe,
            &words(&[
                "cmd.exe",
                "/c",
                "start",
                // A quoted first word is the window title `start` takes.
                "hide writer",
                "/b",
                powershell.to_str().unwrap(),
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                writer.to_str().unwrap(),
                pid_file.to_str().unwrap(),
            ]),
            1024,
            soon()
        ),
        Err(StepError::Refused(_))
    ));
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        gone_soon(pid),
        "the program the command started outlived the refusal"
    );
    assert_eq!(
        run(&mut pipe, &words(&["cmd.exe", "/c", "exit 0"]), 16, soon())
            .unwrap()
            .status,
        0
    );
}

/// A command that starts an ordinary program with outputs of its own and
/// exits 0 answers its status, and that program ends with it: only a
/// resident that leaves the command's job outlives it.
#[test]
fn a_command_that_ends_leaves_nothing_it_started_running() {
    let device = Device::new();
    let launcher = device.home().join("launcher.ps1");
    // Start-Process without redirection starts it through the shell, so it
    // shares no handle of the command's, only its job.
    std::fs::write(
        &launcher,
        format!(
            "$p = Start-Process -PassThru -WindowStyle Hidden -FilePath '{}' -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 600'\r\nSet-Content -Encoding ascii -LiteralPath $args[0] -Value $p.Id\r\nexit 0\r\n",
            system32(r"WindowsPowerShell\v1.0\powershell.exe").display()
        ),
    )
    .unwrap();
    let pid_file = device.home().join("sleeper.pid");
    let mut pipe = device.connect();
    device.probe(&mut pipe);
    let ran = run(
        &mut pipe,
        &words(&[
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            launcher.to_str().unwrap(),
            pid_file.to_str().unwrap(),
        ]),
        1024,
        soon(),
    )
    .unwrap();
    assert_eq!(ran.status, 0, "{}", String::from_utf8_lossy(&ran.stderr));
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        gone_soon(pid),
        "the program the command started outlived it"
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
    device.probe(&mut pipe);
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
    let state = device.state();

    let mut pipe = device.connect();
    device.probe(&mut pipe);
    attach(
        &mut pipe,
        &Device::wire(&program),
        Some(&Device::wire(&state)),
    )
    .unwrap();
    assert_eq!(line(&mut pipe), "args:attach --state-dir");
    assert_eq!(line(&mut pipe), "third:state");

    let mut pipe = device.connect();
    device.probe(&mut pipe);
    assert!(matches!(
        attach(&mut pipe, "C:/no/such/hided.exe", None),
        Err(StepError::Refused(_))
    ));
}
