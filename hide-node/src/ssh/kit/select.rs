//! Which programs a device gets, by one rule on every core
//! (with-hide/hide-private#501): a developer's `<name>-<os>-<arch>` file
//! first, then, for the core's own platform only, its own programs under
//! their system's names, each part on its own; and when neither holds the
//! helper, the release's kit for any platform the contract carries. This runs
//! on the connection worker, never under the runtime mutex, and the bounded
//! synchronous download cannot outlive that worker.

use super::super::{
    CLI_NAME, EstablishError, HELPER_NAME, HOLDER_NAME, HOOKS_NAME, HelperPackages, Payload,
    device_program, platform_label, read_folder, read_package,
};
use super::cache;
use super::contract::CONTRACT;
use std::path::PathBuf;

/// Where the core's own release keeps its assets.
const RELEASES: &str = "https://github.com/with-hide/hide/releases/download";

/// Where a device's programs come from.
enum Source {
    /// This build's folder: a developer's files, or the core's own programs
    /// for its own platform.
    Local,
    /// The kit of this build's release, for another platform.
    Release,
}

impl HelperPackages {
    /// The build for `os`/`arch` (Rust's names: `macos`, `aarch64`). A
    /// `hided-<os>-<arch>` file wins; the daemon's own program (`hided`,
    /// `hided.exe` on Windows) is accepted only for this daemon's own
    /// platform, which is what the package and a development build carry.
    pub fn find(&self, os: &str, arch: &str) -> Result<PathBuf, String> {
        self.find_built_program(HELPER_NAME, os, arch).map_err(|()| {
            format!(
                "This Hide build does not include the device helper for {} {arch}; files and Git on this device stay unavailable until a build that packages it is installed",
                platform_label(os)
            )
        })
    }

    fn source(&self, os: &str, arch: &str) -> Result<Source, EstablishError> {
        if self.find_built_program(HELPER_NAME, os, arch).is_ok() {
            return Ok(Source::Local);
        }
        // A build never takes its own platform's programs from a release:
        // they would be another build's.
        let own = os == std::env::consts::OS && arch == std::env::consts::ARCH;
        if own || CONTRACT.kit(os, arch).is_none() {
            return Err(EstablishError::Unsupported(
                self.find(os, arch).unwrap_err(),
            ));
        }
        if !self.release.published() {
            let folder = self.directory.as_ref().map_or_else(
                || "this build's folder".to_owned(),
                |dir| dir.display().to_string(),
            );
            return Err(EstablishError::Unsupported(format!(
                "This Hide build ({}) has no published release to download the {} {arch} device kit from; put that device's programs in {folder} as <name>-{os}-{arch} (desktop/scripts/device-helpers.mjs stage unpacks a release or CI kit there)",
                self.release.version,
                platform_label(os)
            )));
        }
        Ok(Source::Release)
    }

    /// The release kit for `os`/`arch`, every file of it.
    fn release_kit(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        let failed = |error: anyhow::Error| EstablishError::Download {
            os: os.to_owned(),
            arch: arch.to_owned(),
            reason: format!("Device kit download failed: {error:#}. Retry the device connection"),
        };
        let home = hide_platform::host::home_dir().map_err(|error| {
            failed(anyhow::Error::new(error).context("The device cache home is unavailable"))
        })?;
        let base = format!("{RELEASES}/v{}", self.release.version);
        let files = cache::load(&self.release, os, arch, &home, &base).map_err(failed)?;
        Ok(Payload {
            files,
            missing: Vec::new(),
        })
    }

    /// What a device of `os`/`arch` gets: the helper, the kit's parts, the
    /// pane holder, which runs the device's panes for the core's own
    /// engine, and on Windows the ConPTY runtime the holder loads.
    pub(in crate::ssh) fn payload_for(
        &self,
        os: &str,
        arch: &str,
    ) -> Result<Payload, EstablishError> {
        if let Source::Release = self.source(os, arch)? {
            return self.release_kit(os, arch);
        }
        let mut payload = self.local(os, arch)?;
        // A build that does not carry the holder for the device's system
        // connects nothing rather than a node that could run no pane.
        let holder = self.find_built_program(HOLDER_NAME, os, arch).map_err(|()| {
            EstablishError::Unsupported(format!(
                "This Hide build does not include the pane holder for {} {arch}; the core's own engine cannot run this device's panes",
                platform_label(os)
            ))
        })?;
        payload.files.push(
            read_package(&device_program(HOLDER_NAME, os), &holder)
                .map_err(EstablishError::Install)?,
        );
        // A Windows holder loads the pinned ConPTY runtime from the
        // folder beside the node's program (`hide_kit::holders`).
        if os == "windows" {
            let conpty = self
                .find_built_folder(hide_kit::holders::CONPTY_SOURCE, os, arch)
                .map_err(|()| {
                    EstablishError::Unsupported(format!(
                        "This Hide build does not include the pane holder's ConPTY runtime for {} {arch}; the core's own engine cannot run this device's panes",
                        platform_label(os)
                    ))
                })?;
            payload.files.extend(
                read_folder(hide_kit::holders::CONPTY_SOURCE, &conpty)
                    .map_err(EstablishError::Install)?,
            );
        }
        Ok(payload)
    }

    /// The programs this build puts on a machine of `os`/`arch` besides the
    /// pane holder and its runtime (a core's own install, `install_build`):
    /// the helper, which the machine cannot do without, and the kit's parts.
    /// From this build's folder a part it does not carry is left out and
    /// named, and the kit on the device reports it as missing from the
    /// build.
    pub(in crate::ssh) fn payload(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        match self.source(os, arch)? {
            Source::Local => self.local(os, arch),
            Source::Release => {
                let mut payload = self.release_kit(os, arch)?;
                let kept = [HELPER_NAME, CLI_NAME, HOOKS_NAME].map(|name| device_program(name, os));
                payload.files.retain(|file| kept.contains(&file.relative));
                Ok(payload)
            }
        }
    }

    /// The helper and the kit's parts from this build's folder, each found
    /// by the same rule as the helper.
    fn local(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        let helper = self.find(os, arch).map_err(EstablishError::Unsupported)?;
        let helper = read_package(&device_program(HELPER_NAME, os), &helper)
            .map_err(EstablishError::Install)?;
        let mut payload = Payload {
            files: vec![helper],
            missing: Vec::new(),
        };
        for name in [CLI_NAME, HOOKS_NAME] {
            match self.find_built_program(name, os, arch) {
                Ok(path) => match read_package(&device_program(name, os), &path) {
                    Ok(package) => payload.files.push(package),
                    Err(reason) => payload.missing.push(reason),
                },
                Err(()) => payload.missing.push(format!(
                    "This Hide build does not include {name} for {} {arch}",
                    platform_label(os)
                )),
            }
        }
        Ok(payload)
    }

    /// The program `name` for `os`/`arch`, by the helper's rule: this build's
    /// own copy carries its system's program name (`hided.exe` on Windows),
    /// as the package and a development build name it.
    fn find_built_program(&self, name: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        self.find_named(name, &device_program(name, os), os, arch)
    }

    /// The folder `name` for `os`/`arch`, by the helper's rule: a folder has
    /// the same name on every system (`holder-conpty`).
    fn find_built_folder(&self, name: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        self.find_named(name, name, os, arch)
    }

    /// A developer's `<name>-<os>-<arch>` first; for this daemon's own
    /// platform only, then the file or folder `own` beside it.
    fn find_named(&self, name: &str, own: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        let directory = self.directory.as_ref().ok_or(())?;
        let named = directory.join(format!("{name}-{os}-{arch}"));
        if named.exists() {
            return Ok(named);
        }
        let own = directory.join(own);
        if os == std::env::consts::OS && arch == std::env::consts::ARCH && own.exists() {
            return Ok(own);
        }
        Err(())
    }
}
