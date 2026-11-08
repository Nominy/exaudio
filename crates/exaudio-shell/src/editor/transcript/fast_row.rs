//! Painted transcript rows.
//!
//! An element row ([`super::row`]) is a tree of a few dozen divs and texts that gpui lays out,
//! prepaints and paints again on every frame the grid draws, scrolling included. A plain row
//! (no editor, menu, review chips, diff, decorations or secondary lines) is drawn by
//! [`FastRow`] instead: one layout node, cell boxes placed from a [`Template`] measured once per
//! row shape, texts shaped once per content.
//!
//! It draws exactly what the element row draws:
//! - cell boxes come from the same styles ([`super::row_style`]) laid out by gpui's own layout
//!   engine, so they land on the same device pixels;
//! - backgrounds, borders and clips go through [`Style::paint`] and [`Style::overflow_mask`];
//! - texts are shaped with the runs the text elements would build and painted line by line
//!   like [`TextLayout`] paints them;
//! - hit boxes, hover styles, cursors and mouse handlers are registered in the element row's
//!   order, with its event semantics (press, click, propagation);
//! - the lint and actions cells are the element row's own elements.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::*;
use studio_editor::edit::EditorDoc;
use studio_editor::edit::draft::byte_offset;
use studio_editor::edit::time::TimeField;
use studio_editor::transcript_model::{
    TranscriptRowKind, TranscriptRowView, TranscriptRowVisibleCells,
};

use super::TranscriptView;
use super::layout::{
    CELL_PX, ID_W, REVIEWER_W, TIME_W, TRACK_W, TextMetrics, display_text, has_review_chips,
};
use super::row::{reviewer_color, row_background, speaker_dot_color, time_value};
use super::row_style::{
    centered_cell_box, empty_lint_box, id_box, reviewer_box, row_bar, row_box, row_hover,
    speaker_box, speaker_dot, speaker_label_box, speaker_label_hover, speaker_label_interactive,
    speaker_name_box, static_text_body, static_text_box, static_text_interactive, text_cell_box,
    time_box, time_value_box, time_value_hover, time_value_interactive,
};
use super::wrap::WrappedText;

/// Shaped texts and measured row shapes, kept across frames.
#[derive(Default)]
pub(super) struct FastRowCache {
    texts: HashMap<(SharedString, TextSlot), (TextKey, Rc<ShapedText>)>,
    templates: HashMap<TemplateKey, Rc<Template>>,
}

impl FastRowCache {
    /// Drops what belongs to rows no longer in the grid.
    pub(super) fn retain_rows(&mut self, rows: &[TranscriptRowView]) {
        let ids: HashSet<&str> = rows.iter().map(|row| row.segment_id.as_str()).collect();
        self.texts.retain(|(id, _), _| ids.contains(id.as_ref()));
        // Row shapes depend on the window (width, zoom), not on rows; a handful exist at once.
        if self.templates.len() > 64 {
            self.templates.clear();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TextSlot {
    Id,
    Speaker,
    Start,
    End,
    Reviewer,
    Body,
}

/// What a shaped text depends on. The window's fonts are fixed after start-up (the app never
/// adds fonts at run time), so equal keys shape to equal lines.
#[derive(PartialEq)]
struct TextKey {
    text: SharedString,
    run: TextRun,
    font_size: Pixels,
}

/// A text shaped like a text element shapes it (no wrapping: every fast-row text fits its
/// box, and the row text carries its wraps as line breaks).
struct ShapedText {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    align: TextAlign,
    /// Width of the widest line, as a text element measures it (rounded up).
    width: Pixels,
}

impl ShapedText {
    fn height(&self) -> Pixels {
        self.lines
            .iter()
            .map(|line| line.size(self.line_height).height)
            .fold(Pixels::ZERO, |sum, height| sum + height)
    }

    /// Paints the lines from `bounds.origin` like `TextLayout` paints them.
    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let mut origin = bounds.origin;
        for line in &self.lines {
            line.paint_background(
                origin,
                self.line_height,
                self.align,
                Some(bounds),
                window,
                cx,
            )
            .ok();
            line.paint(
                origin,
                self.line_height,
                self.align,
                Some(bounds),
                window,
                cx,
            )
            .ok();
            origin.y += line.size(self.line_height).height;
        }
    }

    /// `TextLayout::index_for_position` for the text painted at `origin`.
    fn index_for_position(&self, origin: Point<Pixels>, position: Point<Pixels>) -> usize {
        if position.y < origin.y {
            return 0;
        }
        let mut line_origin = origin;
        let mut line_start: usize = 0;
        for line in &self.lines {
            let line_bottom = line_origin.y + line.size(self.line_height).height;
            if position.y > line_bottom {
                line_origin.y = line_bottom;
                line_start += line.len() + 1;
            } else {
                let within = position - line_origin;
                return match line.index_for_position(within, self.line_height) {
                    Ok(index) | Err(index) => line_start + index,
                };
            }
        }
        line_start.saturating_sub(1)
    }

    /// `TextLayout::position_for_index` for the text painted at `origin`.
    fn position_for_index(&self, origin: Point<Pixels>, index: usize) -> Option<Point<Pixels>> {
        let mut line_origin = origin;
        let mut line_start: usize = 0;
        for line in &self.lines {
            let line_end = line_start + line.len();
            if index < line_start {
                break;
            } else if index > line_end {
                line_origin.y += line.size(self.line_height).height;
                line_start = line_end + 1;
            } else {
                return Some(
                    line_origin + line.position_for_index(index - line_start, self.line_height)?,
                );
            }
        }
        None
    }
}

/// Text style of a text element nested in boxes with these styles.
fn nested_text_style(window: &Window, boxes: &[&Style]) -> TextStyle {
    let mut style = window.text_style();
    for node in boxes {
        if let Some(text) = node.text_style() {
            style.refine(text);
        }
    }
    style
}

fn shape(
    cache: &mut FastRowCache,
    segment_id: &SharedString,
    slot: TextSlot,
    text: SharedString,
    text_style: &TextStyle,
    window: &Window,
) -> Option<Rc<ShapedText>> {
    let rem = window.rem_size();
    let font_size = text_style.font_size.to_pixels(rem);
    let key = TextKey {
        run: text_style.to_run(text.len()),
        text,
        font_size,
    };
    let slot_key = (segment_id.clone(), slot);
    if let Some((cached, shaped)) = cache.texts.get(&slot_key)
        && *cached == key
    {
        return Some(Rc::clone(shaped));
    }
    let lines = window
        .text_system()
        .shape_text(
            key.text.clone(),
            font_size,
            std::slice::from_ref(&key.run),
            None,
            None,
        )
        .ok()?
        .into_vec();
    let line_height = window.pixel_snap(text_style.line_height.to_pixels(font_size.into(), rem));
    let width = lines
        .iter()
        .map(|line| line.size(line_height).width)
        .fold(Pixels::ZERO, Pixels::max)
        .ceil();
    let shaped = Rc::new(ShapedText {
        lines,
        line_height,
        align: text_style.text_align,
        width,
    });
    cache.texts.insert(slot_key, (key, Rc::clone(&shaped)));
    Some(shaped)
}

fn style_of(refinement: StyleRefinement) -> Style {
    let mut style = Style::default();
    style.refine(&refinement);
    style
}

/// Device-pixel snapped width of `length` (the layout engine snaps every box length).
fn snap(length: Pixels, window: &Window) -> Pixels {
    window.pixel_snap(length)
}

/// Everything a row decides about its cells before layout.
struct RowStyles {
    row: Style,
    row_hover: Option<StyleRefinement>,
    bar: Option<Style>,
    id: Style,
    speaker: Style,
    speaker_label: Style,
    speaker_label_hover: Option<StyleRefinement>,
    dot: Style,
    name: Style,
    time: Style,
    time_value: Style,
    time_value_hover: Option<StyleRefinement>,
    reviewer: Style,
    text_cell: Style,
    static_text: Style,
    body: Style,
    lint: Style,
    actions: Style,
}

#[allow(clippy::too_many_arguments)]
fn row_styles(
    height: Pixels,
    background: Hsla,
    highlighted: bool,
    read_only: bool,
    editable: bool,
    lint_present: bool,
    dot_color: Hsla,
    lane_color: Hsla,
    reviewer_color: Hsla,
    metrics: TextMetrics,
    theme: &Theme,
) -> RowStyles {
    let none = StyleRefinement::default;
    let mut speaker_label = speaker_label_box(none(), metrics, theme);
    let mut time_value = time_value_box(none(), metrics, theme);
    if !read_only {
        speaker_label = speaker_label_interactive(speaker_label);
        time_value = time_value_interactive(time_value, theme);
    }
    let mut static_text = static_text_box(none(), metrics, theme);
    if editable {
        static_text = static_text_interactive(static_text);
    }
    RowStyles {
        row: style_of(row_box(none(), height, background, theme)),
        row_hover: (!highlighted).then(|| row_hover(theme)(none())),
        bar: highlighted.then(|| style_of(row_bar(none(), theme))),
        id: style_of(id_box(none(), metrics, theme)),
        speaker: style_of(speaker_box(none())),
        speaker_label: style_of(speaker_label),
        speaker_label_hover: (!read_only).then(|| speaker_label_hover(theme)(none())),
        dot: style_of(speaker_dot(none(), dot_color)),
        name: style_of(speaker_name_box(none(), metrics, lane_color)),
        time: style_of(time_box(none())),
        time_value: style_of(time_value),
        time_value_hover: (!read_only).then(|| time_value_hover(theme)(none())),
        reviewer: style_of(reviewer_box(none(), metrics, reviewer_color)),
        text_cell: style_of(text_cell_box(none())),
        static_text: style_of(static_text),
        body: style_of(static_text_body(none())),
        lint: style_of(if lint_present {
            centered_cell_box(none(), super::layout::LINT_W, metrics)
        } else {
            empty_lint_box(none())
        }),
        actions: style_of(centered_cell_box(none(), super::layout::ACTIONS_W, metrics)),
    }
}

/// Row shape the cell boxes are laid out for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TemplateKey {
    width: u32,
    height: u32,
    rem: u32,
    scale: u32,
    line_height: u32,
    body_lines: usize,
    cells: [bool; 8],
    highlighted: bool,
    has_reviewer: bool,
    lint_present: bool,
}

/// Cell boxes of a row shape, relative to the row's origin.
#[derive(Default)]
struct Template {
    bar: Bounds<Pixels>,
    id: Bounds<Pixels>,
    id_text: Bounds<Pixels>,
    speaker_label: Bounds<Pixels>,
    dot: Bounds<Pixels>,
    name: Bounds<Pixels>,
    name_text: Bounds<Pixels>,
    start_value: Bounds<Pixels>,
    start_text: Bounds<Pixels>,
    end_value: Bounds<Pixels>,
    end_text: Bounds<Pixels>,
    reviewer: Bounds<Pixels>,
    reviewer_text: Bounds<Pixels>,
    static_text: Bounds<Pixels>,
    body: Bounds<Pixels>,
    lint: Bounds<Pixels>,
    actions: Bounds<Pixels>,
}

/// Width the speaker name box takes when its text is wider than the label: the template
/// measures it with an over-wide text, so the name box shrinks to everything left beside the
/// dot. Wider than any label, small enough to keep the layout arithmetic exact in `f32`.
const WIDEST_NAME: Pixels = px(1.0e4);

/// What a row shape is laid out from.
struct TemplateInput<'a> {
    styles: &'a RowStyles,
    cells: TranscriptRowVisibleCells,
    text_line_height: Pixels,
    body_lines: usize,
    has_reviewer: bool,
}

/// Lays the cell boxes of a row shape out with the element row's styles in gpui's layout
/// engine. Box positions inside a row do not depend on the row's texts (each text sits at
/// the start of its box; only the speaker name's width does, see [`WIDEST_NAME`]).
fn measure_template(
    row_size: Size<Pixels>,
    input: TemplateInput,
    window: &mut Window,
    cx: &mut App,
) -> Template {
    let TemplateInput {
        styles,
        cells,
        text_line_height,
        body_lines,
        has_reviewer,
    } = input;
    let line = move |width: Pixels, height: Pixels| {
        move |_: Size<Option<Pixels>>, _: Size<AvailableSpace>, _: &mut Window, _: &mut App| {
            size(width, height)
        }
    };
    let leaf = |window: &mut Window, width: Pixels, height: Pixels| {
        window.request_measured_layout(Style::default(), line(width, height))
    };
    let node = |window: &mut Window, cx: &mut App, style: &Style, children: &[LayoutId]| {
        window.request_layout(style.clone(), children.iter().copied(), cx)
    };

    let mut children = Vec::new();
    let bar = styles.bar.as_ref().map(|bar| {
        let id = node(window, cx, bar, &[]);
        children.push(id);
        id
    });
    let id = cells.id.then(|| {
        let text = leaf(window, Pixels::ZERO, text_line_height);
        let cell = node(window, cx, &styles.id, &[text]);
        children.push(cell);
        (cell, text)
    });
    let speaker = cells.track.then(|| {
        let dot = node(window, cx, &styles.dot, &[]);
        let text = leaf(window, WIDEST_NAME, text_line_height);
        let name = node(window, cx, &styles.name, &[text]);
        let label = node(window, cx, &styles.speaker_label, &[dot, name]);
        children.push(node(window, cx, &styles.speaker, &[label]));
        (label, dot, name, text)
    });
    let time_cell = |window: &mut Window, cx: &mut App, children: &mut Vec<LayoutId>| {
        let text = leaf(window, Pixels::ZERO, text_line_height);
        let value = node(window, cx, &styles.time_value, &[text]);
        children.push(node(window, cx, &styles.time, &[value]));
        (value, text)
    };
    let start = cells.start.then(|| time_cell(window, cx, &mut children));
    let end = cells.end.then(|| time_cell(window, cx, &mut children));
    let reviewer = cells.reviewer.then(|| {
        let text = has_reviewer.then(|| leaf(window, Pixels::ZERO, text_line_height));
        let cell = node(window, cx, &styles.reviewer, text.as_slice());
        children.push(cell);
        (cell, text)
    });
    let text = cells.text.then(|| {
        let text = leaf(window, Pixels::ZERO, text_line_height * body_lines as f32);
        let body = node(window, cx, &styles.body, &[text]);
        let static_text = node(window, cx, &styles.static_text, &[body]);
        children.push(node(window, cx, &styles.text_cell, &[static_text]));
        (static_text, body)
    });
    let lint = cells.linter.then(|| {
        let cell = node(window, cx, &styles.lint, &[]);
        children.push(cell);
        cell
    });
    let actions = cells.actions.then(|| {
        let cell = node(window, cx, &styles.actions, &[]);
        children.push(cell);
        cell
    });
    let root = node(window, cx, &styles.row, &children);

    window.with_absolute_element_offset(Point::default(), |window| {
        window.compute_layout(root, row_size.map(AvailableSpace::Definite), cx);
        let mut bounds = |id: LayoutId| window.layout_bounds(id);
        let mut template = Template::default();
        if let Some(bar) = bar {
            template.bar = bounds(bar);
        }
        if let Some((cell, text)) = id {
            template.id = bounds(cell);
            template.id_text = bounds(text);
        }
        if let Some((label, dot, name, text)) = speaker {
            template.speaker_label = bounds(label);
            template.dot = bounds(dot);
            template.name = bounds(name);
            template.name_text = bounds(text);
        }
        if let Some((value, text)) = start {
            template.start_value = bounds(value);
            template.start_text = bounds(text);
        }
        if let Some((value, text)) = end {
            template.end_value = bounds(value);
            template.end_text = bounds(text);
        }
        if let Some((cell, text)) = reviewer {
            template.reviewer = bounds(cell);
            if let Some(text) = text {
                template.reviewer_text = bounds(text);
            }
        }
        if let Some((static_text, body)) = text {
            template.static_text = bounds(static_text);
            template.body = bounds(body);
        }
        if let Some(lint) = lint {
            template.lint = bounds(lint);
        }
        if let Some(actions) = actions {
            template.actions = bounds(actions);
        }
        template
    })
}

/// Inputs gathered for one row by [`fast_row`].
struct RowTexts {
    id: Option<Rc<ShapedText>>,
    speaker: Option<Rc<ShapedText>>,
    start: Option<Rc<ShapedText>>,
    end: Option<Rc<ShapedText>>,
    reviewer: Option<Rc<ShapedText>>,
    body: Option<Rc<ShapedText>>,
}

/// A transcript row painted without its element tree. See the module docs.
pub(super) struct FastRow {
    id: ElementId,
    view: WeakEntity<TranscriptView>,
    cache: Rc<RefCell<FastRowCache>>,
    segment_id: SharedString,
    cells: TranscriptRowVisibleCells,
    styles: RowStyles,
    texts: RowTexts,
    text_wrap: Rc<WrappedText>,
    /// Ghost caret: display index into the row text, colour and height.
    ghost: Option<(usize, Hsla, Pixels)>,
    highlighted: bool,
    has_reviewer: bool,
    lint_present: bool,
    read_only: bool,
    editable: bool,
    lint: Option<AnyElement>,
    actions: Option<AnyElement>,
}

/// Parameters of [`fast_row`].
pub(super) struct FastRowParams<'a> {
    pub row: &'a TranscriptRowView,
    pub doc: &'a EditorDoc,
    pub text_wrap: Rc<WrappedText>,
    pub height: Pixels,
    pub metrics: TextMetrics,
    pub lane_color: Hsla,
    pub is_find_match: bool,
    pub is_playing: bool,
    /// The row's text editor, time editor, speaker menu or speaker rename is mounted.
    pub has_open_editor: bool,
}

/// The painted row for `row` when it shows nothing beyond what [`FastRow`] draws, else `None`
/// (the caller renders the element row).
pub(super) fn fast_row(
    params: FastRowParams,
    cache: &Rc<RefCell<FastRowCache>>,
    view: &TranscriptView,
    window: &mut Window,
    cx: &Context<TranscriptView>,
) -> Option<FastRow> {
    let FastRowParams {
        row,
        doc,
        text_wrap,
        height,
        metrics,
        lane_color,
        is_find_match,
        is_playing,
        has_open_editor,
    } = params;
    let theme = cx.theme();
    let cells = row.visible_cells;
    let draft = doc.draft(&row.segment_id);
    let has_time_error = draft.is_some_and(|draft| {
        draft.time_error(TimeField::Start).is_some() || draft.time_error(TimeField::End).is_some()
    });
    let reviewing = !row.is_read_only && !row.word_views.is_empty();
    let text = display_text(row, doc);
    if row.row_kind != TranscriptRowKind::Annotation
        || has_open_editor
        || reviewing
        || has_review_chips(row)
        || row.diff.is_some()
        || row.annotation_diff.is_some()
        || !row.annotation_kind.is_transcription()
        || has_time_error
        || (cells.text && text.is_empty())
        || !crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx).is_empty()
    {
        return None;
    }

    let (highlighted, background) = row_background(row, doc, is_playing, is_find_match, theme);
    let read_only = row.is_read_only;
    let editable = !row.is_read_only && !doc.is_read_only();
    let lint_present = cells.linter && doc.row_lint(row).is_some();
    let styles = row_styles(
        height,
        background,
        highlighted,
        read_only,
        editable,
        lint_present,
        speaker_dot_color(row, doc, theme),
        lane_color,
        reviewer_color(row, theme),
        metrics,
        theme,
    );

    let segment_id: SharedString = row.segment_id.clone().into();
    let rem = window.rem_size();
    let mut cache_ref = cache.borrow_mut();
    let cache_mut = &mut *cache_ref;
    // A text that would not fit its box wraps (id, times) or truncates (names) in the element
    // row; those rows are not drawn here. The fit check keeps a pixel of slack so rounding can
    // never decide it.
    let fits = |shaped: &ShapedText, available: Pixels| {
        shaped.lines.len() == 1 && shaped.width + px(1.) <= available
    };
    let content = |width: Rems, padding: Pixels| {
        snap(width.to_pixels(rem), window) - snap(padding, window) * 2.
    };
    let cell_px = CELL_PX.to_pixels(rem);
    let inner_px = rems(CELL_PX.0 - 0.25).to_pixels(rem);
    let mut shape_in = |slot, text: SharedString, boxes: &[&Style]| {
        let text_style = nested_text_style(window, boxes);
        shape(cache_mut, &segment_id, slot, text, &text_style, window)
            .filter(|shaped| shaped.align == TextAlign::Left)
    };

    let id = match cells.id {
        true => {
            let shaped = shape_in(TextSlot::Id, row.display_id.clone().into(), &[&styles.id])?;
            Some(fits(&shaped, content(ID_W, cell_px)).then_some(shaped)?)
        }
        false => None,
    };
    let speaker = match cells.track {
        true => {
            let shaped = shape_in(
                TextSlot::Speaker,
                row.speaker_label.clone().into(),
                &[&styles.speaker, &styles.speaker_label, &styles.name],
            )?;
            let label_px = rems(0.25).to_pixels(rem);
            let available = content(TRACK_W, inner_px)
                - snap(label_px, window) * 2.
                - snap(rems(0.5).to_pixels(rem), window)
                - snap(rems(0.375).to_pixels(rem), window);
            Some(fits(&shaped, available).then_some(shaped)?)
        }
        false => None,
    };
    let time_available = content(TIME_W, inner_px)
        - snap(rems(0.25).to_pixels(rem), window) * 2.
        - snap(px(1.), window) * 2.;
    let mut time = |slot, field| {
        let shaped = shape_in(
            slot,
            time_value(row, doc, field),
            &[&styles.time, &styles.time_value],
        )?;
        fits(&shaped, time_available).then_some(shaped)
    };
    let start = match cells.start {
        true => Some(time(TextSlot::Start, TimeField::Start)?),
        false => None,
    };
    let end = match cells.end {
        true => Some(time(TextSlot::End, TimeField::End)?),
        false => None,
    };
    let has_reviewer = row.reviewer.is_some();
    let reviewer = match (cells.reviewer, row.reviewer.as_ref()) {
        (true, Some(reviewer)) => {
            let shaped = shape_in(
                TextSlot::Reviewer,
                reviewer.label.clone().into(),
                &[&styles.reviewer],
            )?;
            Some(fits(&shaped, content(REVIEWER_W, cell_px)).then_some(shaped)?)
        }
        _ => None,
    };
    let body = match cells.text {
        true => Some(shape_in(
            TextSlot::Body,
            text_wrap.display_text(text).into(),
            &[&styles.text_cell, &styles.static_text, &styles.body],
        )?),
        false => None,
    };
    drop(cache_ref);

    let ghost = cells.text.then_some(()).and_then(|()| {
        doc.ghost_cursor(&row.segment_id).map(|ghost| {
            (
                text_wrap.to_display(byte_offset(text, ghost.char_offset)),
                theme.warning,
                metrics.line_height,
            )
        })
    });

    Some(FastRow {
        id: SharedString::from(format!("transcript-row:{}", row.segment_id)).into(),
        view: cx.entity().downgrade(),
        cache: Rc::clone(cache),
        segment_id,
        cells,
        styles,
        texts: RowTexts {
            id,
            speaker,
            start,
            end,
            reviewer,
            body,
        },
        text_wrap,
        ghost,
        highlighted,
        has_reviewer,
        lint_present,
        read_only,
        editable,
        lint: lint_present.then(|| view.lint_cell(row, doc, metrics, cx)),
        actions: cells.actions.then(|| view.actions_cell(row, metrics, cx)),
    })
}

/// Pending press of the speaker label (its click opens the speaker menu).
#[derive(Default)]
struct FastRowState {
    speaker_press: Rc<RefCell<Option<MouseDownEvent>>>,
}

/// Hit boxes and absolute cell bounds of the current frame.
pub(super) struct FastRowFrame {
    template: Rc<Template>,
    row: Hitbox,
    speaker_label: Option<Hitbox>,
    start_value: Option<Hitbox>,
    end_value: Option<Hitbox>,
    static_text: Option<Hitbox>,
    body: Option<Hitbox>,
}

impl IntoElement for FastRow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FastRow {
    type RequestLayoutState = ();
    type PrepaintState = Option<FastRowFrame>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(self.styles.row.clone(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let body_lines = self.texts.body.as_ref().map_or(1, |body| body.lines.len());
        let text_line_height = window.pixel_snap(self.styles.id_line_height(window));
        let key = TemplateKey {
            width: f32::from(bounds.size.width).to_bits(),
            height: f32::from(bounds.size.height).to_bits(),
            rem: f32::from(window.rem_size()).to_bits(),
            scale: window.scale_factor().to_bits(),
            line_height: f32::from(text_line_height).to_bits(),
            body_lines,
            cells: cells_key(self.cells),
            highlighted: self.highlighted,
            has_reviewer: self.has_reviewer,
            lint_present: self.lint_present,
        };
        let cached = self.cache.borrow().templates.get(&key).cloned();
        let template = match cached {
            Some(template) => template,
            None => {
                let template = Rc::new(measure_template(
                    bounds.size,
                    TemplateInput {
                        styles: &self.styles,
                        cells: self.cells,
                        text_line_height,
                        body_lines,
                        has_reviewer: self.has_reviewer,
                    },
                    window,
                    cx,
                ));
                self.cache
                    .borrow_mut()
                    .templates
                    .insert(key, Rc::clone(&template));
                template
            }
        };
        let at =
            |relative: Bounds<Pixels>| Bounds::new(bounds.origin + relative.origin, relative.size);
        let rem = window.rem_size();

        // The element row inserts each box's hit box inside that box's own clip, parents first.
        let row_style = &self.styles.row;
        window.with_content_mask(row_style.overflow_mask(bounds, rem), |window| {
            let row = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let speaker_label = (self.cells.track && !self.read_only)
                .then(|| window.insert_hitbox(at(template.speaker_label), HitboxBehavior::Normal));
            let start_value = (self.cells.start && !self.read_only)
                .then(|| window.insert_hitbox(at(template.start_value), HitboxBehavior::Normal));
            let end_value = (self.cells.end && !self.read_only)
                .then(|| window.insert_hitbox(at(template.end_value), HitboxBehavior::Normal));
            let static_text = (self.cells.text && self.editable)
                .then(|| window.insert_hitbox(at(template.static_text), HitboxBehavior::Normal));
            let body = self
                .cells
                .text
                .then(|| window.insert_hitbox(at(template.body), HitboxBehavior::Normal));
            if let Some(lint) = self.lint.as_mut() {
                let cell = at(template.lint);
                lint.layout_as_root(cell.size.map(AvailableSpace::Definite), window, cx);
                lint.prepaint_at(cell.origin, window, cx);
            }
            if let Some(actions) = self.actions.as_mut() {
                let cell = at(template.actions);
                actions.layout_as_root(cell.size.map(AvailableSpace::Definite), window, cx);
                actions.prepaint_at(cell.origin, window, cx);
            }
            Some(FastRowFrame {
                template,
                row,
                speaker_label,
                start_value,
                end_value,
                static_text,
                body,
            })
        })
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        frame: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(frame) = frame.take() else {
            return;
        };
        let Some(id) = id else {
            return;
        };
        let state = window.with_element_state::<FastRowState, _>(id, |state, _| {
            let state = state.unwrap_or_default();
            let press = Rc::clone(&state.speaker_press);
            (press, state)
        });
        let template = Rc::clone(&frame.template);
        let at =
            |relative: Bounds<Pixels>| Bounds::new(bounds.origin + relative.origin, relative.size);
        let view_id = window.current_view();
        let rem = window.rem_size();

        let row_style = interactive_style(
            &self.styles.row,
            self.styles.row_hover.as_ref(),
            Some(&frame.row),
            window,
            cx,
        );
        row_style.paint(bounds, window, cx, |window, cx| {
            window.with_content_mask(row_style.overflow_mask(bounds, rem), |window| {
                set_cursor(&row_style, &frame.row, window, cx);
                self.paint_row_listeners(&frame, view_id, window);

                if let Some(bar) = &self.styles.bar {
                    bar.paint(at(template.bar), window, cx, |_, _| {});
                }
                if let Some(text) = &self.texts.id {
                    paint_text_box(
                        &self.styles.id,
                        at(template.id),
                        text,
                        at(template.id_text),
                        window,
                        cx,
                    );
                }
                if self.cells.track {
                    self.paint_speaker(&frame, &template, bounds, &state, view_id, window, cx);
                }
                if let Some(text) = &self.texts.start {
                    self.paint_time(
                        TimeField::Start,
                        frame.start_value.as_ref(),
                        at(template.start_value),
                        text,
                        at(template.start_text),
                        view_id,
                        window,
                        cx,
                    );
                }
                if let Some(text) = &self.texts.end {
                    self.paint_time(
                        TimeField::End,
                        frame.end_value.as_ref(),
                        at(template.end_value),
                        text,
                        at(template.end_text),
                        view_id,
                        window,
                        cx,
                    );
                }
                if self.cells.reviewer {
                    let cell = at(template.reviewer);
                    let reviewer = &self.styles.reviewer;
                    reviewer.paint(cell, window, cx, |window, cx| {
                        window.with_content_mask(reviewer.overflow_mask(cell, rem), |window| {
                            if let Some(text) = &self.texts.reviewer {
                                text.paint(at(template.reviewer_text), window, cx);
                            }
                        });
                    });
                }
                if self.cells.text {
                    self.paint_text(&frame, &template, bounds, window, cx);
                }
                if let Some(lint) = self.lint.as_mut() {
                    lint.paint(window, cx);
                }
                if let Some(actions) = self.actions.as_mut() {
                    actions.paint(window, cx);
                }
            });
        });
    }
}

fn cells_key(cells: TranscriptRowVisibleCells) -> [bool; 8] {
    [
        cells.id,
        cells.track,
        cells.start,
        cells.end,
        cells.reviewer,
        cells.text,
        cells.linter,
        cells.actions,
    ]
}

impl RowStyles {
    /// Line height of the row's single-line texts (every cell sets the same one).
    fn id_line_height(&self, window: &Window) -> Pixels {
        let style = nested_text_style(window, &[&self.id]);
        let rem = window.rem_size();
        style
            .line_height
            .to_pixels(style.font_size.to_pixels(rem).into(), rem)
    }
}

/// The element's style with its hover style applied the way a div applies it.
fn interactive_style(
    base: &Style,
    hover: Option<&StyleRefinement>,
    hitbox: Option<&Hitbox>,
    window: &Window,
    cx: &App,
) -> Style {
    let mut style = base.clone();
    if let (Some(hover), Some(hitbox)) = (hover, hitbox)
        && !cx.has_active_drag()
        && !window.last_input_was_touch()
        && hitbox.is_hovered(window)
    {
        style.refine(hover);
    }
    style
}

/// The cursor a div with a hit box sets while painting.
fn set_cursor(style: &Style, hitbox: &Hitbox, window: &mut Window, cx: &App) {
    if cx.has_active_drag() {
        if let Some(cursor) = cx.active_drag_cursor_style() {
            window.set_window_cursor_style(cursor);
        }
    } else if let Some(cursor) = style.mouse_cursor {
        window.set_cursor_style(cursor, hitbox);
    }
}

/// Repaints the view when `hitbox` gains or loses the hover (a div with a hover style does).
fn notify_on_hover_change(hitbox: &Hitbox, view_id: EntityId, window: &mut Window) {
    let hitbox = hitbox.clone();
    let was_hovered = hitbox.is_hovered(window);
    window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture && hitbox.is_hovered(window) != was_hovered {
            cx.notify(view_id);
        }
    });
}

/// Paints a box holding one text (no hit box).
fn paint_text_box(
    style: &Style,
    cell: Bounds<Pixels>,
    text: &ShapedText,
    text_bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let rem = window.rem_size();
    style.paint(cell, window, cx, |window, cx| {
        window.with_content_mask(style.overflow_mask(cell, rem), |window| {
            text.paint(text_bounds, window, cx);
        });
    });
}

impl FastRow {
    fn paint_row_listeners(&self, frame: &FastRowFrame, view_id: EntityId, window: &mut Window) {
        let hitbox = frame.row.clone();
        let view = self.view.clone();
        let segment_id = self.segment_id.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && hitbox.is_hovered(window)
                && let Some(view) = view.upgrade()
            {
                view.update(cx, |this, cx| {
                    this.press_row(&segment_id, event.click_count, window, cx)
                });
            }
        });
        if self.styles.row_hover.is_some() {
            notify_on_hover_change(&frame.row, view_id, window);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_speaker(
        &self,
        frame: &FastRowFrame,
        template: &Template,
        row: Bounds<Pixels>,
        press: &Rc<RefCell<Option<MouseDownEvent>>>,
        view_id: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let at =
            |relative: Bounds<Pixels>| Bounds::new(row.origin + relative.origin, relative.size);
        let label_bounds = at(template.speaker_label);
        let label = interactive_style(
            &self.styles.speaker_label,
            self.styles.speaker_label_hover.as_ref(),
            frame.speaker_label.as_ref(),
            window,
            cx,
        );
        let rem = window.rem_size();
        // The speaker cell box paints nothing; its label does.
        label.paint(label_bounds, window, cx, |window, cx| {
            if let Some(hitbox) = &frame.speaker_label {
                set_cursor(&label, hitbox, window, cx);
                notify_on_hover_change(hitbox, view_id, window);
                self.paint_speaker_click(hitbox, press, window);
            }
            self.styles
                .dot
                .paint(at(template.dot), window, cx, |_, _| {});
            if let Some(text) = &self.texts.speaker {
                // The name box hugs its text (it fits: see `fast_row`).
                let mut name = at(template.name);
                let scale = window.scale_factor();
                let text_width = (text.width * scale).ceil() / scale;
                name.size.width = name.size.width.min(text_width);
                let style = &self.styles.name;
                style.paint(name, window, cx, |window, cx| {
                    window.with_content_mask(style.overflow_mask(name, rem), |window| {
                        let mut text_bounds = at(template.name_text);
                        text_bounds.size.width = name.size.width;
                        text.paint(text_bounds, window, cx);
                    });
                });
            }
        });
    }

    /// A div's click: press inside, release inside.
    fn paint_speaker_click(
        &self,
        hitbox: &Hitbox,
        press: &Rc<RefCell<Option<MouseDownEvent>>>,
        window: &mut Window,
    ) {
        {
            let press = Rc::clone(press);
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    *press.borrow_mut() = Some(event.clone());
                }
            });
        }
        let press = Rc::clone(press);
        let hitbox = hitbox.clone();
        let view = self.view.clone();
        let segment_id = self.segment_id.clone();
        let mut captured: Option<MouseDownEvent> = None;
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| match phase {
            DispatchPhase::Capture => {
                let mut press = press.borrow_mut();
                if press.is_some() && hitbox.is_hovered(window) {
                    captured = press.take();
                } else {
                    press.take();
                }
            }
            DispatchPhase::Bubble => {
                if captured
                    .take()
                    .is_some_and(|down| down.button == MouseButton::Left)
                    && let Some(view) = view.upgrade()
                {
                    view.update(cx, |this, cx| {
                        this.open_speaker_menu(&segment_id, window, cx)
                    });
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_time(
        &self,
        field: TimeField,
        hitbox: Option<&Hitbox>,
        value_bounds: Bounds<Pixels>,
        text: &ShapedText,
        text_bounds: Bounds<Pixels>,
        view_id: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = interactive_style(
            &self.styles.time_value,
            self.styles.time_value_hover.as_ref(),
            hitbox,
            window,
            cx,
        );
        let rem = window.rem_size();
        style.paint(value_bounds, window, cx, |window, cx| {
            window.with_content_mask(style.overflow_mask(value_bounds, rem), |window| {
                if let Some(hitbox) = hitbox {
                    set_cursor(&style, hitbox, window, cx);
                    let hitbox_for_press = hitbox.clone();
                    let view = self.view.clone();
                    let segment_id = self.segment_id.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.button == MouseButton::Left
                            && hitbox_for_press.is_hovered(window)
                        {
                            cx.stop_propagation();
                            if let Some(view) = view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.press_time_cell(
                                        &segment_id,
                                        field,
                                        event.click_count,
                                        window,
                                        cx,
                                    )
                                });
                            }
                        }
                    });
                    notify_on_hover_change(hitbox, view_id, window);
                }
                text.paint(text_bounds, window, cx);
            });
        });
    }

    fn paint_text(
        &self,
        frame: &FastRowFrame,
        template: &Template,
        row: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let at =
            |relative: Bounds<Pixels>| Bounds::new(row.origin + relative.origin, relative.size);
        let Some(text) = self.texts.body.clone() else {
            return;
        };
        let static_bounds = at(template.static_text);
        let body_bounds = at(template.body);
        let static_style = &self.styles.static_text;
        let rem = window.rem_size();
        // The text cell box paints nothing; the static text box sets the cursor and edits at
        // the end on a press beside the text.
        static_style.paint(static_bounds, window, cx, |window, cx| {
            window.with_content_mask(static_style.overflow_mask(static_bounds, rem), |window| {
                if let Some(hitbox) = &frame.static_text {
                    set_cursor(static_style, hitbox, window, cx);
                    let hitbox = hitbox.clone();
                    let view = self.view.clone();
                    let segment_id = self.segment_id.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.button == MouseButton::Left
                            && hitbox.is_hovered(window)
                        {
                            cx.stop_propagation();
                            if let Some(view) = view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.activate_text_at_end(&segment_id, window, cx)
                                });
                            }
                        }
                    });
                }
                let body_style = &self.styles.body;
                body_style.paint(body_bounds, window, cx, |window, cx| {
                    if let Some(hitbox) = &frame.body {
                        set_cursor(body_style, hitbox, window, cx);
                        let hitbox = hitbox.clone();
                        let view = self.view.clone();
                        let segment_id = self.segment_id.clone();
                        let text = Rc::clone(&text);
                        let text_wrap = Rc::clone(&self.text_wrap);
                        let origin = body_bounds.origin;
                        let editable = self.editable;
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble
                                || event.button != MouseButton::Left
                                || !hitbox.is_hovered(window)
                            {
                                return;
                            }
                            let byte = text_wrap
                                .to_source(text.index_for_position(origin, event.position));
                            let modifiers = event.modifiers;
                            // Helper word seek: a plain Alt+Click (no Ctrl/Shift/Meta).
                            let seek = modifiers.alt
                                && !modifiers.control
                                && !modifiers.shift
                                && !modifiers.platform;
                            if !seek && !editable {
                                return;
                            }
                            cx.stop_propagation();
                            if let Some(view) = view.upgrade() {
                                view.update(cx, |this, cx| {
                                    if seek {
                                        this.seek_to_word_at_byte(&segment_id, byte, cx);
                                    } else {
                                        this.activate_text_at_byte(&segment_id, byte, window, cx);
                                    }
                                });
                            }
                        });
                    }
                    let text_bounds = Bounds::new(
                        body_bounds.origin,
                        size(body_bounds.size.width, text.height()),
                    );
                    text.paint(text_bounds, window, cx);
                    if let Some((index, color, height)) = self.ghost
                        && let Some(origin) = text.position_for_index(body_bounds.origin, index)
                    {
                        window.paint_quad(fill(Bounds::new(origin, size(px(2.), height)), color));
                    }
                });
            });
        });
    }
}
