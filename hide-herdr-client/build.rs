use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // The contract is what the pinned Herdr binary reports for
    // `api schema --json`, written beside the pin when it last moved. The
    // core reads its protocol revision from here and nothing else, so the
    // runtime manifest itself is not a build input.
    const SCHEMA_PATH: &str = "../contracts/herdr-api.schema.json";

    println!("cargo:rerun-if-changed={SCHEMA_PATH}");
    println!("cargo:rerun-if-changed=build.rs");

    let schema_text = fs::read_to_string(SCHEMA_PATH)
        .unwrap_or_else(|error| panic!("failed to read canonical Herdr API schema: {error}"));
    let schema: serde_json::Value = serde_json::from_str(&schema_text)
        .unwrap_or_else(|error| panic!("canonical Herdr API schema is invalid JSON: {error}"));
    let protocol = schema
        .get("protocol")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            panic!("canonical Herdr API schema must contain a positive uint32 protocol")
        });
    let schema_version = schema
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            panic!("canonical Herdr API schema must contain a positive uint32 schema_version")
        });

    let generated = format!(
        "pub const HERDR_PROTOCOL_REVISION_SCHEMA: u32 = {protocol};\n\
         pub const HERDR_API_SCHEMA_VERSION: u32 = {schema_version};\n"
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("herdr_contract.rs");
    fs::write(output, generated).expect("failed to write generated Herdr contract constants");
}
