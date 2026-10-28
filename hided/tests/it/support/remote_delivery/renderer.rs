//! Bounded setup client for the fixture's private daemon only.
//! It registers real folders/devices; it never creates a pane capability.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use super::READY_BOUND;

pub struct Renderer {
    stream: BufReader<TcpStream>,
    snapshot: Value,
}

impl Renderer {
    pub fn connect(port: u16, token: &str) -> Result<Self> {
        Self::drawn(port, json!({"token": token, "schema_version": 2}))
    }

    /// A window as the web shell opens one (`client_kind` `web`), which the
    /// daemon counts as drawing the snapshot, so the core runs the reads a
    /// window is there for (PRD labels-in-hided B29).
    pub fn window(port: u16, token: &str) -> Result<Self> {
        Self::drawn(
            port,
            json!({"token": token, "schema_version": 2, "client_kind": "web"}),
        )
    }

    fn drawn(port: u16, handshake: Value) -> Result<Self> {
        let mut renderer = Self::held(port, &handshake)?;
        for _ in 0..8 {
            let frame = renderer
                .read()
                .context("private renderer's initial snapshot")?;
            if frame["type"] == "snapshot" {
                renderer.snapshot = frame["payload"]["rest"].clone();
                return Ok(renderer);
            }
        }
        bail!("private renderer did not provide its initial snapshot")
    }

    /// A window on whatever role the address mounts, answered before any
    /// snapshot: a move's screen or a node holding its screen sends none.
    pub fn connect_held(port: u16, token: &str) -> Result<Self> {
        Self::held(port, &json!({"token": token, "schema_version": 2}))
    }

    fn held(port: u16, handshake: &Value) -> Result<Self> {
        let mut socket = TcpStream::connect(("127.0.0.1", port))?;
        // Only a hang guard: the daemon answers when its state is ready, and
        // right after a move that is the new core's answer through the link.
        // A two-second bound failed the caller's own wait at once, as a bare
        // `os error 11` (issue hide-private#281).
        socket.set_read_timeout(Some(READY_BOUND))?;
        socket.set_write_timeout(Some(Duration::from_secs(2)))?;
        write!(
            socket,
            "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        )?;
        let mut stream = BufReader::new(socket);
        let mut line = String::new();
        stream
            .read_line(&mut line)
            .context("private renderer upgrade answer")?;
        ensure!(
            line.starts_with("HTTP/1.1 101"),
            "private renderer upgrade refused"
        );
        let mut accepted = false;
        let mut header_bytes = line.len();
        loop {
            line.clear();
            stream.read_line(&mut line)?;
            header_bytes += line.len();
            ensure!(header_bytes <= 8 * 1024, "private renderer header cap");
            if line == "\r\n" {
                break;
            }
            if line
                .to_ascii_lowercase()
                .starts_with("sec-websocket-accept:")
            {
                accepted = line
                    .split_once(':')
                    .is_some_and(|(_, value)| value.trim() == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
            }
        }
        ensure!(
            accepted,
            "private renderer handshake differs from RFC challenge"
        );
        let mut renderer = Self {
            stream,
            snapshot: Value::Null,
        };
        renderer.send(1, &serde_json::to_vec(handshake)?)?;
        Ok(renderer)
    }

    pub fn snapshot(&self) -> &Value {
        &self.snapshot
    }

    pub fn event(&mut self, kind: &str, payload: Value) -> Result<()> {
        if let Some(refused) = self.ask(kind, payload)? {
            bail!("private renderer event {kind} refused: {refused}");
        }
        Ok(())
    }

    /// Sends one event and answers the reason it was refused with, none
    /// when it was taken. Frames other than its refusal are skipped.
    pub fn ask(&mut self, kind: &str, payload: Value) -> Result<Option<String>> {
        self.asked(kind, payload, false)
    }

    /// As `ask`, for an event that ends this window's screen when it is
    /// taken, as giving up a move ends the role that drew it: a refusal
    /// comes before anything else, so the screen closing is a take.
    pub fn ask_ending(&mut self, kind: &str, payload: Value) -> Result<Option<String>> {
        self.asked(kind, payload, true)
    }

    fn asked(&mut self, kind: &str, payload: Value, closing_takes: bool) -> Result<Option<String>> {
        // A wait before may have left a short bound on reads.
        self.stream
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(30)))?;
        self.send(
            1,
            &serde_json::to_vec(&json!({"schema_version":2,"kind":kind,"payload":payload}))?,
        )?;
        // The server reads frames in order. Keep this connection alive until
        // its Pong proves it read the preceding event, instead of dropping
        // the socket while a concurrent snapshot write is still pending.
        self.send(9, b"event")?;
        let mut refused = None;
        for _ in 0..64 {
            let (opcode, data) = match self.read_frame() {
                Ok(frame) => frame,
                Err(error) if closing_takes && refused.is_none() && ended(&error) => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            match opcode {
                10 if data == b"event" => return Ok(refused),
                1 => {
                    let frame: Value =
                        serde_json::from_slice(&data).context("private renderer JSON")?;
                    if frame["type"] == "error" {
                        refused = Some(frame["message"].as_str().unwrap_or_default().to_owned());
                    }
                }
                9 => self.send(10, &data)?,
                8 if closing_takes && refused.is_none() => return Ok(None),
                8 => bail!("private renderer closed before event receipt"),
                _ => bail!("unexpected private renderer event opcode"),
            }
        }
        bail!("private renderer event receipt frame cap")
    }

    /// Reads this window's frames until a `core_move` frame whose view
    /// `done` takes arrives within `bound`, and answers that view.
    pub fn core_move_until(
        &mut self,
        bound: Duration,
        done: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = std::time::Instant::now() + bound;
        loop {
            let left = deadline
                .checked_duration_since(std::time::Instant::now())
                .filter(|left| !left.is_zero())
                .context("no core_move frame the window waited for")?;
            self.stream.get_ref().set_read_timeout(Some(left))?;
            let (opcode, data) = self
                .read_frame()
                .context("no core_move frame the window waited for")?;
            match opcode {
                1 => {
                    let frame: Value =
                        serde_json::from_slice(&data).context("private renderer JSON")?;
                    if frame["type"] == "core_move" && done(&frame["payload"]) {
                        return Ok(frame["payload"].clone());
                    }
                }
                9 => self.send(10, &data)?,
                8 => bail!("the window's socket closed before the core_move frame"),
                _ => {}
            }
        }
    }

    /// Reads this window's frames until the daemon closes its socket within
    /// `bound`, and answers the close's code and reason; pings are answered
    /// and every other frame is skipped.
    pub fn closed_within(&mut self, bound: Duration) -> Result<(u16, String)> {
        let deadline = std::time::Instant::now() + bound;
        loop {
            let left = deadline
                .checked_duration_since(std::time::Instant::now())
                .filter(|left| !left.is_zero())
                .context("the window's socket was not closed")?;
            self.stream.get_ref().set_read_timeout(Some(left))?;
            let (opcode, data) = self
                .read_frame()
                .context("the window's socket was not closed")?;
            match opcode {
                8 => {
                    ensure!(data.len() >= 2, "the window's socket closed with no code");
                    return Ok((
                        u16::from_be_bytes([data[0], data[1]]),
                        String::from_utf8_lossy(&data[2..]).into_owned(),
                    ));
                }
                9 => self.send(10, &data)?,
                _ => {}
            }
        }
    }

    fn send(&mut self, opcode: u8, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= 64 * 1024, "private renderer output cap");
        let mask = [31_u8, 17, 89, 203];
        let mut frame = vec![0x80 | opcode];
        if bytes.len() < 126 {
            frame.push(0x80 | bytes.len() as u8);
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4]),
        );
        self.stream.get_mut().write_all(&frame)?;
        Ok(())
    }

    fn read(&mut self) -> Result<Value> {
        for _ in 0..8 {
            let (opcode, data) = self.read_frame()?;
            match opcode {
                1 => return serde_json::from_slice(&data).context("private renderer JSON"),
                9 => self.send(10, &data)?,
                8 => bail!("private renderer closed before snapshot"),
                _ => bail!("unexpected private renderer opcode"),
            }
        }
        bail!("private renderer ping cap")
    }

    fn read_frame(&mut self) -> Result<(u8, Vec<u8>)> {
        let mut header = [0; 2];
        self.stream.read_exact(&mut header)?;
        ensure!(
            header[0] & 0x80 != 0 && header[1] & 0x80 == 0,
            "unexpected private renderer frame"
        );
        let length = match header[1] {
            126 => {
                let mut size = [0; 2];
                self.stream.read_exact(&mut size)?;
                u16::from_be_bytes(size).into()
            }
            127 => {
                let mut size = [0; 8];
                self.stream.read_exact(&mut size)?;
                u64::from_be_bytes(size)
            }
            size => u64::from(size),
        };
        ensure!(length <= 8 * 1024 * 1024, "private renderer input cap");
        let mut data = vec![0; length as usize];
        self.stream.read_exact(&mut data)?;
        Ok((header[0] & 15, data))
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.send(8, &[]);
        let _ = self.stream.get_mut().shutdown(Shutdown::Both);
    }
}

/// Whether reading a frame failed because the daemon ended the connection.
fn ended(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        )
    })
}
