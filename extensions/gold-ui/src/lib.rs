//! Gold's native UI contributions. The shell owns documents, storage and layout; this
//! extension owns model management, inference jobs, their progress and caches.

pub mod editor;
pub mod screens;
pub mod services;

use editor::denoise::DenoiseControl;
use editor::panels::gold_draft::{DraftMode, GoldDraftPanel, open_ai_settings};
use exaudio_extension::manifest::ExtensionManifest;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::screens::ui::localized;
use exaudio_shell::services::AppServices;
use exaudio_shell::{ContributionRegistry, PanelLocation, ShellHandle, actions};
use gpui_kit::*;
use screens::ai_settings::AiSettingsView;
use services::GoldServices;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub const ID: &str = "exaudio.gold";

pub fn manifest() -> anyhow::Result<ExtensionManifest> {
    let manifest: ExtensionManifest = serde_json::from_str(include_str!("../manifest.json"))?;
    manifest.validate()?;
    Ok(manifest)
}

type DraftViews = Rc<RefCell<HashMap<EntityId, WeakEntity<GoldDraftPanel>>>>;

fn draft_view(
    model: Entity<EditorModel>,
    views: &DraftViews,
    cx: &mut App,
) -> Entity<GoldDraftPanel> {
    let id = model.entity_id();
    if let Some(view) = views.borrow().get(&id).and_then(WeakEntity::upgrade) {
        return view;
    }
    let view = cx.new(|cx| GoldDraftPanel::new(model, cx));
    let mut views = views.borrow_mut();
    views.retain(|_, view| view.upgrade().is_some());
    views.insert(id, view.downgrade());
    view
}

/// Registers contributions after the application has registered and approved the manifest.
/// An optional polish provider may be registered before or after Gold activation.
pub fn register(cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<GoldServices>() {
        let services = GoldServices::new(&cx.global::<AppServices>().storage);
        cx.set_global(services);
    }
    let provider = services::registered_polish_provider(cx);
    cx.global_mut::<GoldServices>().polish = provider;
    let views: DraftViews = Rc::new(RefCell::new(HashMap::new()));
    let panel_views = views.clone();
    let unload_views = views.clone();
    let draft_panel_title = localized(cx, "AI draft", "Черновик ИИ");
    let denoise_title = localized(cx, "Denoise", "Убрать шум");
    let settings_title = localized(cx, "AI", "ИИ");
    let registry = cx.global_mut::<ContributionRegistry>();
    registry.register_panel(
        ID,
        "exaudio.gold.draft",
        draft_panel_title,
        PanelLocation::Overlay,
        Rc::new(move |shell, _, cx| {
            let model = shell
                .editor_model(cx)
                .expect("editor overlay requires an open document");
            draft_view(model, &panel_views, cx).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.gold.denoise",
        denoise_title,
        PanelLocation::Transport,
        Rc::new(|shell, window, cx| {
            let model = shell
                .editor_model(cx)
                .expect("transport contribution requires an open document");
            cx.new(|cx| DenoiseControl::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.gold.settings",
        settings_title,
        PanelLocation::Settings,
        Rc::new(|_, window, cx| cx.new(|cx| AiSettingsView::new(window, cx)).into()),
    )?;
    actions::update(cx, |actions| -> anyhow::Result<()> {
        for (id, mode) in [
            ("exaudio.gold.draft", DraftMode::Draft),
            ("exaudio.gold.align", DraftMode::TimestampsOnly),
        ] {
            let views = views.clone();
            actions.bind(
                id,
                Rc::new(move |shell: ShellHandle, _, window, cx| {
                    if let Some(model) = shell.editor_model(cx) {
                        draft_view(model, &views, cx)
                            .update(cx, |panel, cx| panel.open(mode, window, cx));
                    }
                }),
            )?;
        }
        actions.bind(
            "exaudio.gold.transcribe-segment",
            Rc::new(move |shell, args, window, cx| {
                let focused = args
                    .get("segment_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| shell.focused_segment(cx));
                if let Some(model) = shell.editor_model(cx) {
                    draft_view(model, &views, cx).update(cx, |panel, cx| {
                        panel.transcribe_segment(focused, window, cx)
                    });
                }
            }),
        )?;
        actions.bind(
            "exaudio.gold.settings",
            Rc::new(|_, _, window, cx| open_ai_settings(window, cx)),
        )?;
        Ok(())
    })?;
    cx.global_mut::<ContributionRegistry>().on_unload(
        ID,
        Rc::new(move |cx| {
            for view in unload_views
                .borrow_mut()
                .drain()
                .filter_map(|(_, view)| view.upgrade())
            {
                view.update(cx, |view, cx| view.shutdown(cx));
            }
            screens::ai_settings::shutdown(cx);
            if let Some(services) = cx.try_global::<GoldServices>() {
                // The provider belongs to Babel and survives Gold disable/re-enable.
                // Babel's unload hook removes its registration.
                services.inference.local().shutdown();
            }
            // Mounted contributions are dropped by the shell on its next render, cancelling
            // their task handles. Keep the global valid until those views have disappeared.
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    #[test]
    fn manifest_places_draft_and_align_on_the_toolbar() {
        let manifest = super::manifest().expect("manifest validates");
        let toolbar: Vec<&str> = manifest
            .contributes
            .actions
            .iter()
            .filter(|action| {
                action
                    .menus
                    .iter()
                    .any(|menu| menu.menu == "editor/toolbar")
            })
            .map(|action| action.id.as_str())
            .collect();
        assert_eq!(toolbar, ["exaudio.gold.draft", "exaudio.gold.align"]);
        let fill = manifest
            .contributes
            .actions
            .iter()
            .find(|action| action.id == "exaudio.gold.transcribe-segment")
            .unwrap();
        assert_eq!(fill.keys[0].key, "alt-shift-g");
    }
}
