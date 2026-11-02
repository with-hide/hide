//! Native device crypto for the phone browser fixture. Bounded stdio, never shipped.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hide_relay_protocol::{
    admission::DeviceKey,
    noise::{NoiseKeypair, Session},
    qr::PairingQr,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    Open { connection: u32 },
    Receive { connection: u32, frame: String },
    Send { connection: u32, text: String },
    Close { connection: u32 },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = NoiseKeypair::generate()?;
    let device = DeviceKey::generate()?;
    let qr = PairingQr::new(
        &device.public_key(),
        &key.public_key(),
        &[17; 16],
        "개발 기기 <img src=x>",
    )?;
    println!("{}", json!({"qr": qr.encode()?}));
    io::stdout().flush()?;
    let mut sessions = HashMap::<u32, (Session, bool)>::new();
    let mut stdin = io::stdin().lock();
    let started = std::time::Instant::now();
    for _ in 0..10_000 {
        let mut line = String::new();
        let read = io::Read::take(&mut stdin, 400_000).read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if !line.ends_with('\n') {
            return Err("oversized command".into());
        }
        let mut frames = Vec::<String>::new();
        let mut text: Option<String> = None;
        match serde_json::from_str::<Command>(&line)? {
            Command::Open { connection } => {
                if sessions.len() >= 32 || sessions.contains_key(&connection) {
                    return Err("session limit".into());
                }
                sessions.insert(connection, (Session::responder(&key, connection)?, true));
            }
            Command::Close { connection } => {
                sessions.remove(&connection);
            }
            Command::Receive { connection, frame } => {
                let (session, handshake) =
                    sessions.get_mut(&connection).ok_or("missing session")?;
                let bytes = URL_SAFE_NO_PAD.decode(frame)?;
                if *handshake {
                    session.read_handshake(&bytes)?;
                    frames.push(URL_SAFE_NO_PAD.encode(session.write_handshake()?));
                    *handshake = false;
                } else if let Some(plain) =
                    session.recv_message(&bytes, started.elapsed().as_millis() as u64)?
                {
                    text = Some(String::from_utf8(plain)?);
                }
            }
            Command::Send { connection, text } => {
                let (session, _) = sessions.get_mut(&connection).ok_or("missing session")?;
                frames = session
                    .send_message(text.as_bytes())?
                    .iter()
                    .map(|frame| URL_SAFE_NO_PAD.encode(frame))
                    .collect();
            }
        }
        println!("{}", json!({"frames": frames, "text": text}));
        io::stdout().flush()?;
    }
    Err("command limit".into())
}
