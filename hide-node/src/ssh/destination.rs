//! What a device's SSH destination and Herdr session may be, checked before
//! anything reaches `ssh`.

const MAX_FIELD_BYTES: usize = 2048;

pub(super) fn validate_session(session: &str) -> Result<(), String> {
    if !hide_node_link::device::valid_herdr_session(session) {
        return Err("The saved Herdr session name is invalid. Fix the profile in Herdr".to_owned());
    }
    Ok(())
}

/// OpenSSH destination forms: an alias or host, `user@host`, or an
/// `ssh://` address. A literal hostname never requires a Host row, and
/// nothing that `ssh` would read as an option is one.
pub(super) struct Target;

impl Target {
    pub fn parse(target: &str) -> Result<(), String> {
        let bad = || "The saved SSH target is not a supported SSH destination".to_owned();
        if target.is_empty()
            || target.len() > MAX_FIELD_BYTES
            || target.starts_with('-')
            || target.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(bad());
        }
        let uri = target.starts_with("ssh://");
        let destination = target.strip_prefix("ssh://").unwrap_or(target);
        let address = match destination.rsplit_once('@') {
            Some((user, address)) if !user.is_empty() && !user.contains(['@', '/', ':']) => address,
            Some(_) => return Err(bad()),
            None => destination,
        };
        let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
            let (host, rest) = bracketed.split_once(']').ok_or_else(bad)?;
            let port = if rest.is_empty() {
                None
            } else {
                Some(
                    rest.strip_prefix(':')
                        .ok_or_else(bad)?
                        .parse::<u16>()
                        .map_err(|_| bad())?,
                )
            };
            (host, port)
        } else if uri {
            match address.rsplit_once(':') {
                Some((host, port)) => (host, Some(port.parse::<u16>().map_err(|_| bad())?)),
                None => (address, None),
            }
        } else {
            (address, None)
        };
        if host.is_empty()
            || host.starts_with('-')
            || host.contains(['/', '@', '[', ']'])
            || port == Some(0)
        {
            return Err(bad());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A destination `ssh` would read as an option, or one with a zero
    /// port, is never one; nor is a session that names a path.
    #[test]
    fn only_a_destination_and_a_plain_session_pass() {
        assert!(Target::parse("ssh://grab@box:2222").is_ok());
        assert!(Target::parse("-oProxyCommand=x").is_err());
        assert!(Target::parse("ssh://u@host:0").is_err());
        assert!(validate_session("../default").is_err());
        assert!(validate_session("").is_err());
    }
}
