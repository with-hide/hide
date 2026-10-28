//! The agent journey check's Project Memory card (`scripts/agent_journey`):
//! one rule card written into a run's private state for the measured
//! repository, through the store's own writer, so the `guidance_memory` cell
//! can ask whether Hide gives it to an agent at its start. Not a product binary.
//!
//! journey_memory <state dir> <repository> <device id> <title>
//! turns the project's Memory on, writes the card with `<title>` as its
//! session-start index line and body, and prints {"card": "m<N>", "project":
//! <id>}, or exits 1 with the store's refusal on stderr.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [state, repository, device, title] = args.as_slice() else {
        eprintln!("usage: journey_memory <state dir> <repository> <device id> <title>");
        std::process::exit(2);
    };
    match seed(Path::new(state), Path::new(repository), device, title) {
        Ok(answer) => println!("{answer}"),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

fn seed(
    state: &Path,
    repository: &Path,
    device: &str,
    title: &str,
) -> Result<serde_json::Value, String> {
    let project = hide_project::resolve(repository, device)
        .map_err(|error| format!("project_unresolved: {error}"))?;
    let mut store = hide_memory::MemoryStore::open(&hide_memory::database_path(state))
        .map_err(|error| format!("memory_store_unavailable: {error}"))?;
    store
        .ensure_project(&project.id, &project.root, device)
        .and_then(|()| store.set_enabled(&project.id, true, true))
        .map_err(|error| format!("memory_project_refused: {error}"))?;
    let summary = store
        .apply_candidates(
            &hide_memory::AnalysisBatch {
                id: "agent-journey".into(),
                project_id: project.id.clone(),
                provider: "journey".into(),
                analysis_provider: "journey".into(),
                session_id: "agent-journey".into(),
                content_hash: title.into(),
                created_at_unix_ms: 1,
            },
            &[hide_memory::Candidate {
                text: title.into(),
                title: title.into(),
                aliases: Vec::new(),
                kind: hide_memory::CandidateKind::Rule,
                confidence: 0.9,
                salience: 0.9,
                source_offsets: vec![1],
                human_offsets: vec![1],
                source_times: Default::default(),
                source_words: Default::default(),
                cited_paths: Vec::new(),
                relation: hide_memory::CandidateRelation::New,
            }],
        )
        .map_err(|error| format!("memory_card_refused: {error}"))?;
    match summary.outcomes.as_slice() {
        [hide_memory::CandidateOutcome::Created { number }] => Ok(serde_json::json!({
            "card": hide_memory::card_name(*number),
            "project": project.id,
        })),
        outcomes => Err(format!("memory_card_not_created: {outcomes:?}")),
    }
}
