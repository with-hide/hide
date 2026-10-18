//! The journey reads its cache location and cap from the layout owner.
fn main() {
    let home = std::env::args_os()
        .nth(1)
        .expect("usage: journey_cache <home>");
    println!(
        "{}",
        serde_json::json!({
            "path": hide_kit::layout::journey_cli_cache(std::path::Path::new(&home)),
            "cap_bytes": hide_kit::layout::JOURNEY_CLI_CACHE_BYTES,
        })
    );
}
