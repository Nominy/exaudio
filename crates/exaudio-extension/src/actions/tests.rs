use std::collections::BTreeSet;

use super::keys::legacy_chord;
use super::*;
use crate::manifest::{ExtensionManifest, ManifestError};

// ---- when: parsing ----------------------------------------------------------------------

fn parse(source: &str) -> When {
    When::parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"))
}

fn key(name: &str) -> When {
    When::Key(name.into())
}

#[test]
fn and_binds_tighter_than_or_and_not_tighter_than_and() {
    assert_eq!(
        parse("a || b && c"),
        When::Or(vec![key("a"), When::And(vec![key("b"), key("c")])])
    );
    assert_eq!(
        parse("!a && b"),
        When::And(vec![When::Not(Box::new(key("a"))), key("b")])
    );
    assert_eq!(
        parse("a && b || c && d"),
        When::Or(vec![
            When::And(vec![key("a"), key("b")]),
            When::And(vec![key("c"), key("d")]),
        ])
    );
}

#[test]
fn parentheses_override_precedence_and_double_negation_nests() {
    assert_eq!(
        parse("(a || b) && c"),
        When::And(vec![When::Or(vec![key("a"), key("b")]), key("c")])
    );
    assert_eq!(
        parse("!(a && b)"),
        When::Not(Box::new(When::And(vec![key("a"), key("b")])))
    );
    assert_eq!(
        parse("!!a"),
        When::Not(Box::new(When::Not(Box::new(key("a")))))
    );
}

#[test]
fn comparisons_accept_strings_booleans_and_numbers() {
    assert_eq!(
        parse("focus == 'workspace'"),
        When::Eq("focus".into(), Value::Str("workspace".into()))
    );
    assert_eq!(
        parse("editorOpen != false"),
        When::Ne("editorOpen".into(), Value::Bool(false))
    );
    assert_eq!(
        parse("count == 3"),
        When::Eq("count".into(), Value::Num(3.0))
    );
    assert_eq!(
        parse("ratio == -1.5"),
        When::Eq("ratio".into(), Value::Num(-1.5))
    );
    assert_eq!(
        parse("name == 'it\\'s'"),
        When::Eq("name".into(), Value::Str("it's".into()))
    );
    assert_eq!(
        parse("exaudio.babel.hasClaim && mode == 'exaudio.babel'"),
        When::And(vec![
            key("exaudio.babel.hasClaim"),
            When::Eq("mode".into(), Value::Str("exaudio.babel".into())),
        ])
    );
    assert_eq!(
        parse("example.hello-panel.ready == true && !example.hello-panel.busy"),
        When::And(vec![
            When::Eq("example.hello-panel.ready".into(), Value::Bool(true)),
            When::Not(Box::new(key("example.hello-panel.busy"))),
        ])
    );
    let mut panel = Context::new();
    panel.set("example.hello-panel.ready", true);
    assert!(parse("example.hello-panel.ready").eval(&panel));
    assert!(parse("count == -1").eval(&ctx(&[("count", Value::Num(-1.0))])));
}

#[test]
fn parse_errors_report_the_offending_offset() {
    let error = |source: &str| When::parse(source).expect_err(source);
    assert_eq!(
        error(""),
        WhenError {
            pos: 0,
            kind: WhenErrorKind::UnexpectedEnd
        }
    );
    assert_eq!(
        error("a &&"),
        WhenError {
            pos: 4,
            kind: WhenErrorKind::UnexpectedEnd
        }
    );
    assert_eq!(
        error("a == "),
        WhenError {
            pos: 5,
            kind: WhenErrorKind::ExpectedValue
        }
    );
    assert_eq!(
        error("(a && b"),
        WhenError {
            pos: 7,
            kind: WhenErrorKind::ExpectedCloseParen
        }
    );
    assert_eq!(
        error("a == 'open"),
        WhenError {
            pos: 10,
            kind: WhenErrorKind::UnterminatedString
        }
    );
    assert_eq!(
        error("a b"),
        WhenError {
            pos: 2,
            kind: WhenErrorKind::Unexpected('b')
        }
    );
    assert_eq!(
        error("== 1"),
        WhenError {
            pos: 0,
            kind: WhenErrorKind::Unexpected('=')
        }
    );
    assert_eq!(
        error("a == 1.2.3"),
        WhenError {
            pos: 5,
            kind: WhenErrorKind::InvalidNumber
        }
    );
    assert_eq!(
        error("a == trueish"),
        WhenError {
            pos: 5,
            kind: WhenErrorKind::ExpectedValue
        }
    );
}

#[test]
fn display_round_trips_and_parenthesizes_only_where_needed() {
    for source in [
        "a",
        "!a",
        "a && b",
        "a || b && c",
        "(a || b) && c",
        "!(a && b) || c",
        "focus == 'workspace' && !cutPreview",
        "x != 'it\\'s' || y == true || z == 2.5",
        "!!a",
    ] {
        let parsed = parse(source);
        let printed = parsed.to_string();
        assert_eq!(parse(&printed), parsed, "`{source}` printed as `{printed}`");
    }
    assert_eq!(
        parse("((a || b)) && (!(c && d))").to_string(),
        "(a || b) && !(c && d)"
    );
    assert_eq!(parse("a && (b && c)").to_string(), "a && (b && c)");
}

// ---- when: evaluation -------------------------------------------------------------------

fn ctx(pairs: &[(&str, Value)]) -> Context {
    let mut ctx = Context::new();
    for (key, value) in pairs {
        ctx.set(*key, value.clone());
    }
    ctx
}

#[test]
fn missing_keys_are_false_for_bare_keys_and_both_comparisons() {
    let empty = Context::new();
    assert!(!parse("focus").eval(&empty));
    assert!(!parse("focus == 'workspace'").eval(&empty));
    assert!(!parse("focus != 'workspace'").eval(&empty));
    assert!(parse("!focus").eval(&empty));
    assert!(parse("!(focus == 'a')").eval(&empty));
}

#[test]
fn bare_keys_follow_truthiness_of_their_value() {
    assert!(parse("k").eval(&ctx(&[("k", Value::Bool(true))])));
    assert!(!parse("k").eval(&ctx(&[("k", Value::Bool(false))])));
    assert!(parse("k").eval(&ctx(&[("k", Value::Str("x".into()))])));
    assert!(!parse("k").eval(&ctx(&[("k", Value::Str(String::new()))])));
    assert!(parse("k").eval(&ctx(&[("k", Value::Num(2.0))])));
    assert!(!parse("k").eval(&ctx(&[("k", Value::Num(0.0))])));
}

#[test]
fn comparisons_match_by_type_and_value() {
    let c = ctx(&[
        ("focus", Value::Str("dialog".into())),
        ("open", Value::Bool(true)),
        ("n", Value::Num(3.0)),
    ]);
    assert!(parse("focus == 'dialog'").eval(&c));
    assert!(!parse("focus == 'workspace'").eval(&c));
    assert!(parse("focus != 'workspace'").eval(&c));
    assert!(parse("open == true").eval(&c));
    assert!(!parse("open == 'true'").eval(&c));
    assert!(parse("open != 'true'").eval(&c));
    assert!(parse("n == 3").eval(&c));
    assert!(parse("n != 4").eval(&c));
}

#[test]
fn boolean_connectives_evaluate_with_precedence() {
    let c = ctx(&[("a", Value::Bool(true)), ("b", Value::Bool(false))]);
    assert!(parse("a || b && missing").eval(&c));
    assert!(!parse("(a || b) && missing").eval(&c));
    assert!(parse("!b && a").eval(&c));
    assert!(!parse("!(a || b)").eval(&c));
}

#[test]
fn context_set_get_clear() {
    let mut c = Context::new();
    c.set("focus", "workspace");
    c.set("n", 2i64);
    assert_eq!(c.get("focus"), Some(&Value::Str("workspace".into())));
    assert_eq!(c.get("n"), Some(&Value::Num(2.0)));
    assert_eq!(c.clear("focus"), Some(Value::Str("workspace".into())));
    assert_eq!(c.get("focus"), None);
    assert_eq!(c.clear("focus"), None);
}

#[test]
fn compatibility_is_false_only_for_provable_contradictions() {
    let compatible = |a: &str, b: &str| parse(a).compatible(&parse(b));
    assert!(!compatible("focus == 'a'", "focus == 'b'"));
    assert!(!compatible("focus == 'a'", "focus != 'a'"));
    assert!(!compatible("a", "!a"));
    assert!(!compatible("x && focus == 'a'", "focus == 'b' && y"));
    assert!(compatible("focus == 'a'", "focus == 'a'"));
    assert!(compatible("focus == 'a'", "focus != 'b'"));
    assert!(compatible("a", "b"));
    assert!(compatible("a || b", "!a"));
    assert!(compatible("!(a && b)", "a"));
}

// ---- key chords -------------------------------------------------------------------------

#[test]
fn key_chord_parse_normalizes_modifier_order_and_uppercase_shift() {
    let chord = KeyChord::parse("Shift-Alt-r").expect("chord");
    assert_eq!(chord.to_string(), "alt-shift-r");
    assert_eq!(KeyChord::parse("alt-R"), KeyChord::parse("alt-shift-r"));
    assert_eq!(
        KeyChord::parse("shift-secondary-z")
            .expect("chord")
            .to_string(),
        "secondary-shift-z"
    );
    assert_eq!(
        KeyChord::parse("ctrl-shift-s").expect("chord").to_string(),
        "ctrl-shift-s"
    );
    assert_eq!(
        KeyChord::parse("control-win-k").expect("chord").to_string(),
        "ctrl-cmd-k"
    );
}

#[test]
fn key_chord_parse_rejects_empty_and_unknown_modifiers() {
    assert_eq!(KeyChord::parse(""), None);
    assert_eq!(KeyChord::parse("   "), None);
    assert_eq!(KeyChord::parse("hyper-k"), None);
    assert_eq!(KeyChord::parse("ctrl-"), None);
}

#[test]
fn key_chord_parse_accepts_minus_key() {
    assert_eq!(KeyChord::parse("-").expect("chord").key, "-");
    let chord = KeyChord::parse("secondary--").expect("chord");
    assert!(chord.secondary);
    assert_eq!(chord.key, "-");
}

#[test]
fn key_chord_secondary_resolves_to_ctrl_or_cmd() {
    let chord = KeyChord::parse("secondary-z").expect("chord");
    assert_eq!(Some(chord.resolved(false)), KeyChord::parse("ctrl-z"));
    assert_eq!(Some(chord.resolved(true)), KeyChord::parse("cmd-z"));
    assert_eq!(
        Some(
            KeyChord::parse("platform-z")
                .expect("chord")
                .resolved(false)
        ),
        KeyChord::parse("cmd-z")
    );
}

#[test]
fn key_chord_labels_use_platform_modifier_names_and_arrows() {
    let redo = KeyChord::parse("secondary-shift-z").expect("chord");
    assert_eq!(redo.label(false), "Ctrl+Shift+Z");
    assert_eq!(redo.label(true), "Cmd+Shift+Z");
    assert_eq!(
        KeyChord::parse("alt-left").expect("chord").label(false),
        "Alt+←"
    );
    assert_eq!(
        KeyChord::parse("cmd-f3").expect("chord").label(false),
        "Win+F3"
    );
    assert_eq!(
        KeyChord::parse("pagedown").expect("chord").label(true),
        "Pagedown"
    );
}

#[test]
fn key_chord_display_round_trips() {
    for source in [
        "a",
        "ctrl-shift-s",
        "secondary-alt-up",
        "cmd-`",
        "shift--",
        "f12",
    ] {
        let chord = KeyChord::parse(source).expect(source);
        assert_eq!(KeyChord::parse(&chord.to_string()), Some(chord));
    }
}

#[test]
fn legacy_chord_maps_dom_codes_and_modifiers() {
    let chord = |json: serde_json::Value| legacy_chord(&json).map(|c| c.to_string());
    assert_eq!(
        chord(serde_json::json!({"code": "KeyK", "modifiers": ["alt"]})),
        Some("alt-k".into())
    );
    assert_eq!(
        chord(serde_json::json!({"code": "ArrowUp", "modifiers": ["control", "shift"]})),
        Some("ctrl-shift-up".into())
    );
    assert_eq!(
        chord(serde_json::json!({"code": "Backquote", "modifiers": ["logo"]})),
        Some("cmd-`".into())
    );
    assert_eq!(
        chord(serde_json::json!({"code": "F4", "modifiers": []})),
        Some("f4".into())
    );
    assert_eq!(
        chord(serde_json::json!({"code": "Unidentified", "modifiers": []})),
        None
    );
    assert_eq!(
        chord(serde_json::json!({"code": "KeyP", "modifiers": ["hyper"]})),
        None
    );
}

// ---- manifest ---------------------------------------------------------------------------

fn action(id: &str, title: &str, keys: &[&str]) -> ActionManifest {
    ActionManifest {
        id: id.into(),
        title: Localized::en(title),
        category: None,
        when: None,
        palette: true,
        keys: keys
            .iter()
            .map(|key| KeyBindingManifest {
                key: (*key).into(),
                when: None,
                mac: None,
            })
            .collect(),
        menus: Vec::new(),
    }
}

fn manifest_with(actions: serde_json::Value) -> Result<ExtensionManifest, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "id": "acme.tool", "name": "Tool", "version": "1.0.0", "api": "^0.2",
        "runtime": "wasm", "description": "", "license": "MIT",
        "contributes": { "actions": actions }
    }))
}

#[test]
fn manifest_actions_deserialize_with_defaults_and_validate() {
    let manifest = manifest_with(serde_json::json!([
        { "id": "acme.tool.run", "title": { "en": "Run", "ru": "Запуск" },
          "keys": [ { "key": "ctrl-r", "mac": "cmd-r", "when": "focus == 'workspace'" } ],
          "menus": [ { "menu": "editor/overflow", "group": "1_run", "order": 2 } ] },
        { "id": "acme.tool.hidden", "title": { "en": "Hidden" }, "palette": false,
          "when": "!editorOpen" }
    ]))
    .expect("deserializes");
    manifest.validate().expect("valid");
    let run = &manifest.contributes.actions[0];
    assert!(run.palette);
    assert_eq!(run.keys[0].mac.as_deref(), Some("cmd-r"));
    assert_eq!(run.menus[0].order, 2);
    assert!(!manifest.contributes.actions[1].palette);
    let compiled = run.compile("acme.tool").expect("compiles");
    assert_eq!(compiled.keys[0].chord(true).to_string(), "cmd-r");
    assert_eq!(compiled.keys[0].chord(false).to_string(), "ctrl-r");
}

#[test]
fn manifest_without_contributes_validates_and_has_no_actions() {
    let manifest: ExtensionManifest = serde_json::from_value(serde_json::json!({
        "id": "acme.tool", "name": "Tool", "version": "1.0.0", "api": "^0.2",
        "runtime": "wasm", "description": "", "license": "MIT"
    }))
    .expect("deserializes");
    manifest.validate().expect("valid");
    assert!(manifest.contributes.actions.is_empty());
}

#[test]
fn manifest_rejects_unknown_action_fields() {
    assert!(
        manifest_with(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" }, "handler": "x" }
        ]))
        .is_err()
    );
}

#[test]
fn manifest_validation_rejects_malformed_actions() {
    let error = |actions: serde_json::Value| {
        manifest_with(actions)
            .expect("deserializes")
            .validate()
            .expect_err("rejected")
    };
    assert!(matches!(
        error(serde_json::json!([{ "id": "other.ext.run", "title": { "en": "Run" } }])),
        ManifestError::InvalidActionId { .. }
    ));
    assert!(matches!(
        error(serde_json::json!([{ "id": "acme.tool.Run", "title": { "en": "Run" } }])),
        ManifestError::InvalidActionId { .. }
    ));
    assert!(matches!(
        error(serde_json::json!([{ "id": "acme.toolrun", "title": { "en": "Run" } }])),
        ManifestError::InvalidActionId { .. }
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" } },
            { "id": "acme.tool.run", "title": { "en": "Run again" } }
        ])),
        ManifestError::DuplicateAction(id) if id == "acme.tool.run"
    ));
    assert!(matches!(
        error(serde_json::json!([{ "id": "acme.tool.run", "title": { "ru": "Запуск" } }])),
        ManifestError::MissingTitle(_)
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" }, "when": "focus ==" }
        ])),
        ManifestError::InvalidWhen {
            error: WhenError { pos: 8, .. },
            ..
        }
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" },
              "keys": [ { "key": "ctrl-r", "when": "(x" } ] }
        ])),
        ManifestError::InvalidWhen { .. }
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" },
              "menus": [ { "menu": "m", "group": "g", "when": "a &&" } ] }
        ])),
        ManifestError::InvalidWhen { .. }
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" }, "keys": [ { "key": "hyper-r" } ] }
        ])),
        ManifestError::InvalidKey { key, .. } if key == "hyper-r"
    ));
    assert!(matches!(
        error(serde_json::json!([
            { "id": "acme.tool.run", "title": { "en": "Run" },
              "keys": [ { "key": "ctrl-r", "mac": "" } ] }
        ])),
        ManifestError::InvalidKey { .. }
    ));
}

// ---- registry ---------------------------------------------------------------------------

type Registry = ActionRegistry<u32>;

fn chord(source: &str) -> KeyChord {
    KeyChord::parse(source).expect(source)
}

fn with_when(mut action: ActionManifest, when: &str) -> ActionManifest {
    action.when = Some(when.into());
    action
}

fn with_menu(mut action: ActionManifest, menu: &str, group: &str, order: i32) -> ActionManifest {
    action.menus.push(MenuPlacement {
        menu: menu.into(),
        group: group.into(),
        order,
        when: None,
    });
    action
}

fn ids<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Vec<&'a str> {
    entries.into_iter().map(|entry| entry.id.as_str()).collect()
}

fn core_and_extension() -> Registry {
    let mut registry = Registry::new();
    registry
        .register_owner(
            CORE_OWNER,
            "Exaudio",
            vec![
                with_when(
                    action("exaudio.core.split", "Split at playhead", &["s"]),
                    "focus == 'workspace'",
                ),
                action("exaudio.core.save", "Save project", &["secondary-s"]),
                with_when(
                    action("exaudio.core.insert-text", "Insert text", &["s"]),
                    "focus == 'transcriptText'",
                ),
            ],
            Priority::Core,
        )
        .expect("core registers");
    registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![
                action(
                    "acme.ext.save-remote",
                    "Save to Acme cloud",
                    &["secondary-s"],
                ),
                with_when(
                    action("acme.ext.stamp", "Stamp", &["s"]),
                    "focus == 'workspace'",
                ),
            ],
            Priority::Extension,
        )
        .expect("extension registers");
    registry.check_invariants();
    registry
}

fn workspace() -> Context {
    ctx(&[("focus", Value::Str("workspace".into()))])
}

#[test]
fn registration_is_atomic_per_owner() {
    let mut registry = Registry::new();
    let error = registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![
                action("acme.ext.good", "Good", &["ctrl-g"]),
                with_when(action("acme.ext.bad", "Bad", &[]), "focus =="),
            ],
            Priority::Extension,
        )
        .expect_err("batch rejected");
    assert!(matches!(
        error,
        RegistryError::Manifest(ManifestError::InvalidWhen { .. })
    ));
    registry.check_invariants();
    assert_eq!(registry.actions().count(), 0);
    assert!(registry.entry("acme.ext.good").is_none());
    assert_eq!(
        registry.resolve(&chord("ctrl-g"), &Context::new(), false),
        Resolution::default()
    );
    assert!(!registry.is_owner_enabled("acme.ext"));

    registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![action("acme.ext.good", "Good", &["ctrl-g"])],
            Priority::Extension,
        )
        .expect("the owner was not left half-registered");
    registry.check_invariants();
    assert!(registry.entry("acme.ext.good").is_some());
}

#[test]
fn foreign_prefixes_duplicates_and_reregistration_are_rejected() {
    let mut registry = Registry::new();
    assert!(matches!(
        registry.register_owner(
            "acme.ext",
            "Acme",
            vec![action("other.ext.run", "Run", &[])],
            Priority::Extension
        ),
        Err(RegistryError::Manifest(
            ManifestError::InvalidActionId { .. }
        ))
    ));
    assert!(matches!(
        registry.register_owner(
            "acme.ext",
            "Acme",
            vec![action("acme.extra.run", "Run", &[])],
            Priority::Extension
        ),
        Err(RegistryError::Manifest(
            ManifestError::InvalidActionId { .. }
        ))
    ));
    registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![action("acme.ext.sub.run", "Run", &[])],
            Priority::Extension,
        )
        .expect("registers");
    assert!(matches!(
        registry.register_owner(
            "acme.ext.sub",
            "Sub",
            vec![action("acme.ext.sub.run", "Run", &[])],
            Priority::Extension
        ),
        Err(RegistryError::Manifest(ManifestError::DuplicateAction(_)))
    ));
    assert!(matches!(
        registry.register_owner("acme.ext", "Acme", vec![], Priority::Extension),
        Err(RegistryError::OwnerRegistered(_))
    ));
    assert!(matches!(
        registry.register_owner("Bad Owner", "Bad", vec![], Priority::Extension),
        Err(RegistryError::Manifest(ManifestError::InvalidId(_)))
    ));
    registry.check_invariants();
    assert_eq!(ids(registry.actions()), vec!["acme.ext.sub.run"]);
}

#[test]
fn unregister_removes_entries_handlers_and_every_index_row() {
    let mut registry = core_and_extension();
    registry
        .register_owner(
            "acme.menu",
            "Menu",
            vec![with_menu(
                action("acme.menu.item", "Item", &["ctrl-m"]),
                "editor/overflow",
                "a",
                0,
            )],
            Priority::Extension,
        )
        .expect("registers");
    registry.bind("acme.menu.item", 7).expect("bound");
    registry.set_user_binding("acme.ext.stamp", Some(chord("ctrl-m")), None);
    registry.check_invariants();
    assert_eq!(
        registry
            .resolve(&chord("ctrl-m"), &workspace(), false)
            .chosen
            .as_deref(),
        Some("acme.ext.stamp")
    );

    assert!(registry.unregister_owner("acme.menu"));
    assert!(registry.unregister_owner("acme.ext"));
    assert!(!registry.unregister_owner("acme.ext"));
    registry.check_invariants();

    assert_eq!(
        registry.resolve(&chord("ctrl-m"), &Context::new(), false),
        Resolution::default()
    );
    assert!(registry.menu("editor/overflow", &Context::new()).is_empty());
    assert!(registry.search("item", "en", &Context::new()).is_empty());
    assert!(registry.search("stamp", "en", &workspace()).is_empty());
    assert!(!registry.is_bound("acme.menu.item"));
    assert_eq!(registry.owner_of("acme.ext.stamp"), None);
    assert_eq!(
        ids(registry.actions()),
        vec![
            "exaudio.core.insert-text",
            "exaudio.core.save",
            "exaudio.core.split"
        ]
    );
    // The user's override survives for when the extension is reinstalled, without index rows.
    assert!(registry.user().is_overridden("acme.ext.stamp"));
    assert!(registry.bindings_for("acme.ext.stamp", false).is_empty());
    assert_eq!(
        registry
            .resolve(&chord("secondary-s"), &workspace(), false)
            .shadowed,
        Vec::<ActionId>::new()
    );
}

#[test]
fn resolve_prefers_user_then_core_then_extension_and_reports_shadowed() {
    let mut registry = core_and_extension();
    let ctx = workspace();
    let resolution = registry.resolve(&chord("secondary-s"), &ctx, false);
    assert_eq!(resolution.chosen.as_deref(), Some("exaudio.core.save"));
    assert_eq!(
        resolution.shadowed,
        vec!["acme.ext.save-remote".to_string()]
    );
    // The chord index is keyed by the resolved chord: ctrl-s is secondary-s off mac.
    assert_eq!(
        registry
            .resolve(&chord("ctrl-s"), &ctx, false)
            .chosen
            .as_deref(),
        Some("exaudio.core.save")
    );
    assert_eq!(registry.resolve(&chord("ctrl-s"), &ctx, true).chosen, None);
    assert_eq!(
        registry
            .resolve(&chord("cmd-s"), &ctx, true)
            .chosen
            .as_deref(),
        Some("exaudio.core.save")
    );

    registry.set_user_binding("acme.ext.save-remote", Some(chord("secondary-s")), None);
    registry.check_invariants();
    let resolution = registry.resolve(&chord("secondary-s"), &ctx, false);
    assert_eq!(resolution.chosen.as_deref(), Some("acme.ext.save-remote"));
    assert_eq!(resolution.shadowed, vec!["exaudio.core.save".to_string()]);

    // Equal precedence falls back to id order.
    let resolution = registry.resolve(&chord("s"), &ctx, false);
    assert_eq!(resolution.chosen.as_deref(), Some("exaudio.core.split"));
    assert_eq!(resolution.shadowed, vec!["acme.ext.stamp".to_string()]);
}

#[test]
fn resolve_filters_by_action_when_binding_when_and_owner_enabled() {
    let mut registry = core_and_extension();
    registry
        .register_owner(
            "acme.cond",
            "Cond",
            vec![ActionManifest {
                keys: vec![KeyBindingManifest {
                    key: "x".into(),
                    when: Some("textSelection".into()),
                    mac: None,
                }],
                ..action("acme.cond.only-selection", "Only selection", &[])
            }],
            Priority::Extension,
        )
        .expect("registers");
    registry.check_invariants();

    let text = ctx(&[("focus", Value::Str("transcriptText".into()))]);
    assert_eq!(
        registry
            .resolve(&chord("s"), &text, false)
            .chosen
            .as_deref(),
        Some("exaudio.core.insert-text")
    );
    assert_eq!(
        registry.resolve(&chord("s"), &Context::new(), false).chosen,
        None
    );

    assert_eq!(registry.resolve(&chord("x"), &text, false).chosen, None);
    let mut selecting = text.clone();
    selecting.set("textSelection", true);
    assert_eq!(
        registry
            .resolve(&chord("x"), &selecting, false)
            .chosen
            .as_deref(),
        Some("acme.cond.only-selection")
    );

    registry.set_owner_enabled("acme.cond", false);
    assert_eq!(
        registry.resolve(&chord("x"), &selecting, false).chosen,
        None
    );
    assert!(!ids(registry.actions_in(&selecting)).contains(&"acme.cond.only-selection"));
    assert!(ids(registry.actions()).contains(&"acme.cond.only-selection"));
    registry.set_owner_enabled("acme.cond", true);
    assert!(ids(registry.actions_in(&selecting)).contains(&"acme.cond.only-selection"));
}

#[test]
fn mac_alternative_chord_replaces_the_default_on_mac_only() {
    let mut registry = Registry::new();
    registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![ActionManifest {
                keys: vec![KeyBindingManifest {
                    key: "ctrl-alt-r".into(),
                    when: None,
                    mac: Some("cmd-ctrl-r".into()),
                }],
                ..action("acme.ext.run", "Run", &[])
            }],
            Priority::Extension,
        )
        .expect("registers");
    registry.check_invariants();
    let ctx = Context::new();
    assert_eq!(
        registry
            .resolve(&chord("ctrl-alt-r"), &ctx, false)
            .chosen
            .as_deref(),
        Some("acme.ext.run")
    );
    assert_eq!(
        registry.resolve(&chord("ctrl-alt-r"), &ctx, true).chosen,
        None
    );
    assert_eq!(
        registry
            .resolve(&chord("cmd-ctrl-r"), &ctx, true)
            .chosen
            .as_deref(),
        Some("acme.ext.run")
    );
    assert_eq!(
        registry.bindings_for("acme.ext.run", false),
        vec![chord("ctrl-alt-r")]
    );
    assert_eq!(
        registry.bindings_for("acme.ext.run", true),
        vec![chord("cmd-ctrl-r")]
    );
}

#[test]
fn conflicts_list_chords_whose_candidates_can_fire_together() {
    let mut registry = core_and_extension();
    let conflicts = registry.conflicts(false);
    // `s`: split (workspace) vs stamp (workspace) conflict; insert-text (transcriptText) cannot
    // hold with either. `ctrl-s`: save vs save-remote, both unconditional.
    assert_eq!(
        conflicts,
        vec![
            Conflict {
                chord: chord("s"),
                actions: vec!["acme.ext.stamp".into(), "exaudio.core.split".into()],
            },
            Conflict {
                chord: chord("ctrl-s"),
                actions: vec!["acme.ext.save-remote".into(), "exaudio.core.save".into()],
            },
        ]
    );
    assert_eq!(registry.conflicts(true)[1].chord, chord("cmd-s"));

    registry.set_owner_enabled("acme.ext", false);
    assert!(registry.conflicts(false).is_empty());
    registry.set_owner_enabled("acme.ext", true);

    registry.set_user_binding(
        "acme.ext.stamp",
        Some(chord("s")),
        Some(parse("focus == 'dialog'")),
    );
    registry.check_invariants();
    let chords: Vec<KeyChord> = registry
        .conflicts(false)
        .into_iter()
        .map(|conflict| conflict.chord)
        .collect();
    assert_eq!(chords, vec![chord("ctrl-s")]);
}

#[test]
fn menus_are_sorted_by_group_order_id_and_filtered_by_clauses() {
    let mut registry = Registry::new();
    registry
        .register_owner(
            CORE_OWNER,
            "Exaudio",
            vec![
                with_menu(
                    action("exaudio.core.zeta", "Zeta", &[]),
                    "editor/overflow",
                    "1_edit",
                    0,
                ),
                with_menu(
                    action("exaudio.core.alpha", "Alpha", &[]),
                    "editor/overflow",
                    "1_edit",
                    0,
                ),
                with_menu(
                    action("exaudio.core.first", "First", &[]),
                    "editor/overflow",
                    "0_file",
                    5,
                ),
                with_menu(
                    with_when(action("exaudio.core.cut", "Cut", &[]), "cutPreview"),
                    "editor/overflow",
                    "0_file",
                    1,
                ),
                ActionManifest {
                    menus: vec![MenuPlacement {
                        menu: "editor/overflow".into(),
                        group: "0_file".into(),
                        order: 0,
                        when: Some("unsavedChanges".into()),
                    }],
                    ..action("exaudio.core.save", "Save", &[])
                },
                with_menu(
                    action("exaudio.core.other", "Other", &[]),
                    "row/context",
                    "a",
                    0,
                ),
            ],
            Priority::Core,
        )
        .expect("registers");
    registry.check_invariants();

    let items = registry.menu("editor/overflow", &Context::new());
    assert_eq!(
        ids(items.iter().map(|item| item.entry)),
        vec![
            "exaudio.core.first",
            "exaudio.core.alpha",
            "exaudio.core.zeta"
        ]
    );
    assert_eq!(items[0].group, "0_file");
    assert_eq!(items[0].order, 5);
    assert_eq!(items[1].group, "1_edit");

    let full = ctx(&[
        ("cutPreview", Value::Bool(true)),
        ("unsavedChanges", Value::Bool(true)),
    ]);
    assert_eq!(
        ids(registry
            .menu("editor/overflow", &full)
            .iter()
            .map(|item| item.entry)),
        vec![
            "exaudio.core.save",
            "exaudio.core.cut",
            "exaudio.core.first",
            "exaudio.core.alpha",
            "exaudio.core.zeta",
        ]
    );
    assert_eq!(
        ids(registry
            .menu("row/context", &full)
            .iter()
            .map(|item| item.entry)),
        vec!["exaudio.core.other"]
    );
    assert!(registry.menu("missing", &full).is_empty());

    registry.set_owner_enabled(CORE_OWNER, false);
    assert!(registry.menu("editor/overflow", &full).is_empty());
}

#[test]
fn search_ranks_prefix_then_word_start_then_substring_and_needs_every_term() {
    let mut registry = Registry::new();
    registry
        .register_owner(
            CORE_OWNER,
            "Exaudio",
            vec![
                action("exaudio.core.b-split", "Split at playhead", &[]),
                action("exaudio.core.a-split", "Split at playhead", &[]),
                action("exaudio.core.resplit", "Resplit segment", &[]),
                ActionManifest {
                    title: Localized::en("Open project"),
                    category: Some(Localized::en("Split tools")),
                    ..action("exaudio.core.open", "", &[])
                },
                action("exaudio.core.save", "Save project", &[]),
                with_when(action("exaudio.core.hidden", "Split hidden", &[]), "never"),
            ],
            Priority::Core,
        )
        .expect("registers");
    registry.check_invariants();
    let ctx = Context::new();

    assert_eq!(
        ids(registry.search("split", "en", &ctx)),
        vec![
            "exaudio.core.a-split",
            "exaudio.core.b-split",
            "exaudio.core.open",
            "exaudio.core.resplit",
        ]
    );
    assert_eq!(
        ids(registry.search("SPLIT play", "en", &ctx)),
        vec!["exaudio.core.a-split", "exaudio.core.b-split"]
    );
    assert_eq!(
        ids(registry.search("plit play", "en", &ctx)),
        vec!["exaudio.core.a-split", "exaudio.core.b-split"]
    );
    assert_eq!(
        ids(registry.search("project tools", "en", &ctx)),
        vec!["exaudio.core.open"]
    );
    assert!(registry.search("playhead segment", "en", &ctx).is_empty());
    assert_eq!(registry.search("", "en", &ctx).len(), 5);
}

#[test]
fn search_uses_requested_language_with_english_fallback() {
    let mut registry = Registry::new();
    registry
        .register_owner(
            "acme.ext",
            "Acme",
            vec![
                ActionManifest {
                    title: Localized::en("Delete").with("ru", "Удалить"),
                    ..action("acme.ext.delete", "", &[])
                },
                ActionManifest {
                    title: Localized::en("Insert"),
                    category: Some(Localized::en("Editing").with("ru", "Правка")),
                    ..action("acme.ext.insert", "", &[])
                },
            ],
            Priority::Extension,
        )
        .expect("registers");
    let ctx = Context::new();
    assert_eq!(
        ids(registry.search("удал", "ru", &ctx)),
        vec!["acme.ext.delete"]
    );
    assert!(registry.search("delete", "ru", &ctx).is_empty());
    assert_eq!(
        ids(registry.search("insert", "ru", &ctx)),
        vec!["acme.ext.insert"]
    );
    assert_eq!(
        ids(registry.search("правка", "ru", &ctx)),
        vec!["acme.ext.insert"]
    );
    assert_eq!(
        ids(registry.search("acme", "ru", &ctx)),
        vec!["acme.ext.delete"]
    );
    assert_eq!(
        ids(registry.search("delete", "de", &ctx)),
        vec!["acme.ext.delete"]
    );
}

#[test]
fn entry_text_falls_back_to_english_then_owner_name() {
    let mut registry = Registry::new();
    registry
        .register_owner(
            "acme.ext",
            "Acme Tools",
            vec![
                ActionManifest {
                    title: Localized::en("Delete").with("ru", "Удалить"),
                    ..action("acme.ext.delete", "", &[])
                },
                ActionManifest {
                    category: Some(Localized::en("Editing")),
                    ..action("acme.ext.insert", "Insert", &[])
                },
            ],
            Priority::Extension,
        )
        .expect("registers");
    let delete = registry.entry("acme.ext.delete").expect("entry");
    assert_eq!(delete.title_in("ru"), "Удалить");
    assert_eq!(delete.title_in("de"), "Delete");
    assert_eq!(delete.category_in("ru"), "Acme Tools");
    let insert = registry.entry("acme.ext.insert").expect("entry");
    assert_eq!(insert.category_in("ru"), "Editing");
    assert_eq!(registry.owner_of("acme.ext.insert"), Some("acme.ext"));
    assert_eq!(registry.owner_name("acme.ext"), Some("Acme Tools"));
    assert_eq!(insert.priority, Priority::Extension);
}

#[test]
fn handlers_bind_only_registered_actions_and_are_cloned_out() {
    let mut registry = core_and_extension();
    assert!(matches!(
        registry.bind("acme.ext.missing", 1),
        Err(RegistryError::UnknownAction(_))
    ));
    assert!(!registry.is_bound("exaudio.core.save"));
    assert_eq!(registry.handler("exaudio.core.save"), None);
    registry.bind("exaudio.core.save", 42).expect("bound");
    assert!(registry.is_bound("exaudio.core.save"));
    assert_eq!(registry.handler("exaudio.core.save"), Some(42));
    registry.bind("exaudio.core.save", 43).expect("rebound");
    assert_eq!(registry.unbind("exaudio.core.save"), Some(43));
    assert_eq!(registry.unbind("exaudio.core.save"), None);
    assert!(!registry.is_bound("exaudio.core.save"));
    registry.check_invariants();
}

// ---- user bindings ----------------------------------------------------------------------

#[test]
fn legacy_keymap_documents_migrate_to_core_action_ids_and_resave_in_new_format() {
    let user = UserBindings::from_json(
        r#"{
            "deleteSelectedSegment": {"code": "KeyK", "modifiers": ["alt"]},
            "increasePlaybackSpeed": {"code": "Digit3", "modifiers": ["shift"]},
            "openFind": {"code": "F4", "modifiers": []},
            "previousSegment": {"code": "ArrowUp", "modifiers": ["control", "shift"]},
            "resetSpeakerWorkflow": {"code": "Backquote", "modifiers": ["logo"]},
            "toggleOverlay": "alt-l",
            "playPause": 42,
            "textUndo": "hyper-u",
            "completionAccept": {"code": "Unidentified", "modifiers": []}
        }"#,
    );
    let as_strings: Vec<(String, String)> = user
        .bindings
        .iter()
        .map(|binding| (binding.action.clone(), binding.key.to_string()))
        .collect();
    assert_eq!(
        as_strings,
        vec![
            (
                "exaudio.core.delete-selected-segment".into(),
                "alt-k".into()
            ),
            (
                "exaudio.core.increase-playback-speed".into(),
                "shift-3".into()
            ),
            ("exaudio.core.open-find".into(), "f4".into()),
            (
                "exaudio.core.previous-segment".into(),
                "ctrl-shift-up".into()
            ),
            ("exaudio.core.reset-speaker-workflow".into(), "cmd-`".into()),
            ("exaudio.core.toggle-overlay".into(), "alt-l".into()),
        ]
    );
    assert!(user.removed.is_empty());

    let saved: serde_json::Value = serde_json::from_str(&user.to_json()).expect("json");
    assert_eq!(
        saved["bindings"][0],
        serde_json::json!({ "action": "exaudio.core.delete-selected-segment", "key": "alt-k" })
    );
    assert!(saved.get("deleteSelectedSegment").is_none());
    assert_eq!(UserBindings::from_json(&user.to_json()), user);
}

#[test]
fn new_format_round_trips_with_clauses_and_removed_and_drops_bad_entries() {
    let user = UserBindings::from_json(
        r#"{ "bindings": [
                { "action": "exaudio.core.split", "key": "ctrl-shift-s", "when": "focus == 'workspace'" },
                { "action": "acme.ext.run", "key": "hyper-r" },
                { "action": "acme.ext.walk", "key": "ctrl-w", "when": "focus ==" }
             ],
             "removed": [ "exaudio.babel.magic-review" ] }"#,
    );
    assert_eq!(
        user,
        UserBindings {
            bindings: vec![UserBinding {
                action: "exaudio.core.split".into(),
                key: chord("ctrl-shift-s"),
                when: Some(parse("focus == 'workspace'")),
            }],
            removed: BTreeSet::from(["exaudio.babel.magic-review".to_string()]),
        }
    );
    assert_eq!(UserBindings::from_json(&user.to_json()), user);
    assert_eq!(
        UserBindings::from_json(r#"{ "removed": [] }"#),
        UserBindings::default()
    );
    for malformed in ["", "not json", "[]", r#"{"bindings": 3}"#] {
        assert_eq!(UserBindings::from_json(malformed), UserBindings::default());
    }
}

#[test]
fn user_overrides_replace_defaults_and_removed_suppresses_them() {
    let mut registry = core_and_extension();
    let ctx = workspace();
    assert_eq!(
        registry.bindings_for("exaudio.core.split", false),
        vec![chord("s")]
    );

    registry.set_user_binding("exaudio.core.split", Some(chord("ctrl-shift-s")), None);
    registry.check_invariants();
    assert_eq!(
        registry.bindings_for("exaudio.core.split", false),
        vec![chord("ctrl-shift-s")]
    );
    assert_eq!(
        registry
            .resolve(&chord("ctrl-shift-s"), &ctx, false)
            .chosen
            .as_deref(),
        Some("exaudio.core.split")
    );
    assert_eq!(
        registry.resolve(&chord("s"), &ctx, false).chosen.as_deref(),
        Some("acme.ext.stamp")
    );
    assert!(registry.user().is_overridden("exaudio.core.split"));

    registry.remove_default("exaudio.core.split");
    registry.check_invariants();
    assert!(
        registry
            .bindings_for("exaudio.core.split", false)
            .is_empty()
    );
    assert_eq!(
        registry.resolve(&chord("ctrl-shift-s"), &ctx, false).chosen,
        None
    );
    assert!(registry.user().removed.contains("exaudio.core.split"));

    registry.set_user_binding("exaudio.core.split", Some(chord("f9")), None);
    registry.check_invariants();
    assert!(!registry.user().removed.contains("exaudio.core.split"));
    assert_eq!(
        registry
            .resolve(&chord("f9"), &ctx, false)
            .chosen
            .as_deref(),
        Some("exaudio.core.split")
    );

    registry.set_user_binding("exaudio.core.split", None, None);
    registry.check_invariants();
    assert_eq!(
        registry.bindings_for("exaudio.core.split", false),
        vec![chord("s")]
    );
    assert!(!registry.user().is_overridden("exaudio.core.split"));

    registry.remove_default("acme.ext.save-remote");
    registry.check_invariants();
    assert_eq!(
        registry.resolve(&chord("secondary-s"), &ctx, false),
        Resolution {
            chosen: Some("exaudio.core.save".into()),
            shadowed: Vec::new(),
        }
    );
    registry.reset("acme.ext.save-remote");
    registry.check_invariants();
    assert_eq!(
        registry
            .resolve(&chord("secondary-s"), &ctx, false)
            .shadowed,
        vec!["acme.ext.save-remote".to_string()]
    );

    registry.set_user_binding("exaudio.core.save", Some(chord("f2")), None);
    registry.remove_default("exaudio.core.split");
    registry.reset_all();
    registry.check_invariants();
    assert_eq!(registry.user(), &UserBindings::default());
    assert_eq!(
        registry.bindings_for("exaudio.core.save", false),
        vec![chord("secondary-s")]
    );
    assert_eq!(
        registry.bindings_for("exaudio.core.split", false),
        vec![chord("s")]
    );
}

#[test]
fn loading_user_bindings_reindexes_registered_actions_only() {
    let mut registry = core_and_extension();
    let ctx = workspace();
    registry.set_user(UserBindings::from_json(
        r#"{ "bindings": [
                { "action": "exaudio.core.split", "key": "f5" },
                { "action": "acme.missing.action", "key": "f6" }
             ],
             "removed": [ "exaudio.core.save" ] }"#,
    ));
    registry.check_invariants();
    assert_eq!(
        registry
            .resolve(&chord("f5"), &ctx, false)
            .chosen
            .as_deref(),
        Some("exaudio.core.split")
    );
    assert_eq!(registry.resolve(&chord("f6"), &ctx, false).chosen, None);
    assert_eq!(
        registry
            .resolve(&chord("secondary-s"), &ctx, false)
            .chosen
            .as_deref(),
        Some("acme.ext.save-remote")
    );
    assert!(registry.bindings_for("exaudio.core.save", false).is_empty());
    assert!(
        registry
            .bindings_for("acme.missing.action", false)
            .is_empty()
    );
}

#[test]
fn legacy_action_ids_are_kebab_cased_under_the_core_owner() {
    assert_eq!(legacy_action_id("playPause"), "exaudio.core.play-pause");
    assert_eq!(
        legacy_action_id("deleteSelectedSegment"),
        "exaudio.core.delete-selected-segment"
    );
    assert_eq!(legacy_action_id("undo"), "exaudio.core.undo");
}
