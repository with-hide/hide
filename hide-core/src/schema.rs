//! JSON Schema for the snapshot-delta document hided puts on the wire.
//!
//! The C ABI still serializes `SnapshotDeltaWire`. This owned document matches
//! that JSON so TypeScript types are generated from the same shape rather than
//! copied by hand.

use schemars::JsonSchema;
use serde::Serialize;

/// Owned form of `SnapshotDeltaWire` for schema generation.
#[derive(Serialize, JsonSchema)]
pub struct SnapshotDeltaDocument {
    pub schema_version: u32,
    pub revision: u64,
    pub rest: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    pub editor: Option<serde_json::Value>,
    pub changes: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<serde_json::Value>,
    pub find: serde_json::Value,
}

pub fn snapshot_delta_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(SnapshotDeltaDocument)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_delta_schema_names_the_wire_document() {
        let schema = snapshot_delta_schema();
        let json = serde_json::to_value(&schema).expect("schema json");
        let title = json
            .pointer("/title")
            .or_else(|| json.pointer("/definitions/SnapshotDeltaDocument"))
            .or_else(|| json.get("title"));
        assert!(
            json.to_string().contains("SnapshotDeltaDocument"),
            "schema should name SnapshotDeltaDocument, got {title:?}"
        );
    }
}
