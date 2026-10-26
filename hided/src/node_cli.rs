//! `hided node`: this program in its node role on a registered device,
//! started by the core over the stdin and stdout of an SSH exec channel
//! (PRD core-host-node D-02). `serve` answers the node contract until the
//! channel closes, its panes' credentials and commands and its terminals
//! included.

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

const USAGE: &str = "usage: hided node serve|--version";

/// Runs the node subcommand named by `args`, the words after `node`.
pub fn run(args: &[OsString]) -> Result<(), String> {
    let words: Vec<&str> = args
        .iter()
        .map(|arg| arg.to_str().ok_or_else(|| USAGE.to_owned()))
        .collect::<Result<_, _>>()?;
    match words.as_slice() {
        ["start-herdr", pinned, socket] => {
            let pid = crate::herdr_handoff::start_detached_device_server(
                std::path::Path::new(pinned),
                std::path::Path::new(socket),
            )?;
            println!("{pid}");
            Ok(())
        }
        ["serve"] => {
            // The link that started this node is where its records are read:
            // a device's own stderr reaches no log.
            hide_node::diagnostics::install(hide_node::terminal::device::forward_diagnostic);
            // After each kit pass the node brings the device's Herdr server
            // to the build's pinned copy (PRD device-herdr-update D-07).
            hide_host::kit::set_herdr_updater(crate::herdr_handoff::device_update);
            let input = BufReader::new(io::stdin().lock());
            // The node's terminals ride the same link (PRD
            // core-host-node-terminal D-18).
            let terminals = hide_node::terminal::device::NodeTerminals::new();
            hide_host::serve::serve_with_terminals(input, io::stdout(), &terminals)
                .map_err(|error| error.to_string())
        }
        ["--version"] => {
            println!("{}", version_line());
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}
