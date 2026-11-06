//! One content-free outcome each time an outbound device's connection result
//! changes: a scheduled retry that fails as the last one did, or a reconnect
//! that is ready as the last one was, sends nothing.
//! Tokens stay inside the runtime and never enter an analytics event.

use std::collections::BTreeMap;

use super::Runtime;
use crate::product_analytics::{DeviceFailedStep, DeviceOutcome, DevicePlatform, Event};
use crate::remote::EstablishError;

const MAX_ATTEMPTS: usize = 128;

#[derive(Default)]
pub(super) struct Attempts {
    serial: u64,
    pending: BTreeMap<String, Attempt>,
    /// The outcome last sent for each device in this daemon's life; the
    /// next attempt with the same outcome is the same state, not news.
    sent: BTreeMap<String, DeviceOutcome>,
}

struct Attempt {
    token: u64,
    generation: u64,
    platform: DevicePlatform,
    kit_claimed: bool,
}

impl Runtime {
    pub(super) fn begin_device_connect(&mut self, device: &str) {
        let generation = self.device_host_generation(device);
        let attempts = &mut self.device_connect_analytics;
        if attempts.pending.len() >= MAX_ATTEMPTS && !attempts.pending.contains_key(device) {
            crate::diagnostic!(serde_json::json!({
                "component": "product_analytics", "kind": "device_attempt_limit",
            }));
            return;
        }
        attempts.serial += 1;
        attempts.pending.insert(
            device.to_owned(),
            Attempt {
                token: attempts.serial,
                generation,
                platform: DevicePlatform::Unknown,
                kit_claimed: false,
            },
        );
    }

    pub(super) fn device_connect_platform(&mut self, device: &str, os: &str, arch: &str) {
        if let Some(attempt) = self.device_connect_analytics.pending.get_mut(device) {
            attempt.platform = DevicePlatform::of(os, arch);
        }
    }

    /// Only an Apply on the current connection takes this attempt's token.
    /// Status polls and a later repair cannot settle it a second time.
    pub(super) fn claim_device_connect_kit(&mut self, device: &str) -> Option<u64> {
        let generation = self.device_host_generation(device);
        let attempt = self.device_connect_analytics.pending.get_mut(device)?;
        if attempt.generation != generation || attempt.kit_claimed {
            return None;
        }
        attempt.kit_claimed = true;
        Some(attempt.token)
    }

    pub(super) fn fail_device_connect(&mut self, device: &str, error: &EstablishError) {
        let step = match error {
            EstablishError::Download { os, arch, .. } => {
                self.device_connect_platform(device, os, arch);
                DeviceFailedStep::Download
            }
            _ => DeviceFailedStep::Install,
        };
        self.finish_device_connect(device, DeviceOutcome::Failed(step));
    }

    pub(crate) fn complete_device_connect_kit(
        &mut self,
        device: &str,
        token: Option<u64>,
        report: &Result<hide_kit::KitReport, String>,
    ) {
        let Some(attempt) = self.device_connect_analytics.pending.get(device) else {
            return;
        };
        if token != Some(attempt.token) || attempt.generation != self.device_host_generation(device)
        {
            return;
        }
        let outcome = match report {
            Ok(report) if report.refused.is_none() => DeviceOutcome::Ready,
            Ok(_) | Err(_) => DeviceOutcome::Failed(DeviceFailedStep::Install),
        };
        self.finish_device_connect(device, outcome);
    }

    pub(super) fn fail_device_connect_install(&mut self, device: &str) {
        self.finish_device_connect(device, DeviceOutcome::Failed(DeviceFailedStep::Install));
    }

    fn finish_device_connect(&mut self, device: &str, outcome: DeviceOutcome) {
        let attempts = &mut self.device_connect_analytics;
        let Some(attempt) = attempts.pending.remove(device) else {
            return;
        };
        // Only an outcome handed to the sink is remembered as sent.
        let Some(sink) = &self.product_analytics else {
            return;
        };
        if attempts.sent.get(device) == Some(&outcome) {
            return;
        }
        if attempts.sent.len() < MAX_ATTEMPTS || attempts.sent.contains_key(device) {
            attempts.sent.insert(device.to_owned(), outcome);
        } else {
            crate::diagnostic!(serde_json::json!({
                "component": "product_analytics", "kind": "device_outcome_limit",
            }));
        }
        sink.capture(Event::DeviceConnected {
            outcome,
            platform: attempt.platform,
        });
    }

    pub(super) fn forget_device_connect(&mut self, device: &str) {
        self.device_connect_analytics.pending.remove(device);
    }

    /// A removed device's next registration starts with no outcome sent.
    pub(super) fn forget_device_connect_outcome(&mut self, device: &str) {
        self.device_connect_analytics.sent.remove(device);
    }
}
