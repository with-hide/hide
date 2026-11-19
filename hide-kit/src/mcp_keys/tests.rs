//! The key file's contract (PRD skill-mcp-manager B10, B13, B18, D-11,
//! D-39): the account's own private file, each item's keys kept until a
//! permanent delete wipes them, and nothing it says quoting a key.

use super::*;

const KEY: &str = "ghp_k3y-in-the-file";

fn keys(pairs: &[(&str, &str)]) -> Keys {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Secret::new((*value).to_owned())))
        .collect()
}

#[test]
fn an_items_keys_live_in_one_private_file_until_a_permanent_delete_wipes_them() {
    let state = tempfile::tempdir().unwrap();
    let path = crate::layout::mcp_keys(state.path());
    assert!(super::keys(state.path(), "mcp:github").unwrap().is_empty());
    assert!(!path.exists());

    set(state.path(), "mcp:github", &keys(&[("GITHUB_TOKEN", KEY)])).unwrap();
    set(
        state.path(),
        "mcp:linear",
        &keys(&[("Authorization", "Bearer lin")]),
    )
    .unwrap();
    assert!(hide_platform::fs::private::is_private(&path).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let read = super::keys(state.path(), "mcp:github").unwrap();
    assert_eq!(read["GITHUB_TOKEN"].expose(), KEY);
    assert!(!format!("{read:?}").contains(KEY));
    assert_eq!(items(state.path()).unwrap(), ["mcp:github", "mcp:linear"]);

    // An edit replaces the item's keys whole.
    set(
        state.path(),
        "mcp:github",
        &keys(&[("GITHUB_TOKEN", "ghp_new")]),
    )
    .unwrap();
    assert_eq!(
        super::keys(state.path(), "mcp:github").unwrap()["GITHUB_TOKEN"].expose(),
        "ghp_new"
    );

    // A permanent delete wipes the item's keys, and the last one the file.
    wipe(state.path(), "mcp:github").unwrap();
    assert!(super::keys(state.path(), "mcp:github").unwrap().is_empty());
    assert!(!std::fs::read_to_string(&path).unwrap().contains("ghp_new"));
    set(state.path(), "mcp:linear", &Keys::new()).unwrap();
    assert!(!path.exists());
    wipe(state.path(), "mcp:absent").unwrap();
}

#[cfg(unix)]
#[test]
fn a_file_others_can_read_is_refused_and_no_failure_quotes_a_key() {
    use std::os::unix::fs::PermissionsExt;
    let state = tempfile::tempdir().unwrap();
    let path = crate::layout::mcp_keys(state.path());
    set(state.path(), "mcp:github", &keys(&[("GITHUB_TOKEN", KEY)])).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refused = super::keys(state.path(), "mcp:github").unwrap_err();
    assert!(
        matches!(refused, KeyFileError::NotPrivate { .. }),
        "{refused:?}"
    );
    assert!(!format!("{refused} {refused:?}").contains(KEY));

    // A file Hide cannot read as its own, with a key where a list belongs,
    // is refused without the parser's quote of it.
    std::fs::remove_file(&path).unwrap();
    hide_platform::fs::atomic::write_file(
        &path,
        format!("{{\"schema\": 1, \"items\": {{\"mcp:x\": \"{KEY}\"}}}}").as_bytes(),
        hide_platform::fs::Access::Private,
    )
    .unwrap();
    let unreadable = super::keys(state.path(), "mcp:x").unwrap_err();
    assert!(
        matches!(unreadable, KeyFileError::Unreadable { .. }),
        "{unreadable:?}"
    );
    assert!(!format!("{unreadable} {unreadable:?}").contains(KEY));
    let refused = set(state.path(), "mcp:y", &keys(&[("A", "b")])).unwrap_err();
    assert!(matches!(refused, KeyFileError::Unreadable { .. }));
    assert!(
        std::fs::read_to_string(&path).unwrap().contains(KEY),
        "a refused file stays as it was"
    );
}

#[test]
fn a_key_past_the_limits_is_refused_before_the_file_changes() {
    let state = tempfile::tempdir().unwrap();
    let long = "k".repeat(MAX_VALUE_BYTES + 1);
    let refused = set(state.path(), "mcp:x", &keys(&[("A", &long)])).unwrap_err();
    assert_eq!(refused, KeyFileError::TooLarge { what: "a key" });
    let many: Keys = (0..=MAX_KEYS_PER_ITEM)
        .map(|index| (format!("K{index}"), Secret::new("v".to_owned())))
        .collect();
    assert!(set(state.path(), "mcp:x", &many).is_err());
    assert!(!crate::layout::mcp_keys(state.path()).exists());
}
