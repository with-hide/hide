// portable-pty 0.9.0, part of wezterm, Copyright (c) 2018 Wez Furlong,
// MIT License (vendor/portable-pty/LICENSE.md). Taken from Herdr v0.9.5's
// vendor/portable-pty (commit 2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01).
// Herdr's patches in this file: 0001.
// Hide's changes in this file: the pinned ConPTY runtime is required (no system fallback, no environment switch, x86_64 only) and its failure is returned, not a panic, in words that do not name the package's file list; `verify_runtime`; `spawn_command_in_job` starts the child suspended, puts it in the job and only then resumes it.
// vendor/portable-pty/PATCHES.md lists both.
use super::WinChild;
use crate::cmdbuilder::CommandBuilder;
use crate::win::procthreadattr::ProcThreadAttributeList;
use anyhow::{bail, ensure, Error};
use filedescriptor::{FileDescriptor, OwnedHandle};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Error as IoError, Read};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::{mem, ptr};
use winapi::shared::minwindef::{DWORD, HMODULE};
use winapi::shared::winerror::{HRESULT, S_OK};
use winapi::um::handleapi::*;
use winapi::um::jobapi2::AssignProcessToJobObject;
use winapi::um::libloaderapi::{
    FreeLibrary, GetProcAddress, LoadLibraryExW,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use winapi::um::processthreadsapi::*;
use winapi::um::winbase::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use winapi::um::wincon::COORD;
use winapi::um::winnt::HANDLE;

pub type HPCON = HANDLE;

pub const PSUEDOCONSOLE_INHERIT_CURSOR: DWORD = 0x1;
pub const PSEUDOCONSOLE_RESIZE_QUIRK: DWORD = 0x2;
pub const PSEUDOCONSOLE_WIN32_INPUT_MODE: DWORD = 0x4;
#[allow(dead_code)]
pub const PSEUDOCONSOLE_PASSTHROUGH_MODE: DWORD = 0x8;

type CreatePseudoConsoleFn = unsafe extern "system" fn(
    size: COORD,
    h_input: HANDLE,
    h_output: HANDLE,
    flags: DWORD,
    hpc: *mut HPCON,
) -> HRESULT;
type ResizePseudoConsoleFn = unsafe extern "system" fn(hpc: HPCON, size: COORD) -> HRESULT;
type ClosePseudoConsoleFn = unsafe extern "system" fn(hpc: HPCON);

// These fields intentionally mirror the exported Win32 symbol names.
#[allow(non_snake_case)]
struct ConPtyFuncs {
    module: HMODULE,
    owned: bool,
    CreatePseudoConsole: CreatePseudoConsoleFn,
    ResizePseudoConsole: ResizePseudoConsoleFn,
    ClosePseudoConsole: ClosePseudoConsoleFn,
}

unsafe impl Send for ConPtyFuncs {}
unsafe impl Sync for ConPtyFuncs {}

impl Drop for ConPtyFuncs {
    fn drop(&mut self) {
        if self.owned {
            unsafe { FreeLibrary(self.module) };
        }
    }
}

impl ConPtyFuncs {
    unsafe fn from_module(module: HMODULE, owned: bool) -> Result<Self, String> {
        if module.is_null() {
            return Err(IoError::last_os_error().to_string());
        }
        Ok(Self {
            module,
            owned,
            CreatePseudoConsole: load_symbol(module, b"CreatePseudoConsole\0")?,
            ResizePseudoConsole: load_symbol(module, b"ResizePseudoConsole\0")?,
            ClosePseudoConsole: load_symbol(module, b"ClosePseudoConsole\0")?,
        })
    }
}

unsafe fn load_symbol<T: Copy>(module: HMODULE, name: &'static [u8]) -> Result<T, String> {
    let symbol = GetProcAddress(module, name.as_ptr().cast());
    if symbol.is_null() {
        return Err(format!(
            "missing symbol {}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        ));
    }
    Ok(mem::transmute_copy(&symbol))
}

fn load_app_local_conpty(path: &Path) -> Result<ConPtyFuncs, String> {
    let wide_path = wide_string(path.as_os_str());
    let module = unsafe {
        LoadLibraryExW(
            wide_path.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    match unsafe { ConPtyFuncs::from_module(module, true) } {
        Ok(functions) => Ok(functions),
        Err(error) => {
            if !module.is_null() {
                unsafe { FreeLibrary(module) };
            }
            Err(error)
        }
    }
}

fn wide_string(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

// Hide: the pinned runtime beside the program or nothing. Herdr fell back to
// the ConPTY in kernel32 when the bundle was absent, and to it on request
// through an environment variable; a holder that ran on whatever ConPTY the
// system has would run a terminal Hide never tested, so a missing, changed or
// unsupported bundle is an error the caller reports instead of a panic.
fn load_conpty() -> Result<ConPtyFuncs, String> {
    let path = app_local_conpty_path()?;
    load_app_local_conpty(&path).map_err(|error| {
        format!(
            "the pinned ConPTY runtime {} could not be loaded: {error}",
            path.display()
        )
    })
}

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
// The runtime's file list, as the package names it.
const MANIFEST: &str = "herdr-conpty.json";

// Hide: how a file of the runtime is named in an error the operator may
// read; the list's own file name is the package's, not a word for it.
fn shown(name: &str) -> &str {
    if name == MANIFEST {
        "its file list"
    } else {
        name
    }
}
const CONPTY_FILES: &[(&str, &str)] = &[
    (
        "conpty.dll",
        "39fba2713e2495117b1591ae8c32a3b904bea7aa66069cf7815e2844c76d75d8",
    ),
    (
        "x64/OpenConsole.exe",
        "b7fd936c2668b87b9ecf7b3366dc6568afc1c6f981874cba3e955a1c35cf8160",
    ),
    (
        "arm64/OpenConsole.exe",
        "ed7622fd0d3bedc9ab9f122f5e58edf0def9e7999224f52dd395ba9f54edbe09",
    ),
];

fn app_local_conpty_path() -> Result<PathBuf, String> {
    // Hide: only the x64 runtime is supported; Herdr ran any other
    // architecture on the system ConPTY.
    if std::env::consts::ARCH != "x86_64" {
        return Err(format!(
            "the pinned ConPTY runtime runs on x86_64 only, not {}",
            std::env::consts::ARCH
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate this program: {error}"))?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| "this program has no parent directory".to_string())?;
    let bundle = executable_dir.join("conpty");
    if !bundle.exists() {
        return Err(format!(
            "the pinned ConPTY runtime is missing: {} does not exist",
            bundle.display()
        ));
    }
    reject_reparse_point(&bundle)?;

    let mut expected = BTreeSet::from([MANIFEST.to_string()]);
    for (relative, expected_hash) in CONPTY_FILES {
        expected.insert((*relative).to_string());
        verify_bundle_file(&bundle, relative, expected_hash)?;
    }
    let marker = bundle.join(MANIFEST);
    verify_regular_file(&marker).map_err(|_| {
        "the pinned ConPTY runtime's file list is missing or not a regular file".to_string()
    })?;

    let actual = bundle_files(&bundle)?;
    if actual != expected {
        let missing: Vec<&str> = expected.difference(&actual).map(|name| shown(name)).collect();
        let unexpected: Vec<&str> = actual.difference(&expected).map(|name| shown(name)).collect();
        return Err(format!(
            "the pinned ConPTY runtime holds other files than it should: missing {missing:?}, unexpected {unexpected:?}"
        ));
    }
    Ok(bundle.join("conpty.dll"))
}

fn verify_bundle_file(root: &Path, relative: &str, expected_hash: &str) -> Result<(), String> {
    let path = root.join(relative);
    verify_regular_file(&path)?;
    let mut file = File::open(&path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual_hash = format!("{:x}", digest.finalize());
    if actual_hash != expected_hash {
        return Err(format!(
            "{} has hash {actual_hash}, expected {expected_hash}",
            path.display()
        ));
    }
    Ok(())
}

fn verify_regular_file(path: &Path) -> Result<(), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{} is not a regular file", path.display()));
    }
    Ok(())
}

fn reject_reparse_point(path: &Path) -> Result<(), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{} is not a regular directory", path.display()));
    }
    Ok(())
}

fn bundle_files(root: &Path) -> Result<BTreeSet<String>, String> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeSet<String>) -> Result<(), String> {
        reject_reparse_point(directory)?;
        for entry in std::fs::read_dir(directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = path
                .symlink_metadata()
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(format!("{} is a reparse point", path.display()));
            }
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative);
            } else {
                return Err(format!("{} is not a regular file", path.display()));
            }
        }
        Ok(())
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

// Hide: loaded once, and a failure is kept and answered to every caller rather
// than a panic inside `lazy_static`.
static CONPTY: OnceLock<Result<ConPtyFuncs, String>> = OnceLock::new();

fn conpty() -> Result<&'static ConPtyFuncs, Error> {
    CONPTY
        .get_or_init(load_conpty)
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Hide: checks and loads the pinned ConPTY runtime beside this program, so a
/// caller can refuse to start before it creates anything.
pub fn verify_runtime() -> Result<(), Error> {
    conpty().map(|_| ())
}

pub struct PsuedoCon {
    con: HPCON,
}

unsafe impl Send for PsuedoCon {}
unsafe impl Sync for PsuedoCon {}

impl Drop for PsuedoCon {
    fn drop(&mut self) {
        // A console exists only once the runtime loaded.
        if let Ok(conpty) = conpty() {
            unsafe { (conpty.ClosePseudoConsole)(self.con) };
        }
    }
}

impl PsuedoCon {
    pub fn new(size: COORD, input: FileDescriptor, output: FileDescriptor) -> Result<Self, Error> {
        let conpty = conpty()?;
        let mut con: HPCON = INVALID_HANDLE_VALUE;
        let result = unsafe {
            (conpty.CreatePseudoConsole)(
                size,
                input.as_raw_handle() as _,
                output.as_raw_handle() as _,
                PSUEDOCONSOLE_INHERIT_CURSOR
                    | PSEUDOCONSOLE_RESIZE_QUIRK
                    | PSEUDOCONSOLE_WIN32_INPUT_MODE,
                &mut con,
            )
        };
        ensure!(
            result == S_OK,
            "failed to create psuedo console: HRESULT {}",
            result
        );
        Ok(Self { con })
    }

    pub fn resize(&self, size: COORD) -> Result<(), Error> {
        let result = unsafe { (conpty()?.ResizePseudoConsole)(self.con, size) };
        ensure!(
            result == S_OK,
            "failed to resize console to {}x{}: HRESULT: {}",
            size.X,
            size.Y,
            result
        );
        Ok(())
    }

    pub fn spawn_command(&self, cmd: CommandBuilder) -> anyhow::Result<WinChild> {
        self.spawn(cmd, None)
    }

    /// Hide: starts the command suspended, puts it in `job` and only then
    /// lets it run, so nothing it starts can be outside the job. A child that
    /// cannot be put there is ended before it runs.
    pub fn spawn_command_in_job(
        &self,
        cmd: CommandBuilder,
        job: HANDLE,
    ) -> anyhow::Result<WinChild> {
        self.spawn(cmd, Some(job))
    }

    fn spawn(&self, cmd: CommandBuilder, job: Option<HANDLE>) -> anyhow::Result<WinChild> {
        let mut si: STARTUPINFOEXW = unsafe { mem::zeroed() };
        si.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        // Explicitly set the stdio handles as invalid handles otherwise
        // we can end up with a weird state where the spawned process can
        // inherit the explicitly redirected output handles from its parent.
        // For example, when daemonizing wezterm-mux-server, the stdio handles
        // are redirected to a log file and the spawned process would end up
        // writing its output there instead of to the pty we just created.
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;

        let mut attrs = ProcThreadAttributeList::with_capacity(1)?;
        attrs.set_pty(self.con)?;
        si.lpAttributeList = attrs.as_mut_ptr();

        let mut pi: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let (mut exe, mut cmdline) = cmd.cmdline()?;
        let cmd_os = OsString::from_wide(&cmdline);

        let cwd = cmd.current_directory();

        let res = unsafe {
            CreateProcessW(
                exe.as_mut_slice().as_mut_ptr(),
                cmdline.as_mut_slice().as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT
                    | if job.is_some() { CREATE_SUSPENDED } else { 0 },
                cmd.environment_block().as_mut_slice().as_mut_ptr() as *mut _,
                cwd.as_ref()
                    .map(|c| c.as_slice().as_ptr())
                    .unwrap_or(ptr::null()),
                &mut si.StartupInfo,
                &mut pi,
            )
        };
        if res == 0 {
            let err = IoError::last_os_error();
            let msg = format!(
                "CreateProcessW `{:?}` in cwd `{:?}` failed: {}",
                cmd_os,
                cwd.as_ref().map(|c| OsString::from_wide(c)),
                err
            );
            log::error!("{}", msg);
            bail!("{}", msg);
        }

        // Make sure we close out the thread handle so we don't leak it;
        // we do this simply by making it owned
        let main_thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as _) };
        let proc = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as _) };

        if let Some(job) = job {
            let started = if unsafe { AssignProcessToJobObject(job, pi.hProcess) } == 0 {
                Err(format!(
                    "the new process could not be put in its job: {}",
                    IoError::last_os_error()
                ))
            } else if unsafe { ResumeThread(main_thread.as_raw_handle() as _) } == DWORD::MAX {
                Err(format!(
                    "the new process could not be started: {}",
                    IoError::last_os_error()
                ))
            } else {
                Ok(())
            };
            if let Err(msg) = started {
                // Suspended, or inside the job and suspended: it never ran.
                unsafe { TerminateProcess(pi.hProcess, 1) };
                log::error!("{}", msg);
                bail!("{}", msg);
            }
        }

        Ok(WinChild {
            proc: Mutex::new(proc),
        })
    }
}
