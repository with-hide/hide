//! The machine key (PRD D-52) and the proof made with it (D-63).
//!
//! One key per Hide state folder, in a private file inside it, the same on
//! every system. The file is Hide's own format: a magic, the raw 32 byte
//! secret, and the machine id it was made on. No transport's type or
//! spelling is stored. A key whose machine id is not this machine's is
//! replaced at start, so a copied disk or a test's isolated state folder
//! never connects as the operator's machine.

use std::io::{self, Read};
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hide_peer_protocol::PeerKey;
use hide_platform::fs::{Access, atomic, private};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"HIDEKEY1";
const SECRET_BYTES: usize = 32;

/// The most bytes of machine id a key file holds; a real one is 36.
const MAX_MACHINE_ID: usize = 255;

/// A machine's secret key. It signs; it is never printed or serialized.
pub struct MachineKey(SigningKey);

/// Whether [`MachineKey::load_or_create`] found this machine's key or made
/// a new one, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded {
    Found,
    /// There was no key file.
    Created,
    /// The file held a key made on another machine.
    ReplacedForeign,
}

/// Reads the key file when it is this account's own regular file that no
/// other account can read. A key another account could read, or planted
/// under this name, is not connected as: it is an error the operator sees,
/// never a key quietly used or replaced.
fn read_own_private(path: &Path) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut file = private::open_own_file(path, false)?;
    if !private::is_private(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} can be read by another account", path.display()),
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("no random bytes: {error}")))?;
    Ok(bytes)
}

impl MachineKey {
    pub fn generate() -> io::Result<Self> {
        let secret = Zeroizing::new(random::<SECRET_BYTES>()?);
        Ok(Self(SigningKey::from_bytes(&secret)))
    }

    pub fn from_secret(secret: &[u8; SECRET_BYTES]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    /// The raw secret, for the one transport that needs it to open its
    /// endpoint. The caller keeps it only as long as that takes.
    pub fn secret(&self) -> Zeroizing<[u8; SECRET_BYTES]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public(&self) -> PeerKey {
        PeerKey::from_bytes(self.0.verifying_key().to_bytes())
    }

    /// Reads the key at `path` when it was made on `machine_id`, and
    /// otherwise writes a new one there. A file that is not a key file is an
    /// error, never silently replaced: only a foreign machine id is.
    pub fn load_or_create(path: &Path, machine_id: &str) -> io::Result<(Self, Loaded)> {
        if machine_id.is_empty() || machine_id.len() > MAX_MACHINE_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a machine id of 1 to 255 bytes is needed to keep a machine key",
            ));
        }
        let loaded = match read_own_private(path) {
            Ok(bytes) => {
                let (secret, made_on) = parse(&bytes).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{} is not a machine key file", path.display()),
                    )
                })?;
                if made_on == machine_id.as_bytes() {
                    return Ok((Self::from_secret(&secret), Loaded::Found));
                }
                Loaded::ReplacedForeign
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Loaded::Created,
            Err(error) => return Err(error),
        };
        let key = Self::generate()?;
        if let Some(parent) = path.parent() {
            private::create_dir_all(parent)?;
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(
            MAGIC.len() + SECRET_BYTES + 1 + machine_id.len(),
        ));
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&*key.secret());
        bytes.push(machine_id.len() as u8);
        bytes.extend_from_slice(machine_id.as_bytes());
        atomic::write_file(path, &bytes, Access::Private)?;
        Ok((key, loaded))
    }

    fn sign(&self, message: &[u8]) -> [u8; PROOF_BYTES] {
        self.0.sign(message).to_bytes()
    }
}

fn parse(bytes: &[u8]) -> Option<(Zeroizing<[u8; SECRET_BYTES]>, &[u8])> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (secret, rest) = rest.split_at_checked(SECRET_BYTES)?;
    let (&length, made_on) = rest.split_first()?;
    (made_on.len() == length as usize).then_some(())?;
    Some((Zeroizing::new(secret.try_into().ok()?), made_on))
}

/// The bytes of a challenge and of the proof that answers it.
pub const CHALLENGE_BYTES: usize = 32;
pub const PROOF_BYTES: usize = 64;

/// A fresh challenge for the peer to sign.
pub fn challenge() -> io::Result<[u8; CHALLENGE_BYTES]> {
    random()
}

fn proof_message(challenge: &[u8; CHALLENGE_BYTES], prover: &PeerKey, asker: &PeerKey) -> Vec<u8> {
    // Both keys are signed, so a proof made for one peer is no proof to
    // another.
    let mut message = Vec::with_capacity(20 + CHALLENGE_BYTES + 64);
    message.extend_from_slice(b"hide-peer/proof/v1\0");
    message.extend_from_slice(challenge);
    message.extend_from_slice(prover.as_bytes());
    message.extend_from_slice(asker.as_bytes());
    message
}

/// Signs the challenge `asker` sent. A transport that does not authenticate
/// by itself (the loopback, `wsl.exe`) exchanges a challenge and a proof
/// each way first, and that is its authentication (D-63).
pub fn prove(
    key: &MachineKey,
    challenge: &[u8; CHALLENGE_BYTES],
    asker: &PeerKey,
) -> [u8; PROOF_BYTES] {
    key.sign(&proof_message(challenge, &key.public(), asker))
}

/// Whether `proof` is `prover`'s signature over the challenge `asker` sent.
pub fn verify(
    prover: &PeerKey,
    challenge: &[u8; CHALLENGE_BYTES],
    asker: &PeerKey,
    proof: &[u8; PROOF_BYTES],
) -> bool {
    let Ok(verifying) = VerifyingKey::from_bytes(prover.as_bytes()) else {
        return false;
    };
    verifying
        .verify(
            &proof_message(challenge, prover, asker),
            &Signature::from_bytes(proof),
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_kept_for_its_machine_and_replaced_on_another() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers").join("machine.key");

        let (first, loaded) = MachineKey::load_or_create(&path, "machine-a").unwrap();
        assert_eq!(loaded, Loaded::Created);
        assert!(private::is_private(&path).unwrap());
        // The file is Hide's own format: the raw secret after a magic.
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(&bytes[8..40], &first.secret()[..]);

        let (again, loaded) = MachineKey::load_or_create(&path, "machine-a").unwrap();
        assert_eq!((loaded, again.public()), (Loaded::Found, first.public()));

        // The same folder on another machine: a copied disk.
        let (other, loaded) = MachineKey::load_or_create(&path, "machine-b").unwrap();
        assert_eq!(loaded, Loaded::ReplacedForeign);
        assert_ne!(other.public(), first.public());
    }

    #[test]
    fn a_damaged_key_file_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        atomic::write_file(&path, b"HIDEKEY1 short", Access::Private).unwrap();
        let error = MachineKey::load_or_create(&path, "machine-a")
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), b"HIDEKEY1 short");
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_another_account_can_read_is_refused_not_used() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.key");
        MachineKey::load_or_create(&path, "machine-a").unwrap();
        let saved = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let error = MachineKey::load_or_create(&path, "machine-a")
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            saved,
            "and it is left as it was"
        );
    }

    #[test]
    fn a_proof_holds_only_for_its_challenge_and_its_pair() {
        let prover = MachineKey::generate().unwrap();
        let asker = MachineKey::generate().unwrap().public();
        let other = MachineKey::generate().unwrap().public();
        let asked = challenge().unwrap();
        let proof = prove(&prover, &asked, &asker);
        assert!(verify(&prover.public(), &asked, &asker, &proof));
        assert!(!verify(
            &prover.public(),
            &challenge().unwrap(),
            &asker,
            &proof
        ));
        assert!(!verify(&prover.public(), &asked, &other, &proof));
        assert!(!verify(&other, &asked, &asker, &proof));
    }
}
