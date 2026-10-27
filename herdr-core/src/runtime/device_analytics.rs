//! One content-free final outcome per outbound device connect or bootstrap Retry.
//! Tokens stay inside the runtime and never enter an analytics event.

use std::collections::BTreeMap;

use super::Runtime;
use crate::product_analytics::{
    DeviceFailedStep, DeviceHerdr, DeviceOutcome, DevicePlatform, Event,
};
use crate::remote::EstablishError;

const MAX_ATTEMPTS: usize = 128;

#[derive(Default)]
pub(super) struct Attempts {
    serial: u64,
    pending: BTreeMap<String, Attempt>,
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
        use hide_kit::device_herdr::HerdrUpdate;
        let outcome = match report {
            Err(_) => DeviceOutcome::Failed(DeviceFailedStep::Install),
            Ok(report) if report.refused.is_some() => {
                DeviceOutcome::Failed(DeviceFailedStep::Install)
            }
            Ok(report) => match report.herdr.as_deref() {
                Some(HerdrUpdate::Started { .. }) => DeviceOutcome::Ready(DeviceHerdr::Started),
                Some(HerdrUpdate::HandedOff { .. }) => DeviceOutcome::Ready(DeviceHerdr::HandedOff),
                Some(HerdrUpdate::Current { .. }) => DeviceOutcome::Ready(DeviceHerdr::Current),
                Some(HerdrUpdate::Kept { .. }) => DeviceOutcome::Ready(DeviceHerdr::Kept),
                Some(HerdrUpdate::Behind { .. }) => {
                    DeviceOutcome::Failed(DeviceFailedStep::HerdrHandoff)
                }
                Some(HerdrUpdate::Failed { stage, .. }) => {
                    DeviceOutcome::Failed(match stage.as_str() {
                        "upload" => DeviceFailedStep::Install,
                        "record" | "handoff" | "confirm" => DeviceFailedStep::HerdrHandoff,
                        _ => DeviceFailedStep::HerdrStart,
                    })
                }
                None if self
                    .device_registration(device)
                    .is_some_and(|registration| registration.herdr_session.is_some()) =>
                {
                    DeviceOutcome::Ready(DeviceHerdr::Preserved)
                }
                // An old helper's NoServer or a missing pin/updater did not
                // establish a Herdr outcome. Never invent a ready value.
                Some(HerdrUpdate::NoServer { .. }) | None => {
                    DeviceOutcome::Failed(DeviceFailedStep::HerdrStart)
                }
            },
        };
        self.finish_device_connect(device, outcome);
    }

    pub(super) fn fail_device_connect_install(&mut self, device: &str) {
        self.finish_device_connect(device, DeviceOutcome::Failed(DeviceFailedStep::Install));
    }

    fn finish_device_connect(&mut self, device: &str, outcome: DeviceOutcome) {
        let Some(attempt) = self.device_connect_analytics.pending.remove(device) else {
            return;
        };
        if let Some(sink) = &self.product_analytics {
            sink.capture(Event::DeviceConnected {
                outcome,
                platform: attempt.platform,
            });
        }
    }

    pub(super) fn forget_device_connect(&mut self, device: &str) {
        self.device_connect_analytics.pending.remove(device);
    }
}
