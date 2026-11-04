// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/workspace.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: keeps only the public id shapes and their number
// encoding; the counters are the own engine's topology's, not a process-wide
// atomic, so an engine's ids start from what it holds; adds the reading of an
// id back into its number, so a restored engine counts on past every id it
// finds; a workspace this engine makes is spelled `h<n>`, which Herdr never
// spells, and one moved from Herdr keeps its `w<n>`.

//! The ids the own engine gives its workspaces, tabs and panes, in Herdr's
//! shape so every reader of a pane id reads them as it reads Herdr's (PRD
//! engine-bundle-3-core-owns D-4): `h<n>` for a workspace, `<workspace>:t<n>`
//! and `<workspace>:p<n>` for its tabs and panes, each number in a 32-letter
//! alphabet with no 0, I, L, O or U before its last letter. A number is
//! never given twice within its counter, so an id never comes back for
//! another workspace, tab or pane.
//!
//! A workspace this engine makes starts with `h`, a letter Herdr never
//! starts one with (it makes only `w<n>`), so a Herdr that runs on beside
//! Hide and makes workspaces of its own never makes an id this engine made,
//! and a workspace moved from Herdr keeps its `w<n>` (PRD
//! engine-bundle-4-herdr-removal D-26). Tab and pane numbers count within
//! their workspace, so two workspaces that do not meet hold no id in
//! common. [`workspace_of`] is the one reader of both spellings.

const PUBLIC_ID_ALPHABET: &[u8; 32] = b"123456789ABCDEFGHJKMNPQRSTVWXYZ0";

pub(super) fn encode_public_number(mut value: usize) -> String {
    if value == 0 {
        return "0".to_string();
    }

    let mut encoded = Vec::new();
    while value > 0 {
        let digit = (value - 1) % PUBLIC_ID_ALPHABET.len();
        encoded.push(PUBLIC_ID_ALPHABET[digit] as char);
        value = (value - 1) / PUBLIC_ID_ALPHABET.len();
    }
    encoded.iter().rev().collect()
}

/// The number `encode_public_number` spelled, or `None` for a spelling it
/// never makes.
pub(super) fn decode_public_number(spelled: &str) -> Option<usize> {
    if spelled.is_empty() {
        return None;
    }
    let mut value: usize = 0;
    for letter in spelled.bytes() {
        let digit = PUBLIC_ID_ALPHABET
            .iter()
            .position(|known| *known == letter)?;
        value = value
            .checked_mul(PUBLIC_ID_ALPHABET.len())?
            .checked_add(digit + 1)?;
    }
    Some(value)
}

/// Who numbered a workspace id: this engine (`h<n>`) or Herdr, from which
/// the workspace was moved (`w<n>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Numbered {
    Engine,
    Herdr,
}

/// The letter this engine's own workspace ids start with.
pub(super) const ENGINE_WORKSPACE: char = 'h';

/// The letter Herdr's workspace ids start with.
pub(super) const HERDR_WORKSPACE: char = 'w';

/// Who numbered the workspace id `workspace` and its number, or `None` for
/// a spelling neither makes: the one reader of both spellings, which the
/// saved session's check, the counters and the gathering of running panes
/// share.
pub(super) fn workspace_of(workspace: &str) -> Option<(Numbered, usize)> {
    let mut letters = workspace.chars();
    let numbered = match letters.next()? {
        ENGINE_WORKSPACE => Numbered::Engine,
        HERDR_WORKSPACE => Numbered::Herdr,
        _ => return None,
    };
    Some((numbered, decode_public_number(letters.as_str())?))
}

/// The workspace a pane or tab id names and its number there, read from an
/// id this engine's shape spells (`<workspace>:p<n>`, `<workspace>:t<n>`).
pub(super) fn numbered(id: &str, kind: char) -> Option<(&str, usize)> {
    let (workspace, rest) = id.split_once(':')?;
    let number = rest.strip_prefix(kind)?;
    workspace_of(workspace)?;
    Some((workspace, decode_public_number(number)?))
}

/// The number of a workspace id this engine made (`h<n>`): the one count
/// its new workspaces take. A workspace moved from Herdr (`w<n>`) moves no
/// count of this engine's.
pub(super) fn engine_workspace_number(workspace: &str) -> Option<usize> {
    match workspace_of(workspace)? {
        (Numbered::Engine, number) => Some(number),
        (Numbered::Herdr, _) => None,
    }
}

/// The id of the workspace this engine numbers `number`.
pub(super) fn workspace_id(number: usize) -> String {
    format!("{ENGINE_WORKSPACE}{}", encode_public_number(number))
}

/// Whether the workspace, tab or pane id `id` is in a workspace this engine
/// numbered (`h<n>`), not one moved from Herdr or of another shape.
pub(super) fn made_by_engine(id: &str) -> bool {
    let workspace = id.split_once(':').map_or(id, |(workspace, _)| workspace);
    matches!(workspace_of(workspace), Some((Numbered::Engine, _)))
}

pub(super) fn pane_id(workspace_id: &str, pane_number: usize) -> String {
    format!("{workspace_id}:p{}", encode_public_number(pane_number))
}

pub(super) fn tab_id(workspace_id: &str, tab_number: usize) -> String {
    format!("{workspace_id}:t{}", encode_public_number(tab_number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn herdr_workspace_id(number: usize) -> String {
        format!("{HERDR_WORKSPACE}{}", encode_public_number(number))
    }

    #[test]
    fn ids_read_as_herdr_spells_them_with_the_engine_s_own_workspace_letter() {
        assert_eq!(workspace_id(1), "h1");
        assert_eq!(herdr_workspace_id(1), "w1");
        assert_eq!(pane_id("h1", 32), "h1:p0");
        assert_eq!(tab_id("w1", 33), "w1:t11");
        assert_eq!(workspace_id(9 + 1), "hA");
    }

    #[test]
    fn an_id_reads_back_as_the_number_it_was_given() {
        for number in [1, 9, 31, 32, 33, 1024, 1025, 40_000] {
            for workspace in [workspace_id(number), herdr_workspace_id(number)] {
                let pane = pane_id(&workspace, number);
                assert_eq!(numbered(&pane, 'p'), Some((workspace.as_str(), number)));
            }
            assert_eq!(
                workspace_of(&workspace_id(number)),
                Some((Numbered::Engine, number))
            );
            assert_eq!(
                workspace_of(&herdr_workspace_id(number)),
                Some((Numbered::Herdr, number))
            );
            assert_eq!(engine_workspace_number(&workspace_id(number)), Some(number));
            assert_eq!(engine_workspace_number(&herdr_workspace_id(number)), None);
        }
        assert_eq!(numbered("w1:t3", 'p'), None);
        assert_eq!(numbered("pane-1", 'p'), None);
        assert_eq!(numbered("x1:p1", 'p'), None);
        assert_eq!(workspace_of("h"), None);
        assert_eq!(decode_public_number("I"), None);
    }
}
