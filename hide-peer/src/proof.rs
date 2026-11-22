//! The first exchange of a carrier that does not authenticate by itself
//! (PRD D-63): a challenge each way, signed with the machine key
//! ([`crate::key`]). The loopback and a `wsl.exe` carrier ([`crate::pipe`])
//! run the same one.
//!
//! The dialer sends its key and a challenge; the acceptor answers its key,
//! a challenge and its proof; the dialer sends its proof; the acceptor
//! answers one byte once it verified it. A side that reads something else
//! ends the exchange, and neither side waits here for longer than its
//! caller lets it.

use std::io;

use hide_peer_protocol::PeerKey;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::key::{self, CHALLENGE_BYTES, MachineKey, PROOF_BYTES};

const KEY_BYTES: usize = 32;

/// The acceptor's verdict on the dialer's proof.
const VERIFIED: u8 = 1;

async fn read<const N: usize>(from: &mut (impl AsyncRead + Unpin)) -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    from.read_exact(&mut bytes).await?;
    Ok(bytes)
}

/// The dialer's half. `Ok(false)` when the other end did not prove the key
/// that was dialed.
pub async fn dialer(
    key: &MachineKey,
    dialed: &PeerKey,
    send: &mut (impl AsyncWrite + Unpin),
    recv: &mut (impl AsyncRead + Unpin),
) -> io::Result<bool> {
    let me = key.public();
    let asked = key::challenge()?;
    let mut first = Vec::with_capacity(KEY_BYTES + CHALLENGE_BYTES);
    first.extend_from_slice(me.as_bytes());
    first.extend_from_slice(&asked);
    send.write_all(&first).await?;
    send.flush().await?;
    let their_key = PeerKey::from_bytes(read::<KEY_BYTES>(recv).await?);
    let their_challenge = read::<CHALLENGE_BYTES>(recv).await?;
    let their_proof = read::<PROOF_BYTES>(recv).await?;
    if their_key != *dialed || !key::verify(&their_key, &asked, &me, &their_proof) {
        return Ok(false);
    }
    send.write_all(&key::prove(key, &their_challenge, &their_key))
        .await?;
    send.flush().await?;
    Ok(read::<1>(recv).await? == [VERIFIED])
}

/// The acceptor's half: the key the dialer proved, or `None` when it did
/// not prove the key it sent.
pub async fn acceptor(
    key: &MachineKey,
    send: &mut (impl AsyncWrite + Unpin),
    recv: &mut (impl AsyncRead + Unpin),
) -> io::Result<Option<PeerKey>> {
    let me = key.public();
    let their_key = PeerKey::from_bytes(read::<KEY_BYTES>(recv).await?);
    let their_challenge = read::<CHALLENGE_BYTES>(recv).await?;
    let asked = key::challenge()?;
    let mut answer = Vec::with_capacity(KEY_BYTES + CHALLENGE_BYTES + PROOF_BYTES);
    answer.extend_from_slice(me.as_bytes());
    answer.extend_from_slice(&asked);
    answer.extend_from_slice(&key::prove(key, &their_challenge, &their_key));
    send.write_all(&answer).await?;
    send.flush().await?;
    let their_proof = read::<PROOF_BYTES>(recv).await?;
    if !key::verify(&their_key, &asked, &me, &their_proof) {
        return Ok(None);
    }
    send.write_all(&[VERIFIED]).await?;
    send.flush().await?;
    Ok(Some(their_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine that answers with a key it does not hold proves nothing:
    /// the dialer refuses it, and an acceptor refuses a dialer that signs
    /// with another key than the one it sent.
    #[tokio::test]
    async fn a_key_that_is_not_held_proves_nothing() {
        let (honest, other, held) = (
            MachineKey::generate().unwrap(),
            MachineKey::generate().unwrap(),
            MachineKey::generate().unwrap(),
        );
        let other_secret = other.secret();
        let other_acceptor = move || MachineKey::from_secret(&other_secret);
        // The acceptor is `other`; the dialer dialed `held`'s key, and
        // gives the exchange up at the answer, which ends the acceptor's.
        let (near, far) = tokio::io::duplex(1024);
        let accepting = tokio::spawn(async move {
            let (mut far_recv, mut far_send) = tokio::io::split(far);
            acceptor(&other_acceptor(), &mut far_send, &mut far_recv).await
        });
        let dialed = held.public();
        let dialing = {
            let (mut near_recv, mut near_send) = tokio::io::split(near);
            dialer(&honest, &dialed, &mut near_send, &mut near_recv).await
        };
        assert!(!dialing.unwrap());
        assert!(accepting.await.unwrap().is_err());

        // A dialer that sends `held`'s key and signs with its own.
        let (near, far) = tokio::io::duplex(1024);
        let (mut near_recv, mut near_send) = tokio::io::split(near);
        let (mut far_recv, mut far_send) = tokio::io::split(far);
        let (accepting, ()) = tokio::join!(acceptor(&other, &mut far_send, &mut far_recv), async {
            let mut first = held.public().as_bytes().to_vec();
            first.extend_from_slice(&key::challenge().unwrap());
            near_send.write_all(&first).await.unwrap();
            let _their_key = read::<KEY_BYTES>(&mut near_recv).await.unwrap();
            let their_challenge = read::<CHALLENGE_BYTES>(&mut near_recv).await.unwrap();
            let _their_proof = read::<PROOF_BYTES>(&mut near_recv).await.unwrap();
            let forged = key::prove(&honest, &their_challenge, &other.public());
            near_send.write_all(&forged).await.unwrap();
        },);
        assert_eq!(accepting.unwrap(), None);
    }
}
