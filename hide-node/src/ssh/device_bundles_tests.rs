use super::*;
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

/// The release HTTP boundary. Drop wakes and joins its accept loop, including
/// after an assertion fails; no test changes the process HOME or runtime.
struct Release {
    address: std::net::SocketAddr,
    hits: Arc<AtomicUsize>,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}
impl Release {
    fn serve(status: u16, body: Vec<u8>) -> Self {
        Self::serve_with(status, body, || {})
    }
    fn serve_with(
        status: u16,
        body: Vec<u8>,
        mut on_request: impl FnMut() + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
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
                assert!(
                    String::from_utf8_lossy(&request)
                        .starts_with("GET /hide-device-linux-aarch64-1.2.3.tar.gz ")
                );
                count.fetch_add(1, Ordering::SeqCst);
                on_request();
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
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
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

#[test]
fn a_failed_download_preserves_its_reason_and_reports_failed_staging_cleanup() {
    let home = tempfile::tempdir().unwrap();
    let root = hide_kit::layout::device_bundle_cache(home.path());
    let stage = root.join("staging");
    let blocked = stage.clone();
    let (manifest, _) = kit();
    // Change the real filesystem at the HTTP boundary. Cleanup must refuse
    // this non-directory instead of deleting an unrecognized object.
    let server = Release::serve_with(404, Vec::new(), move || {
        fs::remove_dir(&blocked).unwrap();
        fs::write(&blocked, b"unexpected staging object").unwrap();
    });
    let (result, records) = crate::diagnostics::capture(|| {
        load(&manifest, "linux-aarch64", home.path(), &server.base())
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
    assert!(
        record["cleanup_error"]
            .as_str()
            .unwrap()
            .contains("not a folder")
    );
}

fn program() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
    bytes[16] = 3;
    bytes[18] = 183;
    bytes
}

fn fixture(entries: &[(&str, tar::EntryType, Vec<u8>)]) -> (Manifest, Vec<u8>) {
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
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    let mut platforms = serde_json::Map::new();
    for platform in PLATFORMS {
        let files: BTreeMap<_, _> = PROGRAMS
            .iter()
            .map(|name| {
                (
                    *name,
                    serde_json::json!({"sha256": hex_digest(&program()), "size": 64}),
                )
            })
            .collect();
        platforms.insert(platform.to_owned(), serde_json::json!({
            "asset": format!("hide-device-{platform}-1.2.3.tar.gz"), "sha256": hex_digest(&bytes), "size": bytes.len(), "files": files,
        }));
    }
    let manifest = serde_json::from_value(serde_json::json!({"format": 1, "version": "1.2.3", "commit": "a".repeat(40), "order": "42", "platforms": platforms})).unwrap();
    (manifest, bytes)
}

fn kit() -> (Manifest, Vec<u8>) {
    fixture(
        &PROGRAMS
            .iter()
            .map(|name| (*name, tar::EntryType::Regular, program()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn a_verified_download_is_reused_offline_and_corruption_is_repaired_before_upload() {
    let home = tempfile::tempdir().unwrap();
    let (manifest, archive) = kit();
    manifest.validate().unwrap();
    let server = Release::serve(200, archive);
    let first = load(&manifest, "linux-aarch64", home.path(), &server.base()).unwrap();
    assert_eq!(first.files.len(), 4); // three programs plus the Herdr pin
    assert_eq!(first.files.iter().filter(|file| file.executable).count(), 3);
    assert!(first.missing.is_empty());
    assert!(first.herdr.as_ref().unwrap().executable);
    assert_eq!(server.hits(), 1);
    let second = load(
        &manifest,
        "linux-aarch64",
        home.path(),
        "http://127.0.0.1:0",
    )
    .unwrap();
    assert_eq!(first.version(), second.version());
    let root = hide_kit::layout::device_bundle_cache(home.path());
    fs::write(root.join("linux-aarch64/hide-linux-aarch64"), "corrupt").unwrap();
    let repaired = load(&manifest, "linux-aarch64", home.path(), &server.base()).unwrap();
    assert_eq!(first.version(), repaired.version());
    assert_eq!(server.hits(), 2);
    assert!(!root.join("staging").exists());
}

#[test]
fn wrong_archive_digest_or_size_is_never_cached_and_retry_can_succeed() {
    for defect in ["digest", "size", "truncated", "missing"] {
        let home = tempfile::tempdir().unwrap();
        let (manifest, archive) = kit();
        let mut bad = archive.clone();
        let status = match defect {
            "digest" => {
                bad[4] ^= 1;
                200
            }
            "size" => {
                bad.push(0);
                200
            }
            "truncated" => {
                bad.pop();
                200
            }
            _ => 404,
        };
        let failure = Release::serve(status, bad);
        assert!(
            load(&manifest, "linux-aarch64", home.path(), &failure.base()).is_err(),
            "{defect}"
        );
        let root = hide_kit::layout::device_bundle_cache(home.path());
        assert!(!root.join("linux-aarch64").exists());
        assert!(!root.join("staging").exists());
        let retry = Release::serve(200, archive);
        assert!(load(&manifest, "linux-aarch64", home.path(), &retry.base()).is_ok());
    }
}

#[test]
fn archive_links_duplicates_unexpected_paths_and_changed_programs_are_rejected() {
    for defect in ["link", "duplicate", "path", "program"] {
        let home = tempfile::tempdir().unwrap();
        let mut entries: Vec<_> = PROGRAMS
            .iter()
            .map(|name| (*name, tar::EntryType::Regular, program()))
            .collect();
        match defect {
            "link" => entries[0] = ("hided", tar::EntryType::Symlink, Vec::new()),
            "duplicate" => entries.push(entries[0].clone()),
            "path" => entries[0].0 = "sub/hided",
            _ => entries[0].2[63] = 1,
        }
        let (manifest, archive) = fixture(&entries);
        let server = Release::serve(200, archive);
        assert!(
            load(&manifest, "linux-aarch64", home.path(), &server.base()).is_err(),
            "{defect}"
        );
        let root = hide_kit::layout::device_bundle_cache(home.path());
        assert!(!root.join("linux-aarch64").exists());
        assert!(!root.join("staging").exists());
    }
}

#[test]
fn manifest_rejects_wrong_release_names_missing_parts_and_resource_overruns() {
    let (mut manifest, _) = kit();
    manifest.platforms.get_mut("linux-aarch64").unwrap().asset = "another-release.tar.gz".into();
    assert!(manifest.validate().is_err());
    let (mut manifest, _) = kit();
    manifest
        .platforms
        .get_mut("linux-aarch64")
        .unwrap()
        .files
        .remove("herdr");
    assert!(manifest.validate().is_err());
    let (mut manifest, _) = kit();
    manifest
        .platforms
        .get_mut("linux-aarch64")
        .unwrap()
        .files
        .get_mut("herdr")
        .unwrap()
        .size = MAX_BUNDLE;
    assert!(manifest.validate().is_err());
}

#[test]
fn a_local_build_fails_on_the_device_path_without_attempting_a_release_download() {
    let directory = tempfile::tempdir().unwrap();
    let (manifest, _) = kit();
    let mut value = serde_json::json!({"format": 1, "version": "1.2.3-main.abcdef0", "commit": manifest.commit, "order": manifest.order, "platforms": {}});
    let mut platforms = serde_json::Map::new();
    for platform in PLATFORMS {
        let files: BTreeMap<_, _> = PROGRAMS
            .iter()
            .map(|name| {
                (
                    *name,
                    serde_json::json!({"sha256": hex_digest(&program()), "size": 64}),
                )
            })
            .collect();
        platforms.insert(platform.into(), serde_json::json!({"asset": format!("hide-device-{platform}-1.2.3-main.abcdef0.tar.gz"), "sha256": "a".repeat(64), "size": 200, "files": files}));
    }
    value["platforms"] = serde_json::Value::Object(platforms);
    fs::write(
        directory.path().join(MANIFEST),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let reason = payload(Some(directory.path()), "linux", "aarch64")
        .err()
        .unwrap();
    assert!(
        reason.contains("local/main Hide build has no matching device release"),
        "{reason}"
    );
    assert!(reason.contains("Retry"));
}
