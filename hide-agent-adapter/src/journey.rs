//! The agent journey check's contract (`scripts/agent_journey`): which
//! journey cells each adapter declares and the launch facts a spawn of it
//! takes, projected from the declarations so the check never reads Rust
//! source and never keeps a second support table.

#[derive(serde::Serialize)]
pub struct JourneyAdapter {
    pub id: &'static str,
    /// The agent kind `hide agent spawn --kind` takes and Herdr reports.
    pub herdr_kind: &'static str,
    /// What separates the spawn's native arguments from its first prompt;
    /// absent for an agent Hide cannot start.
    pub prompt_flag: Option<&'static str>,
    /// How the start command takes a model, `{}` replaced by the value;
    /// empty when the agent takes none.
    pub model_flag: &'static [&'static str],
    /// The name a row shows for the agent while it has no task title, which
    /// the `title` cell must see replaced (`sidebar::provider_name`).
    pub untitled: &'static str,
    /// Every journey cell, `true` when the adapter declares what it measures;
    /// an undeclared cell reads N/A, never a failure.
    pub cells: JourneyCells,
    /// The most model requests a signed-in CI run of the journey may make
    /// for this agent, which its lane fails above: each turn the declared
    /// cells start, at most `REQUESTS_PER_TURN` each; absent for an agent
    /// Hide cannot start.
    pub request_budget: Option<u32>,
}

/// A token-only turn takes one model response; the rest is room for an
/// agent that answers in more than one (a tool call it chose, a retry it
/// makes itself), and a run past it means the journey's turns are wrong.
pub const REQUESTS_PER_TURN: u32 = 3;

#[derive(serde::Serialize)]
pub struct JourneyCells {
    /// Started through `hide agent spawn`, in a folder the account trusts.
    pub start: bool,
    /// Started in a folder the account never trusted.
    pub start_untrusted: bool,
    /// The first turn's reply, read by Hide's conversation reader.
    pub reply: bool,
    /// Herdr's own integration reports the session.
    pub integration: bool,
    /// The start guidance and Memory reach the first turn.
    pub guidance_memory: bool,
    /// A letter reaches the agent and its reply.
    pub letter: bool,
    pub sleep_wake: bool,
    /// Wake keeps the native session.
    pub resume: bool,
    pub fork: bool,
    pub title: bool,
    pub usage: bool,
}

pub fn journey_contract() -> impl Iterator<Item = JourneyAdapter> {
    super::ADAPTERS.iter().map(|row| {
        let start = row.start.is_some();
        let cells = JourneyCells {
            start,
            start_untrusted: start,
            reply: start && row.conversation.is_some(),
            integration: start,
            guidance_memory: start && row.memory.is_some(),
            letter: start && (row.bell || row.tool_letters.is_some()),
            sleep_wake: start && row.sleep.is_some(),
            resume: start && row.sleep.is_some() && row.resume.is_some(),
            fork: start && row.fork.is_some(),
            title: start && row.titles.is_some(),
            usage: start && matches!(row.usage.tokens, super::usage::Support::Supported(_)),
        };
        // The turns a run starts: the first prompt in the trusted and the
        // never-trusted start, the question about the Memory card the start
        // gave, the letter, the prompt typed into the fork (an agent that
        // tells its session only at its first turn, as Codex does, is
        // registered as a fork only then, and the operator types into a fork
        // to use it) and the prompt typed after Wake. The other cells judge
        // what Hide shows of those turns and start none.
        let turns = [
            start,
            cells.start_untrusted,
            cells.guidance_memory,
            cells.letter,
            cells.fork,
            cells.resume,
        ]
        .into_iter()
        .filter(|&turn| turn)
        .count() as u32;
        JourneyAdapter {
            id: row.id,
            herdr_kind: row.herdr.name,
            prompt_flag: row.start.map(|start| start.prompt_flag()),
            model_flag: row.start.map_or(&[], |start| start.options().model),
            untitled: row.sidebar_label.unwrap_or(row.herdr.name),
            cells,
            request_budget: start.then_some(turns * REQUESTS_PER_TURN),
        }
    })
}
