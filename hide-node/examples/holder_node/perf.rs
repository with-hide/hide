//! The measurements' requests (docs/PERFORMANCE_TESTING.md, The pane
//! holder's path): key-to-frame latency at the node, frame totals, the slow
//! consumer and attaching a Herdr pane, each the same on the Herdr path and
//! the holder path.

use std::time::{Duration, Instant};

use hide_node_link::terminal::{KeyTarget, TerminalControl, TerminalNode};
use serde_json::{Value, json};

use crate::{Node, grid, number, text, wait_limit};

pub fn answer(node: &Node, op: &str, request: &Value) -> Result<Value, String> {
    Ok(match op {
        "attach_herdr" => {
            let pane = text(request, "pane")?;
            let size = grid(request);
            node.terminals.service.control(TerminalControl::Attach {
                pane: pane.clone(),
                size: Some(size),
                manual: false,
            });
            node.terminals.service.view(&pane, size, true);
            json!({})
        }
        "wait_drawn" => {
            let panes: Vec<String> = request["panes"]
                .as_array()
                .ok_or("`panes` is required")?
                .iter()
                .filter_map(|pane| pane.as_str().map(str::to_owned))
                .collect();
            let drawn = node.screens.wait_drawn(&panes, wait_limit(request));
            json!({"drawn": drawn, "panes_drawn": node.screens.drawn()})
        }
        "latency" => latency(node, request)?,
        "totals" => {
            let (frames, bytes, each) = node.screens.totals();
            json!({"frames": frames, "bytes": bytes,
                   "panes": each.into_iter().map(|(pane, count, bytes)| json!({"pane": pane, "frames": count, "bytes": bytes})).collect::<Vec<_>>()})
        }
        "slow" => {
            node.screens.set_slow(number(request, "micros", 0));
            json!({})
        }
        _ => return Err(format!("unknown op {op}")),
    })
}

/// Types `count` tokens into `pane` one after another, each once the one
/// before it was drawn (or given up on), and answers how long each took to
/// reach the node's screen in microseconds; a token never drawn within the
/// limit is lost.
fn latency(node: &Node, request: &Value) -> Result<Value, String> {
    let pane = text(request, "pane")?;
    let count = number(request, "count", 200);
    let prefix = request["prefix"].as_str().unwrap_or("k");
    let gap = Duration::from_millis(number(request, "gap_ms", 0));
    let limit = wait_limit(request);
    let mut micros = Vec::with_capacity(count as usize);
    let mut lost = Vec::new();
    let started = Instant::now();
    for index in 0..count {
        let token = format!("<{prefix}{index:06}>");
        node.screens.arm(&pane, token.as_bytes());
        let typed = Instant::now();
        node.terminals
            .service
            .key(KeyTarget::Pane(pane.clone()), token.clone().into_bytes(), 0);
        match node.screens.wait_seen(limit) {
            Some(seen) => micros.push(seen.saturating_duration_since(typed).as_micros() as u64),
            None => lost.push(token),
        }
        if !gap.is_zero() {
            let until = Instant::now() + gap;
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                if left.is_zero() {
                    break;
                }
                std::thread::park_timeout(left);
            }
        }
    }
    Ok(json!({
        "micros": micros,
        "lost": lost,
        "seconds": started.elapsed().as_secs_f64(),
    }))
}
