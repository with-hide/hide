fn main() {
    let rows = hide_agent_adapter::journey_contract().collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&rows).expect("static contract serializes")
    );
}
