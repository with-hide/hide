//! Claude's macOS keychain presence, the part no file rule can describe.
use hide_agent_adapter::Account;
use hide_node_link::profiles::ProfileLogin;

pub(super) fn keychain(account: &Account) -> Option<ProfileLogin> {
    #[cfg(target_os = "macos")]
    match crate::usage::claude_keychain_item(
        &crate::usage::claude_keychain_service(&account.key),
        false,
    ) {
        Ok(Some(_)) => return Some(ProfileLogin::SignedIn),
        Ok(None) => {}
        Err(_) => {
            super::log(account, "keychain_unreadable");
            return Some(ProfileLogin::Unknown);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = account;
    None
}
