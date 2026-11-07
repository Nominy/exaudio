//! The shell's action registry: one global that the keyboard, menus, toolbar, hotkeys
//! dialog, palette and tray read, and that [`ShellHandle::run`] dispatches through. Editor
//! actions and shell built-ins are registered here under [`CORE_OWNER`]; extensions declare
//! theirs in `contributes.actions` and bind handlers in their activation.

use std::rc::Rc;

use exaudio_extension::actions::{
    ActionManifest, ActionRegistry, CORE_OWNER, Context, KeyBindingManifest, KeyChord, Localized,
    MenuPlacement, Priority, UserBindings, Value, legacy_action_id,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use studio_editor::edit::EditorAction;
use studio_editor::edit::keymap::{ACTION_DEFS, ActionDef, KeyContext};
use studio_editor::i18n::Text;

use crate::ShellHandle;
use crate::screens::ui;
use crate::services::AppServices;

pub const IS_MAC: bool = cfg!(target_os = "macos");

const HOST_OS: OperatingSystem = if IS_MAC {
    OperatingSystem::Mac
} else if cfg!(target_os = "windows") {
    OperatingSystem::Windows
} else {
    OperatingSystem::Linux
};

/// UI storage key of the user's key overrides (`UserBindings` JSON).
pub const STORAGE_KEY: &str = "exaudio.keymap";
/// Key the per-`EditorAction` overrides were saved under before the registry; read once and
/// rewritten under [`STORAGE_KEY`].
const LEGACY_STORAGE_KEY: &str = "studio.native.hotkeyBindings.v1";

pub const MENU_EDITOR_OVERFLOW: &str = "editor/overflow";
pub const MENU_EDITOR_TOOLBAR: &str = "editor/toolbar";
pub const MENU_ROW_CONTEXT: &str = "row/context";

/// Overflow-menu groups of the shell's own actions; groups sort lexically, so extensions pick
/// a `5_<name>` group to land between them.
pub const GROUP_DOCUMENT: &str = "1_document";
pub const GROUP_NAVIGATION: &str = "9_navigation";

pub type ActionHandler = Rc<dyn Fn(ShellHandle, serde_json::Value, &mut Window, &mut App)>;
/// Activates an owner that is not a native extension so its handlers get bound.
pub type ActivationHook = Rc<dyn Fn(&str, &mut App) -> anyhow::Result<()>>;

pub struct Actions(pub ActionRegistry<ActionHandler>);
impl Global for Actions {}

/// Shell-side state next to the registry: extension context keys, the focused transcript
/// row, the WASM activation hook and the last persistence failure.
#[derive(Default)]
struct ShellState {
    context: Context,
    /// Row whose text editor took focus last, with the editor's focus handle so the
    /// snapshot can check the focus live instead of trusting a stale record.
    text_row: Option<(String, FocusHandle)>,
    activation: Option<ActivationHook>,
    persist_error: Option<String>,
}
impl Global for ShellState {}

/// Canonical gpui keystroke text (`ctrl-shift-z`) as parsed by [`KeyChord`].
pub fn keystroke_string(keystroke: &Keystroke) -> String {
    keystroke.unparse(HOST_OS)
}

/// Installs the registry: user bindings from UI storage (migrating the previous format
/// once) and the core actions. Call after [`AppServices`] is installed.
pub fn init(cx: &mut App) {
    let mut registry = ActionRegistry::new();
    let storage = &cx.global::<AppServices>().storage;
    let (user, migrate) = match storage.value(STORAGE_KEY) {
        Some(source) => (UserBindings::from_json(source), needs_migration(source)),
        None => match storage.value(LEGACY_STORAGE_KEY) {
            Some(source) => (UserBindings::from_json(source), true),
            None => (UserBindings::default(), false),
        },
    };
    registry.set_user(user);
    registry
        .register_owner(CORE_OWNER, "Exaudio", core_actions(), Priority::Core)
        .expect("core actions are valid");
    cx.set_global(Actions(registry));
    cx.set_global(ShellState::default());
    if migrate {
        persist(cx);
    }
    bind_core(cx);
    // Only the editor owns Tab; shared pages retain normal keyboard focus traversal.
    cx.bind_keys([
        KeyBinding::new("tab", NoAction, Some("ExaudioEditor")),
        KeyBinding::new("shift-tab", NoAction, Some("ExaudioEditor")),
    ]);
}

/// Whether a stored keymap document is in the previous per-`EditorAction` shape.
fn needs_migration(source: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(source) {
        Ok(serde_json::Value::Object(root)) => {
            !root.contains_key("bindings") && !root.contains_key("removed")
        }
        _ => false,
    }
}

pub fn registry(cx: &App) -> &ActionRegistry<ActionHandler> {
    &cx.global::<Actions>().0
}

/// Applies `f`; user bindings are persisted when they changed.
pub fn update<R>(cx: &mut App, f: impl FnOnce(&mut ActionRegistry<ActionHandler>) -> R) -> R {
    let before = registry(cx).user().clone();
    let result = f(&mut cx.global_mut::<Actions>().0);
    if registry(cx).user() != &before {
        persist(cx);
    }
    result
}

fn persist(cx: &mut App) {
    let json = registry(cx).user().to_json();
    let saved = cx
        .global_mut::<AppServices>()
        .storage
        .set_value(STORAGE_KEY, json);
    cx.global_mut::<ShellState>().persist_error = saved.err().map(|error| error.to_string());
}

/// The failure of the last bindings save, if any (cleared on read).
pub fn take_persist_error(cx: &mut App) -> Option<String> {
    cx.global_mut::<ShellState>().persist_error.take()
}

/// The UI language actions are shown in.
pub fn lang(cx: &App) -> &'static str {
    ui::language(cx)
}

pub fn set_activation_hook(hook: ActivationHook, cx: &mut App) {
    cx.global_mut::<ShellState>().activation = Some(hook);
}

/// Sets a context key. Extension keys are `<ownerId>.<name>`; shell keys are unprefixed.
pub fn set_context(key: &str, value: impl Into<Value>, cx: &mut App) {
    cx.global_mut::<ShellState>().context.set(key, value);
}

pub fn clear_context(key: &str, cx: &mut App) {
    cx.global_mut::<ShellState>().context.clear(key);
}

/// Records the transcript row whose text editor is being focused (`handle` is the editor's).
pub fn set_text_row(segment_id: String, handle: FocusHandle, cx: &mut App) {
    cx.global_mut::<ShellState>().text_row = Some((segment_id, handle));
}

/// Transcript row whose text editor has keyboard focus right now.
pub fn focused_text_row(window: &Window, cx: &App) -> Option<String> {
    cx.global::<ShellState>()
        .text_row
        .as_ref()
        .filter(|(_, handle)| handle.is_focused(window))
        .map(|(id, _)| id.clone())
}

/// The context of this moment: shell keys (`editorOpen`, `focus`, `cutPreview`,
/// `textSelection`, `completionsOpen`, `mode`, `unsavedChanges`) plus extension keys.
pub fn snapshot(window: &mut Window, cx: &mut App) -> Context {
    let mut ctx = cx.global::<ShellState>().context.clone();
    let text_row = focused_text_row(window, cx);
    let focus = if text_row.is_some() {
        "transcriptText"
    } else if window.has_focused_input(cx) {
        "dialog"
    } else {
        "workspace"
    };
    ctx.set("focus", focus);
    let shell = cx.try_global::<ShellHandle>().cloned();
    let root = shell.as_ref().and_then(|shell| shell.0.upgrade());
    let model = root
        .as_ref()
        .and_then(|root| root.read(cx).editor_model(cx));
    ctx.set("editorOpen", model.is_some());
    if let Some(mode) = root.and_then(|root| root.read(cx).launched_mode().map(str::to_owned)) {
        ctx.set("mode", mode);
    }
    let (cut_preview, selection, completions, unsaved) = match &model {
        Some(model) => {
            let doc = model.read(cx).doc();
            let row = text_row.as_deref();
            (
                doc.session().overlay.is_some(),
                row.and_then(|id| doc.draft(id))
                    .is_some_and(|draft| !draft.selection().is_empty()),
                row.is_some_and(|id| doc.completions(id).is_some()),
                doc.session().is_dirty,
            )
        }
        None => (false, false, false, false),
    };
    ctx.set("cutPreview", cut_preview);
    ctx.set("textSelection", selection);
    ctx.set("completionsOpen", completions);
    ctx.set("unsavedChanges", unsaved);
    ctx
}

// ---- dispatch ---------------------------------------------------------------------------

/// What [`ShellHandle::run`] does next for an action id.
#[derive(Clone)]
pub(crate) enum Step<H> {
    Invoke(H),
    /// Known, enabled and unbound: activate the owner, then look again.
    Activate(String),
    Unavailable,
}

pub(crate) fn step<H: Clone>(registry: &ActionRegistry<H>, id: &str, activated: bool) -> Step<H> {
    let Some(owner) = registry.owner_of(id) else {
        return Step::Unavailable;
    };
    if !registry.is_owner_enabled(owner) {
        return Step::Unavailable;
    }
    match registry.handler(id) {
        Some(handler) => Step::Invoke(handler),
        None if activated => Step::Unavailable,
        None => Step::Activate(owner.to_string()),
    }
}

/// Activates `owner`: native extensions through their activator, others through the hook.
pub(crate) fn activate_owner(owner: &str, cx: &mut App) -> anyhow::Result<()> {
    if cx
        .global::<crate::ContributionRegistry>()
        .has_native_activator(owner)
    {
        return crate::contributions::enable_extension(owner, cx);
    }
    match cx.global::<ShellState>().activation.clone() {
        Some(hook) => hook(owner, cx),
        None => anyhow::bail!("no activator for {owner}"),
    }
}

// ---- core actions -----------------------------------------------------------------------

/// `exaudio.core.*` manifests: every [`ACTION_DEFS`] entry plus the shell built-ins.
pub fn core_actions() -> Vec<ActionManifest> {
    let mut actions: Vec<ActionManifest> = ACTION_DEFS.iter().map(editor_action_manifest).collect();
    actions.extend(BUILTINS.iter().map(Builtin::manifest));
    actions
}

/// Action id of an editor action (`exaudio.core.delete-selected-segment`).
pub fn core_id(action: EditorAction) -> String {
    legacy_action_id(&action.id())
}

/// The editor action behind a core action id.
pub fn editor_action(id: &str) -> Option<EditorAction> {
    ACTION_DEFS
        .iter()
        .map(|def| def.action)
        .find(|action| core_id(*action) == id)
}

/// `when` clause of a [`KeyContext`]; `None` for everywhere.
pub fn core_when(context: KeyContext) -> Option<&'static str> {
    match context {
        KeyContext::Global => None,
        KeyContext::Timeline => Some("focus == 'workspace'"),
        KeyContext::CutPreview => Some("focus == 'workspace' && cutPreview"),
        KeyContext::TranscriptText => Some("focus == 'transcriptText'"),
        KeyContext::Transcript => Some("focus != 'dialog'"),
        KeyContext::Dialog => Some("focus == 'dialog'"),
    }
}

fn localized(text: Text) -> Localized {
    Localized::en(text.en).with("ru", text.ru)
}

fn editor_action_manifest(def: &ActionDef) -> ActionManifest {
    let when = if def.action.requires_completions() {
        Some("focus == 'transcriptText' && completionsOpen".to_string())
    } else {
        core_when(def.context).map(str::to_string)
    };
    ActionManifest {
        id: core_id(def.action),
        title: localized(def.description),
        category: Some(localized(def.section)),
        when,
        palette: def.configurable && !is_key_reflex(def.action),
        keys: def
            .defaults
            .iter()
            .map(|key| KeyBindingManifest {
                key: (*key).to_string(),
                when: binding_when(def, key),
                mac: None,
            })
            .collect(),
        menus: Vec::new(),
    }
}

/// Escape/Enter handlers that only make sense as a keystroke, never as a chosen command.
fn is_key_reflex(action: EditorAction) -> bool {
    matches!(
        action,
        EditorAction::EscapePressed
            | EditorAction::ActivateDefaultSurfaceAction
            | EditorAction::CreateRegionSubmit
            | EditorAction::CreateRegionEscape
            | EditorAction::TranscriptEscape
    )
}

/// Keeps the previous precedence on shared default chords: a cut-preview binding beats the
/// timeline binding of the same chord while a cut selection exists, and completion keys beat
/// every other binding while the suggestion list is open.
fn binding_when(def: &ActionDef, key: &str) -> Option<String> {
    let chord = KeyChord::parse(key)?;
    let shared_with = |other: &ActionDef| {
        other
            .defaults
            .iter()
            .filter_map(|key| KeyChord::parse(key))
            .any(|candidate| candidate == chord)
    };
    let mut clauses = Vec::new();
    if def.context == KeyContext::Timeline
        && ACTION_DEFS
            .iter()
            .filter(|other| other.context == KeyContext::CutPreview)
            .any(shared_with)
    {
        clauses.push("!cutPreview");
    }
    if !def.action.requires_completions()
        && def.context != KeyContext::Dialog
        && ACTION_DEFS
            .iter()
            .filter(|other| other.action.requires_completions())
            .any(shared_with)
    {
        clauses.push("!completionsOpen");
    }
    (!clauses.is_empty()).then(|| clauses.join(" && "))
}

/// A shell built-in shown in the ⋯ menu.
struct Builtin {
    id: &'static str,
    title: Text,
    when: Option<&'static str>,
    group: &'static str,
    order: i32,
    run: fn(ShellHandle, &mut Window, &mut App),
}

const BUILTIN_CATEGORY: Text = Text::new("Application", "Приложение");

const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "exaudio.core.import-transcript",
        title: Text::new("Import transcript…", "Импортировать текст…"),
        when: Some("editorOpen"),
        group: GROUP_DOCUMENT,
        order: 0,
        run: crate::screens::formats::import_document,
    },
    Builtin {
        id: "exaudio.core.export",
        title: Text::new("Export…", "Экспорт…"),
        when: Some("editorOpen"),
        group: GROUP_DOCUMENT,
        order: 1,
        run: crate::screens::formats::export_document,
    },
    Builtin {
        id: "exaudio.core.copy-project-id",
        title: Text::new("Copy project ID", "Скопировать ID проекта"),
        when: Some("editorOpen"),
        group: GROUP_DOCUMENT,
        order: 2,
        run: copy_project_id,
    },
    Builtin {
        id: "exaudio.core.start",
        title: Text::new("Start", "Начало"),
        when: None,
        group: GROUP_NAVIGATION,
        order: 0,
        run: |shell, window, cx| shell.show_home(window, cx),
    },
    Builtin {
        id: "exaudio.core.settings",
        title: Text::new("Settings…", "Настройки…"),
        when: None,
        group: GROUP_NAVIGATION,
        order: 1,
        run: |shell, window, cx| shell.show_settings(window, cx),
    },
    Builtin {
        id: "exaudio.core.extensions",
        title: Text::new("Extensions…", "Расширения…"),
        when: None,
        group: GROUP_NAVIGATION,
        order: 2,
        run: |shell, window, cx| shell.show_extensions(window, cx),
    },
    Builtin {
        id: "exaudio.core.keyboard-shortcuts",
        title: Text::new("Keyboard shortcuts…", "Сочетания клавиш…"),
        when: None,
        group: GROUP_NAVIGATION,
        order: 3,
        run: |_, window, cx| crate::editor::panels::dialogs::hotkeys::open(window, cx),
    },
];

impl Builtin {
    fn manifest(&self) -> ActionManifest {
        ActionManifest {
            id: self.id.to_string(),
            title: localized(self.title),
            category: Some(localized(BUILTIN_CATEGORY)),
            when: self.when.map(str::to_string),
            palette: true,
            keys: Vec::new(),
            menus: vec![MenuPlacement {
                menu: MENU_EDITOR_OVERFLOW.to_string(),
                group: self.group.to_string(),
                order: self.order,
                when: None,
            }],
        }
    }
}

fn copy_project_id(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    let Some(model) = shell.editor_model(cx) else {
        return;
    };
    let id = model.read(cx).doc().project().id.clone();
    cx.write_to_clipboard(ClipboardItem::new_string(id));
    window.push_notification(
        ui::localized(cx, "Project ID copied.", "ID проекта скопирован."),
        cx,
    );
}

fn bind_core(cx: &mut App) {
    update(cx, |registry| {
        for def in ACTION_DEFS {
            let action = def.action;
            let handler: ActionHandler = Rc::new(move |shell, args, window, cx| {
                if let Some(screen) = shell.editor_screen(cx) {
                    screen.update(cx, |screen, cx| screen.perform(action, &args, window, cx));
                }
            });
            registry
                .bind(&core_id(action), handler)
                .expect("core action is registered");
        }
        for builtin in BUILTINS {
            let run = builtin.run;
            registry
                .bind(
                    builtin.id,
                    Rc::new(move |shell, _, window, cx| run(shell, window, cx)),
                )
                .expect("core action is registered");
        }
    });
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use exaudio_extension::actions::Resolution;

    fn core_registry() -> ActionRegistry<()> {
        let mut registry = ActionRegistry::new();
        registry
            .register_owner(CORE_OWNER, "Exaudio", core_actions(), Priority::Core)
            .unwrap();
        registry
    }

    fn ctx(focus: &str, extra: &[(&str, bool)]) -> Context {
        let mut ctx = Context::new();
        ctx.set("focus", focus);
        ctx.set("editorOpen", true);
        for (key, value) in extra {
            ctx.set(*key, *value);
        }
        ctx
    }

    fn chosen(registry: &ActionRegistry<()>, key: &str, ctx: &Context) -> Option<EditorAction> {
        let Resolution { chosen, .. } =
            registry.resolve(&KeyChord::parse(key).unwrap(), ctx, false);
        chosen.as_deref().and_then(editor_action)
    }

    #[test]
    fn every_editor_action_registers_under_core_with_its_legacy_id() {
        let registry = core_registry();
        for def in ACTION_DEFS {
            let id = core_id(def.action);
            assert_eq!(id, legacy_action_id(&def.action.id()));
            let entry = registry.entry(&id).expect("registered");
            assert_eq!(entry.owner, CORE_OWNER);
            assert_eq!(entry.title_in("ru"), def.description.ru);
            assert_eq!(entry.category_in("en"), def.section.en);
            assert_eq!(
                entry.palette,
                def.configurable && !is_key_reflex(def.action),
                "{id}"
            );
            assert_eq!(registry.bindings_for(&id, false).len(), def.defaults.len());
        }
        assert!(registry.entry("exaudio.core.copy-project-id").is_some());
    }

    #[test]
    fn key_contexts_map_to_focus_clauses() {
        let registry = core_registry();
        let workspace = ctx("workspace", &[]);
        let text = ctx("transcriptText", &[]);
        let dialog = ctx("dialog", &[]);
        assert_eq!(
            chosen(&registry, "space", &workspace),
            Some(EditorAction::PlayPause)
        );
        assert_eq!(chosen(&registry, "space", &text), None);
        assert_eq!(
            chosen(&registry, "secondary-f", &dialog),
            Some(EditorAction::OpenFind)
        );
        assert_eq!(
            chosen(&registry, "secondary-z", &workspace),
            Some(EditorAction::Undo)
        );
        assert_eq!(
            chosen(&registry, "secondary-z", &text),
            Some(EditorAction::TextUndo)
        );
        assert_eq!(
            chosen(&registry, "escape", &dialog),
            Some(EditorAction::CreateRegionEscape)
        );
        assert_eq!(
            chosen(&registry, "escape", &workspace),
            Some(EditorAction::EscapePressed)
        );
        assert_eq!(
            chosen(&registry, "tab", &text),
            Some(EditorAction::SwitchGhostCursorLane)
        );
        assert_eq!(
            chosen(&registry, "tab", &workspace),
            Some(EditorAction::SwitchGhostCursorLane)
        );
        assert_eq!(chosen(&registry, "tab", &dialog), None);
    }

    #[test]
    fn cut_preview_and_completion_bindings_take_precedence_without_conflicts() {
        let registry = core_registry();
        let workspace = ctx("workspace", &[]);
        let cut = ctx("workspace", &[("cutPreview", true)]);
        assert_eq!(
            chosen(&registry, "s", &workspace),
            Some(EditorAction::SplitAtPlayhead)
        );
        assert_eq!(
            chosen(&registry, "s", &cut),
            Some(EditorAction::CommitCutPreviewSmartSplit)
        );
        assert_eq!(chosen(&registry, "l", &workspace), None);
        let text = ctx("transcriptText", &[]);
        let suggesting = ctx("transcriptText", &[("completionsOpen", true)]);
        assert_eq!(
            chosen(&registry, "tab", &suggesting),
            Some(EditorAction::CompletionAccept)
        );
        assert_eq!(
            chosen(&registry, "escape", &text),
            Some(EditorAction::TranscriptEscape)
        );
        assert_eq!(
            chosen(&registry, "escape", &suggesting),
            Some(EditorAction::CompletionDismiss)
        );
        assert_eq!(chosen(&registry, "down", &text), None);
        assert!(registry.conflicts(false).is_empty());
        assert!(registry.conflicts(true).is_empty());
    }

    #[test]
    fn legacy_keymap_document_is_migrated() {
        assert!(needs_migration(r#"{"deleteSelectedSegment":"x"}"#));
        assert!(needs_migration("{}"));
        assert!(!needs_migration(r#"{"bindings":[]}"#));
        assert!(!needs_migration("nonsense"));
        let user = UserBindings::from_json(r#"{"deleteSelectedSegment":"x"}"#);
        let mut registry = core_registry();
        registry.set_user(user);
        assert_eq!(
            registry.bindings_for("exaudio.core.delete-selected-segment", false),
            vec![KeyChord::parse("x").unwrap()]
        );
        assert!(
            registry
                .user()
                .to_json()
                .contains(r#""action":"exaudio.core.delete-selected-segment""#)
        );
    }

    #[test]
    fn run_activates_the_owner_of_an_unbound_action_once() {
        let mut registry: ActionRegistry<u8> = ActionRegistry::new();
        let action = |id: &str| ActionManifest {
            id: id.to_string(),
            title: Localized::en("Do"),
            category: None,
            when: None,
            palette: true,
            keys: Vec::new(),
            menus: Vec::new(),
        };
        registry
            .register_owner(
                "exaudio.ext",
                "Ext",
                vec![action("exaudio.ext.do")],
                Priority::Extension,
            )
            .unwrap();
        assert!(matches!(
            step(&registry, "exaudio.ext.do", false),
            Step::Activate(owner) if owner == "exaudio.ext"
        ));
        assert!(matches!(
            step(&registry, "exaudio.ext.do", true),
            Step::Unavailable
        ));
        registry.bind("exaudio.ext.do", 7).unwrap();
        assert!(matches!(
            step(&registry, "exaudio.ext.do", false),
            Step::Invoke(7)
        ));
        registry.set_owner_enabled("exaudio.ext", false);
        assert!(matches!(
            step(&registry, "exaudio.ext.do", false),
            Step::Unavailable
        ));
        assert!(matches!(
            step(&registry, "exaudio.other.do", false),
            Step::Unavailable
        ));
    }
}
