//! A page View's screenshot for the phone (PRD mobile-companion-ui B20-B22,
//! D-11): what the desktop app's page draws, taken by the window that holds
//! it.
//!
//! The core lists the View's checkout as a checkout caller (never a pane) to
//! find the display and its area, and `BrowserControl::display_still` asks
//! this machine's desktop window for its picture (`BrowserViews.still` in
//! `desktop/src/main/browser.ts`, Electron's own capture of the page). A still
//! is a read: a page behind another tab is taken as it draws there, and one
//! laid out off screen and never shown has drawn nothing, which the host
//! answers `not_drawn`. The capture `hide browser screenshot` takes through
//! the page's debugger is not used: it answers only while Chromium draws the
//! page on an unlocked screen (`docs/BROWSER_DISPLAYS.md`). Nothing here selects a View, moves keyboard focus,
//! raises a window or loads a page (B22).
//!
//! Why a screenshot cannot be taken goes to one `look.page_unavailable`
//! diagnostic with its reason; the phone is told the reasons the operator
//! can act on (`looks.rs`, `page_reason`): no desktop app open, the window
//! on another computer, a page the app has not drawn.

use std::sync::Arc;
use std::time::Duration;

use hide_core::LookView;
use hide_core::workspace_control::{Query, checkout_caller_id};
use serde_json::json;

use super::{Mobile, now_ms};

/// How long a screenshot may take from the first request to the last byte:
/// the core's view list and the window's capture, so a window that never
/// answers ends the phone's refresh instead of holding it. The window's
/// answer is bounded sooner, by the browser control's 8 s request, and the
/// host's capture by its own 5 s (`STILL_WAIT_MS` in `desktop/src/main/browser.ts`).
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
    let outcome = tokio::time::timeout(DEADLINE, shoot(mobile, view, image_cap))
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
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.page_unavailable",
                "view_id": view.display_id, "reason": reason,
            }));
            Err(reason)
        }
    }
}

async fn shoot(mobile: &Arc<Mobile>, view: &LookView, image_cap: u64) -> Result<Vec<u8>, String> {
    let control = Arc::clone(mobile.browser_control());
    if !control.has_window() {
        return Err("no_window".to_owned());
    }
    let (core, device, checkout) = (
        Arc::clone(mobile.core()),
        view.device_id.clone(),
        view.checkout_path.clone(),
    );
    let display = view.display_id.clone();
    // Both steps are blocking: the core answers on its owner thread, and the
    // window over HTTP.
    tokio::task::spawn_blocking(move || {
        let listed = core
            .workspace_query(
                &device,
                &checkout_caller_id(CALLER_KEY, &checkout),
                Query::ViewList,
            )
            .map_err(|refusal| refusal.reason.to_owned())?;
        let kind = listed
            .views
            .as_deref()
            .and_then(|views| views.iter().find(|shown| shown.view_id == display))
            .map(|shown| shown.kind)
            .ok_or_else(|| "display_closed".to_owned())?;
        if kind != "browser" {
            return Err("not_a_page".to_owned());
        }
        control
            .display_still(&listed, &display, image_cap)
            .map_err(|(reason, _)| reason.to_owned())?
    })
    .await
    .map_err(|_| "worker_lost".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phones_reads_run_as_a_checkout_caller_the_core_resolves_by_path() {
        let id = checkout_caller_id(CALLER_KEY, "/repo");
        assert!(matches!(
            hide_core::workspace_control::Caller::parse(&id),
            hide_core::workspace_control::Caller::Checkout {
                key: "phone",
                path: "/repo",
                gateway: false
            }
        ));
    }
}
