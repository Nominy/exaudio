//! Declarative actions: manifest contributions, `when` clauses, key chords and the registry
//! every surface (menus, keyboard, palette, hotkeys dialog) reads from.

pub mod keys;
pub mod manifest;
pub mod registry;
pub mod when;

#[cfg(test)]
mod tests;

pub use keys::KeyChord;
pub use manifest::{
    ActionManifest, CompiledAction, Contributes, KeyBinding, KeyBindingManifest, Localized,
    MenuEntry, MenuPlacement, validate_actions,
};
pub use registry::{
    ActionId, ActionRegistry, CORE_OWNER, Conflict, Entry, MenuItem, Priority, RegistryError,
    Resolution, UserBinding, UserBindings, legacy_action_id,
};
pub use when::{Context, Value, When, WhenError, WhenErrorKind};
