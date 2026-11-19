//! `hided node`: this program in its node role on a registered device,
//! started by the core over the stdin and stdout of an SSH exec channel
//! (PRD core-host-node D-02) or of the `wsl.exe` that runs it in a WSL
//! distribution (PRD wsl-device D-07). `serve` answers the node contract
//! until its input closes, its panes' credentials and commands and its
//! terminals included.

use std::ffi::OsString;
use std::io::{self, BufReader};

/// What a device answers to `hided node --version`: the role, the build and
/// the node protocol it speaks.
pub fn version_line() -> String {
    format!(
        "hided-node {} protocol {}",
        env!("CARGO_PKG_VERSION"),
        hide_host::protocol::PROTOCOL_VERSION
    )
}

const USAGE: &str = "usage: hided node serve [--guest <folder>]|--version";

/// Runs the node subcommand named by `args`, the words after `node`.
pub fn run(args: &[OsString]) -> Result<(), String> {
    let words: Vec<&str> = args
        .iter()
        .map(|arg| arg.to_str().ok_or_else(|| USAGE.to_owned()))
        .collect::<Result<_, _>>()?;
    match words.as_slice() {
        ["serve"] => {
            // The link that started this node is where its records are read:
            // a device's own stderr reaches no log.
            hide_node::diagnostics::install(hide_node::terminal::device::forward_diagnostic);
            let input = BufReader::new(io::stdin().lock());
            // The node's terminals ride the same link (PRD
            // core-host-node-terminal D-18).
            let terminals = hide_node::terminal::device::NodeTerminals::new();
            hide_host::serve::serve_with_terminals(input, io::stdout(), &terminals)
                .map_err(|error| error.to_string())
        }
        // A guest's node, started and supervised by this machine's hided
        // (PRD device-connect-iroh D-77).
        ["serve", "--guest", dir] => crate::guest_node::run(dir),
        // A word after `serve` this build does not know is a host of
        // another build asking for what this one cannot do: it says so by
        // its exit code, before any file is touched.
        ["serve", ..] => {
            eprintln!("{USAGE}");
            std::process::exit(hide_peer::protocol::launch::EXIT_UNKNOWN_WORD)
        }
        ["--version"] => {
            println!("{}", version_line());
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}
