//! Editor action definitions, audition keys and `EditorDoc::perform` dispatch (ported from
//! the iced app's keybinds.rs / hotkeys.rs tests).
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_editor::edit::keymap::{
    ACTION_DEFS, EditorAction, KeyChord, KeyContext, audition_key_for,
};
use studio_editor::edit::structure::AuditionKey;
use studio_editor::edit::{EditError, Effect, PointerModifiers};
use studio_editor::track_filter::TrackFilter;

fn three_row_doc() -> studio_editor::edit::EditorDoc {
    doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
        seg("c", "lane-2", 6.0, 20.0, "three"),
    ])
}

#[test]
fn every_action_has_exactly_one_definition_with_section_and_description() {
    assert_eq!(ACTION_DEFS.len(), EditorAction::ALL.len());
    for action in EditorAction::ALL.iter().copied() {
        let count = ACTION_DEFS
            .iter()
            .filter(|def| def.action == action)
            .count();
        assert_eq!(count, 1, "{action:?} must have exactly one ActionDef");
        let def = action.def();
        assert_eq!(def.action, action);
        assert!(
            !def.section.en.trim().is_empty() && !def.section.ru.trim().is_empty(),
            "{action:?} section"
        );
        assert!(
            !def.description.en.trim().is_empty() && !def.description.ru.trim().is_empty(),
            "{action:?} description"
        );
    }
}

#[test]
fn every_action_has_parsable_default_keystrokes() {
    for def in ACTION_DEFS {
        assert!(
            !def.defaults.is_empty(),
            "{:?} must have a default binding",
            def.action
        );
        for keystroke in def.defaults {
            assert!(
                KeyChord::parse(keystroke).is_some(),
                "{:?} default `{keystroke}` must parse",
                def.action
            );
        }
    }
}

#[test]
fn only_fixed_transcript_text_keys_are_not_configurable() {
    let fixed: Vec<EditorAction> = ACTION_DEFS
        .iter()
        .filter(|def| !def.configurable)
        .map(|def| def.action)
        .collect();
    assert_eq!(
        fixed,
        vec![
            EditorAction::TextUndo,
            EditorAction::TextRedo,
            EditorAction::TranscriptEscape,
            EditorAction::CompletionPrevious,
            EditorAction::CompletionNext,
            EditorAction::CompletionAccept,
            EditorAction::CompletionDismiss,
        ]
    );
    for action in fixed {
        assert_eq!(action.def().context, KeyContext::TranscriptText);
    }
}

#[test]
fn action_ids_round_trip_and_match_persisted_camel_case_ids() {
    assert_eq!(EditorAction::PlayPause.id(), "playPause");
    assert_eq!(
        EditorAction::DeleteSelectedSegment.id(),
        "deleteSelectedSegment"
    );
    assert_eq!(
        EditorAction::from_id("deleteSelectedSegment"),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(EditorAction::from_id("notAnAction"), None);
    for action in EditorAction::ALL.iter().copied() {
        assert_eq!(EditorAction::from_id(&action.id()), Some(action));
    }
}

#[test]
fn only_tag_suggestion_actions_require_open_suggestion_list() {
    for action in EditorAction::ALL.iter().copied() {
        let expected = matches!(
            action,
            EditorAction::CompletionPrevious
                | EditorAction::CompletionNext
                | EditorAction::CompletionAccept
                | EditorAction::CompletionDismiss
        );
        assert_eq!(action.requires_completions(), expected, "{action:?}");
    }
}

#[test]
fn default_delete_selected_segment_binds_unmodified_d_only() {
    assert_eq!(EditorAction::DeleteSelectedSegment.def().defaults, &["d"]);
}

#[test]
fn default_redo_binds_secondary_shift_z_and_secondary_y() {
    assert_eq!(
        EditorAction::Redo.def().defaults,
        &["secondary-shift-z", "secondary-y"]
    );
}

// ---- audition keys ----------------------------------------------------------------------

#[test]
fn audition_keys_map_unmodified_q_w_e_r() {
    assert_eq!(audition_key_for("q"), Some(AuditionKey::BeforeStart));
    assert_eq!(audition_key_for("w"), Some(AuditionKey::AfterStart));
    assert_eq!(audition_key_for("e"), Some(AuditionKey::BeforeEnd));
    assert_eq!(audition_key_for("r"), Some(AuditionKey::AfterEnd));
}

#[test]
fn audition_keys_ignore_command_modifiers_and_other_keys() {
    for keystroke in [
        "alt-r",
        "ctrl-q",
        "secondary-w",
        "cmd-e",
        "a",
        "t",
        "space",
        "",
    ] {
        assert_eq!(audition_key_for(keystroke), None, "`{keystroke}`");
    }
}

#[test]
fn audition_key_release_removes_only_that_key() {
    let mut doc = three_row_doc();
    doc.set_audition_key(AuditionKey::BeforeStart, true);
    doc.set_audition_key(AuditionKey::AfterEnd, true);

    doc.set_audition_key(AuditionKey::AfterStart, false);
    assert_eq!(doc.pointer().held_audition_keys.len(), 2);

    doc.set_audition_key(AuditionKey::BeforeStart, false);
    assert!(
        !doc.pointer()
            .held_audition_keys
            .contains(&AuditionKey::BeforeStart)
    );
    assert!(
        doc.pointer()
            .held_audition_keys
            .contains(&AuditionKey::AfterEnd)
    );
}

#[test]
fn window_unfocus_clears_audition_key_holds_and_modifiers() {
    let mut doc = three_row_doc();
    doc.set_audition_key(AuditionKey::BeforeStart, true);
    doc.set_audition_key(AuditionKey::AfterEnd, true);
    doc.set_modifiers(PointerModifiers {
        shift: true,
        alt: true,
        secondary: true,
    });

    ok(doc.window_unfocused());

    assert!(doc.pointer().held_audition_keys.is_empty());
    assert_eq!(doc.pointer().modifiers, PointerModifiers::default());
}

// ---- EditorDoc::perform dispatch -------------------------------------------------------

#[test]
fn perform_returns_none_for_app_level_actions() {
    let mut doc = three_row_doc();
    assert!(doc.perform(EditorAction::BackToProjects, None).is_none());
    assert!(
        doc.perform(EditorAction::ActivateDefaultSurfaceAction, None)
            .is_none(),
        "no doc dialog or find bar open: the UI owns the default action"
    );
    assert!(
        doc.perform(EditorAction::CreateRegionSubmit, None)
            .is_none(),
        "no add-region dialog open"
    );
}

#[test]
fn perform_text_actions_without_focused_row_return_none() {
    let mut doc = three_row_doc();
    for action in [
        EditorAction::TextUndo,
        EditorAction::TextRedo,
        EditorAction::TranscriptEscape,
        EditorAction::MoveTextBeforeCaretToPreviousSegment,
        EditorAction::MoveTextAfterCaretToNextSegment,
        EditorAction::CompletionPrevious,
        EditorAction::CompletionNext,
        EditorAction::CompletionAccept,
        EditorAction::CompletionDismiss,
    ] {
        assert!(doc.perform(action, None).is_none(), "{action:?}");
    }
}

#[test]
fn perform_delete_selected_segment_and_undo_restores_it() {
    let mut doc = three_row_doc();
    let _ = doc.select_segment("a");

    let action = EditorAction::DeleteSelectedSegment;
    ok(doc.perform(action, None).expect("doc action"));
    assert_eq!(ids(&doc), vec!["b", "c"]);

    let undo = EditorAction::Undo;
    ok(doc.perform(undo, None).expect("doc action"));
    assert_eq!(ids(&doc), vec!["a", "b", "c"]);
}

#[test]
fn perform_delete_selected_on_read_only_task_is_refused() {
    let mut doc = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    let _ = doc.select_segment("a");

    let result = doc
        .perform(EditorAction::DeleteSelectedSegment, None)
        .expect("doc action");

    assert!(matches!(result, Err(EditError::ReadOnly { .. })));
    assert_eq!(ids(&doc), vec!["a", "b"]);
}

#[test]
fn perform_play_pause_toggles_playback() {
    let mut doc = three_row_doc();
    assert!(!doc.session().is_playing);

    let outcome = ok(doc
        .perform(EditorAction::PlayPause, None)
        .expect("doc action"));
    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StartPlayback));

    let outcome = ok(doc
        .perform(EditorAction::PlayPause, None)
        .expect("doc action"));
    assert!(!doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StopPlayback));
}

#[test]
fn perform_speed_steps_change_speed_without_moving_playhead() {
    let mut doc = three_row_doc();
    let _ = doc.set_speed(1.0);
    doc.session_mut().playhead_seconds = 3.75;

    let outcome = ok(doc
        .perform(EditorAction::IncreasePlaybackSpeed, None)
        .expect("doc action"));
    assert_eq!(doc.session().speed, 1.5);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.5)));

    let outcome = ok(doc
        .perform(EditorAction::DecreasePlaybackSpeed, None)
        .expect("doc action"));
    assert_eq!(doc.session().speed, 1.0);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.0)));
}

#[test]
fn perform_rewind_moves_playhead_back_one_second() {
    let mut doc = three_row_doc();
    doc.session_mut().playhead_seconds = 3.75;

    ok(doc
        .perform(EditorAction::RewindPlaybackOneSecond, None)
        .expect("doc action"));

    assert!((doc.session().playhead_seconds - 2.75).abs() < 1e-9);
}

#[test]
fn perform_speaker_workflow_switch_and_reset() {
    let mut doc = three_row_doc();

    ok(doc
        .perform(EditorAction::SwitchSpeakerWorkflowLane1, None)
        .expect("doc action"));
    let session = doc.session();
    assert!(matches!(
        &session.track_filter,
        TrackFilter::Lane { label, .. } if label == "Speaker 1"
    ));
    assert!(!session.muted_lane_ids.contains("lane-1"));
    assert!(session.muted_lane_ids.contains("lane-2"));
    assert!(!session.collapsed_lane_ids.contains("lane-1"));
    assert!(session.collapsed_lane_ids.contains("lane-2"));

    ok(doc
        .perform(EditorAction::SwitchSpeakerWorkflowLane2, None)
        .expect("doc action"));
    let session = doc.session();
    assert!(matches!(
        &session.track_filter,
        TrackFilter::Lane { label, .. } if label == "Speaker 2"
    ));
    assert!(session.muted_lane_ids.contains("lane-1"));
    assert!(!session.muted_lane_ids.contains("lane-2"));

    ok(doc
        .perform(EditorAction::ResetSpeakerWorkflow, None)
        .expect("doc action"));
    let session = doc.session();
    assert_eq!(session.track_filter, TrackFilter::All);
    assert!(session.muted_lane_ids.is_empty());
    assert!(session.collapsed_lane_ids.is_empty());
}

#[test]
fn perform_focus_next_segment_focuses_adjacent_row_from_start_without_seeking() {
    let mut doc = three_row_doc();
    let _ = doc.select_segment("a");
    doc.session_mut().playhead_seconds = 1.0;

    let outcome = ok(doc
        .perform(EditorAction::FocusNextSegment, None)
        .expect("doc action"));

    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "b".to_string(),
        caret: 0,
    }));

    let outcome = ok(doc
        .perform(EditorAction::FocusPreviousSegment, None)
        .expect("doc action"));
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("a"));
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "a".to_string(),
        caret: 0,
    }));
}

#[test]
fn perform_toggle_overlay_and_selection_loop() {
    let mut doc = three_row_doc();

    ok(doc
        .perform(EditorAction::ToggleOverlay, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_some());
    ok(doc
        .perform(EditorAction::ToggleOverlay, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_none());

    set_overlay(&mut doc, "lane-2", 3.5, 5.1);
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert_eq!(doc.session().loop_range(), overlay_range(&doc));
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert_eq!(doc.session().loop_range(), None);
    assert!(doc.session().overlay.is_some(), "L off keeps the selection");
}

fn looping_selection_doc() -> studio_editor::edit::EditorDoc {
    let mut doc = three_row_doc();
    set_overlay(&mut doc, "lane-2", 3.5, 5.1);
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert!(doc.session().loop_range().is_some());
    doc
}

#[test]
fn escape_closes_create_region_dialog_before_cancelling_looping_selection() {
    let mut doc = looping_selection_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    assert!(doc.create_region().is_some());

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));
    assert!(doc.create_region().is_none());
    assert!(doc.session().loop_range().is_some());

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_none());
    assert!(doc.session().loop_range().is_none());
}

#[test]
fn escape_closes_find_bar_before_dialogs_and_selection() {
    let mut doc = looping_selection_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    let _ = doc.open_find();

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());
    assert!(doc.session().loop_range().is_some());
}

#[test]
fn captured_escape_closes_find_before_create_region_input_reset() {
    let mut doc = three_row_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    let _ = doc.open_find();
    assert!(doc.find_state().open);

    let action = EditorAction::CreateRegionEscape;
    ok(doc.perform(action, None).expect("doc action"));

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());

    ok(doc.perform(action, None).expect("doc action"));
    assert!(
        doc.create_region().is_some(),
        "input escape only resets the field; it never closes the dialog"
    );
}

#[test]
fn activate_default_surface_action_steps_find_when_find_bar_is_open() {
    let mut doc = three_row_doc();
    let _ = doc.open_find();

    let result = doc.perform(EditorAction::ActivateDefaultSurfaceAction, None);

    assert!(matches!(result, Some(Ok(_))));
    assert!(doc.find_state().open);
}

#[test]
fn transcript_escape_in_row_blurs_and_starts_playback() {
    let mut doc = three_row_doc();

    let outcome = ok(doc
        .perform(EditorAction::TranscriptEscape, Some("b"))
        .expect("focused row action"));

    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::BlurText));
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
}
