//! The measurements' pane program: on a raw terminal it appends every byte
//! it reads to its log, and draws each token it is typed, `<` to `>`, alone
//! on the first row, so one token is one change of one row whichever path
//! carries the screen. The log is what the program received, the answer to
//! loss, duplication and order on the input side.

#[cfg(unix)]
pub fn run(log: &std::path::Path) -> i32 {
    use std::io::{Read, Write};

    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("echo: {}: {error}", log.display());
            return 1;
        }
    };
    // SAFETY: termios on this process's own standard input, a terminal.
    unsafe {
        let mut mode: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut mode) == 0 {
            libc::cfmakeraw(&mut mode);
            libc::tcsetattr(0, libc::TCSANOW, &mode);
        }
    }
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(b"\x1b[2J\x1b[Hecho ready");
    let _ = stdout.flush();
    let mut stdin = std::io::stdin();
    let mut buffer = [0u8; 4096];
    let mut token = Vec::new();
    loop {
        let read = match stdin.read(&mut buffer) {
            Ok(0) | Err(_) => return 0,
            Ok(read) => read,
        };
        let _ = file.write_all(&buffer[..read]);
        for &byte in &buffer[..read] {
            match byte {
                b'<' => {
                    token.clear();
                    token.push(byte);
                }
                b'>' if !token.is_empty() => {
                    token.push(byte);
                    let _ = stdout.write_all(b"\x1b[1;1H\x1b[2K");
                    let _ = stdout.write_all(&token);
                    let _ = stdout.flush();
                    token.clear();
                }
                0x04 => return 0,
                _ if !token.is_empty() && token.len() < 64 => token.push(byte),
                _ => {}
            }
        }
    }
}

#[cfg(not(unix))]
pub fn run(_log: &std::path::Path) -> i32 {
    eprintln!(
        "echo: the measurements' pane program runs on Unix; Windows is measured by hide-holder's own tests"
    );
    2
}
