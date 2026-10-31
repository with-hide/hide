//! The processes in a pane's terminal, for an ask (PRD
//! engine-bundle-2-pane-holder D-18): its shell, and the processes in its
//! foreground process group with their names. What is not known is said as
//! not known, never filled with the shell or an empty name.

use hide_holder_protocol::{Answer, Foreground, MAX_PROCESSES, Member, Unknown};

use crate::log::Log;

/// `shell` is the pane's program; `group` the terminal's foreground group,
/// or why it is not known. A group the system would not list is logged with
/// the system's reason, which the answer does not carry.
pub fn processes(shell: u32, group: Result<u32, Unknown>, log: &Log) -> Answer {
    let foreground = match group {
        Ok(group) => match hide_platform::process::group_members(group) {
            Ok(mut pids) => {
                pids.sort_unstable();
                pids.truncate(MAX_PROCESSES);
                Foreground::Known {
                    group,
                    members: pids
                        .into_iter()
                        .map(|pid| Member {
                            pid,
                            name: hide_platform::process::name_of(pid).ok(),
                        })
                        .collect(),
                }
            }
            Err(error) => {
                log.event(
                    "holder.processes_unreadable",
                    &[("group", group.into()), ("error", error.to_string().into())],
                );
                Foreground::Unknown(Unknown::Unreadable)
            }
        },
        Err(why) => Foreground::Unknown(why),
    };
    Answer::Processes { shell, foreground }
}
