//! Box styles of the transcript row cells, shared by the element rows ([`super::row`]) and
//! the painted fast rows ([`super::fast_row`]): both lay out and paint from these exact
//! styles, so a row looks the same whichever way it is drawn.
//!
//! Each function applies the cell's styles to any [`Styled`] value: a `div()` for an element
//! row, a [`StyleRefinement`] for a fast row.

use gpui_kit::component::Theme;
use gpui_kit::*;

use super::layout::{
    CELL_PX, ID_W, LINT_W, REVIEWER_W, ROW_PY, TEXT_PX, TEXT_PY, TIME_W, TRACK_W, TextMetrics,
    WRAP_MARGIN,
};

/// The row box (`h_flex`, top-aligned cells). Its hover style is [`row_hover`].
pub(super) fn row_box<S: Styled>(s: S, height: Pixels, background: Hsla, theme: &Theme) -> S {
    s.flex()
        .flex_row()
        .items_center()
        .relative()
        .w_full()
        .h(height)
        .items_start()
        .py(ROW_PY)
        .overflow_hidden()
        .border_b_1()
        .border_color(theme.border)
        .bg(background)
}

/// Hover style of a row that is not highlighted.
pub(super) fn row_hover(theme: &Theme) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let color = theme.list_hover;
    move |style| style.bg(color)
}

/// The accent bar on the left edge of the highlighted row.
pub(super) fn row_bar<S: Styled>(s: S, theme: &Theme) -> S {
    s.absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(px(2.))
        .bg(theme.primary)
}

pub(super) fn id_box<S: Styled>(s: S, metrics: TextMetrics, theme: &Theme) -> S {
    s.w(ID_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(theme.muted_foreground)
}

/// The speaker cell (`v_flex`) around the speaker label and its secondary lines.
pub(super) fn speaker_box<S: Styled>(s: S) -> S {
    s.flex()
        .flex_col()
        .w(TRACK_W)
        .flex_none()
        .px(rems(CELL_PX.0 - 0.25))
}

/// The pressable speaker label (`h_flex`: colour dot and name). Interactive labels add
/// [`speaker_label_interactive`].
pub(super) fn speaker_label_box<S: Styled>(s: S, metrics: TextMetrics, theme: &Theme) -> S {
    s.flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(metrics.line_height)
        .gap_1p5()
        .items_center()
        .px_1()
        .rounded(theme.radius)
}

pub(super) fn speaker_label_interactive<S: Styled>(s: S) -> S {
    s.cursor_pointer()
}

/// Hover style of an interactive speaker label (and its style while its menu is open).
pub(super) fn speaker_label_hover(
    theme: &Theme,
) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let color = theme.secondary;
    move |style| style.bg(color)
}

pub(super) fn speaker_dot<S: Styled>(s: S, color: Hsla) -> S {
    s.flex_none().size(rems(0.5)).rounded_full().bg(color)
}

pub(super) fn speaker_name_box<S: Styled>(s: S, metrics: TextMetrics, color: Hsla) -> S {
    s.min_w_0()
        .text_sm()
        .line_height(metrics.line_height)
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .truncate()
}

/// A start/end time cell (`v_flex`) around the time value.
pub(super) fn time_box<S: Styled>(s: S) -> S {
    s.flex()
        .flex_col()
        .w(TIME_W)
        .flex_none()
        .px(rems(CELL_PX.0 - 0.25))
}

/// The time value: the time input's box (line-height tall, 1px border, 4px padding, centred
/// text), so the value does not move when the cell opens for editing. Editable values add
/// [`time_value_interactive`].
pub(super) fn time_value_box<S: Styled>(s: S, metrics: TextMetrics, theme: &Theme) -> S {
    s.w_full()
        .h(metrics.line_height)
        .flex()
        .items_center()
        .px_1()
        .border_1()
        .border_color(transparent_black())
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(theme.foreground.opacity(0.85))
}

pub(super) fn time_value_interactive<S: Styled>(s: S, theme: &Theme) -> S {
    s.cursor_text().rounded(theme.radius)
}

pub(super) fn time_value_hover(theme: &Theme) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let color = theme.secondary;
    move |style| style.bg(color)
}

pub(super) fn reviewer_box<S: Styled>(s: S, metrics: TextMetrics, color: Hsla) -> S {
    s.w(REVIEWER_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(color)
        .truncate()
}

/// The text cell (`v_flex`) around the row text and its chips.
pub(super) fn text_cell_box<S: Styled>(s: S) -> S {
    s.flex().flex_col().relative().flex_1().min_w_0()
}

/// The padded box of the static row text. Editable rows add [`static_text_interactive`].
pub(super) fn static_text_box<S: Styled>(s: S, metrics: TextMetrics, theme: &Theme) -> S {
    s.w_full()
        .pl(TEXT_PX)
        .pr(TEXT_PX + WRAP_MARGIN)
        .py(TEXT_PY)
        .text_sm()
        .line_height(metrics.line_height)
        .text_color(theme.foreground)
}

pub(super) fn static_text_interactive<S: Styled>(s: S) -> S {
    s.cursor_text()
}

/// The box right around the row's wrapped text (the ghost caret paints over it).
pub(super) fn static_text_body<S: Styled>(s: S) -> S {
    s.relative().whitespace_nowrap()
}

/// The lint cell of a row without a diagnostic.
pub(super) fn empty_lint_box<S: Styled>(s: S) -> S {
    s.w(LINT_W).flex_none()
}

/// The lint (with a diagnostic) and actions cells: a fixed-width box centring its content.
pub(super) fn centered_cell_box<S: Styled>(s: S, width: Rems, metrics: TextMetrics) -> S {
    s.flex()
        .flex_row()
        .items_center()
        .w(width)
        .flex_none()
        .h(metrics.line_height + TEXT_PY * 2.)
        .justify_center()
}
