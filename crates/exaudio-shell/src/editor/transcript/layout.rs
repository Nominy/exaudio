//! Column geometry and row-height measurement for the virtualized grid.
//!
//! `v_virtual_list` needs every row height up front, so rows are measured by wrapping their
//! text at the current text-column width with the same font, size, padding and wrapping
//! ([`WrappedText`]) the row's text editor uses; the static row text paints the same wrap.
//! Fixed cell metrics are rems, so heights follow the interface zoom. Wraps are cached per
//! row and only recomputed when the text, the column width or the font size (zoom) changes.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::*;
use studio_editor::edit::EditorDoc;
use studio_editor::edit::draft::{SegmentDraft, byte_offset};
use studio_editor::edit::time::TimeField;
use studio_editor::transcript_model::{TranscriptRowKind, TranscriptRowView};

use super::wrap::WrappedText;

pub(super) const ID_W: Rems = rems(2.75);
pub(super) const TRACK_W: Rems = rems(7.5);
pub(super) const TIME_W: Rems = rems(5.25);
pub(super) const REVIEWER_W: Rems = rems(8.75);
pub(super) const LINT_W: Rems = rems(2.75);
pub(super) const ACTIONS_W: Rems = rems(2.5);
/// Horizontal padding of fixed cells.
pub(super) const CELL_PX: Rems = rems(0.375);
/// Vertical padding of every row.
pub(super) const ROW_PY: Rems = rems(0.25);
/// Text-cell padding; matches the small multi-line input (`Size::Small`), whose paddings are
/// fixed pixels that do not follow the interface zoom.
pub(super) const TEXT_PX: Pixels = px(8.);
pub(super) const TEXT_PY: Pixels = px(2.);
/// The input wraps this much short of its text area (fixed pixels too); static text mirrors
/// it.
pub(super) const WRAP_MARGIN: Pixels = px(10.);
/// Secondary lines (annotation type, diff status, time errors).
pub(super) const SMALL_LINE_H: Rems = rems(1.);
/// Height of the low-confidence review chip strip.
pub(super) const CHIP_ROW_H: Rems = rems(1.625);
/// Width reserved for the diff badge in front of the word diff.
pub(super) const DIFF_BADGE_W: Rems = rems(5.25);
/// Gap between an annotation's text and its word diff (`mt_1`).
const DIFF_GAP: Rems = rems(0.25);
/// Fallback text-column width before the first layout.
const DEFAULT_TEXT_W: Rems = rems(40.);

/// Text metrics of the transcript text (`text_sm`, 1.25rem line height) at the window's rem
/// size (which follows the interface zoom).
#[derive(Clone, Copy)]
pub(super) struct TextMetrics {
    pub rem: Pixels,
    pub font_size: Pixels,
    pub line_height: Pixels,
}

impl TextMetrics {
    pub fn for_window(window: &Window) -> Self {
        let rem = window.rem_size();
        Self {
            rem,
            font_size: rems(0.875).to_pixels(rem),
            line_height: rems(1.25).to_pixels(rem),
        }
    }

    pub fn px(&self, length: Rems) -> Pixels {
        length.to_pixels(self.rem)
    }
}

/// Cache key of a measured text: the text itself, wrap width and font size. Every row is
/// checked on each render; comparing the stored text is cheaper than hashing it and cannot
/// collide.
struct CacheKey {
    text: Box<str>,
    width: Pixels,
    font_size: Pixels,
}

impl CacheKey {
    fn new(text: &str, width: Pixels, font_size: Pixels) -> Self {
        Self {
            text: text.into(),
            width,
            font_size,
        }
    }

    fn matches(&self, text: &str, width: Pixels, font_size: Pixels) -> bool {
        self.width == width && self.font_size == font_size && *self.text == *text
    }
}

/// Row wrap and diff line-count caches keyed by segment id, plus the heights of the last
/// measured row list.
#[derive(Default)]
pub(super) struct RowMetrics {
    text: HashMap<String, (CacheKey, Rc<WrappedText>)>,
    aux: HashMap<String, (CacheKey, usize)>,
    font_family: SharedString,
    sizes: Option<(SizesKey, Rc<Vec<Size<Pixels>>>)>,
}

/// Everything row heights derive from. Rows are compared by identity: the doc rebuilds them
/// whenever their contents change, and the key holds them so the address is not reused.
struct SizesKey {
    rows: Rc<[TranscriptRowView]>,
    generation: u64,
    drafts_revision: u64,
    wrap_width: Pixels,
    rem: Pixels,
    font_size: Pixels,
    line_height: Pixels,
    /// Diff rows of an empty word diff measure a localized placeholder.
    language: &'static str,
}

impl SizesKey {
    fn matches(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.rows, &other.rows)
            && self.generation == other.generation
            && self.drafts_revision == other.drafts_revision
            && self.wrap_width == other.wrap_width
            && self.rem == other.rem
            && self.font_size == other.font_size
            && self.line_height == other.line_height
            && std::ptr::eq(self.language, other.language)
    }
}

impl RowMetrics {
    /// Heights of every row for the given text-column width. Scrolling re-renders the grid
    /// without changing any of their inputs, so the last result is reused.
    pub fn row_sizes(
        &mut self,
        rows: &Rc<[TranscriptRowView]>,
        doc: &EditorDoc,
        text_column_width: Pixels,
        font_family: &SharedString,
        window: &Window,
    ) -> Rc<Vec<Size<Pixels>>> {
        if &self.font_family != font_family {
            self.text.clear();
            self.aux.clear();
            self.sizes = None;
            self.font_family = font_family.clone();
        }
        let metrics = TextMetrics::for_window(window);
        let wrap_width = text_wrap_width(text_column_width, metrics);
        let key = SizesKey {
            rows: Rc::clone(rows),
            generation: doc.generation(),
            drafts_revision: doc.drafts().revision(),
            wrap_width,
            rem: metrics.rem,
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            language: studio_editor::i18n::localized("en", "ru"),
        };
        if let Some((cached, sizes)) = &self.sizes
            && cached.matches(&key)
        {
            return Rc::clone(sizes);
        }
        let font = font(font_family.clone());
        let shaper = Shaper {
            font: &font,
            metrics,
            window,
        };

        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = self.row_height(row, doc, wrap_width, &shaper);
                    size(px(0.), height)
                })
                .collect(),
        );
        if self.text.len() > rows.len() || self.aux.len() > rows.len() {
            let ids: HashSet<&str> = rows.iter().map(|row| row.segment_id.as_str()).collect();
            self.text.retain(|id, _| ids.contains(id.as_str()));
            self.aux.retain(|id, _| ids.contains(id.as_str()));
        }
        self.sizes = Some((key, Rc::clone(&sizes)));
        sizes
    }

    fn row_height(
        &mut self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        wrap_width: Pixels,
        shaper: &Shaper,
    ) -> Pixels {
        let metrics = shaper.metrics;
        let line_height = metrics.line_height;
        let draft = doc.draft(&row.segment_id);
        let text_lines = self
            .cached_wrap(&row.segment_id, shown_text(row, draft), wrap_width, shaper)
            .line_count();
        let mut text_block = line_height * text_lines as f32 + TEXT_PY * 2.;

        if row.row_kind == TranscriptRowKind::Annotation && has_review_chips(row) {
            text_block += metrics.px(CHIP_ROW_H);
        }
        if row.diff.is_some() {
            let diff_text = diff_display_text(row);
            let diff_width = (wrap_width - metrics.px(DIFF_BADGE_W)).max(px(40.));
            let diff_lines = match self.aux.get(&row.segment_id) {
                Some((cached, lines))
                    if cached.matches(&diff_text, diff_width, metrics.font_size) =>
                {
                    *lines
                }
                _ => {
                    let lines = shaper.lines(&diff_text, diff_width);
                    let key = CacheKey::new(&diff_text, diff_width, metrics.font_size);
                    self.aux.insert(row.segment_id.clone(), (key, lines));
                    lines
                }
            };
            let diff_block = line_height * diff_lines as f32;
            text_block = if row.row_kind == TranscriptRowKind::Annotation {
                text_block + metrics.px(DIFF_GAP) + diff_block
            } else {
                diff_block + TEXT_PY * 2.
            };
        }

        let small_line = metrics.px(SMALL_LINE_H);
        let mut track_block = line_height;
        if !row.annotation_kind.is_transcription() {
            track_block += small_line;
            if row.annotation_diff.is_some() {
                track_block += small_line;
            }
        }

        let time_block = draft
            .filter(|draft| {
                draft.time_error(TimeField::Start).is_some()
                    || draft.time_error(TimeField::End).is_some()
            })
            .map_or(line_height, |_| line_height + small_line);

        text_block.max(track_block).max(time_block) + metrics.px(ROW_PY) * 2. + px(1.)
    }

    /// The wrap of `row`'s text at the column width (as measured by [`Self::row_sizes`]),
    /// which its static text paints.
    pub fn text_wrap(
        &mut self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_column_width: Pixels,
        window: &Window,
    ) -> Rc<WrappedText> {
        let metrics = TextMetrics::for_window(window);
        let font = font(self.font_family.clone());
        let shaper = Shaper {
            font: &font,
            metrics,
            window,
        };
        self.cached_wrap(
            &row.segment_id,
            display_text(row, doc),
            text_wrap_width(text_column_width, metrics),
            &shaper,
        )
    }

    /// Vertical centre of the wrapped line holding `char_offset` in `row`'s text, measured
    /// from the row's top.
    pub fn caret_line_center(
        &mut self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        char_offset: usize,
        text_column_width: Pixels,
        window: &Window,
    ) -> Pixels {
        let metrics = TextMetrics::for_window(window);
        let byte = byte_offset(display_text(row, doc), char_offset);
        let line = self
            .text_wrap(row, doc, text_column_width, window)
            .line_of_byte(byte);
        metrics.px(ROW_PY) + TEXT_PY + metrics.line_height * (line as f32 + 0.5)
    }

    fn cached_wrap(
        &mut self,
        segment_id: &str,
        text: &str,
        wrap_width: Pixels,
        shaper: &Shaper,
    ) -> Rc<WrappedText> {
        let font_size = shaper.metrics.font_size;
        if let Some((cached, wrap)) = self.text.get(segment_id)
            && cached.matches(text, wrap_width, font_size)
        {
            return Rc::clone(wrap);
        }
        let wrap = Rc::new(shaper.wrap(text, wrap_width));
        let key = CacheKey::new(text, wrap_width, font_size);
        self.text
            .insert(segment_id.to_string(), (key, Rc::clone(&wrap)));
        wrap
    }
}

/// Text shown in a row's text cell: the draft for editable rows, else the row text.
pub(super) fn display_text<'a>(row: &'a TranscriptRowView, doc: &'a EditorDoc) -> &'a str {
    shown_text(row, doc.draft(&row.segment_id))
}

/// [`display_text`] for a row whose draft was already looked up.
fn shown_text<'a>(row: &'a TranscriptRowView, draft: Option<&'a SegmentDraft>) -> &'a str {
    match draft {
        Some(draft) if row.row_kind == TranscriptRowKind::Annotation => draft.text(),
        _ => &row.text,
    }
}

/// Unacknowledged low-confidence words get a review chip strip under the text.
pub(super) fn has_review_chips(row: &TranscriptRowView) -> bool {
    !row.is_read_only
        && row
            .word_views
            .iter()
            .any(|word| word.is_low_confidence && !word.is_acknowledged && word.command.is_some())
}

/// Plain text of a row's word diff (what the diff cell paints, minus styling).
pub(super) fn diff_display_text(row: &TranscriptRowView) -> String {
    let Some(diff) = &row.diff else {
        return String::new();
    };
    if diff.words.is_empty() {
        return if diff.kind == studio_editor::transcript_model::TranscriptDiffCellKind::New {
            row.text.clone()
        } else {
            studio_editor::i18n::localized("No content", "Пусто").to_string()
        };
    }
    let mut text = String::new();
    for (index, word) in diff.words.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(&word.value);
    }
    text
}

struct Shaper<'a> {
    font: &'a Font,
    metrics: TextMetrics,
    window: &'a Window,
}

impl Shaper<'_> {
    fn run(&self, len: usize) -> TextRun {
        TextRun {
            len,
            font: self.font.clone(),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    /// Row text wrapped like the row editor wraps it (shaped single-line widths).
    fn wrap(&self, text: &str, wrap_width: Pixels) -> WrappedText {
        let text_system = self.window.text_system();
        WrappedText::new(text, wrap_width, |line| {
            text_system
                .layout_line(line, self.metrics.font_size, &[self.run(line.len())], None)
                .width
        })
    }

    /// Line count of gpui's own wrapping of `text` (what a wrapping text element paints).
    fn lines(&self, text: &str, wrap_width: Pixels) -> usize {
        if text.is_empty() {
            return 1;
        }
        self.window
            .text_system()
            .shape_text(
                SharedString::from(text.to_string()),
                self.metrics.font_size,
                &[self.run(text.len())],
                Some(wrap_width),
                None,
            )
            .map_or(1, |lines| {
                lines
                    .iter()
                    .map(|line| line.wrap_boundaries().len() + 1)
                    .sum::<usize>()
                    .max(1)
            })
    }
}

/// Width the row text wraps at for a text column of `text_column_width`: the input's text
/// area (the column minus its paddings) minus its wrap margin.
fn text_wrap_width(text_column_width: Pixels, metrics: TextMetrics) -> Pixels {
    let column = if text_column_width > px(0.) {
        text_column_width
    } else {
        metrics.px(DEFAULT_TEXT_W)
    };
    column - TEXT_PX * 2. - WRAP_MARGIN
}
