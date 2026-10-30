use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // `src/engine/herdr/wire.rs`, inside the engine's Herdr implementation, is
    // the only module that names the types generated here, from
    // the contract the pinned Herdr binary reports for `api schema --json`
    // (scripts/bump-herdr.sh writes it beside the pin). The client crate reads
    // the same file for the protocol revision only.
    const SCHEMA_PATH: &str = "../contracts/herdr-api.schema.json";

    println!("cargo:rerun-if-changed={SCHEMA_PATH}");
    println!("cargo:rerun-if-changed=build.rs");

    let schema_text = fs::read_to_string(SCHEMA_PATH)
        .unwrap_or_else(|error| panic!("failed to read canonical Herdr API schema: {error}"));
    let schema: serde_json::Value = serde_json::from_str(&schema_text)
        .unwrap_or_else(|error| panic!("canonical Herdr API schema is invalid JSON: {error}"));

    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    for name in ["request", "success_response", "event", "error_response"] {
        let mut document = schema["schemas"][name].clone();
        rewrite_refs(&mut document, &format!("#/schemas/{name}/$defs/"));
        let root = serde_json::from_value(document).expect("invalid Herdr sub-schema");
        let mut types = typify::TypeSpace::default();
        types
            .add_root_schema(root)
            .unwrap_or_else(|error| panic!("cannot generate {name}: {error}"));
        fs::write(
            output_dir.join(format!("herdr_{name}.rs")),
            types.to_stream().to_string(),
        )
        .expect("failed to write generated Herdr types");
    }
}

fn rewrite_refs(value: &mut serde_json::Value, prefix: &str) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "$ref" {
                    let reference = value.as_str().expect("schema reference must be a string");
                    let definition = reference
                        .strip_prefix(prefix)
                        .expect("reference must remain inside its sub-schema");
                    *value = serde_json::Value::String(format!("#/$defs/{definition}"));
                } else {
                    rewrite_refs(value, prefix);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_refs(value, prefix);
            }
        }
        _ => {}
    }
}
