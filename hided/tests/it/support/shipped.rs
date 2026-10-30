//! A shipped copy of this candidate's executables: the device installs and
//! uploads shipped executables, which carry no debug data, so a fixture
//! stages this build's code with the same property, and ad-hoc signed on
//! macOS.
//!
//! Each version of an executable is stripped once per build folder, not once
//! per test: a debug `hided` is about 140 MB, and five `device_bootstrap`
//! tests that each copied and stripped it at once on a Linux runner ran past
//! the fixture's 10 s bound on a command (with-hide/hide-private#321). The
//! copy is kept beside the test binaries, made under an exclusive lock by
//! the first test process that needs it while the others wait, and handed to
//! each test as a byte copy of its own, which the test may change.

use std::fs::{self, File};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use hide_platform::fs::lock::{Mode, Waited, lock_file};

use super::remote_delivery::{Environment, successful};

/// How long a test waits for another test process that is making the copy.
/// A hang guard, not a deadline: the wait is that process's strip and
/// signing of up to three executables, each bounded by `successful`'s own
/// bound; it is nextest's slow-test period (`.config/nextest.toml`).
const MAKING_GUARD: Duration = Duration::from_secs(60);

/// Puts at `at` a shipped copy of `source`, an executable of this build,
/// replacing nothing: `at` must not exist yet. `environment` runs `strip`
/// and `codesign` the first time this version of `source` is asked for.
pub fn place(source: &Path, at: &Path, environment: &Environment) -> Result<()> {
    let store = std::env::current_exe()?.with_file_name("shipped");
    fs::create_dir_all(&store)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.join(".lock"))?;
    let _held = match lock_file(lock, Mode::Exclusive, MAKING_GUARD, &|| false)? {
        Waited::Locked(held) => held,
        Waited::TimedOut => bail!(
            "another test process held {} past {MAKING_GUARD:?}",
            store.display()
        ),
        Waited::Cancelled => unreachable!("nothing cancels the wait"),
    };
    let (family, kept) = name_of(source)?;
    let shipped = store.join(&kept);
    if !shipped.exists() {
        // An earlier version of the same executable, finished or left half
        // made by a test process that was killed, is never asked for again
        // by this build folder; nor is this version's half-made copy.
        for entry in fs::read_dir(&store)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.strip_prefix('.').unwrap_or(&name).starts_with(&family) {
                fs::remove_file(entry.path())?;
            }
        }
        let making = store.join(format!(".{kept}"));
        let made = make(source, &making, environment).and_then(|()| {
            fs::rename(&making, &shipped)?;
            Ok(())
        });
        if let Err(error) = made {
            if let Err(cleanup) = fs::remove_file(&making)
                && cleanup.kind() != ErrorKind::NotFound
            {
                return Err(error.context(format!(
                    "and {} could not be removed: {cleanup}",
                    making.display()
                )));
            }
            return Err(error);
        }
    }
    if at.exists() {
        bail!("{} already holds a file", at.display());
    }
    // A byte copy, never an APFS clone, whose first start can stall before
    // `main` (docs/TESTING.md, Writing a Rust test, rule 8).
    byte_copy(&shipped, at)?;
    fs::set_permissions(at, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Writes at `making` a stripped copy of `source`, ad-hoc signed on macOS.
fn make(source: &Path, making: &Path, environment: &Environment) -> Result<()> {
    byte_copy(source, making)?;
    fs::set_permissions(making, fs::Permissions::from_mode(0o755))?;
    let mut strip = environment.command("/usr/bin/strip");
    strip.arg(making);
    successful(strip)?;
    #[cfg(target_os = "macos")]
    {
        let mut sign = environment.command("/usr/bin/codesign");
        sign.args(["--force", "--sign", "-"]).arg(making);
        successful(sign)?;
    }
    Ok(())
}

/// The family every version of `source` shares in the store, and this
/// version's name in it: the executable's name, a hash of its folder, and a
/// hash of the build's file (its identity, length and write time).
fn name_of(source: &Path) -> Result<(String, String)> {
    let file = source
        .file_name()
        .with_context(|| format!("{} names no file", source.display()))?
        .to_string_lossy()
        .into_owned();
    let folder = source.parent().unwrap_or(Path::new(""));
    let meta = fs::metadata(source).with_context(|| format!("{}", source.display()))?;
    let id = hide_platform::fs::identity::file_id(source)?;
    let written = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let family = format!(
        "{file}-{:016x}-",
        fnv(folder.as_os_str().as_encoded_bytes())
    );
    let version = format!("{}:{}:{}:{written}", id.volume(), id.index(), meta.len());
    let kept = format!("{family}{:016x}", fnv(version.as_bytes()));
    Ok((family, kept))
}

/// FNV-1a, so a name stays the same across toolchains.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn byte_copy(from: &Path, to: &Path) -> Result<()> {
    std::io::copy(
        &mut File::open(from).with_context(|| format!("{}", from.display()))?,
        &mut File::create_new(to).with_context(|| format!("{}", to.display()))?,
    )?;
    Ok(())
}
