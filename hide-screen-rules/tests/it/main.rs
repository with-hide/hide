// The rule semantics and validation cases are derived from Herdr v0.9.5
// (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/detect/manifest/tests.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: each case compiles its rules as a value and evaluates
// them directly, without Herdr's manifest folders and cache.

mod rules;
