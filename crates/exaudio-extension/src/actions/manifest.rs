//! `contributes.actions` manifest entries and their validation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::keys::KeyChord;
use super::when::When;
use crate::manifest::{ManifestError, valid_id};

/// Text keyed by language code; `en` is required by validation and is the fallback.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Localized(pub BTreeMap<String, String>);

impl Localized {
    pub fn en(text: &str) -> Self {
        Self::default().with("en", text)
    }

    pub fn with(mut self, lang: &str, text: &str) -> Self {
        self.0.insert(lang.to_string(), text.to_string());
        self
    }

    pub fn get(&self, lang: &str) -> Option<&str> {
        self.0.get(lang).map(String::as_str)
    }

    /// Text in `lang`, else `en`, else the first available language.
    pub fn resolve(&self, lang: &str) -> &str {
        self.get(lang)
            .or_else(|| self.get("en"))
            .or_else(|| self.0.values().next().map(String::as_str))
            .unwrap_or_default()
    }

    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

/// Everything an extension contributes to shared registries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributes {
    #[serde(default)]
    pub actions: Vec<ActionManifest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionManifest {
    /// `<owner>.<name>`; the owner is the declaring extension's id.
    pub id: String,
    pub title: Localized,
    #[serde(default)]
    pub category: Option<Localized>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default = "default_true")]
    pub palette: bool,
    #[serde(default)]
    pub keys: Vec<KeyBindingManifest>,
    #[serde(default)]
    pub menus: Vec<MenuPlacement>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBindingManifest {
    pub key: String,
    #[serde(default)]
    pub when: Option<String>,
    /// Alternative chord on macOS (replaces `key` there).
    #[serde(default)]
    pub mac: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuPlacement {
    pub menu: String,
    pub group: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub when: Option<String>,
}

/// Default binding with its clauses parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyBinding {
    pub key: KeyChord,
    pub mac: Option<KeyChord>,
    pub when: Option<When>,
}

impl KeyBinding {
    /// The chord this binding uses on the given platform.
    pub fn chord(&self, mac: bool) -> &KeyChord {
        match (&self.mac, mac) {
            (Some(chord), true) => chord,
            _ => &self.key,
        }
    }
}

/// Menu placement with its clause parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub menu: String,
    pub group: String,
    pub order: i32,
    pub when: Option<When>,
}

/// The parsed parts of an [`ActionManifest`].
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledAction {
    pub when: Option<When>,
    pub keys: Vec<KeyBinding>,
    pub menus: Vec<MenuEntry>,
}

impl ActionManifest {
    /// Validates the action against its owner and parses every clause and chord.
    pub fn compile(&self, owner: &str) -> Result<CompiledAction, ManifestError> {
        let prefixed = self
            .id
            .strip_prefix(owner)
            .is_some_and(|rest| rest.starts_with('.'));
        if !prefixed || !valid_id(&self.id) {
            return Err(ManifestError::InvalidActionId {
                action: self.id.clone(),
                owner: owner.to_string(),
            });
        }
        if self.title.get("en").is_none_or(|en| en.trim().is_empty()) {
            return Err(ManifestError::MissingTitle(self.id.clone()));
        }
        let when = self.clause(self.when.as_deref())?;
        let keys = self
            .keys
            .iter()
            .map(|binding| {
                Ok(KeyBinding {
                    key: self.chord(&binding.key)?,
                    mac: binding
                        .mac
                        .as_deref()
                        .map(|mac| self.chord(mac))
                        .transpose()?,
                    when: self.clause(binding.when.as_deref())?,
                })
            })
            .collect::<Result<_, _>>()?;
        let menus = self
            .menus
            .iter()
            .map(|placement| {
                Ok(MenuEntry {
                    menu: placement.menu.clone(),
                    group: placement.group.clone(),
                    order: placement.order,
                    when: self.clause(placement.when.as_deref())?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(CompiledAction { when, keys, menus })
    }

    fn clause(&self, source: Option<&str>) -> Result<Option<When>, ManifestError> {
        source
            .map(|clause| {
                When::parse(clause).map_err(|error| ManifestError::InvalidWhen {
                    action: self.id.clone(),
                    clause: clause.to_string(),
                    error,
                })
            })
            .transpose()
    }

    fn chord(&self, key: &str) -> Result<KeyChord, ManifestError> {
        KeyChord::parse(key).ok_or_else(|| ManifestError::InvalidKey {
            action: self.id.clone(),
            key: key.to_string(),
        })
    }
}

/// Validates a whole contribution list: every action compiles and ids are unique.
pub fn validate_actions(owner: &str, actions: &[ActionManifest]) -> Result<(), ManifestError> {
    let mut seen = BTreeSet::new();
    for action in actions {
        action.compile(owner)?;
        if !seen.insert(action.id.as_str()) {
            return Err(ManifestError::DuplicateAction(action.id.clone()));
        }
    }
    Ok(())
}
