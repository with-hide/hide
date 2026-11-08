//! Exercise the node boundary: native roots in, content-free accounting out.
use hide_host::serve::{Env, handle_in};
use hide_node_link::{ErrorCode, protocol::Call};
use hide_session::{Agent, usage};

#[test]
fn usage_node_reads_native_metadata_and_resumes_without_conversation_content() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::standalone(Some(home.path().to_owned()));
    let root = home.path().join(".claude/projects");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("native.jsonl");
    std::fs::write(&path, concat!(
        r#"{"type":"assistant","sessionId":"fixture","requestId":"req","timestamp":"2026-10-01T12:00:00Z","message":{"id":"msg","model":"test-model","content":[{"text":"PRIVATE_BODY_SENTINEL"}],"usage":{"input_tokens":3,"output_tokens":7}}}"#,
        "\n"
    )).unwrap();
    let listed: usage::Candidates = serde_json::from_value(
        handle_in(
            Call::UsageFiles {
                agent: Agent::Claude,
                since_unix_ms: 0,
            },
            &env,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(listed.files.len(), 1);
    assert_eq!(
        listed.files[0].path,
        hide_platform::path::to_wire(&path).unwrap()
    );
    assert!(!listed.files[0].path.contains('\\'));
    let call = Call::UsageRead {
        agent: Agent::Claude,
        path: listed.files[0].path.clone(),
        checkpoint: None,
    };
    let wire = serde_json::to_value(&call).unwrap();
    let answer = handle_in(serde_json::from_value(wire).unwrap(), &env).unwrap();
    assert!(!answer.to_string().contains("PRIVATE_BODY_SENTINEL"));
    let first: usage::Step = serde_json::from_value(answer).unwrap();
    assert_eq!(first.events.len(), 1);
    assert_eq!(
        first.events[0].tokens.input + first.events[0].tokens.output,
        10
    );
    let resumed: usage::Step = serde_json::from_value(
        handle_in(
            Call::UsageRead {
                agent: Agent::Claude,
                path: listed.files[0].path.clone(),
                checkpoint: Some(first.checkpoint),
            },
            &env,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(resumed.events.is_empty());

    let outside = home.path().join("private.jsonl");
    std::fs::write(&outside, "PRIVATE_BODY_SENTINEL").unwrap();
    let refused = handle_in(
        Call::UsageRead {
            agent: Agent::Claude,
            path: hide_platform::path::to_wire(&outside).unwrap(),
            checkpoint: None,
        },
        &env,
    )
    .unwrap_err();
    assert!(!refused.message.contains("private.jsonl"));
    assert!(!refused.message.contains("PRIVATE_BODY_SENTINEL"));
    assert_eq!(
        handle_in(
            Call::UsageFiles {
                agent: Agent::Cursor,
                since_unix_ms: 0,
            },
            &env
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
}
