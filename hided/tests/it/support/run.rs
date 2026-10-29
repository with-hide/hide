//! How a fixture's run directory ends, for every fixture that keeps one under
//! `agents/runs/`: a journey that passed leaves nothing there, a journey that
//! failed leaves everything for the lane to upload. A fixture takes a journey
//! as a closure and calls [`finish`] itself, so no test has to remember to
//! remove its folder (issue 899: two of the three delivery journeys did not,
//! about 400 MB under `agents/runs/` each).

use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

/// What [`finish`] needs of a fixture.
pub trait Run {
    /// The run directory, which holds the accounts, staged binaries and logs.
    fn root(&self) -> &Path;

    /// Ends every owned process and confirms each exit. Ending twice is
    /// harmless, since a failed journey is stopped before the error returns
    /// and the fixture's drop stops again.
    fn stop(&mut self) -> Result<()>;

    /// The directory is gone, so the drop must not stop a second time.
    fn removed(&mut self);
}

/// Ends a run by how its journey went.
///
/// Every owned process is stopped first, and teardown is an assertion: a
/// process that did not confirm its exit fails the test even when the journey
/// passed. The directory is removed only when the journey passed and every
/// exit was confirmed; otherwise it is kept, and the first failure is the one
/// returned, with the directory's path.
pub fn finish(run: &mut impl Run, journey: Result<()>) -> Result<()> {
    let stopped = run.stop();
    let failure = match (journey, stopped) {
        (Ok(()), Ok(())) => {
            fs::remove_dir_all(run.root())
                .with_context(|| format!("remove {}", run.root().display()))?;
            run.removed();
            return Ok(());
        }
        (Err(error), _) | (Ok(()), Err(error)) => error,
    };
    if let Err(error) = shed_programs(run.root()) {
        eprintln!(
            "the kept run {} still holds programs: {error:#}",
            run.root().display()
        );
    }
    Err(failure).context(format!("run kept at {}", run.root().display()))
}

/// Removes every native program under a kept run's `root`, leaving its logs,
/// records and scripts. The programs are copies of the build (staged,
/// stripped, installed in each account's helper and bin folders), about
/// 0.9 GB per core-move run, so a lane whose journeys failed one after
/// another filled the runner's disk and failed the rest with it (issue
/// hide-private#120). Links are removed, never followed.
pub fn shed_programs(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let path = entry?.path();
        let kind = fs::symlink_metadata(&path)?.file_type();
        if kind.is_dir() {
            shed_programs(&path)?;
        } else if kind.is_file() && is_program(&path)? {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

/// Whether `path` starts as an ELF or Mach-O program does.
fn is_program(path: &Path) -> Result<bool> {
    let mut magic = [0_u8; 4];
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut read = 0;
    while read < magic.len() {
        match file.read(&mut magic[read..])? {
            0 => return Ok(false),
            count => read += count,
        }
    }
    Ok(matches!(
        magic,
        [0x7f, b'E', b'L', b'F']
            | [0xfe, 0xed, 0xfa, 0xce | 0xcf]
            | [0xce | 0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    struct Fake {
        root: tempfile::TempDir,
        stopped: usize,
        stop_fails: bool,
        removed: bool,
    }

    impl Fake {
        fn new(stop_fails: bool) -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("evidence"), "kept for the lane").unwrap();
            Self {
                root,
                stopped: 0,
                stop_fails,
                removed: false,
            }
        }
    }

    impl Run for Fake {
        fn root(&self) -> &Path {
            self.root.path()
        }

        fn stop(&mut self) -> Result<()> {
            self.stopped += 1;
            if self.stop_fails {
                Err(anyhow!("a process did not confirm its exit"))
            } else {
                Ok(())
            }
        }

        fn removed(&mut self) {
            self.removed = true;
        }
    }

    #[test]
    fn a_journey_that_passed_leaves_no_run_directory() {
        let mut run = Fake::new(false);
        finish(&mut run, Ok(())).unwrap();
        assert!(!run.root().exists());
        assert!(run.removed);
        assert_eq!(run.stopped, 1);
    }

    #[test]
    fn a_journey_that_failed_keeps_its_directory_and_its_own_error() {
        let mut run = Fake::new(false);
        let error = finish(&mut run, Err(anyhow!("the letter never arrived"))).unwrap_err();
        assert!(run.root().join("evidence").is_file());
        assert!(!run.removed);
        assert_eq!(
            run.stopped, 1,
            "owned processes end before the error returns"
        );
        let text = format!("{error:#}");
        assert!(text.contains("the letter never arrived"), "{text}");
        assert!(text.contains(&run.root().display().to_string()), "{text}");
    }

    #[test]
    fn a_kept_run_keeps_its_logs_and_links_but_not_its_programs() {
        let mut run = Fake::new(false);
        let root = run.root().to_path_buf();
        let program = std::env::current_exe().unwrap();
        fs::create_dir_all(root.join("c/home/helper")).unwrap();
        fs::copy(&program, root.join("c/home/helper/hided")).unwrap();
        fs::write(root.join("c/home/gh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(root.join("c/empty"), "").unwrap();
        std::os::unix::fs::symlink(&program, root.join("c/home/herdr")).unwrap();
        finish(&mut run, Err(anyhow!("the helper never became ready"))).unwrap_err();
        assert!(root.join("evidence").is_file());
        assert!(root.join("c/home/gh").is_file());
        assert!(root.join("c/empty").is_file());
        assert!(!root.join("c/home/helper/hided").exists());
        assert!(fs::symlink_metadata(root.join("c/home/herdr")).is_ok());
        assert!(program.is_file(), "a link's target is not the run's");
    }

    #[test]
    fn a_failed_teardown_after_a_failed_journey_does_not_hide_the_journey() {
        let mut run = Fake::new(true);
        let error = finish(&mut run, Err(anyhow!("the letter never arrived"))).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("the letter never arrived"), "{text}");
        assert!(!text.contains("did not confirm"), "{text}");
        assert!(run.root().exists());
    }

    #[test]
    fn a_passed_journey_whose_process_did_not_exit_keeps_the_directory() {
        let mut run = Fake::new(true);
        let error = finish(&mut run, Ok(())).unwrap_err();
        assert!(run.root().join("evidence").is_file());
        assert!(!run.removed);
        assert!(format!("{error:#}").contains("did not confirm its exit"));
    }
}
