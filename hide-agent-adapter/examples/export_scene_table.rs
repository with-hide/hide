//! Writes the block of `docs/INSTALL.md` between its `bell-scenes` markers
//! (`docs/VERIFICATION.md#agent-scenes`): run it with `--markdown`, and the
//! adapter contract refuses a file that differs from it by a byte.
fn main() {
    if !std::env::args().any(|arg| arg == "--markdown") {
        eprintln!("usage: export_scene_table --markdown");
        std::process::exit(2);
    }
    print!("{}", hide_agent_adapter::scene::table_markdown());
}
