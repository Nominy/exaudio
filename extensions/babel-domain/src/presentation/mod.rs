//! Babel's additions to the transcript table, contributed as decorations: the reviewer column,
//! annotation kinds, the low-confidence word review and, while the diff view is on, the
//! comparison with another version of the task.

mod annotation_kind;
mod diff;
mod reviewer;
mod visible_cells;
mod words;

use crate::babel_metadata::project_is_read_only;
use crate::editor_model::diff_toolbar::diff_view_enabled;
use diff::{
    AnnotationDiffStatus, DiffCell, DiffCellKind, WordDiffStatus, annotation_diff_status,
    annotation_removed_ghost_rows, diff_cell, modified_segment_diff_rows,
};
use exaudio_core::{Project, Segment};
use exaudio_editor::transcript_model::{
    LocalizedText, TranscriptAnnotationKind, TranscriptChip, TranscriptNote, TranscriptRowPatch,
    TranscriptStaticRow, TranscriptStyledBody, TranscriptStyledText, TranscriptStyledWord,
    TranscriptTablePatch, TranscriptTone, TranscriptWordMark, TranscriptWordStyle, format_mm_ss,
};
use exaudio_extension::document::Decoration;
use reviewer::ReviewerLookup;
pub(crate) use words::low_confidence_words;

const SOURCE: &str = "exaudio.babel-domain";
const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";
const ACKNOWLEDGE_WORD_COMMAND: &str = "babel.acknowledge-low-confidence-word";

/// The review version the diff view compares with, when one is chosen.
fn selected_compare_action_id(project: &Project) -> Option<&str> {
    project
        .metadata
        .get(META_SELECTED_COMPARE_ACTION_ID)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

#[derive(Debug)]
pub struct BabelPresentationProvider {
    pub acknowledgements: std::sync::Arc<crate::workflow::BabelLowConfidenceCommandProvider>,
}

impl exaudio_extension::document::DecorationProvider for BabelPresentationProvider {
    fn decorate(&self, project: &Project, revision: u64) -> Vec<Decoration> {
        let reviewers = ReviewerLookup::for_project(project);
        let read_only = project_is_read_only(project);
        let diff_view = diff_view_enabled(project);
        let compare_action_id = selected_compare_action_id(project);

        let table = TranscriptTablePatch {
            visible_cells: visible_cells::visible_cells(project),
            extra_column: reviewers.column(),
        };
        let mut decorations = vec![table.decoration(SOURCE)];
        for segment in &project.segments {
            let mut patch = TranscriptRowPatch {
                annotation_kind: Some(annotation_kind::annotation_kind(segment)),
                read_only,
                extra_cell: reviewers.reviewer_at(segment.start_seconds),
                ..Default::default()
            };
            self.review_words(segment, revision, &mut patch);
            if diff_view {
                compare_segment(segment, compare_action_id, &mut patch);
            }
            decorations.push(patch.decoration(SOURCE, &segment.id));
        }
        if diff_view {
            decorations.extend(other_version_rows(project, &reviewers));
        }
        decorations
    }
}

impl BabelPresentationProvider {
    /// Marks the segment's low-confidence words: a click seeks to the word, and a word not
    /// acknowledged at `revision` is underlined and gets a chip that acknowledges it.
    fn review_words(&self, segment: &Segment, revision: u64, patch: &mut TranscriptRowPatch) {
        let acknowledgements = &self.acknowledgements;
        for word in low_confidence_words(segment) {
            let acknowledged = acknowledgements.is_acknowledged(&segment.id, word.index, revision);
            patch.word_marks.push(TranscriptWordMark {
                word_index: word.index,
                underlined: !acknowledged,
                seek_seconds: word.start_seconds,
            });
            if !acknowledged {
                patch.chips.push(TranscriptChip {
                    label: word.text,
                    tooltip: LocalizedText::new("Mark as reviewed", "Отметить как проверенное"),
                    command: ACKNOWLEDGE_WORD_COMMAND.into(),
                    arguments: serde_json::json!({ "word_index": word.index }),
                });
            }
        }
    }
}

/// Adds how `segment` differs from the compared version: the word diff under its text, the
/// annotation status under its speaker and the mark on its timeline region.
fn compare_segment(
    segment: &Segment,
    compare_action_id: Option<&str>,
    patch: &mut TranscriptRowPatch,
) {
    let cell = diff_cell(segment, compare_action_id);
    let annotation_status = annotation_diff_status(segment, compare_action_id);
    patch.styled_text = Some(styled_text(&cell, &segment.text));
    patch.speaker_note = annotation_status.map(annotation_note);
    patch.is_flagged = true;
}

/// Static rows for what only the compared version has: its segments and its annotations.
fn other_version_rows(project: &Project, reviewers: &ReviewerLookup) -> Vec<Decoration> {
    let reviewer_at = |start_seconds: Option<f64>| {
        start_seconds.and_then(|seconds| reviewers.reviewer_at(seconds))
    };
    let mut rows = Vec::new();
    for row in modified_segment_diff_rows(project) {
        let row_id = format!("modified-{}", row.annotation_id);
        let (label, tone) = if row.show_as_added {
            ("Added", TranscriptTone::Success)
        } else {
            ("Deleted", TranscriptTone::Danger)
        };
        let static_row = TranscriptStaticRow {
            sort_seconds: row.start_seconds.unwrap_or(0.0),
            lane_id: row.lane_id.unwrap_or_default(),
            tone,
            label: label.to_string(),
            start_time: time_or(row.start_seconds, "--"),
            end_time: time_or(row.end_seconds, "--"),
            text: String::new(),
            styled_text: Some(styled_text(&row.cell, "")),
            annotation_kind: TranscriptAnnotationKind::Transcription,
            speaker_note: None,
            extra_cell: reviewer_at(row.start_seconds),
        };
        rows.push(static_row.decoration(SOURCE, &row_id));
    }
    for row in annotation_removed_ghost_rows(project) {
        let row_id = format!("annotation-ghost-{}", row.annotation_id);
        let static_row = TranscriptStaticRow {
            sort_seconds: row.start_seconds.unwrap_or(0.0),
            lane_id: String::new(),
            tone: TranscriptTone::Danger,
            label: row.comparison_label,
            start_time: time_or(row.start_seconds, "--:--"),
            end_time: time_or(row.end_seconds, "--:--"),
            text: row.text,
            styled_text: None,
            annotation_kind: TranscriptAnnotationKind::NonTranscription("removed".to_string()),
            speaker_note: Some(annotation_note(AnnotationDiffStatus::UnmatchedReference)),
            extra_cell: reviewer_at(row.start_seconds),
        };
        rows.push(static_row.decoration(SOURCE, &row_id));
    }
    rows
}

fn time_or(seconds: Option<f64>, missing: &str) -> String {
    seconds.map_or_else(|| missing.to_string(), format_mm_ss)
}

/// A diff cell as styled text: its badge and the word diff, or a note when the cell has no
/// words (`row_text` for a new segment).
fn styled_text(cell: &DiffCell, row_text: &str) -> TranscriptStyledText {
    let body = if cell.words.is_empty() {
        TranscriptStyledBody::Note(if cell.kind == DiffCellKind::New {
            LocalizedText::same(row_text)
        } else {
            LocalizedText::new("No content", "Пусто")
        })
    } else {
        TranscriptStyledBody::Words(
            cell.words
                .iter()
                .map(|word| TranscriptStyledWord {
                    text: word.value.clone(),
                    style: match word.status {
                        WordDiffStatus::Unchanged => TranscriptWordStyle::Plain,
                        WordDiffStatus::Added => TranscriptWordStyle::Inserted,
                        WordDiffStatus::Removed => TranscriptWordStyle::Deleted,
                        WordDiffStatus::Unknown => TranscriptWordStyle::Muted,
                    },
                })
                .collect(),
        )
    };
    TranscriptStyledText {
        badge: diff_badge(cell),
        body,
    }
}

fn diff_badge(cell: &DiffCell) -> TranscriptNote {
    match cell.kind {
        DiffCellKind::New => note("New", "Новый", TranscriptTone::Info),
        DiffCellKind::ModifiedAdded => note("Added", "Добавлен", TranscriptTone::Success),
        DiffCellKind::ModifiedDeleted => note("Deleted", "Удалён", TranscriptTone::Danger),
        DiffCellKind::Diff => match cell.change_count {
            0 => note("No changes", "Без изменений", TranscriptTone::Muted),
            1 => note("1 change", "1 изменение", TranscriptTone::Warning),
            count => note(
                format!("{count} changes"),
                format!("{count} изменений"),
                TranscriptTone::Warning,
            ),
        },
    }
}

fn annotation_note(status: AnnotationDiffStatus) -> TranscriptNote {
    match status {
        AnnotationDiffStatus::Matched => note("Unchanged", "Без изменений", TranscriptTone::Muted),
        AnnotationDiffStatus::LabelMismatch => {
            note("Label changed", "Изменена метка", TranscriptTone::Warning)
        }
        AnnotationDiffStatus::SubcategoryMismatch => note(
            "Subcategory changed",
            "Изменена подкатегория",
            TranscriptTone::Warning,
        ),
        AnnotationDiffStatus::IntensityMismatch => note(
            "Intensity changed",
            "Изменена интенсивность",
            TranscriptTone::Warning,
        ),
        AnnotationDiffStatus::UnmatchedReference => {
            note("Removed", "Удалено", TranscriptTone::Danger)
        }
    }
}

fn note(en: impl Into<String>, ru: impl Into<String>, tone: TranscriptTone) -> TranscriptNote {
    TranscriptNote {
        label: LocalizedText::new(en, ru),
        tone,
    }
}

impl exaudio_extension::document::SegmentKindProvider for BabelPresentationProvider {
    fn is_text(&self, segment: &exaudio_core::Segment) -> Option<bool> {
        Some(
            segment
                .metadata
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|kind| kind == "transcription"),
        )
    }
}
