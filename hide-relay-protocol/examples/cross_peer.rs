//! Bounded stdio peer for the native/WASM interop test. Not shipped.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hide_relay_protocol::{
    admission::DeviceKey,
    noise::{NoiseKeypair, Session},
    qr::PairingQr,
};
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = NoiseKeypair::generate()?;
    let device = DeviceKey::generate()?;
    let qr = PairingQr::new(&device.public_key(), &key.public, &[17; 16])?;
    println!("{}", qr.encode()?);
    io::stdout().flush()?;
    let mut session = Session::responder(&key.secret, 7)?;
    let mut stdin = io::stdin().lock();
    let started = std::time::Instant::now();
    let mut step = 0;
    // At most one handshake plus three messages (one spans five frames).
    for _ in 0..8 {
        let mut line = String::new();
        let read = io::Read::take(&mut stdin, 90_000).read_line(&mut line)?;
        if read == 0 || !line.ends_with('\n') {
            return Err("missing or oversized frame".into());
        }
        let bytes = URL_SAFE_NO_PAD.decode(line.trim_end())?;
        let replies = if step == 0 {
            session.read_handshake(&bytes)?;
            step += 1;
            vec![session.write_handshake()?]
        } else {
            let Some(plaintext) =
                session.recv_message(&bytes, started.elapsed().as_millis() as u64)?
            else {
                continue;
            };
            let expected = match step {
                1 => b"phone credential".to_vec(),
                2 => vec![42; hide_relay_protocol::noise::MAX_MESSAGE],
                3 => Vec::new(),
                _ => return Err("too many messages".into()),
            };
            if plaintext != expected {
                return Err("unexpected plaintext".into());
            }
            step += 1;
            session.send_message(&plaintext)?
        };
        for reply in replies {
            println!("{}", URL_SAFE_NO_PAD.encode(reply));
        }
        io::stdout().flush()?;
        if step == 4 {
            return Ok(());
        }
    }
    Err("incomplete messages".into())
}
