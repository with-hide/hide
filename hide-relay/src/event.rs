use serde::Serialize;

/// The complete log allowlist. No wire object or exception is serializable
/// here, and nothing that names a computer or a phone: not a room hash, an
/// address, a key or a token (`docs/RELAY_PROTOCOL.md`, Logs).
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<&'static str>,
}
impl Event {
    pub fn new(event: &'static str, cap: Option<&'static str>) -> Self {
        Self {
            event,
            size: None,
            cap,
        }
    }
}
