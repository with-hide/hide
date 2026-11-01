//! Generates each built-in agent definition's identity as constants from
//! `agents/*.toml`, so `AgentAdapter` and every caller read the TOML's
//! values with no second copy in Rust. Each file goes through the same
//! check of its fields as the operator's files (`src/definition/file.rs`),
//! so a built-in file that fails one fails the build; only the file's name
//! is checked here, since only the build maps a file to a constant. The
//! whole text is also included, for the loader that compiles the screen
//! rules (`src/definition.rs`).

use std::fmt::Write as _;
use std::path::Path;

#[path = "src/definition/file.rs"]
#[allow(dead_code)]
mod file;

fn main() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("agents");
    println!("cargo:rerun-if-changed={}", folder.display());
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| {
            entry
                .expect("agents folder entry")
                .file_name()
                .into_string()
                .expect("UTF-8 name")
        })
        .filter(|name| name.ends_with(".toml"))
        .collect();
    names.sort();
    let mut out = String::new();
    let mut all = Vec::new();
    for name in &names {
        let path = folder.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
        let fail = |what: String| -> ! { panic!("built-in agent definition {what}") };
        let parsed = file::read(name, &text, file::Origin::Builtin).unwrap_or_else(|e| fail(e));
        if format!("{}.toml", parsed.id) != *name {
            fail(format!("{name}: the file is named after its id"));
        }
        let display = file::required(name, "name", parsed.name).unwrap_or_else(|e| fail(e));
        let executables =
            file::required(name, "executables", parsed.executables).unwrap_or_else(|e| fail(e));
        let hooks = file::required(name, "hooks", parsed.hooks).unwrap_or_else(|e| fail(e));
        let constant = parsed.id.to_uppercase().replace('-', "_");
        writeln!(
            out,
            "pub(crate) const {constant}: Builtin = Builtin {{ id: {id:?}, name: {display:?}, executables: &{executables:?}, aliases: &{aliases:?}, home: {home}, hooks: Hooks::{hooks:?}, start_args: &{start:?}, resume_session: {session}, resume_last: {last}, text: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/agents/{name}\")) }};",
            id = parsed.id,
            aliases = parsed.aliases.unwrap_or_default(),
            home = option(parsed.home),
            start = parsed.start_args.unwrap_or_default(),
            session = option(parsed.resume_session),
            last = option(parsed.resume_last),
        )
        .unwrap();
        all.push(constant);
    }
    writeln!(
        out,
        "pub(crate) const ALL: &[Builtin] = &[{}];",
        all.join(", ")
    )
    .unwrap();
    let target = Path::new(&std::env::var("OUT_DIR").unwrap()).join("builtin.rs");
    std::fs::write(&target, out).unwrap_or_else(|error| panic!("{}: {error}", target.display()));
}

fn option(words: Option<Vec<String>>) -> String {
    match words {
        Some(words) => format!("Some(&{words:?})"),
        None => "None".to_owned(),
    }
}
