//! The flags of the commands a move or an update runs on the other machine
//! (`hided core-move <step>`, `hided core-update`): one parser, so the two
//! read a flag the same way, and a state folder left out is the folder the
//! process would use on any other start, which a placement written for the
//! default folder leaves out.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Flags {
    pub state_dir: PathBuf,
    pub intent: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub ai: Option<String>,
    pub answer: Option<PathBuf>,
    pub previous: Option<PathBuf>,
}

/// Reads `args` as `--flag value` pairs, each one of `allowed`; anything
/// else is `usage`.
pub fn parse<'a>(
    args: impl IntoIterator<Item = &'a OsString>,
    allowed: &[&str],
    usage: &str,
) -> Result<Flags, String> {
    let absolute = |flag: &str, value: String| {
        if Path::new(&value).is_absolute() {
            Ok(PathBuf::from(value))
        } else {
            Err(format!("{flag} must be an absolute path"))
        }
    };
    let mut flags = Flags::default();
    let mut state_dir = None;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let flag = flag
            .to_str()
            .filter(|flag| allowed.contains(flag))
            .ok_or(usage)?;
        let value = args
            .next()
            .and_then(|value| value.to_str())
            .ok_or(usage)?
            .to_owned();
        match flag {
            "--state-dir" => state_dir = Some(absolute(flag, value)?),
            "--intent" => flags.intent = Some(super::checked_intent(&value)?.to_owned()),
            "--source" => flags.source = Some(value),
            "--target" => flags.target = Some(value),
            "--ai" => flags.ai = Some(value),
            "--answer" => flags.answer = Some(absolute(flag, value)?),
            "--previous" => flags.previous = Some(absolute(flag, value)?),
            _ => return Err(usage.to_owned()),
        }
    }
    flags.state_dir = match state_dir {
        Some(dir) => dir,
        None => {
            let home = hide_platform::host::home_dir()
                .map_err(|error| format!("no home folder: {error}"))?;
            hide_kit::layout::state_dir_from_process(&home)
        }
    };
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flag the command does not take, and a relative path, are refused.
    #[test]
    fn only_the_command_s_own_flags_are_read() {
        let args: Vec<OsString> = ["--previous", "/x/hided"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse(&args, &["--intent"], "usage").unwrap_err(), "usage");
        let relative: Vec<OsString> = ["--state-dir", "here"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert!(parse(&relative, &["--state-dir"], "usage").is_err());
    }
}
