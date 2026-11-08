//! What `hided` was started to be, read from its arguments once, before
//! anything else runs: a closed list of roles, each with its own words.
//! The daemon is the role of no arguments at all (what `hide connect`, a
//! moved core's login item and the test fixtures start), so an argument this
//! list does not name is refused before any state folder is opened, rather
//! than read as one more way to start a core (with-hide/hide-private#670).

use std::ffi::OsString;

/// What one `hided` process runs. A role that takes its own words keeps
/// them, and its own parser refuses what it does not know.
#[derive(Debug, Eq, PartialEq)]
pub enum Role<'a> {
    /// The daemon on the state folder its environment names.
    Daemon,
    /// The daemon as a moved core's login item starts it
    /// (`login_item::before_core_at_login` first).
    CoreLogin,
    /// A pane's entry point that becomes its shell in this process.
    ShellExec(&'a [OsString]),
    /// The node on a device, over the link that started it.
    Node(&'a [OsString]),
    /// The pipe a node on another machine runs here over SSH.
    Attach(&'a [OsString]),
    /// A step of a core move.
    CoreMove(&'a [OsString]),
    /// The update of the core here to this build.
    CoreUpdate(&'a [OsString]),
    /// The Unix opener supervisor; it reads the whole argument list.
    OpenHelper(&'a [OsString]),
    /// The build's version, and nothing else.
    Version,
    /// The roles and their words, and nothing else.
    Help,
}

pub const USAGE: &str = "usage: hided [--version | --help]
  hided                   the daemon on the state folder HIDE_STATE_DIR names (hide connect starts it)
  hided core-login        the daemon, as a moved core's login item starts it
  hided node serve|--version
  hided attach [--state-dir <absolute dir>]
  hided core-move <step> ...
  hided core-update ...
  hided shell-exec --shell <absolute executable path> --login";

/// The role `args` (the arguments after the program) name, or why none.
pub fn parse(args: &[OsString]) -> Result<Role<'_>, String> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Role::Daemon);
    };
    let only = |role: Role<'static>| {
        if rest.is_empty() {
            Ok(role)
        } else {
            Err(format!(
                "hided {} takes no further arguments\n{USAGE}",
                first.to_string_lossy()
            ))
        }
    };
    match first.to_str() {
        Some("shell-exec") => Ok(Role::ShellExec(rest)),
        Some("node") => Ok(Role::Node(rest)),
        Some("attach") => Ok(Role::Attach(rest)),
        Some("core-move") => Ok(Role::CoreMove(rest)),
        Some("core-update") => Ok(Role::CoreUpdate(rest)),
        Some("--open-helper") => Ok(Role::OpenHelper(args)),
        Some("core-login") => only(Role::CoreLogin),
        Some("--version") => only(Role::Version),
        Some("--help" | "-h" | "help") => only(Role::Help),
        _ => Err(format!(
            "hided does not know {:?}\n{USAGE}",
            first.to_string_lossy()
        )),
    }
}

/// What `hided --version` prints: the release and, for a build made from a
/// checkout, its commit, as `hide --version` prints them.
pub fn version_line() -> String {
    match crate::cli::COMMIT {
        Some(commit) => format!("hided {} {commit}", crate::cli::VERSION),
        None => format!("hided {}", crate::cli::VERSION),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_is_the_daemon_and_nothing_else_is() {
        assert_eq!(parse(&[]), Ok(Role::Daemon));
        for refused in [
            &["--nonsense"][..],
            &["foo"],
            &["-V"],
            &["--version", "extra"],
            &["core-login", "extra"],
            &["--help", "node"],
            &[""],
        ] {
            let error = parse(&words(refused)).unwrap_err();
            assert!(error.contains(USAGE), "{refused:?}: {error}");
        }
    }

    #[test]
    fn each_role_keeps_its_own_words() {
        let node = words(&["node", "serve"]);
        assert_eq!(parse(&node), Ok(Role::Node(&node[1..])));
        let helper = words(&["--open-helper", "3", "/usr/bin/open", "x"]);
        assert_eq!(parse(&helper), Ok(Role::OpenHelper(&helper[..])));
        assert_eq!(parse(&words(&["core-login"])), Ok(Role::CoreLogin));
        assert_eq!(parse(&words(&["--version"])), Ok(Role::Version));
        assert_eq!(parse(&words(&["-h"])), Ok(Role::Help));
    }
}
