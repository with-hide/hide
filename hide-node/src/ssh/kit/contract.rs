//! The device kits a release carries, as `contracts/device-kits.json` names
//! them: the one place a device platform is named. The packaging scripts read
//! the same file (`desktop/scripts/device-helpers.mjs`), and
//! `scripts/check-device-kits-single-source.py` refuses a platform name
//! written anywhere else in the kit code.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The contract, read once from the file compiled into this build; its own
/// test parses and checks it, so a broken file fails the build's tests.
pub(in crate::ssh) static CONTRACT: LazyLock<Contract> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../contracts/device-kits.json"))
        .expect("contracts/device-kits.json must parse")
});

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ssh) struct Contract {
    /// The release manifest's format; a manifest of any other is refused.
    pub format: u32,
    /// The manifest's file name, an asset of the release.
    pub manifest: String,
    /// An archive's asset name, with `{platform}` and `{version}` in it.
    pub asset: String,
    /// The programs every kit carries, by their names without a suffix.
    pub programs: Vec<String>,
    /// Each platform a release carries a kit for, by `<os>-<arch>`.
    pub kits: BTreeMap<String, Kit>,
    /// Platforms refused before anything is installed, with the reason the
    /// device's row shows.
    pub refused: BTreeMap<String, String>,
}

/// One platform's kit. The contract also names its system and processor,
/// its Rust target and which job makes it, which only the packaging
/// scripts read; its key names the same system and processor.
#[derive(Debug, Deserialize)]
pub(in crate::ssh) struct Kit {
    /// What the device's system starts a program by (`.exe` on Windows).
    pub suffix: String,
    /// What every program's first bytes must say.
    pub header: Header,
    /// The kit's other files under their `/`-separated paths, and whether
    /// each is a program on the device.
    pub runtime: BTreeMap<String, bool>,
}

/// An executable's header, checked without running it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(in crate::ssh) enum Header {
    /// A 64-bit little-endian ELF executable or shared object for `machine`.
    Elf { machine: u16 },
    /// A 64-bit Mach-O executable for `cputype`.
    MachO { cputype: u32 },
    /// A PE image for `machine`.
    Pe { machine: u16 },
}

impl Contract {
    /// The kit for `os`/`arch`, if a release carries one.
    pub fn kit(&self, os: &str, arch: &str) -> Option<&Kit> {
        self.kits.get(&platform(os, arch))
    }

    /// The archive's asset name for `platform` in release `version`.
    pub fn asset(&self, platform: &str, version: &str) -> String {
        self.asset
            .replace("{platform}", platform)
            .replace("{version}", version)
    }
}

impl Kit {
    /// The program `name`'s file name on this kit's system.
    pub fn program(&self, name: &str) -> String {
        format!("{name}{}", self.suffix)
    }

    /// Every file of the kit and whether it is a program on the device: the
    /// contract's programs, then the runtime files.
    pub fn files(&self, programs: &[String]) -> BTreeMap<String, bool> {
        programs
            .iter()
            .map(|name| (self.program(name), true))
            .chain(
                self.runtime
                    .iter()
                    .map(|(path, &program)| (path.clone(), program)),
            )
            .collect()
    }
}

/// A platform's name in the contract.
pub(in crate::ssh) fn platform(os: &str, arch: &str) -> String {
    format!("{os}-{arch}")
}

/// Whether a device of `os`/`arch` may be served: a platform the contract
/// refuses answers its reason, and one it carries no kit for answers that.
pub(in crate::ssh) fn admit(os: &str, arch: &str, label: &str) -> Result<(), String> {
    let contract = &*CONTRACT;
    if let Some(reason) = contract.refused.get(&platform(os, arch)) {
        return Err(reason.clone());
    }
    if contract.kit(os, arch).is_none() {
        return Err(format!(
            "The device runs {label} on {arch}, for which Hide ships no device kit"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract parses, names each kit by its own system and processor,
    /// refuses no platform it also carries, and lists the programs the core
    /// sends (`HelperPackages`), each runtime file under a plain relative
    /// path.
    #[test]
    fn the_contract_is_whole_and_names_each_kit_by_its_platform() {
        let contract = &*CONTRACT;
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../../../../contracts/device-kits.json")).unwrap();
        assert_eq!(contract.format, 4);
        assert_eq!(
            contract.programs,
            ["hided", "hide", "hide-agent-hooks", "hide-holder"]
        );
        for (name, kit) in &contract.kits {
            let os = raw["kits"][name]["os"].as_str().unwrap();
            let arch = raw["kits"][name]["arch"].as_str().unwrap();
            assert_eq!(*name, platform(os, arch));
            assert!(!contract.refused.contains_key(name), "{name}");
            assert_eq!(kit.suffix, if os == "windows" { ".exe" } else { "" });
            for path in kit.runtime.keys() {
                assert!(
                    !path.starts_with('/')
                        && !path.contains('\\')
                        && path.split('/').all(|part| !part.is_empty() && part != ".."),
                    "{path}"
                );
            }
        }
        assert_eq!(
            contract.asset("linux-aarch64", "1.2.3"),
            "hide-device-linux-aarch64-1.2.3.tar.gz"
        );
    }

    /// The headers name the processors the kits are for, written here in
    /// the form their specifications give them.
    #[test]
    fn each_kit_header_names_its_processor() {
        let contract = &*CONTRACT;
        let header = |os: &str, arch: &str| contract.kit(os, arch).unwrap().header;
        assert_eq!(header("linux", "x86_64"), Header::Elf { machine: 62 });
        assert_eq!(header("linux", "aarch64"), Header::Elf { machine: 183 });
        assert_eq!(
            header("macos", "aarch64"),
            Header::MachO {
                cputype: 0x0100_000c
            }
        );
        assert_eq!(header("windows", "x86_64"), Header::Pe { machine: 0x8664 });
    }

    /// A Windows ARM PC and an Intel Mac are refused with the reason the
    /// row shows; a platform the contract never names is refused too.
    #[test]
    fn a_platform_outside_the_kits_is_refused_with_its_reason() {
        assert!(admit("linux", "aarch64", "Linux").is_ok());
        assert!(admit("windows", "x86_64", "Windows").is_ok());
        let windows_arm = admit("windows", "aarch64", "Windows").unwrap_err();
        assert!(windows_arm.contains("x64 PCs only"), "{windows_arm}");
        let intel = admit("macos", "x86_64", "macOS").unwrap_err();
        assert!(intel.contains("Apple Silicon"), "{intel}");
        let other = admit("freebsd", "x86_64", "FreeBSD").unwrap_err();
        assert!(other.contains("no device kit"), "{other}");
    }
}
