fn main() {
    // `hide ... | head` closes our stdout early; end quietly, not with a panic.
    hide_platform::stdio::exit_quietly_when_stdout_closes();
    // Started by ssh as its askpass while Hide connects a device, `hide`
    // hands ssh's question to the device row and prints the answer (PRD
    // system-ssh-device-link D-05); the prompt may be any bytes.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(code) = hide_node::ssh::askpass::client(&raw) {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().collect();
    let parsed = hided::cli::parse_args(&args);
    if let Err(error) = &parsed
        && matches!(
            args.get(1).map(String::as_str),
            Some("workspace" | "file" | "diff" | "view" | "browser")
        )
    {
        println!(
            "{}",
            serde_json::json!({
                "ok": false,
                "reason": "invalid_arguments",
                "next_action": error,
            })
        );
        std::process::exit(2);
    }
    match parsed.and_then(hided::cli::run) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
