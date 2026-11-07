//! Platform-neutral contracts for Exaudio extensions and modes.
//!
//! Extension packages and bundled implementations share manifests, version requirements and
//! activation planning. This crate defines no checking rules, model runtime or task platform.

pub mod actions;
pub mod document;
pub mod format;
pub mod manifest;
pub mod resolve;

pub use actions::{
    ActionId, ActionManifest, ActionRegistry, Context, Contributes, Entry, KeyChord, Localized,
    Priority, RegistryError, Resolution, UserBindings, When, WhenError,
};
pub use manifest::{
    API_VERSION, ExtensionManifest, ManifestError, ModeLaunch, ModeManifest, Permission,
    ProviderManifest, Runtime,
};
pub use resolve::{ActivationPlan, ResolveError, resolve_mode};
