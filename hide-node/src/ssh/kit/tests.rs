//! The kit fetch and check, against a local HTTP release: every contract
//! platform's kit by the one code path, the build identity a manifest must
//! name, and what an archive or a cache slot may hold.

use super::archive::tests::program;
use super::cache;
use super::contract::{CONTRACT, platform};
use super::manifest::ReleaseIdentity;
use crate::ssh::host::{HelperPackages, UNPUBLISHED, hex_digest, program_by_contract};
use hide_node_link::device::EstablishError;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

/// The build every release fixture here was published for.
const BUILD: ReleaseIdentity = ReleaseIdentity {
    version: "1.2.3",
    commit: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    order: 42,
};

/// The release HTTP boundary: each path's status and body, and how often it
/// was asked for. Drop wakes and joins its accept loop, including after an
/// assertion fails; no test changes the process HOME or runtime.
struct Release {
    address: std::net::SocketAddr,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}
impl Release {
    fn serve(routes: BTreeMap<String, (u16, Vec<u8>)>) -> Self {
        Self::serve_with(routes, |_| {})
    }
    fn serve_with(
        routes: BTreeMap<String, (u16, Vec<u8>)>,
        mut on_request: impl FnMut(&str) + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let count = hits.clone();
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if receiver.try_recv().is_ok() {
                    break;
                }
                let mut stream = stream.unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert!(request.len() < 8192);
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&request).into_owned();
                let path = line
                    .strip_prefix("GET /")
                    .and_then(|rest| rest.split(' ').next())
                    .unwrap()
                    .to_owned();
                *count.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                on_request(&path);
                let (status, body) = routes.get(&path).cloned().unwrap_or((404, Vec::new()));
                write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            address,
            hits,
            stop,
            worker: Some(worker),
        }
    }
    fn base(&self) -> String {
        format!("http://{}", self.address)
    }
    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
    fn archive_hits(&self) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.ends_with(".tar.gz"))
            .map(|(_, hits)| hits)
            .sum()
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

/// One archive entry: its name, its type and its bytes.
type Entry = (String, tar::EntryType, Vec<u8>);

/// The files of `platform`'s kit as a release makes them: each program with
/// its kit's header, each runtime file with bytes of its own.
fn entries(platform: &str) -> Vec<Entry> {
    let kit = &CONTRACT.kits[platform];
    kit.files(&CONTRACT.programs)
        .into_iter()
        .map(|(name, is_program)| {
            let mut bytes = if is_program {
                program(kit.header)
            } else {
                b"runtime".to_vec()
            };
            bytes.extend_from_slice(name.as_bytes());
            (name, tar::EntryType::Regular, bytes)
        })
        .collect()
}

fn archive(entries: &[Entry]) -> Vec<u8> {
    let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(gzip);
    for (name, kind, bytes) in entries {
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(*kind);
        if kind.is_symlink() {
            header.set_link_name("outside").unwrap();
        }
        header.set_cksum();
        archive
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

/// A release of a build: every kit's archive and a manifest naming each
/// kit's own files.
struct Fixture {
    manifest: serde_json::Value,
    archives: BTreeMap<String, Vec<u8>>,
}

/// `identity`'s release, with `shipped`'s entries in place of its own in
/// that one platform's archive, while the manifest still names the kit's
/// own files.
fn release(identity: &ReleaseIdentity, shipped: Option<(&str, Vec<Entry>)>) -> Fixture {
    let mut platforms = serde_json::Map::new();
    let mut archives = BTreeMap::new();
    for name in CONTRACT.kits.keys() {
        let listed = entries(name);
        let archived = match &shipped {
            Some((target, shipped)) if *target == name.as_str() => shipped.clone(),
            _ => listed.clone(),
        };
        let bytes = archive(&archived);
        let files: serde_json::Map<_, _> = listed
            .iter()
            .map(|(file, _, data)| {
                (
                    file.clone(),
                    serde_json::json!({"sha256": hex_digest(data), "size": data.len()}),
                )
            })
            .collect();
        let asset = CONTRACT.asset(name, identity.version);
        platforms.insert(
            name.clone(),
            serde_json::json!({"asset": asset, "sha256": hex_digest(&bytes), "size": bytes.len(), "files": files}),
        );
        archives.insert(asset, bytes);
    }
    Fixture {
        manifest: serde_json::json!({
            "format": CONTRACT.format,
            "version": identity.version,
            "commit": identity.commit.unwrap(),
            "order": identity.order.to_string(),
            "platforms": platforms,
        }),
        archives,
    }
}

impl Fixture {
    fn routes(&self) -> BTreeMap<String, (u16, Vec<u8>)> {
        let mut routes: BTreeMap<_, _> = self
            .archives
            .iter()
            .map(|(asset, bytes)| (asset.clone(), (200, bytes.clone())))
            .collect();
        routes.insert(
            CONTRACT.manifest.clone(),
            (200, serde_json::to_vec(&self.manifest).unwrap()),
        );
        routes
    }
}

fn split(platform: &str) -> (&str, &str) {
    platform.split_once('-').unwrap()
}

fn cache_root(home: &std::path::Path) -> std::path::PathBuf {
    hide_kit::layout::device_bundle_cache(home)
}

/// Every platform the contract carries is fetched and checked by the same
/// code path: its files under their names, the programs first, each marked
/// as the device runs it; then reused with no network at all, the manifest
/// included, while the cache keeps all of them.
#[test]
fn every_platform_kit_is_fetched_checked_and_reused_offline() {
    let home = tempfile::tempdir().unwrap();
    let fixture = release(&BUILD, None);
    let server = Release::serve(fixture.routes());
    for name in CONTRACT.kits.keys() {
        let (os, arch) = split(name);
        let files = cache::load(&BUILD, os, arch, home.path(), &server.base()).unwrap();
        let kit = &CONTRACT.kits[name];
        let mut expected: Vec<(String, bool)> = CONTRACT
            .programs
            .iter()
            .map(|program| (kit.program(program), true))
            .collect();
        expected.extend(
            kit.runtime
                .iter()
                .map(|(file, &program)| (file.clone(), program)),
        );
        assert_eq!(
            files
                .iter()
                .map(|file| (file.relative.clone(), file.executable))
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        assert_eq!(files[0].relative, kit.program("hided"));
    }
    assert_eq!(server.hits(&CONTRACT.manifest), 1);
    assert_eq!(server.archive_hits(), CONTRACT.kits.len());
    for name in CONTRACT.kits.keys() {
        let (os, arch) = split(name);
        cache::load(&BUILD, os, arch, home.path(), "http://127.0.0.1:0").unwrap();
        assert!(cache_root(home.path()).join(name).is_dir());
    }
    assert!(!cache_root(home.path()).join("staging").exists());
}

/// A manifest that names another version, commit or build order than this
/// build was compiled with is refused before any archive is fetched, and
/// nothing of it is kept.
#[test]
fn a_manifest_of_another_build_is_refused_before_any_archive() {
    let others = [
        ReleaseIdentity {
            version: "1.2.4",
            ..BUILD
        },
        ReleaseIdentity {
            commit: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            ..BUILD
        },
        ReleaseIdentity { order: 43, ..BUILD },
    ];
    for other in others {
        let home = tempfile::tempdir().unwrap();
        let fixture = release(&other, None);
        let mut routes = fixture.routes();
        // Served where this build looks, so only the identity can refuse it.
        for (asset, bytes) in &fixture.archives {
            routes.insert(
                asset.replace(other.version, BUILD.version),
                (200, bytes.clone()),
            );
        }
        let server = Release::serve(routes);
        let reason = format!(
            "{:#}",
            cache::load(&BUILD, "linux", "aarch64", home.path(), &server.base())
                .err()
                .unwrap()
        );
        assert!(
            reason.contains("names another build"),
            "{other:?}: {reason}"
        );
        assert_eq!(server.archive_hits(), 0, "{other:?}");
        let root = cache_root(home.path());
        assert!(!root.join(&CONTRACT.manifest).exists());
        assert!(!root.join("linux-aarch64").exists());
    }
}

/// A program with another platform's header, an entry the manifest does
/// not list, a link, a folder entry, a duplicate and a changed program are
/// each refused, and no slot or staging folder is left; the release's own
/// archive then succeeds.
#[test]
fn a_wrong_header_or_an_unlisted_entry_is_refused_and_never_cached() {
    let windows = "windows-x86_64";
    for defect in [
        "header",
        "unlisted",
        "link",
        "folder",
        "duplicate",
        "changed",
    ] {
        let home = tempfile::tempdir().unwrap();
        let mut shipped = entries(windows);
        let helper = shipped
            .iter()
            .position(|(name, _, _)| name == "hided.exe")
            .unwrap();
        let mut listed_override = None;
        match defect {
            // The manifest names these very bytes, so only the header check
            // can refuse them.
            "header" => {
                let mut elf = program(super::contract::Header::Elf { machine: 62 });
                elf.extend_from_slice(b"hided.exe");
                shipped[helper] = ("hided.exe".to_owned(), tar::EntryType::Regular, elf);
                listed_override = Some(shipped.clone());
            }
            "unlisted" => {
                shipped.push(("extra".to_owned(), tar::EntryType::Regular, b"x".to_vec()))
            }
            "link" => {
                shipped[helper] = ("hided.exe".to_owned(), tar::EntryType::Symlink, Vec::new())
            }
            "folder" => shipped.insert(
                0,
                (
                    "holder-conpty/".to_owned(),
                    tar::EntryType::Directory,
                    Vec::new(),
                ),
            ),
            "duplicate" => shipped.push(shipped[helper].clone()),
            _ => shipped[helper].2[100] ^= 1,
        }
        let mut fixture = release(&BUILD, Some((windows, shipped)));
        if let Some(listed) = listed_override {
            let files = &mut fixture.manifest["platforms"][windows]["files"];
            for (name, _, bytes) in &listed {
                files[name] = serde_json::json!({"sha256": hex_digest(bytes), "size": bytes.len()});
            }
        }
        let server = Release::serve(fixture.routes());
        let reason = format!(
            "{:#}",
            cache::load(&BUILD, "windows", "x86_64", home.path(), &server.base())
                .err()
                .unwrap()
        );
        let root = cache_root(home.path());
        assert!(!root.join(windows).exists(), "{defect}: {reason}");
        assert!(!root.join("staging").exists(), "{defect}");
        if defect == "header" {
            assert!(reason.contains("another platform"), "{reason}");
        }
    }
    let home = tempfile::tempdir().unwrap();
    let server = Release::serve(release(&BUILD, None).routes());
    assert!(cache::load(&BUILD, "windows", "x86_64", home.path(), &server.base()).is_ok());
}

#[test]
fn wrong_archive_digest_or_size_is_never_cached_and_retry_can_succeed() {
    let good = release(&BUILD, None);
    let asset = CONTRACT.asset("linux-aarch64", BUILD.version);
    for defect in ["digest", "size", "truncated", "missing"] {
        let home = tempfile::tempdir().unwrap();
        let mut routes = good.routes();
        let (status, bytes) = routes.get_mut(&asset).unwrap();
        match defect {
            "digest" => bytes[4] ^= 1,
            "size" => bytes.push(0),
            "truncated" => {
                bytes.pop();
            }
            _ => *status = 404,
        }
        let failure = Release::serve(routes);
        assert!(
            cache::load(&BUILD, "linux", "aarch64", home.path(), &failure.base()).is_err(),
            "{defect}"
        );
        let root = cache_root(home.path());
        assert!(!root.join("linux-aarch64").exists());
        assert!(!root.join("staging").exists());
        let retry = Release::serve(good.routes());
        assert!(cache::load(&BUILD, "linux", "aarch64", home.path(), &retry.base()).is_ok());
    }
}

/// A cached kit whose bytes changed is fetched again before anything of
/// it is sent, and a slot an earlier format left (its files named
/// `<name>-<platform>`) is replaced by this release's.
#[test]
fn a_changed_or_old_slot_is_replaced_before_upload() {
    let home = tempfile::tempdir().unwrap();
    let server = Release::serve(release(&BUILD, None).routes());
    let first = cache::load(&BUILD, "linux", "aarch64", home.path(), &server.base()).unwrap();
    let root = cache_root(home.path());
    fs::write(root.join("linux-aarch64/hide"), "corrupt").unwrap();
    let repaired = cache::load(&BUILD, "linux", "aarch64", home.path(), &server.base()).unwrap();
    assert_eq!(
        first.iter().map(|f| &f.digest).collect::<Vec<_>>(),
        repaired.iter().map(|f| &f.digest).collect::<Vec<_>>()
    );
    assert_eq!(server.archive_hits(), 2);

    let old = root.join("linux-x86_64");
    hide_platform::fs::private::create_dir(&old).unwrap();
    fs::write(old.join("hided-linux-x86_64"), "format 3").unwrap();
    cache::load(&BUILD, "linux", "x86_64", home.path(), &server.base()).unwrap();
    assert!(!old.join("hided-linux-x86_64").exists());
    assert!(old.join("hided").is_file());
}

#[test]
fn a_failed_download_preserves_its_reason_and_reports_failed_staging_cleanup() {
    let home = tempfile::tempdir().unwrap();
    let root = cache_root(home.path());
    let stage = root.join("staging");
    let blocked = stage.clone();
    let asset = CONTRACT.asset("linux-aarch64", BUILD.version);
    let mut routes = release(&BUILD, None).routes();
    routes.remove(&asset);
    // Change the real filesystem at the HTTP boundary. Cleanup must refuse
    // this non-directory instead of deleting an unrecognized object.
    let server = Release::serve_with(routes, move |path| {
        if path.ends_with(".tar.gz") {
            fs::remove_dir_all(&blocked).unwrap();
            fs::write(&blocked, b"unexpected staging object").unwrap();
        }
    });
    let (result, records) = crate::diagnostics::capture(|| {
        cache::load(&BUILD, "linux", "aarch64", home.path(), &server.base())
    });
    let reason = format!("{:#}", result.err().unwrap());
    assert!(
        reason.contains("device asset could not be downloaded"),
        "{reason}"
    );
    assert!(reason.contains("staging cleanup failed"), "{reason}");
    assert!(reason.contains("not a folder"), "{reason}");
    assert!(!root.join("linux-aarch64").exists());
    assert_eq!(fs::read(&stage).unwrap(), b"unexpected staging object");
    let record = records.iter().find(|r| r["phase"] == "finish").unwrap();
    assert_eq!(record["kind"], "host.device_bundle_cleanup_failed");
    assert_eq!(record["platform"], "linux-aarch64");
    assert_eq!(record["version"], "1.2.3");
    assert!(
        record["primary_error"]
            .as_str()
            .unwrap()
            .contains("device asset could not be downloaded")
    );
}

/// The cache keeps within its budget by removing the least recently used
/// slot first, a slot no record names before any it does, and never more
/// than the incoming kit needs.
#[test]
fn the_cache_evicts_the_least_recently_used_kit_past_its_budget() {
    let mib = 1024 * 1024;
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    let mut sizes: BTreeMap<String, u64> = [("a", 300 * mib), ("b", 300 * mib), ("c", 300 * mib)]
        .into_iter()
        .map(|(name, size)| (name.to_owned(), size))
        .collect();
    // Most recently used first: `b`, then `a`; `c` is named by no record.
    let recent = ["b".to_owned(), "a".to_owned()];
    cache::evict(root.path(), &mut sizes, &recent, 300 * mib).unwrap();
    assert!(!root.path().join("c").exists());
    assert!(root.path().join("a").exists() && root.path().join("b").exists());
    cache::evict(root.path(), &mut sizes, &recent, 500 * mib).unwrap();
    assert!(!root.path().join("a").exists());
    assert!(root.path().join("b").exists());
    assert_eq!(sizes.keys().collect::<Vec<_>>(), ["b"]);
}

/// A build that no release was published for refuses another platform's
/// kit before any download, and its reason names the developer folder and
/// the file names that folder takes.
#[test]
fn a_local_build_names_the_developer_folder_instead_of_downloading() {
    let folder = tempfile::tempdir().unwrap();
    let packages = HelperPackages::new(Some(folder.path().to_path_buf()), UNPUBLISHED);
    let own = platform(std::env::consts::OS, std::env::consts::ARCH);
    let foreign = CONTRACT.kits.keys().find(|name| **name != own).unwrap();
    let (os, arch) = split(foreign);
    let Err(EstablishError::Unsupported(reason)) = packages.payload_for(os, arch) else {
        panic!("a local build reached for a release kit");
    };
    assert!(reason.contains("no published release"), "{reason}");
    assert!(
        reason.contains(&folder.path().display().to_string()),
        "{reason}"
    );
    assert!(reason.contains(&format!("<name>-{foreign}")), "{reason}");
}

/// The Windows kit marks its files as programs exactly as a Windows core
/// reads its own folder, so a Windows device holds one version folder for
/// the same bytes whichever core sent them.
#[test]
fn the_windows_kit_marks_its_programs_as_a_windows_core_reads_them() {
    let kit = &CONTRACT.kits["windows-x86_64"];
    for (name, program) in kit.files(&CONTRACT.programs) {
        assert_eq!(program_by_contract(&name), Some(program), "{name}");
    }
}
