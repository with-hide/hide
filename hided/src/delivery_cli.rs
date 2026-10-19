//! Small typed command surface for the core-owned durable mailbox.

use herdr_core::delivery::{BODY_LIMIT, Command, HOOK_LETTERS};

use crate::env::{self, Env};

pub const USAGE: &str = "hide request send <target> --intent <key> --body <text> [--kind request|block|report|progress] [--now]\nhide request reply <id> --intent <key> --body <text>\nhide request ack|cancel|show <id>\nhide inbox [--hook [--prompt-digest <sha256>] [--session <id>] [--tool|--turn-end]]\nhide watch start <target> [--observer <id>] [--actor <id>]\nhide watch assign <watch-or-target-id> --observer <id> [--actor <id>] [--expected-generation <n>] [--approval <text>]\nhide watch stop <id>\nhide watch list\nDelivery commands require a running daemon and a current agent pane; they do not require an open renderer.";

pub fn parse<'a>(
    topic: &str,
    mut args: impl Iterator<Item = &'a String>,
) -> Result<Command, String> {
    let verb = args.next().map(String::as_str);
    if topic == "inbox" {
        return match verb {
            None => Ok(Command::Inbox),
            Some("--hook") => {
                let mut prompt_digest = None;
                let mut session = None;
                let mut point = hide_node_link::panes::PullPoint::Prompt;
                while let Some(flag) = args.next() {
                    match flag.as_str() {
                        "--tool" if point == hide_node_link::panes::PullPoint::Prompt => {
                            point = hide_node_link::panes::PullPoint::Tool
                        }
                        "--turn-end" if point == hide_node_link::panes::PullPoint::Prompt => {
                            point = hide_node_link::panes::PullPoint::TurnEnd
                        }
                        "--prompt-digest" if prompt_digest.is_none() => {
                            prompt_digest = Some(
                                args.next()
                                    .filter(|digest| {
                                        herdr_core::delivery::valid_prompt_digest(digest)
                                    })
                                    .ok_or(USAGE)?
                                    .clone(),
                            );
                        }
                        "--session" if session.is_none() => {
                            session = Some(
                                args.next()
                                    .filter(|id| herdr_core::delivery::valid_session(id))
                                    .ok_or(USAGE)?
                                    .clone(),
                            );
                        }
                        _ => return Err(USAGE.into()),
                    }
                }
                // A tool event's ask names its session.
                if point != hide_node_link::panes::PullPoint::Prompt && session.is_none() {
                    return Err(USAGE.into());
                }
                Ok(Command::Pull {
                    prompt_digest,
                    session,
                    point,
                })
            }
            Some("--confirm") => {
                let ids: Vec<_> = args.cloned().collect();
                if ids.is_empty() || ids.len() > HOOK_LETTERS || !ids.iter().all(|id| key(id)) {
                    return Err(USAGE.into());
                }
                Ok(Command::Confirm { ids })
            }
            _ => Err(USAGE.into()),
        };
    }
    if topic == "watch" && verb == Some("list") && args.next().is_none() {
        return Ok(Command::WatchList);
    }
    let subject = args.next().filter(|value| key(value)).ok_or(USAGE)?.clone();
    if topic == "watch" {
        let mut observer = None;
        let mut actor = None;
        let mut expected_generation = None;
        let mut approval = None;
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(USAGE)?;
            match flag.as_str() {
                "--observer" if observer.is_none() && key(value) => observer = Some(value.clone()),
                "--actor" if actor.is_none() && key(value) => actor = Some(value.clone()),
                "--approval"
                    if approval.is_none()
                        && verb == Some("assign")
                        && herdr_core::delivery::watch::valid_approval(value) =>
                {
                    approval = Some(value.clone());
                }
                "--expected-generation"
                    if expected_generation.is_none() && verb == Some("assign") =>
                {
                    expected_generation = Some(value.parse::<u64>().map_err(|_| USAGE)?);
                }
                _ => return Err(USAGE.into()),
            }
        }
        return match verb {
            Some("start") => Ok(Command::WatchStart {
                target: subject,
                observer,
                actor,
            }),
            Some("assign") if approval.is_none() || expected_generation.is_some() => {
                Ok(Command::WatchAssign {
                    id: subject,
                    observer: observer.ok_or(USAGE)?,
                    actor,
                    expected_generation,
                    approval,
                })
            }
            Some("stop") if observer.is_none() && actor.is_none() => {
                Ok(Command::WatchStop { id: subject })
            }
            _ => Err(USAGE.into()),
        };
    }
    match verb {
        Some("send" | "reply") => {
            let mut intent = None;
            let mut body = None;
            let mut kind = None;
            let mut now = false;
            while let Some(flag) = args.next() {
                if flag == "--now" && verb == Some("send") && !now {
                    now = true;
                    continue;
                }
                let value = args.next().ok_or(USAGE)?.clone();
                match flag.as_str() {
                    "--intent" if intent.is_none() && key(&value) => intent = Some(value),
                    "--body" if body.is_none() => body = Some(value),
                    "--kind"
                        if kind.is_none()
                            && verb == Some("send")
                            && matches!(
                                value.as_str(),
                                "request" | "block" | "report" | "progress"
                            ) =>
                    {
                        kind = Some(value)
                    }
                    _ => return Err(USAGE.into()),
                }
            }
            let intent = intent.ok_or(USAGE)?;
            let body = body
                .filter(|body: &String| !body.trim().is_empty())
                .ok_or(USAGE)?;
            if body.len() > BODY_LIMIT {
                return Err("capacity".into());
            }
            if verb == Some("send") {
                Ok(Command::Send {
                    target: subject,
                    intent,
                    body,
                    kind: kind.unwrap_or_else(|| "request".into()),
                    now,
                })
            } else {
                Ok(Command::Reply {
                    id: subject,
                    intent,
                    body,
                })
            }
        }
        Some("ack" | "cancel" | "show") if args.next().is_none() => match verb {
            Some("ack") => Ok(Command::Ack { id: subject }),
            Some("cancel") => Ok(Command::Cancel { id: subject }),
            _ => Ok(Command::Show { id: subject }),
        },
        _ => Err(USAGE.into()),
    }
}

fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && !value.chars().any(char::is_control)
}

pub fn run(env: &Env, command: Command) -> Result<(), String> {
    let result = (|| {
        if let Command::Pull {
            prompt_digest,
            session,
            point,
        } = &command
        {
            return crate::workspace_cli::request_hook(
                env,
                hide_node_link::panes::HookCommand::LetterPull {
                    prompt_digest: prompt_digest.clone(),
                    session: session.clone(),
                    point: *point,
                },
            );
        }
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        let hint = std::env::var(env::HERDR_PANE_ID).ok();
        crate::workspace_cli::request_delivery(&mut credential, command, hint.as_deref())
    })();
    match result {
        Ok(answer) => {
            println!("{answer}");
            if answer["ok"] == true {
                Ok(())
            } else {
                Err(answer["reason"]
                    .as_str()
                    .unwrap_or("delivery_unavailable")
                    .to_owned())
            }
        }
        Err(code) => {
            println!(
                "{}",
                serde_json::json!({"ok":false,"reason":code,"next_action":"Check the running daemon and current agent pane, then retry the same intent"})
            );
            Err(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse_line(line: &[&str]) -> Result<Command, String> {
        let args: Vec<_> = line.iter().map(|value| (*value).to_owned()).collect();
        parse(&args[0], args.iter().skip(1))
    }
    #[test]
    fn typed_cli_requires_intent_and_has_no_wait_or_dispatch_surface() {
        assert_eq!(parse_line(&["inbox"]).unwrap(), Command::Inbox);
        assert!(parse_line(&["inbox", "wait"]).is_err());
        let digest = herdr_core::delivery::prompt_digest("prompt");
        assert_eq!(
            parse_line(&["inbox", "--hook"]).unwrap(),
            Command::Pull {
                prompt_digest: None,
                session: None,
                point: hide_node_link::panes::PullPoint::Prompt,
            }
        );
        assert_eq!(
            parse_line(&["inbox", "--hook", "--prompt-digest", &digest]).unwrap(),
            Command::Pull {
                prompt_digest: Some(digest.clone()),
                session: None,
                point: hide_node_link::panes::PullPoint::Prompt,
            }
        );
        assert_eq!(
            parse_line(&[
                "inbox",
                "--hook",
                "--session",
                "abc",
                "--prompt-digest",
                &digest
            ])
            .unwrap(),
            Command::Pull {
                prompt_digest: Some(digest.clone()),
                session: Some("abc".into()),
                point: hide_node_link::panes::PullPoint::Prompt,
            }
        );
        assert_eq!(
            parse_line(&["inbox", "--hook", "--session", "abc", "--tool"]).unwrap(),
            Command::Pull {
                prompt_digest: None,
                session: Some("abc".into()),
                point: hide_node_link::panes::PullPoint::Tool
            }
        );
        assert!(parse_line(&["inbox", "--hook", "--tool"]).is_err());
        assert_eq!(
            parse_line(&["inbox", "--hook", "--session", "abc", "--turn-end"]).unwrap(),
            Command::Pull {
                prompt_digest: None,
                session: Some("abc".into()),
                point: hide_node_link::panes::PullPoint::TurnEnd,
            }
        );
        assert!(parse_line(&["inbox", "--hook", "--turn-end"]).is_err());
        assert!(
            parse_line(&[
                "inbox",
                "--hook",
                "--session",
                "abc",
                "--tool",
                "--turn-end"
            ])
            .is_err()
        );
        assert!(parse_line(&["inbox", "--hook", "--session", "abc", "--tool", "--tool"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--prompt-digest", &digest, "--tool"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--prompt-digest", &digest, "extra"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--prompt-digest", "prompt"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--prompt-digest"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--bell"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--session"]).is_err());
        assert!(parse_line(&["inbox", "--hook", "--session", "-x"]).is_err());
        assert!(parse_line(&["inbox", "--prompt-digest", &digest]).is_err());
        assert!(parse_line(&["request", "send", "target", "--body", "body"]).is_err());
        assert_eq!(
            parse_line(&[
                "request", "send", "target", "--intent", "key", "--body", "body"
            ])
            .unwrap(),
            Command::Send {
                target: "target".into(),
                intent: "key".into(),
                body: "body".into(),
                kind: "request".into(),
                now: false,
            }
        );
        assert!(parse_line(&["watch", "start", "target", "--force"]).is_err());
        assert!(parse_line(&["inbox", "--confirm", "a", "b", "c", "d", "e", "f"]).is_err());
    }

    #[test]
    fn reports_blocks_and_reassignment_have_typed_bounded_flags() {
        for kind in ["report", "block", "progress"] {
            assert!(
                matches!(parse_line(&["request","send","parent","--intent","done","--body","body","--kind",kind]).unwrap(), Command::Send {kind: parsed, now: false, ..} if parsed == kind)
            );
        }
        assert!(matches!(
            parse_line(&[
                "request", "send", "parent", "--now", "--intent", "done", "--body", "body"
            ])
            .unwrap(),
            Command::Send { now: true, .. }
        ));
        assert!(
            parse_line(&[
                "request", "send", "parent", "--intent", "done", "--body", "body", "--now", "--now"
            ])
            .is_err()
        );
        assert!(
            parse_line(&[
                "request", "reply", "letter-1", "--intent", "done", "--body", "body", "--now"
            ])
            .is_err()
        );
        assert!(matches!(
            parse_line(&[
                "watch",
                "assign",
                "watch-1",
                "--observer",
                "next",
                "--actor",
                "parent",
                "--expected-generation",
                "2"
            ])
            .unwrap(),
            Command::WatchAssign {
                expected_generation: Some(2),
                ..
            }
        ));
        assert!(parse_line(&["watch", "start", "target", "--interval", "60"]).is_err());
        assert!(
            parse_line(&[
                "watch",
                "assign",
                "target",
                "--observer",
                "next",
                "--expected-generation",
                "-1"
            ])
            .is_err()
        );
        assert!(
            parse_line(&[
                "request", "reply", "letter-1", "--intent", "done", "--body", "body", "--kind",
                "report"
            ])
            .is_err()
        );
    }

    #[test]
    fn handover_approval_is_explicit_bounded_and_requires_a_generation() {
        assert!(matches!(
            parse_line(&["watch", "assign", "watch-1", "--observer", "next", "--approval", "approved", "--expected-generation", "1"]).unwrap(),
            Command::WatchAssign { approval: Some(text), expected_generation: Some(1), .. } if text == "approved"
        ));
        for approval in ["", " ", "line\nbreak"] {
            assert!(
                parse_line(&[
                    "watch",
                    "assign",
                    "watch-1",
                    "--observer",
                    "next",
                    "--approval",
                    approval,
                    "--expected-generation",
                    "1"
                ])
                .is_err()
            );
        }
        assert!(
            parse_line(&[
                "watch",
                "assign",
                "watch-1",
                "--observer",
                "next",
                "--approval",
                "approved"
            ])
            .is_err()
        );
        assert!(!herdr_core::delivery::watch::valid_approval(
            &"x".repeat(257)
        ));
    }
}
