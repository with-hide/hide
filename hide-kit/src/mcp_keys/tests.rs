//! The key file's contract (PRD skill-mcp-manager B10, B13, B18, D-11,
//! D-39): the account's own private file in an item's folder, moving with
//! the folder, and nothing it says quoting a key.

use super::*;

const KEY: &str = "ghp_k3y-in-the-file";

fn keys(pairs: &[(&str, &str)]) -> Keys {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Secret::new((*value).to_owned())))
        .collect()
}

#[test]
fn an_items_keys_live_in_a_private_file_of_its_folder_and_go_with_it() {
    let store = tempfile::tempdir().unwrap();
    let item = store.path().join("items/mcp-github");
    let path = item.join(FILE);
    assert!(read(&item).unwrap().is_empty());
    assert!(!item.exists(), "a read makes nothing");

    write(&item, &keys(&[("GITHUB_TOKEN", KEY)])).unwrap();
    assert!(hide_platform::fs::private::is_private(&path).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let kept = read(&item).unwrap();
    assert_eq!(kept["GITHUB_TOKEN"].expose(), KEY);
    assert!(!format!("{kept:?}").contains(KEY));

    // The same keys again change no byte; an edit replaces them whole.
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    write(&item, &keys(&[("GITHUB_TOKEN", KEY)])).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    write(&item, &keys(&[("GITHUB_TOKEN", "ghp_new")])).unwrap();
    assert_eq!(read(&item).unwrap()["GITHUB_TOKEN"].expose(), "ghp_new");

    // The folder moves to the undo list and back with its keys, and a
    // permanent delete takes them with it.
    let trash = store.path().join("trash/mcp-github-1");
    std::fs::create_dir_all(trash.parent().unwrap()).unwrap();
    std::fs::rename(&item, &trash).unwrap();
    assert!(read(&item).unwrap().is_empty());
    assert_eq!(read(&trash).unwrap()["GITHUB_TOKEN"].expose(), "ghp_new");
    std::fs::remove_dir_all(&trash).unwrap();
    assert!(read(&trash).unwrap().is_empty());

    // No keys is no file.
    write(&item, &keys(&[("A", "b")])).unwrap();
    write(&item, &Keys::new()).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn a_file_others_can_read_is_refused_and_no_failure_quotes_a_key() {
    use std::os::unix::fs::PermissionsExt;
    let store = tempfile::tempdir().unwrap();
    let item = store.path().join("mcp-github");
    let path = item.join(FILE);
    write(&item, &keys(&[("GITHUB_TOKEN", KEY)])).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refused = read(&item).unwrap_err();
    assert!(
        matches!(refused, KeyFileError::NotPrivate { .. }),
        "{refused:?}"
    );
    assert!(!format!("{refused} {refused:?}").contains(KEY));

    // A file Hide cannot read as its own, with a key where a table belongs,
    // is refused without the parser's quote of it, and is not replaced.
    std::fs::remove_file(&path).unwrap();
    hide_platform::fs::atomic::write_file(
        &path,
        format!("{{\"schema\": 1, \"keys\": \"{KEY}\"}}").as_bytes(),
        hide_platform::fs::Access::Private,
    )
    .unwrap();
    let unreadable = read(&item).unwrap_err();
    assert!(
        matches!(unreadable, KeyFileError::Unreadable { .. }),
        "{unreadable:?}"
    );
    assert!(!format!("{unreadable} {unreadable:?}").contains(KEY));
    let refused = write(&item, &keys(&[("A", "b")])).unwrap_err();
    assert!(matches!(refused, KeyFileError::Unreadable { .. }));
    assert!(
        std::fs::read_to_string(&path).unwrap().contains(KEY),
        "a refused file stays as it was"
    );
}

#[test]
fn a_key_past_the_limits_is_refused_before_the_file_changes() {
    let store = tempfile::tempdir().unwrap();
    let item = store.path().join("mcp-x");
    let long = "k".repeat(MAX_VALUE_BYTES + 1);
    let refused = write(&item, &keys(&[("A", &long)])).unwrap_err();
    assert_eq!(refused, KeyFileError::TooLarge { what: "a key" });
    let many: Keys = (0..=MAX_KEYS_PER_ITEM)
        .map(|index| (format!("K{index}"), Secret::new("v".to_owned())))
        .collect();
    assert!(write(&item, &many).is_err());
    assert!(!item.exists());
}
