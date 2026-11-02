//! Binary WebSocket envelope: version(1), kind(1), connection(4, BE), payload.
use crate::Error;

pub const VERSION: u8 = 1;
pub const MAX_FRAME: usize = 65_536;
pub const HEADER_LEN: usize = 6;
pub const MAX_PAYLOAD: usize = MAX_FRAME - HEADER_LEN;
pub const MAX_PLAINTEXT: usize = MAX_PAYLOAD - 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Kind {
    Handshake = 1,
    Transport = 2,
}

/// The relay assigns a nonzero id for one phone socket and never reuses it
/// while the device socket lives. A new id always gets a new Noise session.
pub struct Frame<'a> {
    pub kind: Kind,
    pub connection: u32,
    pub payload: &'a [u8],
}

impl<'a> Frame<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FRAME {
            return Err(Error::SizeLimit);
        }
        if bytes.len() < HEADER_LEN {
            return Err(Error::InvalidMessage);
        }
        if bytes[0] != VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let kind = match bytes[1] {
            1 => Kind::Handshake,
            2 => Kind::Transport,
            _ => return Err(Error::InvalidMessage),
        };
        let connection = u32::from_be_bytes(bytes[2..6].try_into().unwrap());
        let frame = Self {
            kind,
            connection,
            payload: &bytes[HEADER_LEN..],
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&[VERSION, self.kind as u8]);
        bytes.extend_from_slice(&self.connection.to_be_bytes());
        bytes.extend_from_slice(self.payload);
        Ok(bytes)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(Error::SizeLimit);
        }
        if self.connection == 0 || self.payload.len() < 16 {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }
}
