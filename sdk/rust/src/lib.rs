//! Build with `cargo build --target wasm32-unknown-unknown --release`, then
//! `wasm-tools component new <module.wasm> -o extension.wasm`.
//! The SDK uses only Exaudio host imports. Do not use std filesystem, network, clock or
//! random APIs: the host deliberately does not link WASI capabilities.
//!
//! User-invocable commands are *actions* declared in `manifest.json`
//! (`contributes.actions`: id `<extension-id>.<name>`, localized title, default keys and
//! menu placements). The shell lists them without activating the extension and calls
//! [`Guest::on_action`] when one runs; route that call through [`dispatch_action`].
pub mod bindings {
    wit_bindgen::generate!({ path: "../../wit", world: "extension", pub_export_macro: true });
}
pub use bindings::exaudio::extension::{host, types};
pub use bindings::exports::exaudio::extension::guest::Guest;
use serde::Deserialize;
pub use serde_json;

/// Arguments of an action invocation. The shell adds `document_id` (the open document's
/// handle) when a document is open and `segment_id` for row-scoped invocations; any other
/// members a caller of [`run_action`] passed are in `extra`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ActionArgs {
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub segment_id: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ActionArgs {
    /// Parses the JSON the host passes to `on-action` (`null` means no arguments).
    pub fn parse(args: &str) -> Result<Self, String> {
        match serde_json::from_str::<serde_json::Value>(args)
            .map_err(|error| format!("action arguments are not JSON: {error}"))?
        {
            serde_json::Value::Null => Ok(Self::default()),
            value => serde_json::from_value(value)
                .map_err(|error| format!("unexpected action arguments: {error}")),
        }
    }
    /// The document handle, or an error telling the user to open one.
    pub fn document(&self) -> Result<&str, String> {
        self.document_id
            .as_deref()
            .ok_or_else(|| "open a document first".into())
    }
}

/// One entry of an action table: the declared id and its handler.
pub type ActionHandler = fn(ActionArgs) -> Result<(), String>;

/// Routes `Guest::on_action` to the handler declared for `id`; unknown ids are an error
/// (the host only invokes ids from the manifest, so an unknown id is a missing handler).
pub fn dispatch_action(
    table: &[(&str, ActionHandler)],
    id: &str,
    args: &str,
) -> Result<(), String> {
    let handler = table
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, handler)| *handler)
        .ok_or_else(|| format!("no handler for action `{id}`"))?;
    handler(ActionArgs::parse(args)?)
}

/// Asks the shell to run one of this extension's declared actions or an `exaudio.core.*`
/// action, as if the user had invoked it. Unavailable during `activate`/`deactivate`.
pub fn run_action(id: &str, args: &serde_json::Value) -> Result<(), String> {
    host::run_action(id, &args.to_string())
}

/// Sets a `when` context key (`<extension-id>.<name>`) for the extension's clauses, e.g.
/// `set_context("example.hello-panel.ready", true)`.
pub fn set_context(key: &str, value: impl Into<types::Scalar>) -> Result<(), String> {
    host::set_context(key, &value.into())
}

/// Makes `key` read as `false` again.
pub fn clear_context(key: &str) -> Result<(), String> {
    host::clear_context(key)
}

impl From<bool> for types::Scalar {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}
impl From<&str> for types::Scalar {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}
impl From<String> for types::Scalar {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<f64> for types::Scalar {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

/// Build a revision-pinned, single undo-unit insertion. UTF-8 byte offsets are
/// checked locally for ergonomics and authoritatively checked again by the host.
pub fn insert_text(
    document: &types::Document,
    segment_id: &str,
    byte: usize,
    text: String,
    label: String,
) -> Result<types::Transaction, String> {
    let segment = document
        .segments
        .iter()
        .find(|segment| segment.id == segment_id)
        .ok_or("segment not found")?;
    if !segment.text.is_char_boundary(byte) {
        return Err("invalid UTF-8 insertion boundary".into());
    }
    let byte = u32::try_from(byte).map_err(|_| "text offset too large")?;
    Ok(types::Transaction {
        document_id: document.id.clone(),
        expected_revision: document.revision,
        label,
        edits: vec![types::Edit::ReplaceText(types::TextEdit {
            segment_id: segment_id.into(),
            start_byte: byte,
            end_byte: byte,
            text,
        })],
    })
}

/// Registers a sidebar panel holding one button. Handle the button in `Guest::on_ui_event`
/// (`contribution == panel`, `node == button_id`, `kind == Click`), typically by calling
/// [`run_action`] with one of the manifest's declared actions.
pub fn button_panel(panel: &str, title: &str, button_id: &str, button: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: panel.into(),
        kind: types::ContributionKind::Panel,
        title: title.into(),
    })?;
    host::publish_ui(&types::UiTree {
        contribution: panel.into(),
        nodes: vec![
            types::UiNode {
                id: "root".into(),
                parent: None,
                widget: types::Widget::Stack,
                config: types::WidgetConfig::Layout(types::LayoutConfig {
                    gap: 8,
                    padding: 12,
                }),
                text: title.into(),
                value: String::new(),
                enabled: true,
            },
            types::UiNode {
                id: button_id.into(),
                parent: Some(0),
                widget: types::Widget::Button,
                config: types::WidgetConfig::Plain,
                text: button.into(),
                value: String::new(),
                enabled: true,
            },
        ],
    })
}

/// Callbacks of a provider type (an account the host signs into while opening a mode).
/// Route `Guest::on_event` through [`provider_event`] to dispatch them.
pub trait ProviderHandler {
    /// The shell asked for a sign-in: publish UI for `contribution = setup.provider` and
    /// finish with [`provider_setup_done`] or [`provider_setup_cancel`] using
    /// `setup.request_id`. `setup.instance` is set when re-authenticating an account.
    fn on_provider_setup(setup: types::ProviderSetup) -> Result<(), String>;
    /// Answer with [`publish_status`] before returning; an unanswered request is an error.
    fn on_provider_status(instance: types::ProviderInstance) -> Result<(), String>;
    /// Delete the instance's credentials; the host removes its namespace afterwards.
    fn on_provider_sign_out(instance: types::ProviderInstance) -> Result<(), String>;
    /// The shell abandoned a setup (dialog closed, timeout). Drop any pending state.
    fn on_provider_setup_cancelled(_request_id: u64) -> Result<(), String> {
        Ok(())
    }
}
/// Dispatches provider events to `T`; returns `None` for every other event so the caller
/// keeps handling documents, modes and files itself.
pub fn provider_event<T: ProviderHandler>(event: &types::Event) -> Option<Result<(), String>> {
    Some(match event {
        types::Event::ProviderSetup(setup) => T::on_provider_setup(setup.clone()),
        types::Event::ProviderSetupCancelled(request) => T::on_provider_setup_cancelled(*request),
        types::Event::ProviderStatus(instance) => T::on_provider_status(instance.clone()),
        types::Event::ProviderSignOut(instance) => T::on_provider_sign_out(instance.clone()),
        _ => return None,
    })
}
/// Registers a provider type declared in the manifest's `providers`.
pub fn register_provider(id: &str, title: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: id.into(),
        kind: types::ContributionKind::Provider,
        title: title.into(),
    })
}
/// Registers a mode declared in the manifest's `modes`.
pub fn register_mode(id: &str, title: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: id.into(),
        kind: types::ContributionKind::Mode,
        title: title.into(),
    })
}
/// Reads one secret of an account this extension's provider owns (requires `secrets`).
pub fn instance_secret_get(
    instance: &types::ProviderInstance,
    key: &str,
) -> Result<Option<String>, String> {
    host::instance_secret_get(instance, key)
}
/// Writes (`Some`) or deletes (`None`) one secret of an account (requires `secrets`).
pub fn instance_secret_set(
    instance: &types::ProviderInstance,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    host::instance_secret_set(instance, key, value)
}
/// Finishes a setup. `id` must be stable per account and consist of `[a-z0-9-]`; `label`
/// is what users see. See [`instance_id`] for deriving one from a display name.
pub fn provider_setup_done(
    provider: &str,
    request_id: u64,
    id: &str,
    label: &str,
) -> Result<(), String> {
    host::provider_setup_done(&types::ProviderSetupResult {
        provider: provider.into(),
        request_id,
        id: id.into(),
        label: label.into(),
    })
}
pub fn provider_setup_cancel(provider: &str, request_id: u64) -> Result<(), String> {
    host::provider_setup_cancel(provider, request_id)
}
/// Answers a `provider-status` event.
pub fn publish_status(
    instance: &types::ProviderInstance,
    state: types::ProviderState,
    detail: &str,
) -> Result<(), String> {
    host::publish_provider_status(&types::ProviderStatus {
        instance: instance.clone(),
        state,
        detail: detail.into(),
    })
}
/// A path-safe instance id from a human identifier: lowercase `[a-z0-9-]`, at most 64
/// bytes, `"account"` when nothing survives.
pub fn instance_id(source: &str) -> String {
    let mut id = String::with_capacity(source.len().min(64));
    let mut dash = true;
    for c in source.chars().flat_map(char::to_lowercase) {
        if id.len() >= 64 {
            break;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            id.push(c);
            dash = false;
        } else if !dash {
            id.push('-');
            dash = true;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        id.push_str("account");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greet(args: ActionArgs) -> Result<(), String> {
        let document = args.document()?.to_owned();
        Err(format!(
            "greet:{document}:{}:{}",
            args.segment_id.unwrap_or_default(),
            args.extra
                .get("loud")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        ))
    }
    fn noop(_: ActionArgs) -> Result<(), String> {
        Ok(())
    }
    const TABLE: &[(&str, ActionHandler)] = &[("example.x.greet", greet), ("example.x.noop", noop)];

    #[test]
    fn dispatch_routes_declared_ids_and_parses_arguments() {
        assert_eq!(dispatch_action(TABLE, "example.x.noop", "null"), Ok(()));
        assert_eq!(
            dispatch_action(
                TABLE,
                "example.x.greet",
                r#"{"document_id":"document:1","segment_id":"s","loud":true}"#
            ),
            Err("greet:document:1:s:true".into())
        );
        assert_eq!(
            dispatch_action(TABLE, "example.x.greet", "null"),
            Err("open a document first".into())
        );
    }

    #[test]
    fn dispatch_rejects_unknown_ids_and_malformed_arguments() {
        assert!(
            dispatch_action(TABLE, "example.x.missing", "null")
                .unwrap_err()
                .contains("example.x.missing")
        );
        assert!(
            dispatch_action(TABLE, "example.x.noop", "{")
                .unwrap_err()
                .contains("not JSON")
        );
        assert!(dispatch_action(TABLE, "example.x.noop", "[1]").is_err());
    }

    #[test]
    fn instance_ids_are_path_safe() {
        assert_eq!(instance_id("Alice Smith"), "alice-smith");
        assert_eq!(instance_id("!!!"), "account");
    }
}
