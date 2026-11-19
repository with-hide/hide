//! What puts a core on a machine and starts it there, whoever asks: the
//! login item or fixture process that runs it (`starter`), the flags of
//! the commands run on that machine (`flags`), what the machine says about
//! itself first (`checks`), and the one owned `ssh` those commands and
//! their files go over (`remote`). A core move drives these (`core_move`),
//! and so do a core's update over SSH (`core_update`) and the start of a
//! Hide on a machine reached over SSH (PRD device-connect-iroh D-46); none
//! of them reads a move's journal, its handover or its dialog.

pub mod checks;
pub mod flags;
pub mod remote;
pub mod starter;

/// The longest intent id.
const MAX_INTENT: usize = 64;

/// The id one move or update runs under, which each of its commands on the
/// other machine names: lowercase letters, digits and `-`, at most 64.
pub fn checked_intent(intent: &str) -> Result<&str, String> {
    if intent.is_empty()
        || intent.len() > MAX_INTENT
        || !intent
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("{intent:?} is not an intent id"));
    }
    Ok(intent)
}

/// A fresh id for an intent of `kind`, such as `move` or `update`.
pub fn new_intent(kind: &str) -> String {
    let mut bytes = [0_u8; 8];
    getrandom::getrandom(&mut bytes).expect("getrandom");
    format!("{kind}-{}", hex::encode(bytes))
}
