//! Participant registration and child creation use the daemon's same durable
//! store and pane capability as the mailbox.
use crate::env::{self, Env};
use herdr_core::{coordination::Command, delivery::Command as Delivery};
use std::collections::BTreeMap;
pub const USAGE: &str = "hide agent register [--check] [--machine <device>] --host-scope <scope> --session <session> --instance <terminal> --name <name> --pane <pane> [--parent <id>] [--project <path>]\nhide agent list\nhide agent show <id|here>\nhide agent end <id> [--actor <id>]\nhide agent catalog [--machine <device id>]\nhide agent spawn [--parent <here|id>] [--machine <device id>] --name <name> --intent <key> --kind <kind> --repo <path> --branch <branch> [--path <path>] [--model <model>] [--effort <effort>] [--brief <text> | --brief-file <path>] [-- <native args>]";
pub const SPAWN_HELP: &str = "hide agent spawn [--parent <here|id>] [--machine <device id>] --name <name> --intent <key> --kind <kind> --repo <path> --branch <branch> [--path <path>] [--model <model>] [--effort <effort>] [--brief <text> | --brief-file <path>] [-- <native args>]

Choose the model and effort with --model and --effort, not after --:
  Hide starts the agent with them and starts it with them again whenever it wakes, forks or reopens the agent.
  Native arguments after -- reach this start only; a restart resumes the conversation without them.

Give the agent its brief with --brief <text> or --brief-file <path> (at most 16 KiB):
  Hide starts the agent and submits the brief as its first prompt once the agent takes input,
  and records the same text as a delivered letter under the spawn's intent, so the agent starts on it
  without anyone typing and a later wake finds it. A mission node's worker (an intent of the form
  <mission-id>:<node-key>) needs one; without it the spawn is refused with mission_brief_missing.
  Running the same command again never submits the brief twice.

Choose responsibility when spawning:
  With --parent here (or your own id): delegate work you will supervise.
    The child belongs to you, appears below you, and starts an automatic watch.
  Without --parent: hand work off to the operator as an independent root.
    The operator handles its questions and completion; no automatic watch starts.

Both modes open their own tab without changing the current screen or keyboard focus.
The origin field records who spawned the agent in both modes; it grants no authority.
Use delegation for work you will collect and report, and handoff for independent work the operator will handle.
Responsibility cannot be changed after spawn; reusing an intent with another mode is refused.

Start the agent on a connected device with --machine <device id>, from the machine that runs Hide:
  The id is the device_id `hide workspace info` shows and the machine `hide agent list` shows for agents there.
  --repo and --path are then paths on that device. Your own id, or no --machine, starts the agent here.
  An agent running on a device cannot use --machine: it is refused with machine_not_permitted.
  A device id that is unknown or not connected, a repository missing on the device or an agent not installed there is refused before anything is created.
  Reusing an intent with another machine is refused; running the same command again returns the same agent.";

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<Delivery, String> {
    let verb = args.next().ok_or(USAGE)?.as_str();
    let mut flags = BTreeMap::new();
    let mut check = false;
    let mut id = None;
    let mut native = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => {}
            "--check" if verb == "register" && !check => check = true,
            "--" if verb == "spawn" => {
                native.extend(args.cloned());
                break;
            }
            flag if flag.starts_with("--") => {
                let allowed = match verb {
                    "register" => [
                        "--machine",
                        "--host-scope",
                        "--session",
                        "--instance",
                        "--name",
                        "--pane",
                        "--parent",
                        "--project",
                    ]
                    .as_slice(),
                    "spawn" => [
                        "--parent",
                        "--machine",
                        "--name",
                        "--intent",
                        "--kind",
                        "--repo",
                        "--branch",
                        "--path",
                        "--model",
                        "--effort",
                        "--brief",
                        "--brief-file",
                    ]
                    .as_slice(),
                    "end" => ["--actor"].as_slice(),
                    _ => &[],
                };
                if !allowed.contains(&flag) || flags.contains_key(flag) {
                    return Err(format!("Unknown or repeated flag: {flag}"));
                }
                let value = args
                    .next()
                    .filter(|value| {
                        !value.is_empty()
                            && !value.starts_with("--")
                            // A brief is prose with line breaks; every other
                            // value is one line.
                            && (flag == "--brief" || !value.chars().any(char::is_control))
                    })
                    .ok_or(USAGE)?
                    .clone();
                flags.insert(flag.to_owned(), value);
            }
            _ if matches!(verb, "show" | "end") && id.is_none() => id = Some(arg.clone()),
            _ => return Err(USAGE.into()),
        }
    }
    let mut take = |name: &str| {
        flags
            .remove(name)
            .ok_or_else(|| format!("Required flag: {name}"))
    };
    let command = match verb {
        "register" => {
            let host_scope = take("--host-scope")?;
            let session = take("--session")?;
            if !herdr_core::delivery::valid_session(&session) {
                return Err(format!(
                    "--session takes a native session id or session file path of at most {} bytes that does not start with -",
                    herdr_core::delivery::SESSION_LIMIT
                ));
            }
            let instance = take("--instance")?;
            let name = take("--name")?;
            let pane = take("--pane")?;
            Command::Register {
                check,
                machine: flags.remove("--machine"),
                host_scope,
                session,
                instance,
                name,
                pane,
                parent: flags.remove("--parent"),
                project: flags.remove("--project"),
            }
        }
        "spawn" => {
            let name = take("--name")?;
            let intent = take("--intent")?;
            let kind = take("--kind")?;
            let repo = take("--repo")?;
            let branch = take("--branch")?;
            let brief = brief(flags.remove("--brief"), flags.remove("--brief-file"))?;
            Command::Spawn {
                mission: None,
                brief,
                parent: flags.remove("--parent"),
                machine: flags.remove("--machine"),
                name,
                intent,
                kind,
                repo,
                branch,
                path: flags.remove("--path"),
                args: native,
                launch: hide_agent_adapter::LaunchChoice {
                    model: flags.remove("--model"),
                    effort: flags.remove("--effort"),
                },
                // Only Hide's own identity gives a task; no flag names one.
                task: None,
            }
        }
        "list" => Command::List,
        "show" => Command::Show {
            id: id.ok_or(USAGE)?,
        },
        "end" => Command::End {
            id: id.ok_or(USAGE)?,
            actor: flags.remove("--actor"),
        },
        _ => return Err(USAGE.into()),
    };
    Ok(Delivery::Agents { command })
}
/// The brief from `--brief` or `--brief-file`, one of them at most: the
/// file is read here, in the caller's own account, and refused before the
/// daemon is asked when it cannot be read, is blank or is past a letter's
/// body limit.
fn brief(text: Option<String>, file: Option<String>) -> Result<Option<String>, String> {
    let brief = match (text, file) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => return Err("Give --brief or --brief-file, not both".into()),
        (Some(text), None) => text,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map_err(|error| format!("--brief-file {path} could not be read: {error}"))?,
    };
    if brief.trim().is_empty() {
        return Err("The brief is blank".into());
    }
    if brief.len() > herdr_core::delivery::BODY_LIMIT {
        return Err(format!(
            "The brief is {} bytes; a brief holds at most {} bytes",
            brief.len(),
            herdr_core::delivery::BODY_LIMIT
        ));
    }
    Ok(Some(brief))
}

pub fn run(env: &Env, command: Delivery) -> Result<(), String> {
    let result = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        let hint = std::env::var(env::HERDR_PANE_ID).ok();
        crate::workspace_cli::request_delivery(&mut credential, command, hint.as_deref())
    })();
    match result {
        Ok(answer) if answer["ok"] == true => {
            println!(
                "{}",
                serde_json::json!({"ok":true,"value":answer["result"]})
            );
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("agent_unavailable")
                .to_owned();
            // The daemon's next step is what the caller can act on.
            let message = answer["next_action"].as_str().unwrap_or(&code);
            println!(
                "{}",
                serde_json::json!({"ok":false,"error":{"code":code,"message":message}})
            );
            Err(code)
        }
        Err(code) => {
            // A command that outlasts the wait may still be running; its
            // intent makes the same command return the same result.
            let message = if code == "request_timeout" {
                "The request may still be running; run the same command again to get its result"
            } else {
                code.as_str()
            };
            println!(
                "{}",
                serde_json::json!({"ok":false,"error":{"code":code,"message":message}})
            );
            Err(code)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn line(args: &[&str]) -> Result<Delivery, String> {
        let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        parse(args.iter())
    }
    #[test]
    fn spawn_preserves_native_arguments_and_refuses_retired_flags() {
        let args = [
            "spawn",
            "--parent",
            "here",
            "--name",
            "worker",
            "--intent",
            "one",
            "--kind",
            "codex",
            "--repo",
            "/fixture",
            "--branch",
            "topic",
            "--",
            "--model",
            "fixture model",
        ];
        let Delivery::Agents {
            command: Command::Spawn { args, parent, .. },
        } = line(&args).unwrap()
        else {
            panic!("spawn")
        };
        assert_eq!(parent.as_deref(), Some("here"));
        assert_eq!(args, ["--model", "fixture model"]);
        for flag in [
            "--no-watch",
            "--reconcile-pane",
            "--resume-start",
            "--session",
            "--interval",
        ] {
            assert!(line(&["spawn", flag, "value"]).is_err());
        }
    }
    #[test]
    fn machine_names_the_device_for_either_mode_and_is_in_every_help() {
        let spawn = |extra: &[&str]| {
            let mut args = vec![
                "spawn", "--name", "worker", "--intent", "one", "--kind", "codex", "--repo",
                "/fixture", "--branch", "topic",
            ];
            args.extend_from_slice(extra);
            line(&args)
        };
        for (extra, parent) in [
            (&["--machine", "mini"][..], None),
            (&["--parent", "here", "--machine", "mini"][..], Some("here")),
        ] {
            let Delivery::Agents {
                command:
                    Command::Spawn {
                        machine,
                        parent: named,
                        ..
                    },
            } = spawn(extra).unwrap()
            else {
                panic!("spawn")
            };
            assert_eq!(machine.as_deref(), Some("mini"));
            assert_eq!(named.as_deref(), parent);
        }
        let Delivery::Agents {
            command: Command::Spawn { machine, .. },
        } = spawn(&[]).unwrap()
        else {
            panic!("spawn")
        };
        assert_eq!(machine, None);
        assert!(spawn(&["--machine", "mini", "--machine", "studio"]).is_err());
        assert!(spawn(&["--machine"]).is_err());
        // The operator learns the flag from `hide --help` and from the spawn
        // help, which says where its value comes from.
        assert!(USAGE.contains("[--machine <device id>]"));
        assert!(
            SPAWN_HELP.starts_with("hide agent spawn [--parent <here|id>] [--machine <device id>]")
        );
        assert!(SPAWN_HELP.contains("`hide workspace info`"));
        assert!(SPAWN_HELP.contains("`hide agent list`"));
    }

    #[test]
    fn spawn_takes_the_model_and_effort_as_its_launch_choice_apart_from_native_arguments() {
        let spawn = |extra: &[&str]| {
            let mut args = vec![
                "spawn", "--name", "worker", "--intent", "one", "--kind", "claude", "--repo",
                "/fixture", "--branch", "topic",
            ];
            args.extend_from_slice(extra);
            line(&args)
        };
        let Delivery::Agents {
            command: Command::Spawn { launch, args, .. },
        } = spawn(&["--model", "sonnet", "--effort", "high", "--", "--verbose"]).unwrap()
        else {
            panic!("spawn")
        };
        assert_eq!(
            (launch.model.as_deref(), launch.effort.as_deref()),
            (Some("sonnet"), Some("high"))
        );
        assert_eq!(args, ["--verbose"]);
        let Delivery::Agents {
            command: Command::Spawn { launch, .. },
        } = spawn(&[]).unwrap()
        else {
            panic!("spawn")
        };
        assert!(launch.is_default());
        assert!(spawn(&["--effort", "high", "--effort", "low"]).is_err());
        assert!(SPAWN_HELP.contains("[--model <model>] [--effort <effort>]"));
    }

    /// The brief rides the spawn: from the flag or from a file, never both,
    /// with line breaks kept, and refused before the daemon is asked when it
    /// is blank or past a letter's body (PRD mission fixes (a)).
    #[test]
    fn spawn_takes_its_brief_from_text_or_a_file_and_explains_it_in_its_help() {
        let spawn = |extra: &[&str]| {
            let mut args = vec![
                "spawn",
                "--name",
                "worker",
                "--intent",
                "mission-1:api",
                "--kind",
                "claude",
                "--repo",
                "/fixture",
                "--branch",
                "topic",
            ];
            args.extend_from_slice(extra);
            line(&args)
        };
        let brief_of = |parsed: Delivery| {
            let Delivery::Agents {
                command: Command::Spawn { brief, .. },
            } = parsed
            else {
                panic!("spawn")
            };
            brief
        };
        assert_eq!(brief_of(spawn(&[]).unwrap()), None);
        assert_eq!(
            brief_of(spawn(&["--brief", "Node api: add the call.\nReport by letter."]).unwrap()),
            Some("Node api: add the call.\nReport by letter.".into())
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("brief-api.md");
        std::fs::write(&file, "# api\n\nAdd the call.\n").unwrap();
        assert_eq!(
            brief_of(spawn(&["--brief-file", file.to_str().unwrap()]).unwrap()),
            Some("# api\n\nAdd the call.\n".into())
        );
        assert!(spawn(&["--brief", "x", "--brief-file", file.to_str().unwrap()]).is_err());
        assert!(
            spawn(&[
                "--brief-file",
                dir.path().join("missing.md").to_str().unwrap()
            ])
            .is_err()
        );
        assert!(spawn(&["--brief", " \n "]).is_err());
        let long = "x".repeat(herdr_core::delivery::BODY_LIMIT + 1);
        assert!(spawn(&["--brief", &long]).is_err());
        assert!(USAGE.contains("[--brief <text> | --brief-file <path>]"));
        assert!(SPAWN_HELP.contains("--brief-file <path>"));
        assert!(SPAWN_HELP.contains("mission_brief_missing"));
    }

    #[test]
    fn omitted_parent_selects_handoff() {
        let Delivery::Agents {
            command: Command::Spawn { parent, args, .. },
        } = line(&[
            "spawn",
            "--name",
            "worker",
            "--intent",
            "one",
            "--kind",
            "codex",
            "--repo",
            "/fixture",
            "--branch",
            "topic",
            "--",
            "--model",
            "fixture model",
        ])
        .unwrap()
        else {
            panic!("spawn")
        };
        assert!(parent.is_none());
        assert_eq!(args, ["--model", "fixture model"]);
    }

    #[test]
    fn register_surface_matches_external_caller() {
        let register = |machine: &[&str]| {
            let mut args = vec!["register", "--check"];
            args.extend_from_slice(machine);
            args.extend_from_slice(&[
                "--host-scope",
                "fixture",
                "--session",
                "session",
                "--instance",
                "terminal",
                "--name",
                "worker",
                "--pane",
                "w1:p1",
                "--json",
            ]);
            match line(&args) {
                Ok(Delivery::Agents {
                    command: Command::Register { machine, .. },
                }) => machine,
                other => panic!("register: {other:?}"),
            }
        };
        assert_eq!(register(&[]), None);
        assert_eq!(register(&["--machine", "mini"]).as_deref(), Some("mini"));
        assert!(line(&["list", "unexpected"]).is_err());
        assert!(line(&["end", "agent-1", "--actor", "agent-2"]).is_ok());
    }
}
