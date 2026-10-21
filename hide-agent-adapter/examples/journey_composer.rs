//! The agent journey check's composer judge (`scripts/agent_journey`): one
//! screen Herdr read with styling, judged by the same declaration and parser
//! the core uses before it releases a held letter. Not a product binary.
//!
//! journey_composer <agent id or Herdr kind> <screen file>
//! prints {"verdict": "empty" | "not_empty" | "not_found"}, or exits non-zero
//! with the reason on stderr: 2 for a wrong call, 1 for an agent that is
//! unknown or declares no composer, or a screen that cannot be read.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [agent, screen] = args.as_slice() else {
        eprintln!("usage: journey_composer <agent id or Herdr kind> <screen file>");
        std::process::exit(2);
    };
    match judge(agent, screen) {
        Ok(code) => println!("{}", serde_json::json!({ "verdict": code })),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

fn judge(agent: &str, screen: &str) -> Result<&'static str, String> {
    let row = hide_agent_adapter::adapter(agent).ok_or("agent_unknown")?;
    let composer = row.composer.supported().ok_or("composer_undeclared")?;
    let screen =
        std::fs::read_to_string(screen).map_err(|error| format!("screen_unreadable: {error}"))?;
    Ok(composer.judge(&screen).code())
}
