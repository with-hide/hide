//! The one language Hide AI writes in for the operator (PRD
//! ai-output-language D-02, D-10): the core's explicit interface language,
//! else this machine's primary language, else English. Labels, pull request
//! summaries, Memory, its promotion and the Factory all ask here, and a
//! device's requests follow the core's answer, never the device's own.

use serde_json::json;

use super::Runtime;
use crate::model::InterfaceLanguage;

/// This machine's primary language as an interface language, `None` when
/// the system names none or one the interface does not have, resolved by
/// the shell's rules (docs/LOCALIZATION.md). On macOS the read starts a
/// child process, so it is made once per daemon, before the runtime exists
/// and never under its lock; a new login, which changes it, starts a new
/// daemon.
pub(crate) fn read_system_language() -> Option<InterfaceLanguage> {
    let tag = hide_platform::host::primary_language();
    let language = tag.as_deref().ok().and_then(InterfaceLanguage::from_system);
    if language.is_none() {
        crate::diagnostic!(json!({
            "component": "language",
            "kind": "language.system_fallback",
            "reason": match &tag {
                Ok(_) => "unsupported_system_language".to_owned(),
                Err(error) => format!("{:?}", error.kind()),
            },
        }));
    }
    language
}

impl From<InterfaceLanguage> for hide_ai::OutputLanguage {
    fn from(language: InterfaceLanguage) -> Self {
        match language {
            InterfaceLanguage::English => Self::English,
            InterfaceLanguage::Korean => Self::Korean,
            InterfaceLanguage::SimplifiedChinese => Self::SimplifiedChinese,
            InterfaceLanguage::Japanese => Self::Japanese,
        }
    }
}

impl Runtime {
    /// Takes the system language [`read_system_language`] read at launch.
    pub(crate) fn set_system_language(&mut self, language: Option<InterfaceLanguage>) {
        self.system_language = language;
    }

    /// The language every Hide AI text a person reads is asked in. A stored
    /// choice that is invalid reads as English, as the snapshot shows it.
    pub(crate) fn output_language(&self) -> hide_ai::OutputLanguage {
        self.interface_language().into()
    }

    /// The language the core words a line in that it writes outside any
    /// screen, the doorbell's bell line among them: the same answer as
    /// [`Self::output_language`], from state the runtime holds, so it is read
    /// under the lock without I/O.
    pub(crate) fn interface_language(&self) -> InterfaceLanguage {
        let choice = self
            .snapshot
            .ui_state
            .interface_language
            .as_ref()
            .map(|value| {
                serde_json::from_value(value.clone()).unwrap_or(InterfaceLanguage::English)
            });
        choice
            .or(self.system_language)
            .unwrap_or(InterfaceLanguage::English)
    }
}
