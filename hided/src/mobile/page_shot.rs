//! A page View's screenshot for the phone (PRD mobile-companion-ui B20-B22,
//! D-11): what the desktop app shows of the page now, taken by the same
//! page-control path `hide browser screenshot` uses.
//!
//! The chain is the CLI's, without a pane to start from: the core lists the
//! View's checkout (a checkout caller, never a pane) for the View's area and
//! whether it is its area's front tab, `BrowserControl::relay` opens the
//! desktop window's gateway, and `browser_page::still` attaches and takes
//! the picture, refusing a hidden display with `display_hidden` as the CLI
//! does. Nothing here selects a View, moves keyboard focus or raises a
//! window (B22): a page that is not in front is answered "cannot be taken
//! now", never brought forward.
//!
//! Why a screenshot cannot be taken is the operator's to know, not the
//! phone's (design principle 13): every refusal is one `look.page_unavailable`
//! diagnostic with its reason, and the phone is told only that it could not.

use std::sync::Arc;
use std::time::Duration;

use herdr_core::LookView;
use herdr_core::workspace_control::{Query, checkout_caller_id};
use serde_json::json;

use super::{Mobile, now_ms};
use crate::browser_control::Relay;

/// How long a screenshot may take from the first request to the last byte.
/// The page commands' own steps are shorter; this ends a gateway that never
/// answers instead of holding the phone's refresh.
const DEADLINE: Duration = Duration::from_secs(20);

/// The key of the checkout caller the phone's reads run as: any letters and
/// digits, since the core finds the checkout by its path.
const CALLER_KEY: &str = "phone";

/// One picture of a page: the image, when it was taken and by which machine's
/// desktop app.
pub struct Shot {
    pub jpeg: Vec<u8>,
    pub taken_unix_ms: u64,
    pub device_id: String,
}

/// Takes the screenshot of `view`, a browser View the phone is allowed to
/// see. `Err` is the reason it cannot be taken, already logged.
pub async fn take(mobile: &Arc<Mobile>, view: &LookView, image_cap: u64) -> Result<Shot, String> {
    let outcome = tokio::time::timeout(DEADLINE, shoot(mobile, view))
        .await
        .unwrap_or_else(|_| Err("timeout".to_owned()))
        .and_then(|jpeg| {
            // The cap the phone checks too (D-10): a picture past it is not sent.
            if jpeg.len() as u64 > image_cap {
                Err("too_large".to_owned())
            } else {
                Ok(jpeg)
            }
        });
    match outcome {
        Ok(jpeg) => Ok(Shot {
            jpeg,
            taken_unix_ms: now_ms(),
            device_id: mobile.node().as_str().to_owned(),
        }),
        Err(reason) => {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.page_unavailable",
                "view_id": view.display_id, "reason": reason,
            }));
            Err(reason)
        }
    }
}

async fn shoot(mobile: &Arc<Mobile>, view: &LookView) -> Result<Vec<u8>, String> {
    let control = Arc::clone(mobile.browser_control());
    // Held for the whole screenshot, so a phone that keeps pressing refresh
    // cannot crowd out the relays `hide browser` commands need.
    let _relay = control.acquire_relay().map_err(|(reason, _)| reason.to_owned())?;
    let (core, device, checkout) = (
        Arc::clone(mobile.core()),
        view.device_id.clone(),
        view.checkout_path.clone(),
    );
    let (display, window) = (view.display_id.clone(), Arc::clone(&control));
    // Both steps are blocking: the core answers on its owner thread, and the
    // window's gateway over HTTP.
    let found = tokio::task::spawn_blocking(move || {
        let listed = core
            .workspace_query(&device, &checkout_caller_id(CALLER_KEY, &checkout), Query::ViewList)
            .map_err(|refusal| refusal.reason.to_owned())?;
        let shown = listed
            .views
            .as_deref()
            .and_then(|views| views.iter().find(|shown| shown.view_id == display))
            .map(|shown| (shown.kind, shown.selected))
            .ok_or_else(|| "display_closed".to_owned())?;
        let relay = window
            .relay(&listed, &display, None)
            .map_err(|(reason, _)| reason.to_owned())?;
        Ok::<_, String>((shown, relay))
    })
    .await
    .map_err(|_| "worker_lost".to_owned())??;
    let ((kind, selected), relay) = found;
    if kind != "browser" {
        return Err("not_a_page".to_owned());
    }
    let gateway = match relay {
        Relay::Core(url) => crate::browser_relay::connect(&url)
            .await
            .map_err(|(reason, _)| reason.to_owned())?,
        // The desktop window is on another machine: its page is not drawn by
        // this Mac's app, which is the one a phone's picture comes from.
        Relay::Own(_) | Relay::Through { .. } => return Err("window_elsewhere".to_owned()),
    };
    crate::browser_page::still(gateway, &view.display_id, selected)
        .await
        .map_err(|failure| failure.reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phones_reads_run_as_a_checkout_caller_the_core_resolves_by_path() {
        let id = checkout_caller_id(CALLER_KEY, "/repo");
        assert!(matches!(
            herdr_core::workspace_control::Caller::parse(&id),
            herdr_core::workspace_control::Caller::Checkout {
                key: "phone",
                path: "/repo",
                gateway: false
            }
        ));
    }
}
