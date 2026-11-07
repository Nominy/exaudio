//! Bundled command palette: a dialog listing every action the registry offers in the current
//! context, filtered as the user types. It knows no action id but its own; titles,
//! categories, bindings and availability all come from `exaudio_shell::actions`.

use std::rc::Rc;

use exaudio_extension::ExtensionManifest;
use exaudio_extension::actions::{Context as ActionContext, Entry, Priority};
use exaudio_shell::ShellHandle;
use exaudio_shell::actions::{self, ActionHandler, IS_MAC};
use exaudio_shell::screens::ui::localized;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub const ID: &str = "exaudio.palette";
const OPEN: &str = "exaudio.palette.open";

pub fn manifest() -> Result<ExtensionManifest, serde_json::Error> {
    serde_json::from_str(include_str!("../manifest.json"))
}

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    let handler: ActionHandler = Rc::new(|shell, _, window, cx| open(shell, window, cx));
    actions::update(cx, |registry| registry.bind(OPEN, handler))?;
    Ok(())
}

/// The empty-query list: core actions first, then by category, then title, in `lang`.
pub fn default_order<'a>(entries: impl Iterator<Item = &'a Entry>, lang: &str) -> Vec<&'a Entry> {
    let mut entries: Vec<&Entry> = entries.collect();
    entries.sort_by(|a, b| {
        (a.priority != Priority::Core)
            .cmp(&(b.priority != Priority::Core))
            .then_with(|| a.category_in(lang).cmp(b.category_in(lang)))
            .then_with(|| a.title_in(lang).cmp(b.title_in(lang)))
            .then_with(|| a.id.cmp(&b.id))
    });
    entries
}

/// One list entry, resolved for the current language and platform.
#[derive(Clone)]
struct Row {
    id: String,
    title: SharedString,
    category: SharedString,
    key: Option<SharedString>,
}

struct PaletteDialog {
    shell: ShellHandle,
    /// Context of the moment the palette was opened: once the dialog has focus every
    /// workspace-scoped `when` would fail, and that is the context the chosen action runs in.
    ctx: ActionContext,
    input: Entity<InputState>,
    rows: Vec<Row>,
    selected: usize,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl PaletteDialog {
    fn new(
        shell: ShellHandle,
        ctx: ActionContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "Type a command name",
                "Введите название команды",
            ))
        });
        let subscriptions = vec![cx.subscribe_in(&input, window, Self::on_input_event)];
        let mut this = Self {
            shell,
            ctx,
            input,
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    /// Rebuilds the rows from the registry for the current query.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value();
        let query = query.trim();
        let ctx = &self.ctx;
        let lang = actions::lang(cx);
        let registry = actions::registry(cx);
        let entries: Vec<&Entry> = if query.is_empty() {
            default_order(registry.actions_in(ctx).filter(|e| e.palette), lang)
        } else {
            registry
                .search(query, lang, ctx)
                .into_iter()
                .filter(|e| e.palette)
                .collect()
        };
        self.rows = entries
            .into_iter()
            .map(|entry| Row {
                id: entry.id.clone(),
                title: entry.title_in(lang).to_string().into(),
                category: entry.category_in(lang).to_string().into(),
                key: registry
                    .bindings_for(&entry.id, IS_MAC)
                    .first()
                    .map(|chord| chord.label(IS_MAC).into()),
            })
            .collect();
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.refresh(cx),
            // Key bindings dispatch before key listeners: Enter arrives here from the input,
            // and the dialog's own Confirm is disarmed in `open`.
            InputEvent::PressEnter { .. } => self.run(self.selected, window, cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let step: isize = match event.keystroke.key.as_str() {
            "up" => -1,
            "down" => 1,
            _ => return,
        };
        cx.stop_propagation();
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        self.selected = (self.selected as isize + step).rem_euclid(len) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    /// Closes the palette, then runs the row's action in the restored focus.
    fn run(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let id = row.id.clone();
        let shell = self.shell.clone();
        window.close_dialog(cx);
        shell.run(&id, serde_json::Value::Null, window, cx);
    }
}

impl Render for PaletteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let selected = index == self.selected;
            h_flex()
                .id(("palette-row", index))
                .gap_3()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .text_sm()
                .cursor_pointer()
                .when(selected, |el| {
                    el.bg(theme.accent).text_color(theme.accent_foreground)
                })
                .when(!selected, |el| el.hover(|style| style.bg(theme.list_hover)))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.run(index, window, cx);
                }))
                .child(div().min_w_0().truncate().child(row.title.clone()))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(if selected {
                            theme.accent_foreground.opacity(0.75)
                        } else {
                            theme.muted_foreground
                        })
                        .child(row.category.clone()),
                )
                .child(div().flex_1())
                .children(row.key.clone().map(|key| {
                    div()
                        .flex_none()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(if selected {
                            theme.accent_foreground.opacity(0.75)
                        } else {
                            theme.muted_foreground
                        })
                        .child(key)
                }))
        });
        let empty = self.rows.is_empty().then(|| {
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(localized(cx, "No matching commands", "Ничего не найдено"))
        });
        v_flex()
            .id("palette-dialog")
            .capture_key_down(cx.listener(Self::on_key_down))
            .gap_2()
            .child(Input::new(&self.input).small().cleanable(true))
            .child(
                v_flex()
                    .id("palette-list")
                    .max_h(rems(28.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .gap_0p5()
                    .children(rows)
                    .children(empty),
            )
    }
}

/// Opens the palette with its query input focused. The context is read before the dialog
/// takes focus so the list shows what the workspace offers, not what a dialog offers.
fn open(shell: ShellHandle, window: &mut Window, cx: &mut App) {
    let ctx = actions::snapshot(window, cx);
    let view = cx.new(|cx| PaletteDialog::new(shell, ctx, window, cx));
    let input = view.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .w(rems(36.).to_pixels(window.rem_size()))
            .close_button(false)
            // Enter runs the selected row (see `on_input_event`); it never confirms the dialog.
            .on_ok(|_, _, _| false)
            .child(view.clone())
    });
    input.update(cx, |input, cx| input.focus(window, cx));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use exaudio_extension::actions::{
        ActionManifest, ActionRegistry, CORE_OWNER, Context, Localized, Priority,
    };

    use super::default_order;

    fn action(id: &str, title: &str, category: Option<(&str, &str)>) -> ActionManifest {
        ActionManifest {
            id: id.into(),
            title: Localized(BTreeMap::from([
                ("en".to_string(), title.to_string()),
                ("ru".to_string(), format!("ru {title}")),
            ])),
            category: category.map(|(en, ru)| {
                Localized(BTreeMap::from([
                    ("en".to_string(), en.to_string()),
                    ("ru".to_string(), ru.to_string()),
                ]))
            }),
            when: None,
            palette: true,
            keys: Vec::new(),
            menus: Vec::new(),
        }
    }

    #[test]
    fn empty_query_lists_core_first_then_category_then_title() {
        let mut registry: ActionRegistry<()> = ActionRegistry::new();
        registry
            .register_owner(
                "acme.ext",
                "Acme",
                vec![
                    action("acme.ext.zap", "Zap", Some(("Tools", "Инструменты"))),
                    action("acme.ext.plain", "Plain", None),
                ],
                Priority::Extension,
            )
            .unwrap();
        registry
            .register_owner(
                CORE_OWNER,
                "Core",
                vec![
                    action(
                        "exaudio.core.split",
                        "Split segment",
                        Some(("Segments", "Сегменты")),
                    ),
                    action("exaudio.core.undo", "Undo", Some(("Editing", "Правка"))),
                    action(
                        "exaudio.core.delete",
                        "Delete segment",
                        Some(("Segments", "Сегменты")),
                    ),
                ],
                Priority::Core,
            )
            .unwrap();
        let ctx = Context::new();

        let en: Vec<&str> = default_order(registry.actions_in(&ctx), "en")
            .into_iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(
            en,
            [
                "exaudio.core.undo",
                "exaudio.core.delete",
                "exaudio.core.split",
                "acme.ext.plain",
                "acme.ext.zap",
            ]
        );

        // Russian categories sort in Cyrillic order; an action without a category sorts
        // under its owner's name.
        let ru: Vec<&str> = default_order(registry.actions_in(&ctx), "ru")
            .into_iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(
            ru,
            [
                "exaudio.core.undo",
                "exaudio.core.delete",
                "exaudio.core.split",
                "acme.ext.plain",
                "acme.ext.zap",
            ]
        );
    }
}
