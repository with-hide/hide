//! The picture of a GitHub skill's owner, for the screen's icon tile: fetched
//! once from github.com when a source of that owner is read or checked, kept
//! in the store's `avatars` folder and reported with the store, so the screen
//! never reaches the web for it and shows it offline. A fetch that fails
//! leaves the tile's GitHub glyph, is logged, and is tried again on the next
//! read or check of that owner.
//!
//! A picture is kept only while an item of the store or a source answer the
//! store still holds belongs to its owner (`keep_only`, each pass): a source
//! read and never installed loses its picture with its answer. The folder
//! holds at most [`COUNT_LIMIT`] pictures and [`TOTAL_LIMIT`] bytes; a new
//! one makes room by dropping unused ones, oldest first, and is not kept when
//! every one kept is in use (engineering rule 15).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;

use super::log;

const FOLDER: &str = "avatars";
const DEADLINE: Duration = Duration::from_secs(10);
/// A 64-pixel picture is a few KiB; anything past this is not one.
const BYTES_LIMIT: u64 = 64 * 1024;
/// The most pictures the folder keeps.
pub(crate) const COUNT_LIMIT: usize = 64;
/// The most bytes the folder's pictures take together.
pub(crate) const TOTAL_LIMIT: u64 = 1024 * 1024;
const SUFFIX: &str = ".img";

/// The GitHub account a stored git address belongs to (`https://github.com/<owner>/…`
/// or `git@github.com:<owner>/…`, the two spellings the store keeps), or
/// `None` for any other host.
pub(crate) fn owner(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let owner = rest.split('/').next()?;
    // GitHub's account names: letters, digits and hyphens, at most 39.
    (!owner.is_empty()
        && owner.len() <= 39
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then_some(owner)
}

fn path(dir: &Path, owner: &str) -> PathBuf {
    dir.join(FOLDER).join(format!("{owner}{SUFFIX}"))
}

/// Fetches `owner`'s picture unless the store already holds it; `used` are
/// the owners whose pictures must stay (this one among them).
pub(crate) fn ensure(dir: &Path, owner: &str, used: &BTreeSet<&str>) {
    if path(dir, owner).is_file() {
        return;
    }
    let saved = fetch(owner).and_then(|bytes| keep(dir, owner, &bytes, used));
    if let Err(reason) = saved {
        log("store.avatar_failed", None, &format!("{owner}: {reason}"));
    }
}

/// One picture kept in the folder: its owner, size and when it was written.
struct Kept {
    owner: String,
    bytes: u64,
    written: std::time::SystemTime,
}

fn kept(dir: &Path) -> Vec<Kept> {
    let Ok(entries) = std::fs::read_dir(dir.join(FOLDER)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let owner = name.strip_suffix(SUFFIX)?.to_owned();
            let metadata = entry.metadata().ok()?;
            Some(Kept {
                owner,
                bytes: metadata.len(),
                written: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            })
        })
        .collect()
}

/// Writes `owner`'s picture once the folder has room for it, dropping the
/// oldest pictures no one in `used` needs; refused when only used ones are
/// left and it still does not fit.
fn keep(dir: &Path, owner: &str, bytes: &[u8], used: &BTreeSet<&str>) -> Result<(), String> {
    let mut kept = kept(dir);
    kept.sort_by_key(|picture| picture.written);
    let size = bytes.len() as u64;
    let fits = |kept: &[Kept]| {
        kept.len() < COUNT_LIMIT
            && kept.iter().map(|picture| picture.bytes).sum::<u64>() + size <= TOTAL_LIMIT
    };
    while !fits(&kept) {
        let Some(oldest) = kept
            .iter()
            .position(|picture| !used.contains(picture.owner.as_str()))
        else {
            return Err(format!(
                "the {COUNT_LIMIT} pictures or {TOTAL_LIMIT} bytes the store keeps are all in use"
            ));
        };
        let dropped = kept.remove(oldest);
        std::fs::remove_file(path(dir, &dropped.owner)).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(dir.join(FOLDER)).map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file(
        &path(dir, owner),
        bytes,
        hide_platform::fs::Access::Private,
    )
    .map(drop)
    .map_err(|error| error.to_string())
}

/// Drops every kept picture whose owner is not in `used`, under the
/// account lock each pass.
pub(crate) fn keep_only(dir: &Path, used: &BTreeSet<&str>) {
    for picture in kept(dir) {
        if !used.contains(picture.owner.as_str())
            && let Err(error) = std::fs::remove_file(path(dir, &picture.owner))
        {
            log(
                "store.avatar_failed",
                None,
                &format!("{}: {error}", picture.owner),
            );
        }
    }
}

fn fetch(owner: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DEADLINE))
        .build()
        .into();
    let mut response = agent
        .get(&format!("https://github.com/{owner}.png?size=64"))
        .call()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(BYTES_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > BYTES_LIMIT {
        return Err(format!("the picture is over {BYTES_LIMIT} bytes"));
    }
    kind(&bytes).ok_or_else(|| "the answer is not a PNG or JPEG picture".to_owned())?;
    Ok(bytes)
}

/// The picture's media type, read from its first bytes.
fn kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The pictures the store holds of `owners`, as data addresses the screen
/// draws as they are; an owner with none is left out.
pub(crate) fn reported<'a>(
    dir: Option<&Path>,
    owners: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    let Some(dir) = dir else {
        return BTreeMap::new();
    };
    owners
        .into_iter()
        .filter_map(|owner| {
            let bytes = std::fs::read(path(dir, owner)).ok()?;
            let kind = kind(&bytes)?;
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Some((owner.to_owned(), format!("data:{kind};base64,{data}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_is_read_from_both_spellings_the_store_keeps_and_no_other_host() {
        assert_eq!(
            owner("https://github.com/anthropics/skills.git"),
            Some("anthropics")
        );
        assert_eq!(owner("git@github.com:acme-co/kit.git"), Some("acme-co"));
        assert_eq!(owner("https://gitlab.com/acme/kit.git"), None);
        assert_eq!(owner("file:///srv/archify.git"), None);
        assert_eq!(owner("https://github.com/../kit.git"), None);
    }

    #[test]
    fn a_kept_picture_is_reported_as_a_data_address_and_anything_else_is_left_out() {
        let dir = std::env::temp_dir().join(super::super::source::fresh_id("avatar-"));
        std::fs::create_dir_all(dir.join(FOLDER)).unwrap();
        std::fs::write(path(&dir, "acme"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        std::fs::write(path(&dir, "broken"), b"<html>").unwrap();
        let shown = reported(Some(&dir), ["acme", "broken", "missing"]);
        assert_eq!(shown.keys().collect::<Vec<_>>(), vec!["acme"]);
        assert!(shown["acme"].starts_with("data:image/png;base64,"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(super::super::source::fresh_id("avatar-"));
        std::fs::create_dir_all(dir.join(FOLDER)).unwrap();
        dir
    }

    fn owners(dir: &Path) -> BTreeSet<String> {
        kept(dir).into_iter().map(|picture| picture.owner).collect()
    }

    const PICTURE: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    #[test]
    fn a_picture_no_item_or_kept_answer_uses_is_dropped_and_the_used_ones_stay() {
        let dir = folder();
        for owner in ["acme", "gone"] {
            std::fs::write(path(&dir, owner), PICTURE).unwrap();
        }
        keep_only(&dir, &BTreeSet::from(["acme"]));
        assert_eq!(owners(&dir), BTreeSet::from(["acme".to_owned()]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_folder_drops_its_oldest_unused_picture_first_and_keeps_no_new_one_when_all_are_used()
    {
        let dir = folder();
        let names = (0..COUNT_LIMIT)
            .map(|n| format!("o{n:02}"))
            .collect::<Vec<_>>();
        for name in &names {
            std::fs::write(path(&dir, name), PICTURE).unwrap();
            // Each one written after the last, so their age is their order.
            let at = std::time::UNIX_EPOCH
                + Duration::from_secs(
                    1_000 + names.iter().position(|kept| kept == name).unwrap() as u64,
                );
            std::fs::File::options()
                .write(true)
                .open(path(&dir, name))
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        // o00 is the oldest, but in use: o01 goes.
        let used = BTreeSet::from(["o00", "new"]);
        keep(&dir, "new", PICTURE, &used).unwrap();
        let now = owners(&dir);
        assert_eq!(now.len(), COUNT_LIMIT);
        assert!(now.contains("o00") && now.contains("new") && !now.contains("o01"));

        let every = now
            .iter()
            .map(String::as_str)
            .chain(["late"])
            .collect::<BTreeSet<_>>();
        assert!(keep(&dir, "late", PICTURE, &every).is_err());
        assert!(!path(&dir, "late").exists());
        assert_eq!(owners(&dir).len(), COUNT_LIMIT);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_folder_keeps_no_more_bytes_than_its_limit() {
        let dir = folder();
        let big = vec![0u8; (TOTAL_LIMIT / 2) as usize];
        std::fs::write(path(&dir, "first"), &big).unwrap();
        std::fs::write(path(&dir, "second"), &big).unwrap();
        keep(&dir, "third", PICTURE, &BTreeSet::from(["second", "third"])).unwrap();
        assert_eq!(
            owners(&dir),
            BTreeSet::from(["second".to_owned(), "third".to_owned()])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
