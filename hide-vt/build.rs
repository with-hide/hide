// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// crates/ghostty-vt/build.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: the source and Zig come from the inputs
// scripts/prepare-vt-inputs.py prepared and verified, Zig runs with package
// fetching off and its caches in the worktree, the Zig version is checked
// before the build, and Herdr's optimize, SIMD, system-dir and Windows libc
// switches are gone. See hide-vt/PATCHES.md.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

const PREPARE: &str = "python3 scripts/prepare-vt-inputs.py";

// The integrations libghostty's build can take from the system; every one is
// built from the pinned source instead, so `--system` only turns fetching off.
const SYSTEM_INTEGRATIONS: [&str; 11] = [
    "freetype",
    "harfbuzz",
    "fontconfig",
    "libpng",
    "zlib",
    "oniguruma",
    "glslang",
    "spirv-cross",
    "simdutf",
    "gtk4-layer-shell",
    "highway",
];

fn zig_target(target: &str) -> &'static str {
    match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "x86_64-apple-darwin" => "x86_64-macos",
        "aarch64-apple-darwin" => "aarch64-macos",
        "x86_64-pc-windows-msvc" => "x86_64-windows-msvc",
        "aarch64-pc-windows-msvc" => "aarch64-windows-msvc",
        other => panic!(
            "hide-vt does not build for {other}: libghostty-vt has no Zig target mapped for it"
        ),
    }
}

// The name of the Zig archive scripts/prepare-vt-inputs.py installs for the
// machine that runs the build.
fn zig_host(host: &str) -> &'static str {
    let arm = host.starts_with("aarch64");
    match (host.split('-').nth(2), arm) {
        (Some("darwin"), true) => "aarch64-macos",
        (Some("darwin"), false) if host.starts_with("x86_64") => "x86_64-macos",
        (Some("linux"), true) => "aarch64-linux",
        (Some("linux"), false) if host.starts_with("x86_64") => "x86_64-linux",
        (Some("windows"), true) => "aarch64-windows",
        (Some("windows"), false) if host.starts_with("x86_64") => "x86_64-windows",
        _ => panic!("hide-vt pins no Zig for a {host} build machine"),
    }
}

// The same folder scripts/prepare-vt-inputs.py names: HIDE_BUILD_CACHE, else
// ~/.cache/hide-build, else %LOCALAPPDATA%\hide-build on Windows.
fn cache_root() -> PathBuf {
    if let Some(explicit) = env::var_os("HIDE_BUILD_CACHE").filter(|value| !value.is_empty()) {
        return PathBuf::from(explicit);
    }
    if cfg!(windows) {
        let local = env::var_os("LOCALAPPDATA")
            .expect("LOCALAPPDATA is not set, so hide-vt cannot find its prepared build inputs; set HIDE_BUILD_CACHE");
        return PathBuf::from(local).join("hide-build");
    }
    let home = env::var_os("HOME").expect(
        "HOME is not set, so hide-vt cannot find its prepared build inputs; set HIDE_BUILD_CACHE",
    );
    PathBuf::from(home).join(".cache").join("hide-build")
}

fn manifest_digest(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw).replace("\r\n", "\n");
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn zig_version(zig: &Path) -> String {
    let output = Command::new(zig).arg("version").output().unwrap_or_else(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            panic!(
                "Zig was not found at {}. hide-vt builds libghostty-vt with the Zig its prepared \
                 build inputs hold; run `{PREPARE}` once where the network is reachable, or set ZIG \
                 to a Zig binary of the version hide-vt/build-inputs.json pins",
                zig.display()
            );
        }
        panic!("could not run {} version: {err}", zig.display());
    });
    assert!(
        output.status.success(),
        "{} version failed: {}",
        zig.display(),
        output.status
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest_path = manifest_dir.join("build-inputs.json");
    let raw = fs::read(&manifest_path).expect("failed to read hide-vt/build-inputs.json");
    let manifest: Value =
        serde_json::from_slice(&raw).expect("hide-vt/build-inputs.json is not JSON");
    let wanted_zig = manifest["zig"]["version"]
        .as_str()
        .expect("hide-vt/build-inputs.json names no Zig version");
    let digest = manifest_digest(&raw);
    let host = env::var("HOST").expect("HOST");
    let target = env::var("TARGET").expect("TARGET");
    let zig_target = zig_target(&target);
    let inputs = cache_root()
        .join("vt")
        .join(format!("{digest}-{}", zig_host(&host)));
    let stamp = inputs.join("ready.json");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    println!("cargo:rerun-if-changed={}", stamp.display());
    println!("cargo:rerun-if-env-changed=HIDE_BUILD_CACHE");
    println!("cargo:rerun-if-env-changed=HIDE_VT_ZIG_CACHE");
    println!("cargo:rerun-if-env-changed=ZIG");

    let ready: Option<Value> = fs::read(&stamp)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    assert!(
        ready.as_ref().and_then(|stamp| stamp["digest"].as_str()) == Some(digest.as_str()),
        "hide-vt's build inputs are not prepared at {}: run `{PREPARE}` once where the network \
         is reachable, then build again; the build itself downloads nothing",
        inputs.display()
    );

    let zig = env::var_os("ZIG").map(PathBuf::from).unwrap_or_else(|| {
        inputs
            .join("zig")
            .join(if cfg!(windows) { "zig.exe" } else { "zig" })
    });
    let found_zig = zig_version(&zig);
    assert!(
        found_zig == wanted_zig,
        "hide-vt needs Zig {wanted_zig}, but {} is Zig {found_zig}; unset ZIG to use the prepared \
         Zig, or point it at a Zig {wanted_zig} binary",
        zig.display()
    );

    let source = inputs.join("source");
    let version_string = fs::read_to_string(source.join("VERSION"))
        .expect("failed to read the prepared libghostty-vt VERSION")
        .trim()
        .to_string();
    println!("cargo:rustc-env=HIDE_VT_SOURCE_DIR={}", source.display());

    // Install into this build's OUT_DIR so concurrent builds for different targets
    // from one checkout never link each other's archive.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let zig_prefix = out_dir.join("zig-out");
    // Zig's caches hold about 260 MB per target. CI names a folder it keeps
    // between runs; otherwise they live in OUT_DIR for this build only.
    let kept_cache = env::var_os("HIDE_VT_ZIG_CACHE").filter(|value| !value.is_empty());
    let zig_cache = kept_cache
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| out_dir.join("zig-cache"));

    let mut command = Command::new(&zig);
    command
        .arg("build")
        .arg("--prefix")
        .arg(&zig_prefix)
        .arg("--cache-dir")
        .arg(zig_cache.join("local"))
        .arg("--global-cache-dir")
        .arg(zig_cache.join("global"))
        // Turns package fetching off: every package comes from the prepared folder.
        .arg("--system")
        .arg(inputs.join("packages"))
        .args(
            SYSTEM_INTEGRATIONS
                .iter()
                .map(|name| format!("-fno-sys={name}")),
        )
        .arg("-Demit-lib-vt")
        .arg("-Doptimize=ReleaseFast")
        .arg("-Dsimd=true")
        .arg(format!("-Dtarget={zig_target}"))
        .arg(format!("-Dversion-string={version_string}"))
        .arg("-Demit-xcframework=false");
    let status = command
        .current_dir(&source)
        .status()
        .unwrap_or_else(|err| panic!("failed to run zig build for libghostty-vt: {err}"));
    assert!(
        status.success(),
        "zig build for libghostty-vt failed: {status}. A package missing from {} means \
         hide-vt/build-inputs.json does not pin everything this target needs",
        inputs.join("packages").display()
    );
    if kept_cache.is_none() {
        let _ = fs::remove_dir_all(&zig_cache);
    }

    let mut lib_dir = zig_prefix.join("lib");
    if target.contains("-apple-") {
        // Apple's linker prefers the sibling dylib for `-l`, so search a directory
        // holding only the static archive.
        let static_dir = out_dir.join("lib");
        fs::create_dir_all(&static_dir).expect("failed to create libghostty-vt link directory");
        fs::copy(
            lib_dir.join("libghostty-vt.a"),
            static_dir.join("libghostty-vt.a"),
        )
        .expect("failed to copy libghostty-vt static archive");
        lib_dir = static_dir;
    }
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-lib=static=ghostty-vt-static");
    } else {
        println!("cargo:rustc-link-lib=static=ghostty-vt");
    }
}
