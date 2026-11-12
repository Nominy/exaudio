//! What an extension adds to the transcript table through [`Decoration`]s: one table patch
//! (cell visibility and an extra column), a patch per segment row and static rows that have
//! no segment behind them. The payloads are plain data; the row builder applies them and the
//! views draw them without knowing who supplied them.

use exaudio_extension::document::Decoration;
use serde::{Deserialize, Serialize};

use super::{TranscriptAnnotationKind, TranscriptRowVisibleCells};

/// Kind of a decoration carrying a [`TranscriptTablePatch`]; its segment id is unused.
pub const TRANSCRIPT_TABLE_DECORATION: &str = "exaudio.transcript.table.v2";
/// Kind of a decoration carrying the [`TranscriptRowPatch`] of the row of its segment.
pub const TRANSCRIPT_ROW_DECORATION: &str = "exaudio.transcript.row.v2";
/// Kind of a decoration carrying a [`TranscriptStaticRow`]; its segment id is the row's id.
pub const TRANSCRIPT_STATIC_ROW_DECORATION: &str = "exaudio.transcript.static-row.v2";

/// Text in both interface languages; the views show the active one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedText {
    pub en: String,
    pub ru: String,
}

impl LocalizedText {
    pub fn new(en: impl Into<String>, ru: impl Into<String>) -> Self {
        Self {
            en: en.into(),
            ru: ru.into(),
        }
    }

    /// Text that reads the same in both languages (document content, names).
    pub fn same(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            en: text.clone(),
            ru: text,
        }
    }

    /// The variant for the active language.
    pub fn get(&self) -> &str {
        if crate::i18n::is_russian() {
            &self.ru
        } else {
            &self.en
        }
    }
}

/// Colour role of a badge, note or static row, resolved against the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptTone {
    Muted,
    Info,
    Success,
    Warning,
    Danger,
}

/// A column between the end time and the text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptExtraColumn {
    pub header: LocalizedText,
    pub width_rems: f32,
}

/// A row's cell in the extra column. Without a hue the text has the foreground colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptExtraCell {
    pub text: String,
    /// Hue of the text colour in degrees (`0..360`).
    pub color_hue: Option<u16>,
}

/// A small toned line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptNote {
    pub label: LocalizedText,
    pub tone: TranscriptTone,
}

/// Read-only text behind a toned badge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptStyledText {
    pub badge: TranscriptNote,
    pub body: TranscriptStyledBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptStyledBody {
    /// Words drawn in order, separated by single spaces, each in its own style.
    Words(Vec<TranscriptStyledWord>),
    /// One muted italic text.
    Note(LocalizedText),
}

impl TranscriptStyledText {
    /// The body without its styling (what it measures and wraps as).
    pub fn plain_text(&self) -> String {
        match &self.body {
            TranscriptStyledBody::Words(words) => {
                let mut text = String::new();
                for (index, word) in words.iter().enumerate() {
                    if index > 0 {
                        text.push(' ');
                    }
                    text.push_str(&word.text);
                }
                text
            }
            TranscriptStyledBody::Note(note) => note.get().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptStyledWord {
    pub text: String,
    pub style: TranscriptWordStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptWordStyle {
    Plain,
    /// Success colour on a tinted background.
    Inserted,
    /// Danger colour on a tinted background, struck through.
    Deleted,
    /// Muted colour.
    Muted,
}

/// A whitespace-separated word of the row text an extension points at. A row with word marks
/// shows its text with them instead of opening for editing in place, unless it is read-only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptWordMark {
    /// Index of the word among the whitespace-separated words of the row text.
    pub word_index: usize,
    /// Wavy underline in the warning colour.
    pub underlined: bool,
    /// A click on the word seeks playback here.
    pub seek_seconds: Option<f64>,
}

/// A button under the row text (label behind a check mark) that runs a document command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptChip {
    pub label: String,
    pub tooltip: LocalizedText,
    pub command: String,
    pub arguments: serde_json::Value,
}

/// Table-wide presentation: which cells every row shows and the extra column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptTablePatch {
    pub visible_cells: TranscriptRowVisibleCells,
    pub extra_column: Option<TranscriptExtraColumn>,
}

impl TranscriptTablePatch {
    pub fn decoration(&self, source: &str) -> Decoration {
        decoration(source, TRANSCRIPT_TABLE_DECORATION, "", self)
    }
}

/// Additions to the row of one segment. Absent and empty fields leave the row as built.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptRowPatch {
    pub annotation_kind: Option<TranscriptAnnotationKind>,
    /// Makes the row read-only (a patch cannot unlock a row of a read-only document).
    pub read_only: bool,
    pub extra_cell: Option<TranscriptExtraCell>,
    /// Line under the speaker and the annotation kind.
    pub speaker_note: Option<TranscriptNote>,
    /// Drawn under the row text.
    pub styled_text: Option<TranscriptStyledText>,
    pub word_marks: Vec<TranscriptWordMark>,
    pub chips: Vec<TranscriptChip>,
    /// Draws a bar under the segment's region on the timeline.
    pub is_flagged: bool,
}

impl TranscriptRowPatch {
    pub fn decoration(&self, source: &str, segment_id: &str) -> Decoration {
        decoration(source, TRANSCRIPT_ROW_DECORATION, segment_id, self)
    }
}

/// A toned, non-interactive row without a segment: number, label in the speaker cell, times,
/// the extra cell and either `styled_text` or the muted `text`; dashes in the lint and actions
/// cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptStaticRow {
    /// Position among the rows, which are ordered by start time.
    pub sort_seconds: f64,
    /// Lane the row is filtered with; an empty id shows it under every track filter.
    pub lane_id: String,
    pub tone: TranscriptTone,
    pub label: String,
    pub start_time: String,
    pub end_time: String,
    pub text: String,
    pub styled_text: Option<TranscriptStyledText>,
    /// The row is as tall as a segment row of this kind and `speaker_note`; neither is drawn.
    pub annotation_kind: TranscriptAnnotationKind,
    pub speaker_note: Option<TranscriptNote>,
    pub extra_cell: Option<TranscriptExtraCell>,
}

impl TranscriptStaticRow {
    /// `row_id` must differ from every segment id; a row that reuses one is dropped.
    pub fn decoration(&self, source: &str, row_id: &str) -> Decoration {
        decoration(source, TRANSCRIPT_STATIC_ROW_DECORATION, row_id, self)
    }
}

fn decoration(source: &str, kind: &str, segment_id: &str, payload: &impl Serialize) -> Decoration {
    Decoration {
        source: source.into(),
        kind: kind.into(),
        segment_id: segment_id.into(),
        range: None,
        label: String::new(),
        // A payload that does not encode is null, which the row builder skips.
        data: serde_json::to_value(payload).unwrap_or_default(),
    }
}
