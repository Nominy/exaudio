//! Keyboard shortcuts dialog: every registered action by category (core first, then the
//! extensions by name), with rebinding (key capture), conflict warnings and reset per
//! action or for all. Overrides persist through [`actions::update`].

use exaudio_extension::actions::{CORE_OWNER, Entry, KeyChord, Priority};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::keymap::ACTION_DEFS;

use crate::actions::{self, IS_MAC, keystroke_string};
use crate::editor::panels::ui::{caption, dialog_footer, dialog_header, muted};
use crate::screens::ui::localized;

/// "Couldn't save the shortcut." with the storage error behind it.
fn not_saved(error: &str, cx: &App) -> Notice {
    Notice::Error(
        format!(
            "{} — {error}",
            localized(
                cx,
                "Couldn't save the shortcut.",
                "Не удалось сохранить сочетание."
            )
        )
        .into(),
    )
}

/// Keys that only modify another key; a binding needs a real key.
const MODIFIER_KEYS: &[&str] = &[
    "shift", "control", "ctrl", "alt", "option", "platform", "cmd", "super", "fn", "function",
];

#[derive(Clone)]
enum Notice {
    Info(SharedString),
    Warning(SharedString),
    Error(SharedString),
}

struct HotkeysDialog {
    focus_handle: FocusHandle,
    /// Action id whose new binding is being captured.
    capturing: Option<String>,
    notice: Option<Notice>,
}

/// A category of the list: its heading and the actions under it.
struct Section<'a> {
    title: &'a str,
    entries: Vec<&'a Entry>,
}

/// Core actions in `ACTION_DEFS` order (then the built-ins), then every other owner by
/// name; within an owner, categories in first-seen order.
fn sections<'a>(entries: impl Iterator<Item = &'a Entry>, lang: &str) -> Vec<Section<'a>> {
    let mut entries: Vec<&Entry> = entries.collect();
    let def_index = |entry: &Entry| {
        ACTION_DEFS
            .iter()
            .position(|def| actions::core_id(def.action) == entry.id)
            .unwrap_or(ACTION_DEFS.len())
    };
    entries.sort_by(|a, b| {
        let core = |entry: &Entry| entry.priority != Priority::Core;
        core(a)
            .cmp(&core(b))
            .then_with(|| a.owner_name.cmp(&b.owner_name))
            .then_with(|| def_index(a).cmp(&def_index(b)))
    });
    let mut sections: Vec<Section> = Vec::new();
    for entry in entries {
        let title = entry.category_in(lang);
        match sections
            .iter_mut()
            .find(|section| section.title == title && section.entries[0].owner == entry.owner)
        {
            Some(section) => section.entries.push(entry),
            None => sections.push(Section {
                title,
                entries: vec![entry],
            }),
        }
    }
    sections
}

/// Fixed text-editing keys of the editor are shown but not rebindable.
fn is_fixed(entry: &Entry) -> bool {
    entry.owner == CORE_OWNER
        && actions::editor_action(&entry.id).is_some_and(|action| !action.def().configurable)
}

impl HotkeysDialog {
    fn start_capture(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.capturing = Some(id);
        self.notice = Some(Notice::Info(
            localized(cx, "Press the new keys.", "Нажмите новое сочетание.").into(),
        ));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if self.capturing.take().is_some() {
            self.notice = Some(Notice::Info(localized(cx, "Cancelled", "Отменено").into()));
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.capturing.clone() else {
            return;
        };
        cx.stop_propagation();
        if MODIFIER_KEYS.contains(&event.keystroke.key.as_str()) {
            return;
        }
        let keystroke = keystroke_string(&event.keystroke);
        let Some(chord) = KeyChord::parse(&keystroke) else {
            self.notice = Some(Notice::Error(
                localized(
                    cx,
                    "That key can't be bound.",
                    "Эту клавишу нельзя назначить.",
                )
                .into(),
            ));
            self.capturing = None;
            cx.notify();
            return;
        };
        let resolved = chord.resolved(IS_MAC);
        actions::update(cx, |registry| {
            registry.set_user_binding(&id, Some(chord), None)
        });
        let lang = actions::lang(cx);
        let registry = actions::registry(cx);
        let others: Vec<String> = registry
            .conflicts(IS_MAC)
            .into_iter()
            .filter(|conflict| conflict.chord == resolved && conflict.actions.contains(&id))
            .flat_map(|conflict| conflict.actions)
            .filter(|other| other != &id)
            .filter_map(|other| registry.entry(&other))
            .map(|entry| entry.title_in(lang).to_string())
            .collect();
        self.notice = Some(match actions::take_persist_error(cx) {
            Some(error) => not_saved(&error, cx),
            None if others.is_empty() => {
                Notice::Info(localized(cx, "Shortcut changed", "Сочетание изменено").into())
            }
            None => Notice::Warning(
                localized(
                    cx,
                    "Shortcut changed. {chord} is also used for {actions}.",
                    "Сочетание изменено. {chord} также используется для: {actions}.",
                )
                .replace("{chord}", &resolved.label(IS_MAC))
                .replace("{actions}", &others.join(", "))
                .into(),
            ),
        });
        self.capturing = None;
        cx.notify();
    }

    fn reset(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.capturing.as_deref() == Some(id) {
            self.capturing = None;
        }
        actions::update(cx, |registry| registry.reset(id));
        self.notice = Some(match actions::take_persist_error(cx) {
            Some(error) => not_saved(&error, cx),
            None => Notice::Info(localized(cx, "Shortcut reset", "Сочетание сброшено").into()),
        });
        cx.notify();
    }

    fn reset_all(&mut self, cx: &mut Context<Self>) {
        self.capturing = None;
        actions::update(cx, |registry| registry.reset_all());
        self.notice = Some(match actions::take_persist_error(cx) {
            Some(error) => not_saved(&error, cx),
            None => {
                Notice::Info(localized(cx, "All shortcuts reset", "Все сочетания сброшены").into())
            }
        });
        cx.notify();
    }

    fn render_row(&self, entry: &Entry, conflicted: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let registry = actions::registry(cx);
        let labels: Vec<String> = registry
            .bindings_for(&entry.id, IS_MAC)
            .iter()
            .map(|chord| chord.label(IS_MAC))
            .collect();
        let overridden = registry.user().is_overridden(&entry.id);
        let capturing = self.capturing.as_deref() == Some(entry.id.as_str());
        let fixed = is_fixed(entry);
        let id = entry.id.clone();
        let title = entry.title_in(actions::lang(cx)).to_string();
        let theme = cx.theme();
        let keys = h_flex().gap_1().flex_wrap().children(if capturing {
            vec![
                div()
                    .text_xs()
                    .text_color(theme.primary)
                    .child(localized(cx, "Press keys…", "Нажмите клавиши…"))
                    .into_any_element(),
            ]
        } else {
            labels
                .into_iter()
                .map(|label| keycap(label, cx).into_any_element())
                .collect()
        });
        h_flex()
            .id(SharedString::from(format!("hotkey-{}", entry.id)))
            .px_3()
            .py_1p5()
            .gap_3()
            .rounded(theme.radius)
            .when(capturing, |this| this.bg(theme.accent))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(div().text_sm().child(title))
                    .when(overridden, |this| {
                        this.child(div().text_xs().text_color(theme.info).child(localized(
                            cx,
                            "changed",
                            "изменено",
                        )))
                    })
                    .when(conflicted, |this| {
                        this.child(div().text_xs().text_color(theme.warning).child(localized(
                            cx,
                            "conflict",
                            "конфликт",
                        )))
                    }),
            )
            .child(div().w(rems(13.75)).flex_none().child(keys))
            .child(
                h_flex()
                    .w(rems(9.375))
                    .flex_none()
                    .gap_1()
                    .justify_end()
                    .when(!fixed, |this| {
                        let change_id = id.clone();
                        let reset_id = id.clone();
                        this.child(if capturing {
                            Button::new("cancel")
                                .xsmall()
                                .ghost()
                                .label(localized(cx, "Cancel", "Отмена"))
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_capture(cx)))
                        } else {
                            Button::new("change")
                                .xsmall()
                                .outline()
                                .label(localized(cx, "Change", "Изменить"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_capture(change_id.clone(), window, cx);
                                }))
                        })
                        .child(
                            Button::new("reset")
                                .xsmall()
                                .ghost()
                                .label(localized(cx, "Reset", "Сбросить"))
                                .disabled(!overridden)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.reset(&reset_id, cx)),
                                ),
                        )
                    })
                    .when(fixed, |this| {
                        this.child(muted(localized(cx, "fixed", "фиксировано"), cx))
                    }),
            )
    }
}

fn keycap(label: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .py_0p5()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .child(label)
}

impl Render for HotkeysDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The list keeps its height unless the window is too short for it and the dialog
        // chrome (header, notices, footer).
        let rem = window.rem_size();
        let list_height = rems(28.75)
            .to_pixels(rem)
            .min(window.viewport_size().height - rems(16.).to_pixels(rem))
            .max(rems(8.).to_pixels(rem));
        let lang = actions::lang(cx);
        let registry = actions::registry(cx);
        let conflicts = registry.conflicts(IS_MAC);
        let user = registry.user();
        let any_overridden = !user.bindings.is_empty() || !user.removed.is_empty();
        let conflicted: std::collections::BTreeSet<&str> = conflicts
            .iter()
            .flat_map(|conflict| conflict.actions.iter().map(String::as_str))
            .collect();
        let conflict_lines: Vec<SharedString> = conflicts
            .iter()
            .map(|conflict| {
                let titles: Vec<&str> = conflict
                    .actions
                    .iter()
                    .filter_map(|id| registry.entry(id))
                    .map(|entry| entry.title_in(lang))
                    .collect();
                format!("{}: {}", conflict.chord.label(IS_MAC), titles.join(" / ")).into()
            })
            .collect();
        let sections: Vec<(String, Vec<Entry>)> = sections(registry.actions(), lang)
            .into_iter()
            .map(|section| {
                (
                    section.title.to_string(),
                    section.entries.into_iter().cloned().collect(),
                )
            })
            .collect();

        let mut list = v_flex().gap_3();
        for (title, entries) in sections {
            let mut group = v_flex()
                .gap_0p5()
                .child(div().px_3().child(caption(title, cx)));
            for entry in &entries {
                let row = self.render_row(entry, conflicted.contains(entry.id.as_str()), cx);
                group = group.child(row);
            }
            list = list.child(group);
        }

        let theme = cx.theme();
        let notice = self.notice.clone().map(|notice| {
            let (text, color) = match notice {
                Notice::Info(text) => (text, theme.muted_foreground),
                Notice::Warning(text) => (text, theme.warning),
                Notice::Error(text) => (text, theme.danger),
            };
            div().text_xs().text_color(color).child(text)
        });
        let hint = self.capturing.is_some().then(|| {
            SharedString::from(localized(
                cx,
                "Press the new keys.",
                "Нажмите новое сочетание.",
            ))
        });
        v_flex()
            .id("hotkeys-dialog")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::on_key_down))
            .gap_3()
            .child(dialog_header(
                localized(cx, "Keyboard shortcuts", "Сочетания клавиш"),
                hint,
                cx,
            ))
            .when(!conflict_lines.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_0p5()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.warning)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.warning)
                                .child(localized(
                                    cx,
                                    "These shortcuts conflict",
                                    "Эти сочетания конфликтуют",
                                )),
                        )
                        .children(conflict_lines.into_iter().map(|line| muted(line, cx))),
                )
            })
            .children(notice)
            .child(
                div()
                    .id("hotkeys-list")
                    .h(list_height)
                    .overflow_y_scrollbar()
                    .child(list),
            )
            .child(
                dialog_footer()
                    .child(
                        Button::new("hotkeys-reset-all")
                            .ghost()
                            .label(localized(cx, "Reset all", "Сбросить все"))
                            .disabled(!any_overridden)
                            .on_click(cx.listener(|this, _, _, cx| this.reset_all(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("hotkeys-close")
                            .primary()
                            .label(localized(cx, "Close", "Закрыть"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

/// Opens the keyboard shortcuts dialog.
pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| HotkeysDialog {
        focus_handle: cx.focus_handle(),
        capturing: None,
        notice: None,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        // While a key is being captured Escape is a candidate binding, not "close".
        let guard = view.clone();
        dialog
            .w(rems(47.5).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| guard.read(cx).capturing.is_none())
            .child(view.clone())
    });
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use exaudio_extension::actions::{ActionManifest, ActionRegistry, Localized};

    #[test]
    fn sections_list_core_in_definition_order_then_extensions_by_name() {
        let mut registry: ActionRegistry<()> = ActionRegistry::new();
        registry
            .register_owner(
                CORE_OWNER,
                "Exaudio",
                actions::core_actions(),
                Priority::Core,
            )
            .unwrap();
        let action = |id: &str, category: &str| ActionManifest {
            id: id.to_string(),
            title: Localized::en(id),
            category: Some(Localized::en(category)),
            when: None,
            palette: true,
            keys: Vec::new(),
            menus: Vec::new(),
        };
        registry
            .register_owner(
                "exaudio.zeta",
                "Zeta",
                vec![action("exaudio.zeta.one", "Zeta tools")],
                Priority::Extension,
            )
            .unwrap();
        registry
            .register_owner(
                "exaudio.alpha",
                "Alpha",
                vec![
                    action("exaudio.alpha.b", "Second"),
                    action("exaudio.alpha.a", "First"),
                ],
                Priority::Extension,
            )
            .unwrap();
        let sections = sections(registry.actions(), "en");
        let titles: Vec<&str> = sections.iter().map(|section| section.title).collect();
        assert_eq!(titles[0], ACTION_DEFS[0].section.en);
        assert_eq!(
            sections[0].entries[0].id,
            actions::core_id(ACTION_DEFS[0].action)
        );
        let tail = &titles[titles.len() - 3..];
        assert_eq!(tail, ["First", "Second", "Zeta tools"]);
        assert!(titles.contains(&"Application"));
    }
}
