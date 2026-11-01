// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a),
// src/workspace.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: keeps only the public id shapes and their number
// encoding; the counters are the own engine's topology's, not a process-wide
// atomic, so an engine's ids start from what it holds; adds the reading of an
// id back into its number, so a restored engine counts on past every id it
// finds.

//! The ids the own engine gives its workspaces, tabs and panes, in Herdr's
//! shape so every reader of a pane id reads them as it reads Herdr's (PRD
//! engine-bundle-3-core-owns D-4): `w<n>` for a workspace, `<workspace>:t<n>`
//! and `<workspace>:p<n>` for its tabs and panes, each number in a 32-letter
//! alphabet with no 0, I, L, O or U before its last letter. A number is
//! never given twice within its counter, so an id never comes back for
//! another workspace, tab or pane.

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

/// The workspace a pane or tab id names and its number there, read from an
/// id this engine's shape spells (`<workspace>:p<n>`, `<workspace>:t<n>`).
pub(super) fn numbered(id: &str, kind: char) -> Option<(&str, usize)> {
    let (workspace, rest) = id.split_once(':')?;
    let number = rest.strip_prefix(kind)?;
    workspace_number(workspace)?;
    Some((workspace, decode_public_number(number)?))
}

/// The number of a workspace id (`w<n>`).
pub(super) fn workspace_number(workspace: &str) -> Option<usize> {
    decode_public_number(workspace.strip_prefix('w')?)
}

pub(super) fn workspace_id(number: usize) -> String {
    format!("w{}", encode_public_number(number))
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

    #[test]
    fn ids_read_as_herdr_spells_them() {
        assert_eq!(workspace_id(1), "w1");
        assert_eq!(pane_id("w1", 32), "w1:p0");
        assert_eq!(tab_id("w1", 33), "w1:t11");
        assert_eq!(workspace_id(9 + 1), "wA");
    }

    #[test]
    fn an_id_reads_back_as_the_number_it_was_given() {
        for number in [1, 9, 31, 32, 33, 1024, 1025, 40_000] {
            let pane = pane_id(&workspace_id(number), number);
            assert_eq!(
                numbered(&pane, 'p'),
                Some((workspace_id(number).as_str(), number))
            );
            assert_eq!(workspace_number(&workspace_id(number)), Some(number));
        }
        assert_eq!(numbered("w1:t3", 'p'), None);
        assert_eq!(numbered("pane-1", 'p'), None);
        assert_eq!(decode_public_number("I"), None);
    }
}
