//! The one-time link a host shares (PRD B1, D-05).
//!
//! `hide://pair#v=1&id=<host key>&dev=<host device>&relay=<relay URL>&s=<secret>`,
//! with one `a=<address>` for each direct address the host listens on, so
//! two machines on one network pair with no relay and no discovery.
//! Everything is after the `#`: a program that takes the text for a URL
//! sends none of it anywhere.
//!
//! The text is a credential until it is used or its nine minutes are over.
//! Nothing here puts it, or the secret, into an error or a `Debug` line, and
//! a reader keeps the parsed link, never the text.

use std::fmt;
use std::io;
use std::net::SocketAddr;

use hide_peer_protocol::{Candidate, PeerKey};
use zeroize::{Zeroize, Zeroizing};

use crate::limits;

const PREFIX: &str = "hide://pair#";

/// The link format this build writes and reads.
pub const VERSION: &str = "1";

pub const SECRET_BYTES: usize = 32;

/// An offer's secret: what a link carries and a guest shows the host.
#[derive(Clone)]
pub struct Secret([u8; SECRET_BYTES]);

impl Secret {
    pub fn random() -> io::Result<Self> {
        let mut bytes = [0u8; SECRET_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|error| io::Error::other(format!("no random bytes: {error}")))?;
        Ok(Self(bytes))
    }

    pub const fn from_bytes(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// Reads 64 lower hex characters and nothing else.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != SECRET_BYTES * 2 {
            return None;
        }
        let mut bytes = [0u8; SECRET_BYTES];
        for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
            *byte = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Some(Self(bytes))
    }

    pub fn to_hex(&self) -> Zeroizing<String> {
        let mut text = Zeroizing::new(String::with_capacity(SECRET_BYTES * 2));
        for byte in self.0 {
            text.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
            text.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
        }
        text
    }

    pub(crate) fn bytes(&self) -> &[u8; SECRET_BYTES] {
        &self.0
    }

    /// Whether `other` is this secret. How long the answer takes says
    /// nothing about where the two differ: the hashes are compared, and
    /// that comparison takes the same time whatever they hold.
    pub fn matches(&self, other: &Self) -> bool {
        blake3::hash(&self.0) == blake3::hash(&other.0)
    }
}

fn nibble(character: u8) -> Option<u8> {
    match character {
        b'0'..=b'9' => Some(character - b'0'),
        b'a'..=b'f' => Some(character - b'a' + 10),
        _ => None,
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The text is not a pairing link.
    NotALink,
    /// A link of a format this build does not read: a newer Hide wrote it.
    Newer,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotALink => "the text is not a pairing link",
            Self::Newer => "the pairing link is of a newer format",
        })
    }
}

impl std::error::Error for LinkError {}

/// Whether `device` is spelled as a product device id: what a link, a
/// pairing message and an `accept-ssh` request may carry.
pub fn valid_device(device: &str) -> bool {
    !device.is_empty()
        && device.len() <= limits::MAX_DEVICE_BYTES
        && device
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Whether `name` can be shown as a machine's name: short, and with no
/// character that draws nothing or turns the text around it, so an allow
/// window reads as what it is.
pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= limits::MAX_NAME_BYTES
        && !name.chars().any(|character| {
            character.is_control()
                || matches!(character, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

/// A machine's name made showable: what [`valid_name`] refuses is left out
/// and the rest is cut to its limit. `None` when nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let mut clean = String::new();
    for character in name.trim().chars() {
        if clean.len() + character.len_utf8() > limits::MAX_NAME_BYTES {
            break;
        }
        let mut one = [0u8; 4];
        if valid_name(character.encode_utf8(&mut one)) || character == ' ' {
            clean.push(character);
        }
    }
    let clean = clean.trim().to_owned();
    valid_name(&clean).then_some(clean)
}

fn valid_relay(relay: &str) -> bool {
    (relay.starts_with("https://") || relay.starts_with("http://"))
        && relay.len() <= 256
        && relay
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'&' | b'#'))
}

/// A parsed link.
#[derive(Debug)]
pub struct PairLink {
    /// The host's connection id: what is dialed.
    pub host: PeerKey,
    /// The host's product device id, to show and to check its answers by.
    pub device: String,
    pub relay: Option<String>,
    pub addrs: Vec<SocketAddr>,
    pub secret: Secret,
}

impl PairLink {
    pub fn parse(text: &str) -> Result<Self, LinkError> {
        let text = text.trim();
        let fields = text.strip_prefix(PREFIX).ok_or(LinkError::NotALink)?;
        if text.len() > limits::MAX_LINK_BYTES {
            return Err(LinkError::NotALink);
        }
        let (mut version, mut host, mut device, mut relay, mut secret) =
            (None, None, None, None, None);
        let mut addrs = Vec::new();
        for field in fields.split('&') {
            let (name, value) = field.split_once('=').ok_or(LinkError::NotALink)?;
            match name {
                "v" => version = Some(value),
                "id" => host = Some(PeerKey::from_hex(value).map_err(|_| LinkError::NotALink)?),
                "dev" => device = Some(value),
                "relay" => relay = Some(value),
                "s" => secret = Some(Secret::from_hex(value).ok_or(LinkError::NotALink)?),
                "a" if addrs.len() < limits::MAX_LINK_ADDRS => {
                    addrs.push(value.parse().map_err(|_| LinkError::NotALink)?);
                }
                // More addresses than are tried, or a field of a later
                // build that this one can pair without.
                _ => {}
            }
        }
        match version {
            Some(VERSION) => {}
            Some(_) => return Err(LinkError::Newer),
            None => return Err(LinkError::NotALink),
        }
        let device = device.filter(|device| valid_device(device));
        let relay = match relay {
            Some(relay) if valid_relay(relay) => Some(relay.to_owned()),
            Some(_) => return Err(LinkError::NotALink),
            None => None,
        };
        match (host, device, secret) {
            (Some(host), Some(device), Some(secret)) => Ok(Self {
                host,
                device: device.to_owned(),
                relay,
                addrs,
                secret,
            }),
            _ => Err(LinkError::NotALink),
        }
    }

    /// Where the link says the host is, in the order to try: its own
    /// addresses, then its relay.
    pub fn candidates(&self) -> Vec<Candidate> {
        self.addrs
            .iter()
            .copied()
            .map(Candidate::Ip)
            .chain(self.relay.clone().map(Candidate::Relay))
            .collect()
    }

    /// The link's text. `None` when what it would carry is not what
    /// [`PairLink::parse`] reads back: a device id or a relay URL this
    /// format cannot spell, or more than a link holds.
    pub fn text(&self) -> Option<Zeroizing<String>> {
        if !valid_device(&self.device) || self.relay.as_deref().is_some_and(|url| !valid_relay(url))
        {
            return None;
        }
        let mut text = Zeroizing::new(format!(
            "{PREFIX}v={VERSION}&id={}&dev={}",
            self.host.to_hex(),
            self.device
        ));
        if let Some(relay) = &self.relay {
            text.push_str("&relay=");
            text.push_str(relay);
        }
        text.push_str("&s=");
        text.push_str(&self.secret.to_hex());
        for addr in self.addrs.iter().take(limits::MAX_LINK_ADDRS) {
            let field = format!("&a={addr}");
            // An address that does not fit is left out; the relay and the
            // other addresses still reach the host.
            if text.len() + field.len() <= limits::MAX_LINK_BYTES {
                text.push_str(&field);
            }
        }
        (text.len() <= limits::MAX_LINK_BYTES).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> PairLink {
        PairLink {
            host: PeerKey::from_bytes([7; 32]),
            device: "14ad0319-60d7-5a7c-ae5a-f71550dc8a60".into(),
            relay: Some("https://device-relay.withhide.dev".into()),
            addrs: vec![
                "192.168.0.4:7842".parse().unwrap(),
                "[fe80::1]:7842".parse().unwrap(),
            ],
            secret: Secret::from_bytes([0xa5; 32]),
        }
    }

    #[test]
    fn a_link_reads_back_as_what_was_shared() {
        let shared = link();
        let text = shared.text().unwrap();
        assert_eq!(
            text.as_str(),
            format!(
                "hide://pair#v=1&id={}&dev=14ad0319-60d7-5a7c-ae5a-f71550dc8a60&relay=https://device-relay.withhide.dev&s={}&a=192.168.0.4:7842&a=[fe80::1]:7842",
                "07".repeat(32),
                "a5".repeat(32)
            )
        );
        // Pasted with the space and line end a terminal adds.
        let read = PairLink::parse(&format!("  {}\n", text.as_str())).unwrap();
        assert_eq!(read.host, shared.host);
        assert_eq!(read.device, shared.device);
        assert!(read.secret.matches(&shared.secret));
        assert_eq!(
            read.candidates(),
            vec![
                Candidate::Ip("192.168.0.4:7842".parse().unwrap()),
                Candidate::Ip("[fe80::1]:7842".parse().unwrap()),
                Candidate::Relay("https://device-relay.withhide.dev".into()),
            ]
        );
    }

    #[test]
    fn a_link_with_no_relay_and_a_field_of_a_later_build_still_pairs() {
        let mut shared = link();
        shared.relay = None;
        let text = format!("{}&later=thing", shared.text().unwrap().as_str());
        let read = PairLink::parse(&text).unwrap();
        assert_eq!(read.relay, None);
        assert_eq!(read.addrs.len(), 2);
    }

    #[test]
    fn what_is_not_a_link_of_this_format_is_refused() {
        let text = link().text().unwrap();
        let text = text.as_str();
        assert_eq!(
            PairLink::parse(&text.replace("v=1", "v=2")).unwrap_err(),
            LinkError::Newer
        );
        for broken in [
            "https://withhide.dev".to_owned(),
            text.replace("hide://pair#", "hide://pair?"),
            text.replace("v=1&", ""),
            text.replace(&"a5".repeat(32), "a5"),
            text.replace(&"a5".repeat(32), &"A5".repeat(32)),
            text.replace(&"07".repeat(32), "07"),
            text.replace("dev=14ad0319", "dev=14ad:0319"),
            text.replace("relay=https://", "relay=ftp://"),
            text.replace("a=192.168.0.4:7842", "a=192.168.0.4"),
            format!("{text}&pad={}", "x".repeat(1024)),
        ] {
            assert_eq!(
                PairLink::parse(&broken).unwrap_err(),
                LinkError::NotALink,
                "a broken link was read"
            );
        }
    }

    #[test]
    fn a_link_is_never_longer_than_a_reader_takes() {
        let mut shared = link();
        shared.addrs = (0..40)
            .map(|index| {
                format!("[2001:db8:aaaa:bbbb:cccc:dddd:eeee:{index:x}]:7842")
                    .parse()
                    .unwrap()
            })
            .collect();
        let text = shared.text().unwrap();
        assert!(text.len() <= limits::MAX_LINK_BYTES);
        assert_eq!(
            PairLink::parse(&text).unwrap().addrs.len(),
            limits::MAX_LINK_ADDRS
        );
        // A device id the format cannot spell is not written at all.
        shared.device = "a&b".into();
        assert!(shared.text().is_none());
    }

    #[test]
    fn neither_the_secret_nor_a_name_that_misleads_is_shown() {
        assert_eq!(format!("{:?}", Secret::from_bytes([1; 32])), "Secret(..)");
        assert!(!format!("{:?}", link()).contains(&"a5".repeat(4)));
        assert!(valid_name("Windows PC"));
        assert!(valid_name("호연의 MacBook"));
        for name in ["", "   ", "a\nb", "safe\u{202e}txt.exe", &"x".repeat(65)] {
            assert!(!valid_name(name), "{name:?}");
        }
        assert_eq!(
            clean_name("  Mac\u{202e}Book\n ").as_deref(),
            Some("MacBook")
        );
        assert_eq!(clean_name(&"가".repeat(40)).unwrap().len(), 63);
        assert_eq!(clean_name("\n\t"), None);
    }
}
