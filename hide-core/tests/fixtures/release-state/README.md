# Release state folders

Each `<tag>/` is the state folder the published release `<tag>` wrote, made by `scripts/release_state/generate.py <tag>` from that release's own programs, a core and one dialed SSH device in containers; `<tag>.json` beside it names the release's commit and what each step of the scenario did.
`hide-core/src/node_migration/upgrade_tests.rs` runs a start's migrations over a copy of every one; `scripts/release_state/README.md` says how a release adds its folder and when one comes out.
