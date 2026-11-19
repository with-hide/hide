//! The MCP's [`Applier`]: the seam the MCP half of the store fills (PRD
//! skill-mcp-manager B8-B14, D-10, D-11, D-27, D-28, D-32).
//!
//! An MCP's column is its agent, since each agent keeps its own settings
//! file; its place is that file in the agent's default account and in each
//! profile account (`[account.paths] mcp`), and one entry is the server's
//! entry in it, read, written and taken out in the agent's format
//! (`mcp_format`) with every other byte of the file kept (B11). Its body
//! holds the server's spec and the names of its keys; the key values live in
//! Hide's private key file, never in the store's record, and [`Applier::purge`]
//! deletes them (D-11, D-39).
//!
//! Until that half lands the applier lists no place and every agent reads
//! "not yet", so the store's loop, switches, undo list and report already
//! carry the kind.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    Applier, Carried, Column, Ctx, Entry, ItemBody, ItemKind, Place, Source, StoreItem,
    StoreRefusal, StoreRefusalCause, Version,
};
use crate::agents::ADAPTERS;

/// An MCP server as the store keeps it, without its key values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpBody {
    /// The server's spec in Hide's own shape; the MCP half names its fields.
    #[serde(default)]
    pub spec: serde_json::Value,
}

impl McpBody {
    /// What every agent's copy is compared with: the spec as written.
    pub(crate) fn digest(&self) -> String {
        super::skill::hex(&Sha256::digest(self.spec.to_string().as_bytes()))
    }

    /// The short version beside the name (B36).
    pub(crate) fn version(&self) -> Version {
        Version::Remote
    }
}

pub(crate) struct McpApplier;

const NOT_YET: &str = "this build does not write MCP servers yet";

impl Applier for McpApplier {
    fn kind(&self) -> ItemKind {
        ItemKind::Mcp
    }

    fn columns(&self) -> Vec<Column> {
        ADAPTERS
            .iter()
            .map(|adapter| Column {
                id: adapter.id.to_owned(),
                agents: vec![adapter.id.to_owned()],
                also: Vec::new(),
            })
            .collect()
    }

    fn places(&self, _ctx: &Ctx<'_>) -> Vec<Place> {
        Vec::new()
    }

    fn supports(&self, _item: &StoreItem, _column: &Column) -> Result<(), String> {
        Err(NOT_YET.to_owned())
    }

    fn entry(&self, _place: &Place, _name: &str) -> Result<Option<Entry>, String> {
        Ok(None)
    }

    fn entries(&self, _place: &Place) -> Result<Vec<(String, Entry)>, String> {
        Ok(Vec::new())
    }

    fn write(&self, _ctx: &Ctx<'_>, _place: &Place, _item: &StoreItem) -> Result<String, String> {
        Err(NOT_YET.to_owned())
    }

    fn remove(&self, _place: &Place, _name: &str) -> Result<(), String> {
        Err(NOT_YET.to_owned())
    }

    fn adopt(
        &self,
        _ctx: &Ctx<'_>,
        _place: &Place,
        _name: &str,
        _into: &Path,
    ) -> Result<(Source, ItemBody), StoreRefusal> {
        Err(StoreRefusal::new(StoreRefusalCause::Unsupported, NOT_YET))
    }

    fn purge(&self, _ctx: &Ctx<'_>, _item: &StoreItem) -> Result<(), String> {
        Ok(())
    }

    /// Carries the server's spec and its key values read from Hide's key
    /// file (D-11, B28); the keys travel only inside the device link.
    fn export(&self, _ctx: &Ctx<'_>, _item: &StoreItem) -> Result<Carried, StoreRefusal> {
        Err(StoreRefusal::new(StoreRefusalCause::Unsupported, NOT_YET))
    }

    /// Keeps the carried spec as the body and writes the key values into
    /// Hide's key file.
    fn import(
        &self,
        _ctx: &Ctx<'_>,
        _name: &str,
        _content: &Carried,
        _into: &Path,
    ) -> Result<ItemBody, StoreRefusal> {
        Err(StoreRefusal::new(StoreRefusalCause::Unsupported, NOT_YET))
    }
}
