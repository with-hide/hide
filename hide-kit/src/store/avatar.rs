//! The picture of a GitHub skill's owner, for the screen's icon tile: fetched
//! once from github.com when a source of that owner is read or checked, kept
//! in the store's `avatars` folder and reported with the store, so the screen
//! never reaches the web for it and shows it offline. A fetch that fails
//! leaves the tile's GitHub glyph, is logged, and is tried again on the next
//! read or check of that owner.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;

use super::log;

const FOLDER: &str = "avatars";
const DEADLINE: Duration = Duration::from_secs(10);
/// A 64-pixel picture is a few KiB; anything past this is not one.
const BYTES_LIMIT: u64 = 64 * 1024;

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
    dir.join(FOLDER).join(format!("{owner}.img"))
}

/// Fetches `owner`'s picture unless the store already holds it.
pub(crate) fn ensure(dir: &Path, owner: &str) {
    let path = path(dir, owner);
    if path.is_file() {
        return;
    }
    let saved = fetch(owner).and_then(|bytes| {
        std::fs::create_dir_all(dir.join(FOLDER)).map_err(|error| error.to_string())?;
        hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
            .map(drop)
            .map_err(|error| error.to_string())
    });
    if let Err(reason) = saved {
        log("store.avatar_failed", None, &format!("{owner}: {reason}"));
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
}
