//! "Instructions & library" beside a Babel task: a slim strip that opens the task project's
//! [`ProjectResourcesView`] (how to do the task, grading guide, library) in a side column.
//! Nothing loads until the column is opened; the open/closed choice lasts for the session.

use exaudio_babel_services::babel::META_PROJECT_ID;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use gpui_kit::*;

use crate::screens::project_resources::ProjectResourcesView;
use crate::text::localized;
use exaudio_shell::editor::model::EditorModel;

/// Whether the column is open, shared by every task for the session.
struct ResourcesOpen(bool);

impl Global for ResourcesOpen {}

pub struct ResourcesPanel {
    model: Entity<EditorModel>,
    view: Option<Entity<ProjectResourcesView>>,
    _observe: Subscription,
}

impl ResourcesPanel {
    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe_in(&model, window, |this, _, window, cx| {
            if this.view.is_some() && Self::is_open(cx) {
                this.sync(window, cx);
            }
            cx.notify();
        });
        Self {
            model,
            view: None,
            _observe: observe,
        }
    }

    fn is_open(cx: &App) -> bool {
        cx.try_global::<ResourcesOpen>().is_some_and(|open| open.0)
    }

    /// The open task's Babel project id.
    fn project_id(&self, cx: &App) -> Option<String> {
        self.model
            .read(cx)
            .doc()
            .project()
            .metadata
            .get(META_PROJECT_ID)
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    }

    /// Points the view at the open task's project (creating it on first open) with the
    /// document's own transport.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_id(cx) else {
            self.view = None;
            return;
        };
        let client = crate::services::document_client(&self.model, cx);
        match &self.view {
            Some(view) => view.update(cx, |view, cx| {
                view.set_project(project_id, client, window, cx);
            }),
            None => {
                self.view =
                    Some(cx.new(|cx| ProjectResourcesView::new(project_id, client, window, cx)));
            }
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !Self::is_open(cx);
        cx.set_global(ResourcesOpen(open));
        if open {
            self.sync(window, cx);
        }
        cx.notify();
    }
}

impl Render for ResourcesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.project_id(cx).is_none() {
            return div().into_any_element();
        }
        let open = Self::is_open(cx);
        if open && self.view.is_none() {
            self.sync(window, cx);
        }
        let border = cx.theme().border;
        let background = cx.theme().background;
        let title = localized(cx, "Instructions & library", "Инструкции и библиотека");
        let view = self.view.clone().filter(|_| open);
        let Some(view) = view else {
            return v_flex()
                .h_full()
                .flex_none()
                .p_1()
                .border_l_1()
                .border_color(border)
                .child(
                    Button::new("resources-panel-open")
                        .ghost()
                        .small()
                        .icon(IconName::FileText)
                        .tooltip(title)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx))),
                )
                .into_any_element();
        };
        v_flex()
            .h_full()
            .w(rems(30.))
            .flex_none()
            .occlude()
            .border_l_1()
            .border_color(border)
            .bg(background)
            .child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("resources-panel-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip(localized(
                                cx,
                                "Close instructions & library",
                                "Закрыть инструкции и библиотеку",
                            ))
                            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().p_3().child(view))
            .into_any_element()
    }
}
