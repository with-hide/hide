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

    /// One connection: `sh` running the remote command in the account's
    /// home, as `sshd` runs it in the login shell; `greeting` is what a login
    /// file prints before it.
    fn connect(&self, greeting: &str) -> UnaskedStart {
        self.connect_through(Path::new("sh"), greeting, &[])
    }

    /// [`Device::connect`] with `shell` as the account's login shell, and
    /// `path` before the `PATH` it finds programs on.
    fn connect_through(&self, shell: &Path, greeting: &str, path: &[&Path]) -> UnaskedStart {
        let mut command = std::process::Command::new(shell);
        let mut search: Vec<PathBuf> = path.iter().map(|folder| folder.to_path_buf()).collect();
        search.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command
            .args(["-c", &format!("{greeting}{}", remote_command())])
            .env("PATH", std::env::join_paths(search).unwrap())
            .current_dir(self.home())
            .env("HOME", self.home())
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
        HelperPackages::new(Some(builds), UNPUBLISHED)
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

/// The one remote command starts the script in every POSIX login shell an
/// account may have: each parses the whole line and replaces itself with
/// `sh` at its first statement, so the parts for `cmd.exe` and PowerShell
/// never run. The shells are named in `HIDE_TEST_*_BIN`, so this is ignored
/// by default; the rust lane's shell step runs it with `--ignored`.
#[test]
#[ignore = "needs zsh, bash, fish, tcsh and csh: set HIDE_TEST_ZSH_BIN, HIDE_TEST_BASH_BIN, HIDE_TEST_FISH_BIN, HIDE_TEST_TCSH_BIN and HIDE_TEST_CSH_BIN and run with --ignored"]
fn the_remote_command_starts_the_script_in_every_login_shell() {
    for variable in [
        "HIDE_TEST_ZSH_BIN",
        "HIDE_TEST_BASH_BIN",
        "HIDE_TEST_FISH_BIN",
        "HIDE_TEST_TCSH_BIN",
        "HIDE_TEST_CSH_BIN",
    ] {
        let shell = PathBuf::from(
            std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} names the shell")),
        );
        let device = Device::new();
        let mut pipe = device.connect_through(&shell, "", &[]);
        let facts =
            probe(&mut pipe, soon()).unwrap_or_else(|error| panic!("{variable}: {error:?}"));
        assert_eq!(Path::new(&facts.home), device.home(), "{variable}");
        assert_eq!(facts.os, std::env::consts::OS, "{variable}");
    }
}

/// An MSYS or Cygwin `sh` on Windows (Git Bash as the account's shell) hands
/// the link to the PowerShell wrapper at the script's first statement; the
/// wrapper asks for its script, gets it once, and answers the probe as
/// Windows. Here `uname` says MINGW and `powershell.exe` is a stand-in that
/// records what it was started with and read.
#[test]
fn an_msys_sh_on_windows_hands_the_link_to_the_powershell_script() {
    let device = Device::new();
    let bin = device.dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let program = |name: &str, body: &str| {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    program("uname", "echo MINGW64_NT-10.0-26200\n");
    let record = device.dir.path().join("powershell");
    program(
        "powershell.exe",
        &format!(
            "printf '%s\\n' \"$@\" > '{record}.args'\n\
             printf 'HIDE1\\tscript\\tpowershell\\n'\n\
             IFS= read -r size\n\
             head -c \"$size\" > '{record}.script'\n\
             printf 'HIDE1\\tprobe\\t사용자\\tD:/계정/사용자\\tWindows\\tamd64\\t4C4C4544-0042\\tyes\\tgzip\\n'\n",
            record = record.display()
        ),
    );
    let mut pipe = device.connect_through(Path::new("sh"), "", &[&bin]);
    let facts = probe(&mut pipe, soon()).unwrap();
    assert_eq!(
        (
            facts.user.as_str(),
            facts.home.as_str(),
            facts.os.as_str(),
            facts.arch.as_str()
        ),
        ("사용자", "D:/계정/사용자", "windows", "x86_64")
    );
    assert_eq!(facts.machine_id, "4c4c4544-0042");
    let args = std::fs::read_to_string(record.with_extension("args")).unwrap();
    let wrapper = powershell_wrapper();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            wrapper.as_str()
        ]
    );
    // What follows the request is the PowerShell script, which the sh
    // script had not taken.
    assert_eq!(
        std::fs::read_to_string(record.with_extension("script")).unwrap(),
        powershell_text()
    );
}

/// The PowerShell wrapper rides inside every login shell's quotes at once,
/// so it holds no character one of them would read there: a quote of either
/// kind, `cmd.exe`'s operators, escape and variables, csh's history, fish's
/// escape, or a command substitution.
#[test]
fn the_powershell_wrapper_holds_nothing_a_login_shell_reads_inside_quotes() {
    let wrapper = powershell_wrapper();
    let read = [
        '\'', '"', '&', '|', '<', '>', '^', '%', '!', '\\', '`', '\n',
    ];
    assert!(
        !wrapper.contains(&read[..]),
        "{:?}",
        wrapper.matches(&read[..]).collect::<Vec<_>>()
    );
}

#[test]
fn a_probe_answer_names_only_a_machine_hide_runs_on() {
    let fields = |line: &str| line.split('\t').map(str::to_owned).collect::<Vec<_>>();
    let facts = read_probe(&fields(
        "probe\talice\t/home/alice\tLinux\taarch64\tAB-12\tyes\tgzip",
    ))
    .unwrap();
    assert_eq!(facts.machine_id, "ab-12");
    assert!(
        facts.unpacks,
        "a device that names gzip takes its files packed"
    );
    let unpacked = read_probe(&fields(
        "probe\talice\t/home/alice\tLinux\taarch64\tAB-12\tyes\t-",
    ))
    .unwrap();
    assert!(
        !unpacked.unpacks,
        "a device without gzip takes them as they are"
    );
    assert_eq!(
        (facts.os.as_str(), facts.arch.as_str()),
        ("linux", "aarch64")
    );
    // P1-5: a machine with no usable id is refused, never bound to nothing.
    for id in ["-", "", "a b", "ab\u{7}c", &"a".repeat(129)] {
        assert_eq!(
            read_probe(&fields(&format!(
                "probe\talice\t/home/alice\tLinux\tx86_64\t{id}\tyes\tgzip"
            ))),
            Err(super::NO_MACHINE_ID.to_owned()),
            "{id:?}"
        );
    }
    for refused in [
        "probe\talice\t/home/alice\tLinux\tx86_64\t-\tnone\tgzip",
        "probe\tAlice Smith\t/home/alice\tLinux\tx86_64\t-\tyes\tgzip",
        "probe\talice\trelative\tLinux\tx86_64\t-\tyes\tgzip",
        "probe\talice\t/home/alice\tFreeBSD\tx86_64\t-\tyes\tgzip",
        "probe\talice\t/home/alice\tLinux\triscv64\t-\tyes\tgzip",
        "fail\tsomething",
        // A Windows account's home is a drive path, and its name has none
        // of the characters Windows reserves in one.
        "probe\talice\t/home/alice\tWindows\tamd64\tAB-12\tyes\tgzip",
        "probe\ta\\b\tD:/profiles/a\tWindows\tamd64\tAB-12\tyes\tgzip",
        "probe\talice\tD:/profiles/alice\tLinux\tx86_64\tAB-12\tyes\tgzip",
    ] {
        assert!(read_probe(&fields(refused)).is_err(), "{refused}");
    }
    let windows = read_probe(&fields(
        "probe\tAlice Smith\tD:/profiles/Alice Smith\tWindows\tamd64\t5A1B2C3D-0000-4E5F\tyes\tgzip",
    ))
    .unwrap();
    assert_eq!(
        (
            windows.user.as_str(),
            windows.home.as_str(),
            windows.os.as_str(),
            windows.arch.as_str()
        ),
        (
            "Alice Smith",
            "D:/profiles/Alice Smith",
            "windows",
            "x86_64"
        )
    );
    assert_eq!(windows.machine_id, "5a1b2c3d-0000-4e5f");
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
        install(
            &mut pipe,
            "~/.hide/helper",
            &facts.home,
            &payload,
            facts.unpacks,
        )
        .unwrap()
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

/// with-hide/hide-private#582: a device that unpacks gzip is sent each
/// file packed, and it lands as the same bytes; one that does not is sent
/// the bytes as they are. Either way the install answers what crossed the
/// link.
#[test]
fn a_packed_install_crosses_the_link_smaller_and_lands_whole() {
    let helper = "helper build 1 ".repeat(64 * 1024);
    for unpacks in [true, false] {
        let device = Device::new();
        let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, &helper);
        let mut pipe = device.connect("");
        let facts = probe(&mut pipe, soon()).unwrap();
        assert!(facts.unpacks, "this machine's sh finds gzip");
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        let installed =
            install(&mut pipe, "~/.hide/helper", &facts.home, &payload, unpacks).unwrap();
        assert_eq!((installed.upload.sent, installed.upload.reused), (3, 0));
        assert_eq!(
            std::fs::read(&installed.helper_path).unwrap(),
            helper.as_bytes()
        );
        let upload = &installed.upload;
        let own = (helper.len() + "command build 1".len() + "hooks build 1".len()) as u64;
        assert_eq!(upload.bytes, own);
        if unpacks {
            assert!(upload.wire * 10 < upload.bytes, "{upload:?}");
        } else {
            assert_eq!(upload.wire, upload.bytes, "{upload:?}");
        }
    }
}

/// A packed file that does not unpack fails as one that did not arrive
/// whole, and the script still read all of it: the next step answers.
#[test]
fn a_packed_file_that_does_not_unpack_fails_and_the_link_stays_in_step() {
    let device = Device::new();
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let root = format!("{}/.hide/helper", facts.home);
    let [kind, _] = &step(&mut pipe, &format!("root\t{root}\n"), soon()).unwrap()[..] else {
        panic!("the root step did not answer");
    };
    assert_eq!(kind, "root");
    // Past any pipe buffer, so a reader that stopped early would leave
    // the rest on the link.
    let garbage = vec![b'x'; 4 * 1024 * 1024];
    pipe.send(
        format!("zput\tv1/hided\t00\t700\t{}\n", garbage.len()).as_bytes(),
        soon(),
    )
    .unwrap();
    pipe.send(&garbage, soon()).unwrap();
    assert_eq!(
        read_step(&mut pipe, soon()).unwrap(),
        words(&["fail", "v1/hided did not arrive whole"])
    );
    assert!(!device.home().join(".hide/helper/v1/hided").exists());
    let [kind, _] = &step(&mut pipe, &format!("root\t{root}\n"), soon()).unwrap()[..] else {
        panic!("the step after the broken file did not answer");
    };
    assert_eq!(kind, "root");
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
    let Err(EstablishError::Install(reason)) = install(
        &mut pipe,
        "~/open/helper",
        &facts.home,
        &payload,
        facts.unpacks,
    ) else {
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
    // It names each folder variable it was started with, and each agent's
    // home.
    let moved: String = HOME_FOLDER_VARIABLES
        .iter()
        .map(|name| format!("${{{name}+ {name}}}"))
        .collect();
    let agents: String = agent_home_variables()
        .iter()
        .map(|(name, _)| format!(" ${{{name}:-}}"))
        .collect();
    let helper = format!(
        "#!/bin/sh\necho \"args:$*\"\necho \"home:$HOME pwd:$(pwd -P)\"\necho \"moved:{moved}\"\necho \"agents:{agents}\"\n"
    );
    let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, &helper);
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let payload = packages.payload(&facts.os, &facts.arch).unwrap();
    let installed = install(
        &mut pipe,
        "~/.hide/helper",
        &facts.home,
        &payload,
        facts.unpacks,
    )
    .unwrap();
    start(&mut pipe, &installed.helper_relative, None).unwrap();
    assert_eq!(pipe.line(soon()).unwrap(), b"args:node serve");
    let home = std::fs::canonicalize(device.home()).unwrap();
    assert_eq!(
        String::from_utf8(pipe.line(soon()).unwrap()).unwrap(),
        format!("home:{} pwd:{}", facts.home, home.display())
    );
    let all: String = HOME_FOLDER_VARIABLES
        .iter()
        .map(|name| format!(" {name}"))
        .collect();
    assert_eq!(
        String::from_utf8(pipe.line(soon()).unwrap()).unwrap(),
        format!("moved:{all}")
    );
    let account: String = agent_home_variables()
        .iter()
        .map(|(name, _)| format!(" {}", device.home().join("account").join(name).display()))
        .collect();
    assert_eq!(
        String::from_utf8(pipe.line(soon()).unwrap()).unwrap(),
        format!("agents:{account}")
    );

    // A home of the node's own (`HIDE_DEV_DEVICE_HOME`) takes the account's
    // place, made private where it was missing, none of the account's
    // folders named apart from its home reaches the helper, so each one is
    // the default under the new home, and each agent's home is its folder
    // there, made private.
    let own = format!("{}/candidate/home", facts.home);
    let mut pipe = device.connect("");
    let facts = probe(&mut pipe, soon()).unwrap();
    let installed = install(
        &mut pipe,
        "~/.hide/helper",
        &facts.home,
        &payload,
        facts.unpacks,
    )
    .unwrap();
    start(&mut pipe, &installed.helper_relative, Some(&own)).unwrap();
    pipe.line(soon()).unwrap();
    let made = home.join("candidate/home");
    assert_eq!(
        String::from_utf8(pipe.line(soon()).unwrap()).unwrap(),
        format!("home:{own} pwd:{}", made.display())
    );
    assert_eq!(pipe.line(soon()).unwrap(), b"moved:");
    let agents: String = agent_home_variables()
        .iter()
        .map(|(_, folder)| format!(" {own}/{folder}"))
        .collect();
    assert_eq!(
        String::from_utf8(pipe.line(soon()).unwrap()).unwrap(),
        format!("agents:{agents}")
    );
    assert_eq!(mode(&made), 0o700);
    for (_, folder) in agent_home_variables() {
        assert_eq!(mode(&made.join(&folder)), 0o700, "{folder}");
    }
}

/// Every folder below the install folder must be a real folder of the
/// account's that no other account can change, as the install folder is: a
/// link there would take the files somewhere else, and a folder another
/// account can change would let it swap what runs. Neither is used or
/// written through, and the folder outside stays as it was.
#[test]
fn a_link_or_an_open_folder_below_the_install_folder_is_refused() {
    let device = Device::new();
    let packages = device.packages(std::env::consts::OS, std::env::consts::ARCH, "helper 1");
    let install_once = || {
        let mut pipe = device.connect("");
        let facts = probe(&mut pipe, soon()).unwrap();
        let payload = packages.payload(&facts.os, &facts.arch).unwrap();
        install(
            &mut pipe,
            "~/.hide/helper",
            &facts.home,
            &payload,
            facts.unpacks,
        )
    };
    let first = install_once().unwrap();
    let (version, _) = first.helper_relative.split_once('/').unwrap();
    let version = device.home().join(".hide/helper").join(version);
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
    std::os::unix::fs::symlink(&outside, &version).unwrap();
    let Err(EstablishError::Install(reason)) = install_once() else {
        panic!("an install through a link below the install folder was accepted");
    };
    assert!(reason.contains("is a link"), "{reason}");
    assert_eq!(contents(), before);

    std::fs::remove_file(&version).unwrap();
    std::fs::create_dir(&version).unwrap();
    std::fs::set_permissions(&version, std::fs::Permissions::from_mode(0o770)).unwrap();
    let Err(EstablishError::Install(reason)) = install_once() else {
        panic!("an install into a folder others can change was accepted");
    };
    assert!(reason.contains("can be changed"), "{reason}");
    assert_eq!(std::fs::read_dir(&version).unwrap().count(), 0);
}

/// `HIDE_DEV_DEVICE_HOME` names a home below the account's or an absolute
/// one on the device's own system, and nothing else.
#[test]
fn a_device_home_is_below_the_account_s_or_absolute_on_the_device() {
    use super::super::super::device::{parse_device_home, resolve_device_home};
    for good in [
        "~/.hide-dev/connect/home",
        "/srv/candidate",
        "C:/hide-dev/home",
    ] {
        assert_eq!(parse_device_home(good).as_deref(), Ok(good));
    }
    for bad in ["relative", "~/", "~/a/../b", "/a//b", "~/a\nb", ""] {
        assert!(parse_device_home(bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        resolve_device_home("~/.hide-dev/home", "D:/계정/사용자", "windows").unwrap(),
        "D:/계정/사용자/.hide-dev/home"
    );
    assert_eq!(
        resolve_device_home("C:/hide-dev/home", "C:/Users/me", "windows").unwrap(),
        "C:/hide-dev/home"
    );
    assert!(resolve_device_home("/srv/home", "C:/Users/me", "windows").is_err());
    assert!(resolve_device_home("C:/hide-dev/home", "/home/me", "linux").is_err());
}

#[test]
fn a_step_naming_a_line_end_is_never_sent() {
    let device = Device::new();
    let mut pipe = device.connect("");
    probe(&mut pipe, soon()).unwrap();
    assert!(start(&mut pipe, "v1/hided", Some("/tmp/a\nstart\tx")).is_err());
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
