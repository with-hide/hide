use serde::Serialize;

/// The complete log allowlist. No wire object or exception is serializable here.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<&'static str>,
}
impl Event {
    pub(crate) fn new(
        event: &'static str,
        room: Option<String>,
        cap: Option<&'static str>,
    ) -> Self {
        Self {
            event,
            room,
            size: None,
            cap,
        }
    }
}
