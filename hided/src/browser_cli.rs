//! Resolve a browser CLI argument on the calling machine before submitting
//! a pane-scoped Workspace action.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
/// What the argument asks to load: a URL with a scheme as written, a file or
/// folder that exists as its `file:` URL, and anything else as a host, over
/// http when it is a loopback address (a dev server) and https otherwise.
pub fn address(target: &str, cwd: &Path) -> Result<String, String> {
    let text = target.trim();
    if text.is_empty() {
        return Err("nothing to open".to_owned());
    }
    if has_scheme(text) {
        return Ok(text.to_owned());
    }
    let path = cwd.join(text);
    if path.exists() {
        let real = hide_platform::fs::identity::canonical(&path)
            .map_err(|error| format!("{text}: {error}"))?;
        let real =
            hide_platform::path::to_wire(&real).map_err(|error| format!("{text}: {error}"))?;
        return Ok(hide_platform::path::to_file_url(&real, ""));
    }
    if text.starts_with(['/', '.', '~']) {
        return Err(format!("no such file: {text}"));
    }
    if text.chars().any(char::is_whitespace) {
        return Err(format!("not an address: {text}"));
    }
    let scheme = if is_loopback(text) { "http" } else { "https" };
    Ok(format!("{scheme}://{text}"))
}

/// Whether `text` starts with a URL scheme, which `localhost:3000` (a host
/// and a port) does not.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once(':') else {
        return false;
    };
    // A one-letter "scheme" is a Windows drive (`C:\\a.html`); no registered
    // URL scheme is one letter long.
    let named = scheme.len() > 1
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let port = rest.split(['/', '?', '#']).next().unwrap_or("");
    named && !(!port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
}

fn is_loopback(text: &str) -> bool {
    let host = if text.starts_with('[') {
        text.split_inclusive(']').next().unwrap_or("")
    } else {
        text.split([':', '/', '?', '#']).next().unwrap_or("")
    };
    loopback_ip(host).is_some()
}

/// One classification for the CLI and SSH Browser route, so an admitted
/// loopback address can never fall through to a Mac-local page load: the
/// loopback names and ranges, and the unspecified addresses (`0.0.0.0`,
/// `[::]`, `[::ffff:0.0.0.0]`), which a dial on a machine connects to that
/// machine itself, each answered as the loopback address it reaches.
pub(crate) fn loopback_ip(host: &str) -> Option<IpAddr> {
    let host = host.trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") || host == "0.0.0.0" {
        return Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        return address.is_loopback().then_some(IpAddr::V4(address));
    }
    let bracketless = host
        .strip_prefix('[')
        .and_then(|part| part.strip_suffix(']'))
        .unwrap_or(host);
    let address = bracketless.parse::<Ipv6Addr>().ok()?;
    if address.is_loopback() {
        return Some(IpAddr::V6(address));
    }
    if address.is_unspecified() {
        return Some(IpAddr::V6(Ipv6Addr::LOCALHOST));
    }
    match address.to_ipv4_mapped().or_else(|| address.to_ipv4()) {
        Some(mapped) if mapped.is_unspecified() => Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        Some(mapped) if mapped.is_loopback() => Some(IpAddr::V4(mapped)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An unspecified address reaches the machine that dials it, so it is
    // that machine's loopback, as `0.0.0.0` always was.
    #[test]
    fn the_unspecified_addresses_are_the_machine_itself() {
        for host in ["0.0.0.0", "[::]", "::", "[::ffff:0.0.0.0]", "[::ffff:0:0]"] {
            assert!(
                loopback_ip(host).is_some_and(|ip| ip.is_loopback()),
                "{host}"
            );
        }
        for host in ["127.0.0.1", "[::1]", "localhost", "[::ffff:127.0.0.1]"] {
            assert!(loopback_ip(host).is_some(), "{host}");
        }
        for host in ["10.0.0.1", "[fe80::1]", "example.com", "[::ffff:10.0.0.1]"] {
            assert_eq!(loopback_ip(host), None, "{host}");
        }
    }

    #[test]
    fn an_argument_becomes_the_address_it_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("보고서 1.html"), "<p>ok</p>").unwrap();
        let real = dir.path().canonicalize().unwrap();
        let file =
            hide_platform::path::to_file_url(&real.join("보고서 1.html").display().to_string(), "");
        assert_eq!(address("보고서 1.html", dir.path()), Ok(file.clone()));
        assert_eq!(
            address(
                &real.join("보고서 1.html").display().to_string(),
                Path::new("/")
            ),
            Ok(file)
        );
        for (input, expected) in [
            ("https://example.com/a?b#c", "https://example.com/a?b#c"),
            ("about:blank", "about:blank"),
            ("localhost:3000", "http://localhost:3000"),
            ("127.0.0.1:8080/docs", "http://127.0.0.1:8080/docs"),
            ("127.0.0.2:8080/docs", "http://127.0.0.2:8080/docs"),
            ("localhost.:3000", "http://localhost.:3000"),
            ("[::1]:5173", "http://[::1]:5173"),
            ("example.com", "https://example.com"),
            ("docs.rs/serde", "https://docs.rs/serde"),
        ] {
            assert_eq!(
                address(input, dir.path()).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in ["", "./missing.html", "/no/such/file.html", "two words"] {
            assert!(address(input, dir.path()).is_err(), "{input}");
        }
    }

    #[test]
    fn a_windows_drive_is_a_path_and_not_a_scheme() {
        assert!(!has_scheme(r"C:\site\index.html"));
        assert!(!has_scheme("C:/site/index.html"));
        assert!(!has_scheme("localhost:3000"));
        assert!(has_scheme("https://example.com"));
        assert!(has_scheme("about:blank"));
    }
}
