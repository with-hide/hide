//! Generates each built-in agent definition's identity as constants from
//! `agents/*.toml`, so `AgentAdapter` and every caller read the TOML's
//! values with no second copy in Rust and a built-in file the schema
//! refuses fails the build. The whole text is also included, for the
//! loader that compiles the screen rules (`src/definition.rs`).

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
        let parsed: file::File =
            toml::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let fail = |what: &str| -> ! { panic!("built-in agent definition {name}: {what}") };
        if parsed.schema != file::SCHEMA {
            fail("schema must be 1");
        }
        if format!("{}.toml", parsed.id) != *name {
            fail("the file is named after its id");
        }
        if parsed.based_on.is_some() {
            fail("based_on is for user files only");
        }
        let display = parsed
            .name
            .as_deref()
            .unwrap_or_else(|| fail("name is required"));
        let executables = parsed
            .executables
            .as_deref()
            .unwrap_or_else(|| fail("executables is required"));
        if executables.is_empty() {
            fail("executables is empty");
        }
        let hooks = parsed.hooks.unwrap_or_else(|| fail("hooks is required"));
        let home: Vec<&str> = match parsed.home.as_deref() {
            None => Vec::new(),
            Some(home) => home
                .strip_prefix("~/")
                .unwrap_or_else(|| fail("home starts with ~/"))
                .split('/')
                .collect(),
        };
        let resume = parsed.resume.unwrap_or_default();
        let constant = parsed.id.to_uppercase().replace('-', "_");
        writeln!(
            out,
            "pub(crate) const {constant}: Builtin = Builtin {{ id: {id:?}, name: {display:?}, executables: &{executables:?}, aliases: &{aliases:?}, home: {home}, hooks: Hooks::{hooks:?}, start_args: &{start:?}, resume_session: {session}, resume_last: {last}, text: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/agents/{name}\")) }};",
            id = parsed.id,
            aliases = parsed.aliases.unwrap_or_default(),
            home = if parsed.home.is_some() { format!("Some(&{home:?})") } else { "None".to_owned() },
            start = parsed.start.unwrap_or_default().args,
            session = option(resume.session),
            last = option(resume.last),
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
