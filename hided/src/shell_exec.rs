//! Internal pane entry point. It replaces itself with the selected login
//! shell before any daemon, node, runtime or owner-watch initialization.

use std::convert::Infallible;
use std::ffi::OsString;
use std::path::Path;

const USAGE: &str = "usage: hided shell-exec --shell <absolute executable path> --login";

pub fn run(args: &[OsString]) -> Result<Infallible, String> {
    let [shell_flag, shell, login_flag] = args else {
        return Err(USAGE.into());
    };
    if shell_flag != "--shell" || login_flag != "--login" {
        return Err(USAGE.into());
    }
    let shell = Path::new(shell);
    hide_platform::process::exec_login_shell(shell).map_err(|error| {
        format!(
            "Could not enter login shell {}: {error}. Select an absolute path to an installed executable shell",
            shell.display()
        )
    })
}
