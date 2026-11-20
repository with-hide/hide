//! What puts a core on a machine and starts it there, whoever asks: the
//! login item or fixture process that runs it (`starter`), the flags of
//! the commands run on that machine (`flags`), what the machine says about
//! itself first (`checks`), and the one owned `ssh` those commands and
//! their files go over (`remote`). A core move drives these (`core_move`),
//! and so does a core's update over SSH (`core_update`); none of them reads
//! a move's journal, its handover or its dialog. The start of a Hide on a
//! helper device's machine (PRD device-connect-iroh D-45, D-46) installs
//! nothing through here: the device helper's kit already holds the build,
//! `pairing_ssh` runs its `hide connect` on the device's own SSH session,
//! and of these parts it reads one check, the desktop session the target's
//! `hide device look-ssh` answers with (`checks::desktop_session`).

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
