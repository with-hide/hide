//! TEMPORARY (with-hide/hide-private#405 measurement, removed before merge):
//! how a holder's color answer reaches a Windows program's console input,
//! read as a Go TUI reads it (`probe405.py`). Run by name with
//! `--run-ignored only`; `HIDE_PROBE_405_OUT` names the JSON it writes.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::support::{Home, Start, current_version, node_theme, source, start_themed, wait_until};

#[test]
#[ignore = "with-hide/hide-private#405 measurement, run by name"]
fn probe_405_color_answer_records() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let out = std::env::var_os("HIDE_PROBE_405_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.path().join("probe405.json"));
    let _ = std::fs::remove_file(&out);
    let probe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/it/probe405.py");
    let program: Vec<OsString> = vec!["python".into(), probe.into(), out.clone().into()];
    let holder = match start_themed(
        &folder.program,
        &home.socket("p"),
        &program,
        &[],
        home.path(),
        home.path(),
        Some(&node_theme()),
    ) {
        Start::Ready(holder) => holder,
        Start::Failed(reason) => panic!("the holder did not start: {reason}"),
    };
    // 120 rounds of at most 2 s each would pass the shared bound; each one
    // waits for its answer, so the probe ends in seconds when answers come.
    for _ in 0..10 {
        if out.exists() {
            break;
        }
        let _ = std::panic::catch_unwind(|| wait_until("the probe's report", || out.exists()));
    }
    let report = std::fs::read_to_string(&out).expect("the probe wrote no report");
    let document: serde_json::Value = serde_json::from_str(&report).unwrap();
    println!("probe405 summary {}", document["summary"]);
    for round in document["rounds"].as_array().unwrap() {
        if round["split"] == true || round["end"] != "whole" {
            println!("probe405 round {round}");
        }
    }
    drop(holder);
}
