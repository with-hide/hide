//! The identity a listing shows and a trash checks (PRD S5.5 B17, B18), on
//! every system: an item replaced while the prompt was open is refused, and
//! no system answers "unchanged" for want of an identity.

use std::fs;
use std::path::Path;

use hide_host::list::list;
use hide_host::mutate::trash;
use hide_host::{ErrorCode, Root};

fn inode_of(root: &Root, name: &str) -> u64 {
    list(root.dir(), Path::new(""), root.real_path(), None)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
        .inode
}

#[test]
fn an_item_has_one_identity_until_it_is_replaced() {
    let outer = tempfile::tempdir().unwrap();
    let checkout = outer.path().join("checkout");
    fs::create_dir(&checkout).unwrap();
    fs::write(checkout.join("a.txt"), "a").unwrap();
    fs::write(checkout.join("b.txt"), "b").unwrap();
    let root = Root::open(&checkout).unwrap();

    let confirmed = inode_of(&root, "a.txt");
    assert_eq!(inode_of(&root, "a.txt"), confirmed, "listing twice agrees");
    assert_ne!(confirmed, inode_of(&root, "b.txt"), "two items differ");

    fs::rename(checkout.join("a.txt"), checkout.join("old.txt")).unwrap();
    fs::write(checkout.join("a.txt"), "replacement").unwrap();
    assert_ne!(inode_of(&root, "a.txt"), confirmed);

    let refused = trash(&root, Path::new("a.txt"), Some(confirmed)).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    assert_eq!(
        refused.message,
        "a.txt changed while the prompt was open; nothing was moved"
    );
    assert_eq!(
        fs::read_to_string(checkout.join("a.txt")).unwrap(),
        "replacement"
    );
    assert_eq!(fs::read_to_string(checkout.join("old.txt")).unwrap(), "a");
}
