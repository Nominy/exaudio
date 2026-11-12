mod actions;
mod contribution;
mod row_availability;
mod time;
use crate::i18n::localized;
use crate::track_filter::TrackFilter;
pub(crate) use actions::build_transcript_action_menu;
pub use actions::{
    TranscriptActionCommand, TranscriptActionMenuEntry, TranscriptTrackOptionView,
    build_transcript_track_options, selected_track_option,
};
pub use contribution::{
    LocalizedText, TRANSCRIPT_ROW_DECORATION, TRANSCRIPT_STATIC_ROW_DECORATION,
    TRANSCRIPT_TABLE_DECORATION, TranscriptChip, TranscriptExtraCell, TranscriptExtraColumn,
    TranscriptNote, TranscriptRowPatch, TranscriptStaticRow, TranscriptStyledBody,
    TranscriptStyledText, TranscriptStyledWord, TranscriptTablePatch, TranscriptTone,
    TranscriptWordMark, TranscriptWordStyle,
};
use exaudio_core::Project;
use exaudio_extension::document::{Decoration, Diagnostic, DiagnosticCollection, Severity};
pub use row_availability::compute_row_availability;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
pub use time::format_mm_ss;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptAnnotationKind {
    Transcription,
    NonTranscription(String),
}
impl TranscriptAnnotationKind {
    pub fn is_transcription(&self) -> bool {
        matches!(self, Self::Transcription)
    }
    pub fn label(&self) -> String {
        match self {
            Self::Transcription => localized("transcription", "расшифровка").into(),
            Self::NonTranscription(label) => label.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptRowVisibleCells {
    pub id: bool,
    pub track: bool,
    pub start: bool,
    pub end: bool,
    pub text: bool,
    pub linter: bool,
    pub actions: bool,
}
impl Default for TranscriptRowVisibleCells {
    fn default() -> Self {
        Self {
            id: true,
            track: true,
            start: true,
            end: true,
            text: true,
            linter: true,
            actions: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptRowView {
    pub row_kind: TranscriptRowKind,
    pub sort_start_seconds: f64,
    /// The segment's id; the row's own id for a static row.
    pub segment_id: String,
    pub display_id: String,
    pub speaker_label: String,
    pub lane_id: String,
    pub selected_lane_id: String,
    pub selected_track_option: Option<TranscriptTrackOptionView>,
    pub track_options: Vec<TranscriptTrackOptionView>,
    pub annotation_kind: TranscriptAnnotationKind,
    pub start_time: String,
    pub end_time: String,
    pub visible_cells: TranscriptRowVisibleCells,
    /// The column between the end time and the text, the same for every row.
    pub extra_column: Option<Arc<TranscriptExtraColumn>>,
    pub extra_cell: Option<TranscriptExtraCell>,
    pub text: String,
    pub is_selected: bool,
    pub lint: Option<Diagnostic>,
    pub lint_issues: Vec<Diagnostic>,
    pub speaker_note: Option<TranscriptNote>,
    pub styled_text: Option<TranscriptStyledText>,
    pub word_marks: Vec<TranscriptWordMark>,
    pub chips: Vec<TranscriptChip>,
    pub is_flagged: bool,
    /// A warning of the row is hidden because the user asserted the text is right.
    pub has_asserted_warning: bool,
    pub is_read_only: bool,
    pub can_insert_above: bool,
    pub can_insert_below: bool,
    pub can_merge_above: bool,
    pub can_merge_below: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptRowKind {
    /// The row of a project segment.
    Segment,
    /// A [`TranscriptStaticRow`] in its tone.
    Static(TranscriptTone),
}

/// Extension-produced presentation data for one document revision.
pub(crate) struct RowContributions<'a> {
    pub diagnostics: &'a [DiagnosticCollection],
    pub decorations: Vec<Decoration>,
}

/// One row per segment, patched by the row decorations, plus the decorated static rows;
/// filtered by track, ordered by start time and numbered in that order.
pub(crate) fn build_transcript_rows(
    project: &Project,
    track_filter: &TrackFilter,
    selected_segment_id: Option<&str>,
    asserted_warning_keys: &BTreeSet<String>,
    contributions: RowContributions<'_>,
    read_only: bool,
) -> Vec<TranscriptRowView> {
    let RowContributions {
        diagnostics,
        decorations,
    } = contributions;
    let availability = compute_row_availability(project);
    let track_options = build_transcript_track_options(project);
    let mut rows = BTreeMap::new();
    for segment in &project.segments {
        let available = availability.get(&segment.id).copied().unwrap_or_default();
        rows.insert(
            segment.id.clone(),
            TranscriptRowView {
                row_kind: TranscriptRowKind::Segment,
                sort_start_seconds: segment.start_seconds,
                segment_id: segment.id.clone(),
                display_id: String::new(),
                speaker_label: speaker_label(project, &segment.speaker_id),
                lane_id: segment.lane_id.clone(),
                selected_lane_id: segment.lane_id.clone(),
                selected_track_option: selected_track_option(&track_options, &segment.lane_id),
                track_options: track_options.clone(),
                annotation_kind: TranscriptAnnotationKind::Transcription,
                start_time: format_mm_ss(segment.start_seconds),
                end_time: format_mm_ss(segment.end_seconds),
                visible_cells: TranscriptRowVisibleCells::default(),
                extra_column: None,
                extra_cell: None,
                text: segment.text.clone(),
                is_selected: false,
                lint: None,
                lint_issues: Vec::new(),
                speaker_note: None,
                styled_text: None,
                word_marks: Vec::new(),
                chips: Vec::new(),
                is_flagged: false,
                has_asserted_warning: false,
                is_read_only: read_only,
                can_insert_above: available.can_insert_above,
                can_insert_below: available.can_insert_below,
                can_merge_above: available.can_merge_above,
                can_merge_below: available.can_merge_below,
            },
        );
    }
    let mut table: Option<TranscriptTablePatch> = None;
    for decoration in decorations {
        match decoration.kind.as_str() {
            TRANSCRIPT_TABLE_DECORATION => {
                if let Some(patch) = payload(decoration.data) {
                    table = Some(patch);
                }
            }
            TRANSCRIPT_ROW_DECORATION => {
                let Some(row) = rows
                    .get_mut(&decoration.segment_id)
                    .filter(|row| row.row_kind == TranscriptRowKind::Segment)
                else {
                    continue;
                };
                if let Some(patch) = payload(decoration.data) {
                    apply_row_patch(row, patch);
                }
            }
            TRANSCRIPT_STATIC_ROW_DECORATION => {
                // A static row never takes the place of a segment's row.
                let is_segment = rows
                    .get(&decoration.segment_id)
                    .is_some_and(|row| row.row_kind == TranscriptRowKind::Segment);
                if is_segment {
                    continue;
                }
                let Some(row) = payload(decoration.data) else {
                    continue;
                };
                rows.insert(
                    decoration.segment_id.clone(),
                    static_row(decoration.segment_id, row, &track_options),
                );
            }
            _ => {}
        }
    }
    if let Some(table) = table {
        let extra_column = table.extra_column.map(Arc::new);
        for row in rows.values_mut() {
            row.visible_cells = table.visible_cells;
            row.extra_column.clone_from(&extra_column);
        }
    }
    let mut rows: Vec<_> = rows
        .into_values()
        .filter(|row| row.lane_id.is_empty() || track_filter.matches_lane(&row.lane_id))
        .collect();
    rows.sort_by(|a, b| a.sort_start_seconds.total_cmp(&b.sort_start_seconds));
    for (index, row) in rows.iter_mut().enumerate() {
        row.display_id = (index + 1).to_string();
        row.is_selected = selected_segment_id == Some(row.segment_id.as_str());
        row.lint_issues = diagnostics
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| issue.segment_id == row.segment_id)
            .filter(|issue| {
                let asserted = issue.severity == Severity::Warning
                    && asserted_warning_keys.contains(&diagnostic_key(issue));
                row.has_asserted_warning |= asserted;
                !asserted
            })
            .cloned()
            .collect();
        row.lint = row.lint_issues.first().cloned();
        if row.is_read_only {
            row.can_insert_above = false;
            row.can_insert_below = false;
            row.can_merge_above = false;
            row.can_merge_below = false;
        }
    }
    rows
}

/// The payload of a decoration; `None` when it is not a `T`.
fn payload<T: serde::de::DeserializeOwned>(data: serde_json::Value) -> Option<T> {
    serde_json::from_value(data).ok()
}

fn apply_row_patch(row: &mut TranscriptRowView, patch: TranscriptRowPatch) {
    if let Some(annotation_kind) = patch.annotation_kind {
        row.annotation_kind = annotation_kind;
    }
    row.is_read_only |= patch.read_only;
    if patch.extra_cell.is_some() {
        row.extra_cell = patch.extra_cell;
    }
    if patch.speaker_note.is_some() {
        row.speaker_note = patch.speaker_note;
    }
    if patch.styled_text.is_some() {
        row.styled_text = patch.styled_text;
    }
    row.word_marks.extend(patch.word_marks);
    row.chips.extend(patch.chips);
    row.is_flagged |= patch.is_flagged;
}

fn static_row(
    row_id: String,
    row: TranscriptStaticRow,
    track_options: &[TranscriptTrackOptionView],
) -> TranscriptRowView {
    TranscriptRowView {
        row_kind: TranscriptRowKind::Static(row.tone),
        sort_start_seconds: row.sort_seconds,
        segment_id: row_id,
        display_id: String::new(),
        speaker_label: row.label,
        selected_lane_id: row.lane_id.clone(),
        selected_track_option: selected_track_option(track_options, &row.lane_id),
        lane_id: row.lane_id,
        track_options: track_options.to_vec(),
        annotation_kind: row.annotation_kind,
        start_time: row.start_time,
        end_time: row.end_time,
        visible_cells: TranscriptRowVisibleCells::default(),
        extra_column: None,
        extra_cell: row.extra_cell,
        text: row.text,
        is_selected: false,
        lint: None,
        lint_issues: Vec::new(),
        speaker_note: row.speaker_note,
        styled_text: row.styled_text,
        word_marks: Vec::new(),
        chips: Vec::new(),
        is_flagged: false,
        has_asserted_warning: false,
        is_read_only: true,
        can_insert_above: false,
        can_insert_below: false,
        can_merge_above: false,
        can_merge_below: false,
    }
}

pub fn diagnostic_key(issue: &Diagnostic) -> String {
    format!(
        "{}::{}::{}::{}",
        issue.segment_id, issue.source, issue.rule, issue.message
    )
}
pub(crate) fn speaker_label(project: &Project, speaker_id: &str) -> String {
    project
        .speakers
        .iter()
        .find(|speaker| speaker.id == speaker_id)
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| speaker_id.to_string())
}
