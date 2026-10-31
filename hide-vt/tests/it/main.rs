//! hide-vt's integration tests, built as one binary (docs/BUILD.md, "One
//! integration test binary per crate"). They hold the parser to what a holder
//! needs from it on every system: the built library links, bytes go in, the
//! screen's text comes out, and a query the child sends is answered.

mod terminal;
