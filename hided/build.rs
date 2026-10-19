//! Embeds `web/dist` into a release `hided` (PRD D-03), and the version and
//! commit `hide version` reports into every build.
//!
//! A release binary carries the web shell so `hide` runs from one file; it
//! fails to build without `web/dist`, because a release that silently served
//! a "UI not built" page would be a broken product, not a diagnostic. A debug
//! build embeds nothing and reads the directory at run time, so `pnpm build`
//! shows up without a cargo rebuild.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    build_identity(&manifest);
    product_analytics_config();
    let dist = manifest.join("../web/dist");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("ui_embed.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let release = env::var("PROFILE").as_deref() == Ok("release");
    if !release {
        fs::write(&out, "pub static FILES: &[(&str, &[u8])] = &[];\n").expect("write ui_embed.rs");
        return;
    }
    // The directory itself, so a file added or removed by a rebuild of the
    // web shell re-runs this script; per-file lines below catch edits. Only
    // a release embeds, so only a release watches the directory.
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed={}", dist.join("assets").display());
    if !dist.join("index.html").is_file() {
        panic!(
            "web/dist/index.html is missing at {}; run `pnpm --dir web build` before a release build of hided",
            dist.display()
        );
    }
    let mut files = Vec::new();
    collect(&dist, &dist, &mut files);
    files.sort();
    let mut source = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (relative, absolute) in &files {
        println!("cargo:rerun-if-changed={}", absolute.display());
        source.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            relative,
            absolute.display().to_string()
        ));
    }
    source.push_str("];\n");
    fs::write(&out, source).expect("write ui_embed.rs");
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).expect("read web/dist") {
        let entry = entry.expect("dist entry");
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, files);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("under dist")
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
}

/// `HIDE_VERSION` is the version a package ships (`desktop/scripts/package.mjs`
/// passes it to its release build); without it the crate's version stands.
/// The commit is `HIDE_COMMIT`, which the package also passes; a release
/// build without it reads the checkout's `HEAD`. A debug build reports no
/// commit unless it is given one, because following `HEAD` would rebuild
/// `hided` after every commit an agent makes, and a build from a tree with
/// no Git has no commit to report.
fn build_identity(manifest: &Path) {
    println!("cargo:rerun-if-env-changed=HIDE_VERSION");
    println!("cargo:rerun-if-env-changed=HIDE_COMMIT");
    println!("cargo:rerun-if-env-changed=HIDE_BUILD_ORDER");
    // The package's place in main's history, which orders two builds of one
    // release (`hided::build_order`); a build no package numbered is 0.
    let order = env::var("HIDE_BUILD_ORDER")
        .ok()
        .filter(|order| !order.is_empty())
        .unwrap_or_else(|| "0".to_owned());
    assert!(
        order.bytes().all(|byte| byte.is_ascii_digit()) && order.parse::<u64>().is_ok(),
        "HIDE_BUILD_ORDER must be a commit count, not '{order}'"
    );
    println!("cargo:rustc-env=HIDE_BUILD_ORDER={order}");
    let version = env::var("HIDE_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION"));
    println!("cargo:rustc-env=HIDE_BUILD_VERSION={version}");
    let release = env::var("PROFILE").as_deref() == Ok("release");
    let commit = match env::var("HIDE_COMMIT")
        .ok()
        .filter(|commit| !commit.is_empty())
    {
        Some(commit) => commit,
        None if release => head_commit(manifest).unwrap_or_default(),
        None => String::new(),
    };
    println!("cargo:rustc-env=HIDE_BUILD_COMMIT={commit}");
}

fn git(manifest: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// The checkout's `HEAD`, re-read when `HEAD` or the branch it names moves.
/// Only paths that exist are watched: Cargo reruns a script on every build
/// while a watched path is missing.
fn head_commit(manifest: &Path) -> Option<String> {
    let commit = git(manifest, &["rev-parse", "HEAD"])?;
    let watch = |name: &str| {
        if let Some(path) = git(
            manifest,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
        ) {
            // A branch's commit lives in its loose ref until Git packs it,
            // and a new loose ref changes the folder it is written to.
            let path = PathBuf::from(path);
            if let Some(existing) = path.ancestors().find(|path| path.exists()) {
                println!("cargo:rerun-if-changed={}", existing.display());
            }
        }
    };
    watch("HEAD");
    if let Some(branch) = git(manifest, &["symbolic-ref", "-q", "HEAD"]) {
        watch(&branch);
        watch("packed-refs");
    }
    Some(commit)
}

/// Optional build input: a public, write-only PostHog project token.
/// Absent or empty means no transport, including contributor and test builds.
/// Values live in the build environment, never source or diagnostics.
fn product_analytics_config() {
    const KEY: &str = "HIDE_POSTHOG_PROJECT_TOKEN";
    println!("cargo:rerun-if-env-changed={KEY}");
    let token = match env::var(KEY) {
        Ok(value) if value.is_empty() => value,
        Ok(value) => {
            assert!(
                value.starts_with("phc_")
                    && (20..=128).contains(&value.len())
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "HIDE_POSTHOG_PROJECT_TOKEN must be a public project token"
            );
            value
        }
        Err(env::VarError::NotPresent) => String::new(),
        Err(_) => panic!("HIDE_POSTHOG_PROJECT_TOKEN is not valid UTF-8"),
    };
    println!("cargo:rustc-env={KEY}={token}");
}
