//! Rendering of one transcript row: id, track, times, extra, text, lint and actions cells,
//! plus static rows.

use std::ops::Range;
use std::rc::Rc;

use exaudio_editor::edit::EditorDoc;
use exaudio_editor::edit::draft::byte_offset;
use exaudio_editor::edit::time::TimeField;
use exaudio_editor::editor_model::EditorSession;
use exaudio_editor::timeline_model::LaneColor;
use exaudio_editor::transcript_model::{
    TranscriptActionCommand, TranscriptActionMenuEntry, TranscriptExtraColumn, TranscriptRowKind,
    TranscriptRowView, TranscriptStyledBody, TranscriptStyledText, TranscriptTone,
    TranscriptWordStyle,
};
use exaudio_extension::document::Severity;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable as _, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::TranscriptView;
use super::layout::{
    ACTIONS_W, BADGE_W, CELL_PX, LINT_W, ROW_PY, SMALL_LINE_H, TEXT_PX, TEXT_PY, TIME_W, TRACK_W,
    TextMetrics, WRAP_MARGIN, display_text, has_chips, has_word_marks,
};
use super::row_style::{
    centered_cell_box, empty_lint_box, extra_box, id_box, row_bar, row_box, row_hover, speaker_box,
    speaker_dot, speaker_label_box, speaker_label_hover, speaker_label_interactive,
    speaker_name_box, static_text_body, static_text_box, static_text_interactive, text_cell_box,
    time_box, time_value_box, time_value_hover, time_value_interactive,
};
use super::wrap::WrappedText;
use crate::actions;
use crate::screens::ui::localized;

/// Per-row inputs gathered by the view before rendering.
pub(super) struct RowParams<'a> {
    pub row: &'a TranscriptRowView,
    pub doc: &'a EditorDoc,
    /// The row text wrapped like its editor wraps it (static text paints these rows).
    pub text_wrap: Rc<WrappedText>,
    pub height: Pixels,
    pub metrics: TextMetrics,
    pub lane_color: Hsla,
    pub is_find_match: bool,
    pub is_playing: bool,
    /// The pooled text editor is mounted in this row.
    pub text_editing: bool,
    /// The pooled time editor is mounted in this row's start/end cell.
    pub time_editing: Option<TimeField>,
}

impl TranscriptView {
    /// The element row. Plain rows are drawn by [`super::fast_row::FastRow`] instead, which
    /// mirrors this tree for them: a change to a segment row's boxes goes through
    /// [`super::row_style`] (shared), and a new cell, state or interaction must be added to
    /// `FastRow` or to the conditions that leave a row to this element tree.
    pub(super) fn render_row(&self, params: RowParams, cx: &Context<Self>) -> AnyElement {
        match params.row.row_kind {
            TranscriptRowKind::Segment => self.render_segment_row(params, cx),
            TranscriptRowKind::Static(tone) => render_static_row(params, tone, cx),
        }
    }

    fn render_segment_row(&self, params: RowParams, cx: &Context<Self>) -> AnyElement {
        let RowParams {
            row,
            doc,
            text_wrap,
            height,
            metrics,
            lane_color,
            is_find_match,
            is_playing,
            text_editing,
            time_editing,
        } = params;
        let theme = cx.theme();
        let cells = row.visible_cells;
        let (highlighted, background) = row_background(row, doc, is_playing, is_find_match, theme);
        let segment_id: SharedString = row.segment_id.clone().into();

        row_box(
            div().id(SharedString::from(format!(
                "transcript-row:{}",
                row.segment_id
            ))),
            height,
            background,
            theme,
        )
        .when(!highlighted, |row| row.hover(row_hover(theme)))
        .when(highlighted, |row| row.child(row_bar(div(), theme)))
        .on_mouse_down(MouseButton::Left, {
            let segment_id = segment_id.clone();
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.press_row(&segment_id, event.click_count, window, cx);
            })
        })
        .when(cells.id, |el| el.child(id_cell(row, metrics, theme)))
        .when(cells.track, |el| {
            el.child(self.speaker_cell(row, doc, metrics, lane_color, cx))
        })
        .when(cells.start, |el| {
            el.child(self.time_cell(row, doc, TimeField::Start, time_editing, metrics, cx))
        })
        .when(cells.end, |el| {
            el.child(self.time_cell(row, doc, TimeField::End, time_editing, metrics, cx))
        })
        .when_some(row.extra_column.as_deref(), |el, column| {
            el.child(extra_cell(row, column, metrics, theme))
        })
        .when(cells.text, |el| {
            el.child(self.text_cell(row, doc, &text_wrap, metrics, text_editing, cx))
        })
        .when(cells.linter, |el| {
            el.child(self.lint_cell(row, doc, metrics, cx))
        })
        .when(cells.actions, |el| {
            el.child(self.actions_cell(row, metrics, cx))
        })
        .into_any_element()
    }

    /// Speaker cell: a dot in the speaker's colour and the speaker label (lane-coloured). The
    /// press selects the row like a click on its body; the click opens the speaker menu, and
    /// `Rename…` mounts the pooled name editor here.
    fn speaker_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        metrics: TextMetrics,
        lane_color: Hsla,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let project = doc.project();
        let menu_open = self.speaker_menu.as_deref() == Some(row.segment_id.as_str());
        let renaming = self
            .speaker_rename
            .as_ref()
            .is_some_and(|(id, _)| id.as_ref() == row.segment_id);
        let speaker_id = project
            .segment(&row.segment_id)
            .map(|segment| segment.speaker_id.as_str());
        let dot_color = speaker_dot_color(row, doc, theme);

        let content = if renaming {
            div()
                .w_full()
                .h(metrics.line_height)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Input::new(&self.speaker_input)
                        .xsmall()
                        .h(metrics.line_height),
                )
                .into_any_element()
        } else {
            let segment_id: SharedString = row.segment_id.clone().into();
            speaker_label_box(
                div().id(SharedString::from(format!(
                    "transcript-speaker:{}",
                    row.segment_id
                ))),
                metrics,
                theme,
            )
            .child(speaker_dot(div(), dot_color))
            .child(speaker_name_box(div(), metrics, lane_color).child(row.speaker_label.clone()))
            .when(!row.is_read_only, |label| {
                speaker_label_interactive(label)
                    .hover(speaker_label_hover(theme))
                    .when(menu_open, |label| label.bg(theme.secondary))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_speaker_menu(&segment_id, window, cx);
                    }))
            })
            .into_any_element()
        };

        speaker_box(div())
            .child(content)
            .when(!row.annotation_kind.is_transcription(), |cell| {
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(row.annotation_kind.label()),
                )
            })
            .when_some(row.speaker_note.as_ref(), |cell, note| {
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(tone_color(note.tone, theme))
                        .child(note.label.get().to_string()),
                )
            })
            .when(menu_open && !row.is_read_only, |cell| {
                cell.child(self.speaker_menu(row, doc, speaker_id, cx))
            })
            .into_any_element()
    }

    /// Menu under the speaker cell: every project speaker (check on the row's), `Rename…` for
    /// the row's speaker and, unless speakers are the audio tracks, `New speaker`.
    fn speaker_menu(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        current_speaker_id: Option<&str>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let segment_id: SharedString = row.segment_id.clone().into();
        let can_add = !doc.environment().policy.speakers_bound_to_lanes;
        let item = |id: SharedString| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .text_sm()
                .hover(|style| style.bg(theme.accent))
                .cursor_pointer()
        };

        let speakers = doc
            .project()
            .speakers
            .iter()
            .enumerate()
            .map(|(index, speaker)| {
                let is_current = current_speaker_id == Some(speaker.id.as_str());
                let speaker_id: SharedString = speaker.id.clone().into();
                let segment_id = segment_id.clone();
                item(SharedString::from(format!(
                    "transcript-speaker-option:{}:{}",
                    row.segment_id, speaker.id
                )))
                .flex()
                .items_center()
                .gap_2()
                .when(is_current, |item| item.bg(theme.list_active))
                .child(
                    div()
                        .flex_none()
                        .size(rems(0.5))
                        .rounded_full()
                        .bg(speaker_color(index, theme)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(speaker.label.clone()),
                )
                .child(div().flex_none().w(rems(1.)).when(is_current, |slot| {
                    slot.child(Icon::new(IconName::Check).xsmall())
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_speaker(&segment_id, &speaker_id, cx);
                }))
            })
            .collect::<Vec<_>>();

        let rename = current_speaker_id.map(|speaker_id| {
            let speaker_id = speaker_id.to_string();
            let segment_id = segment_id.clone();
            item(SharedString::from(format!(
                "transcript-speaker-rename:{}",
                row.segment_id
            )))
            .child(localized(cx, "Rename…", "Переименовать…"))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.begin_speaker_rename(&segment_id, &speaker_id, window, cx);
            }))
        });
        let add = can_add.then(|| {
            let segment_id = segment_id.clone();
            item(SharedString::from(format!(
                "transcript-speaker-add:{}",
                row.segment_id
            )))
            .child(localized(cx, "New speaker", "Новый спикер"))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.add_speaker(&segment_id, window, cx);
            }))
        });

        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                v_flex()
                    .id(SharedString::from(format!(
                        "transcript-speaker-menu:{}",
                        row.segment_id
                    )))
                    .occlude()
                    .mt_1()
                    .min_w(rems(11.))
                    .p_1()
                    .gap_0p5()
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .shadow_md()
                    .children(speakers)
                    .child(div().my_1().h(px(1.)).bg(theme.border))
                    .children(rename)
                    .children(add)
                    .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                        this.close_speaker_menu(cx);
                    })),
            ),
        )
        .with_priority(1)
    }

    fn time_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        field: TimeField,
        editing: Option<TimeField>,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let draft = doc.draft(&row.segment_id);
        let value = time_value(row, doc, field);
        let error = draft.and_then(|draft| draft.time_error(field));

        let content = if editing == Some(field) {
            div()
                .w_full()
                .h(metrics.line_height)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(&self.time_input).xsmall().h(metrics.line_height))
                .into_any_element()
        } else {
            let segment_id: SharedString = row.segment_id.clone().into();
            time_value_box(
                div().id(SharedString::from(format!(
                    "transcript-time:{}:{}",
                    row.segment_id,
                    field_key(field)
                ))),
                metrics,
                theme,
            )
            .child(value)
            .when(!row.is_read_only, |cell| {
                time_value_interactive(cell, theme)
                    .hover(time_value_hover(theme))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.press_time_cell(&segment_id, field, event.click_count, window, cx);
                        }),
                    )
            })
            .into_any_element()
        };

        time_box(div())
            .child(content)
            .when_some(error, |cell, error| {
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(theme.danger)
                        .truncate()
                        .child(error),
                )
            })
            .into_any_element()
    }

    fn text_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &Rc<WrappedText>,
        metrics: TextMetrics,
        text_editing: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let chips = has_chips(row);
        let marked = has_word_marks(row);
        let extension_decorations =
            crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx);
        let decoration_label: SharedString = extension_decorations
            .iter()
            .map(|(range, _, label)| format!("{} [{}..{}]", label, range.start, range.end))
            .collect::<Vec<_>>()
            .join("\n")
            .into();

        let text = if text_editing {
            div()
                .relative()
                .w_full()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Textarea::new(&self.text_input).appearance(false).small())
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().ring.opacity(0.6)),
                )
                .into_any_element()
        } else if marked {
            self.marked_text(row, doc, text_wrap, metrics, cx)
        } else {
            self.static_text(row, doc, text_wrap, metrics, cx)
        };

        text_cell_box(div())
            .child(text)
            .when(!extension_decorations.is_empty(), |cell| {
                cell.child(
                    div()
                        .id(SharedString::from(format!(
                            "extension-decorations:{}",
                            row.segment_id
                        )))
                        .absolute()
                        .top_0()
                        .right_0()
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .bg(cx.theme().muted)
                        .child(format!("{}", extension_decorations.len()))
                        .tooltip(move |window, cx| {
                            Tooltip::new(decoration_label.clone()).build(window, cx)
                        }),
                )
            })
            .when(chips, |cell| cell.child(self.chips(row, cx)))
            .when_some(row.styled_text.as_ref(), |cell, styled| {
                cell.child(
                    div()
                        .mt_1()
                        .pl(TEXT_PX)
                        .pr(TEXT_PX + WRAP_MARGIN)
                        .child(styled_text(styled, metrics, cx.theme())),
                )
            })
            .into_any_element()
    }

    /// Read-only rendering of a row's text, broken into the editor's rows; a click mounts the
    /// editor at the clicked caret.
    fn static_text(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &Rc<WrappedText>,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let text = display_text(row, doc);
        let editable = !row.is_read_only && !doc.is_read_only();
        let segment_id: SharedString = row.segment_id.clone().into();

        let body = if text.is_empty() {
            div()
                .text_color(theme.muted_foreground)
                .child(if row.annotation_kind.is_transcription() {
                    localized(cx, "What was said…", "Что было сказано…")
                } else {
                    localized(cx, "Annotation…", "Аннотация…")
                })
                .into_any_element()
        } else {
            let highlights = crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx)
                .into_iter()
                .map(|(range, style, _)| (text_wrap.to_display_range(range), style))
                .collect();
            let styled = StyledText::new(text_wrap.display_text(text))
                .with_highlights(crate::wasm_ui::disjoint_highlights(highlights));
            let layout = styled.layout().clone();
            let ghost = doc
                .ghost_cursor(&row.segment_id)
                .map(|ghost| text_wrap.to_display(byte_offset(text, ghost.char_offset)));
            let caret_layout = layout.clone();
            let ghost_color = theme.warning;
            let line_height = metrics.line_height;
            let text_wrap = Rc::clone(text_wrap);

            static_text_body(div().id(SharedString::from(format!(
                "transcript-text:{}",
                row.segment_id
            ))))
            .child(styled)
            .when_some(ghost, |el, ghost| {
                el.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            if let Some(origin) = caret_layout.position_for_index(ghost) {
                                window.paint_quad(fill(
                                    Bounds::new(origin, size(px(2.), line_height)),
                                    ghost_color,
                                ));
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let byte =
                        text_wrap.to_source(match layout.index_for_position(event.position) {
                            Ok(index) | Err(index) => index,
                        });
                    let modifiers = event.modifiers;
                    // Word seek: a plain Alt+Click (no Ctrl/Shift/Meta).
                    if modifiers.alt
                        && !modifiers.control
                        && !modifiers.shift
                        && !modifiers.platform
                    {
                        cx.stop_propagation();
                        this.seek_to_word_at_byte(&segment_id, byte, cx);
                    } else if editable {
                        cx.stop_propagation();
                        this.activate_text_at_byte(&segment_id, byte, window, cx);
                    }
                }),
            )
            .into_any_element()
        };

        let segment_id: SharedString = row.segment_id.clone().into();
        static_text_box(
            div().id(SharedString::from(format!(
                "transcript-text-cell:{}",
                row.segment_id
            ))),
            metrics,
            theme,
        )
        .when(editable, |el| {
            static_text_interactive(el).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.activate_text_at_end(&segment_id, window, cx);
                }),
            )
        })
        .child(body)
        .into_any_element()
    }

    /// Text of a row with word marks: marked words are underlined and seek on click; the text
    /// opens for editing once the row has no marks. Broken into the editor's rows like
    /// [`Self::static_text`].
    fn marked_text(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &WrappedText,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let text = display_text(row, doc);
        let tokens = word_ranges(text);
        let mut ranges = Vec::new();
        let mut seeks = Vec::new();
        let mut highlights = Vec::new();
        for mark in &row.word_marks {
            let Some(range) = tokens
                .get(mark.word_index)
                .map(|range| text_wrap.to_display_range(range.clone()))
            else {
                continue;
            };
            if mark.underlined {
                highlights.push((
                    range.clone(),
                    HighlightStyle {
                        underline: Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(theme.warning),
                            wavy: true,
                        }),
                        ..Default::default()
                    },
                ));
            }
            if let Some(start) = mark.seek_seconds {
                ranges.push(range);
                seeks.push(start);
            }
        }
        highlights.extend(
            crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx)
                .into_iter()
                .map(|(range, style, _)| (text_wrap.to_display_range(range), style)),
        );
        highlights.sort_by_key(|(range, _)| range.start);
        let styled = StyledText::new(text_wrap.display_text(text))
            .with_highlights(crate::wasm_ui::disjoint_highlights(highlights));
        let model = self.model.clone();

        div()
            .w_full()
            .pl(TEXT_PX)
            .pr(TEXT_PX + WRAP_MARGIN)
            .py(TEXT_PY)
            .text_sm()
            .line_height(metrics.line_height)
            .text_color(theme.foreground)
            .whitespace_nowrap()
            .child(
                InteractiveText::new(
                    SharedString::from(format!("transcript-marked:{}", row.segment_id)),
                    styled,
                )
                .on_click(ranges, move |index, _, cx| {
                    let Some(&start) = seeks.get(index) else {
                        return;
                    };
                    model.update(cx, |model, cx| model.run(cx, |doc| doc.seek(start)));
                }),
            )
            .into_any_element()
    }

    fn chips(&self, row: &TranscriptRowView, cx: &Context<Self>) -> impl IntoElement {
        let chips = row.chips.iter().map(|chip| {
            let command = chip.command.clone();
            let arguments = chip.arguments.clone();
            Button::new(SharedString::from(format!(
                "transcript-chip:{}:{}:{}",
                row.segment_id, chip.command, chip.arguments
            )))
            .label(chip.label.clone())
            .icon(IconName::Check)
            .xsmall()
            .outline()
            .tooltip(chip.tooltip.get().to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| doc.execute_command(&command, &arguments))
                });
            }))
        });

        h_flex()
            .pl(TEXT_PX)
            .pt_1()
            .gap_1()
            .overflow_hidden()
            .children(chips)
    }

    pub(super) fn lint_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let Some((severity, message)) = doc.row_lint(row) else {
            return empty_lint_box(div()).into_any_element();
        };
        let color = lint_color(severity, theme);
        let tooltip: SharedString = message.into();
        let dot = div()
            .id(SharedString::from(format!(
                "transcript-diagnostic:{}",
                row.segment_id
            )))
            .size(rems(0.625))
            .rounded_full()
            .bg(color)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));

        centered_cell_box(div(), LINT_W, metrics)
            .child(dot)
            .into_any_element()
    }

    pub(super) fn actions_cell(
        &self,
        row: &TranscriptRowView,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let cell = centered_cell_box(div(), ACTIONS_W, metrics);
        if row.is_read_only {
            return cell.into_any_element();
        }
        let entries = EditorSession::transcript_action_menu(row);
        let segment_id: SharedString = row.segment_id.clone().into();
        let view = cx.entity().downgrade();
        let fixes: Vec<_> = row
            .lint_issues
            .iter()
            .flat_map(|issue| issue.fixes.iter().cloned())
            .collect();
        let revision = self.model.read(cx).revision();
        let model = self.model.clone();

        cell.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(SharedString::from(format!(
                    "transcript-actions:{}",
                    row.segment_id
                )))
                .icon(IconName::EllipsisVertical)
                .ghost()
                .xsmall()
                .dropdown_menu(move |mut menu, window, cx| {
                    for entry in &entries {
                        if matches!(
                            *entry,
                            TranscriptActionMenuEntry::Item {
                                command: TranscriptActionCommand::MergeAbove,
                                ..
                            }
                        ) {
                            // Enabled when the playhead is inside the row, read when the menu
                            // opens (the row does not re-render per playback frame).
                            let doc = model.read(cx).doc();
                            let playhead = doc.session().playhead_seconds;
                            let inside =
                                doc.project().segment(&segment_id).is_some_and(|segment| {
                                    segment.start_seconds <= playhead
                                        && playhead < segment.end_seconds
                                });
                            let view = view.clone();
                            let segment_id = segment_id.clone();
                            menu = menu.item(
                                PopupMenuItem::new(localized(
                                    cx,
                                    "Split at playhead",
                                    "Разделить по курсору",
                                ))
                                .icon(Icon::from(gpui_kit::assets::IconName::Scissors))
                                .disabled(!inside)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.split_row_at_playhead(&segment_id, window, cx);
                                    })
                                    .ok();
                                }),
                            );
                        }
                        menu = match *entry {
                            TranscriptActionMenuEntry::Separator => menu.separator(),
                            TranscriptActionMenuEntry::Item { command, enabled } => {
                                let view = view.clone();
                                let segment_id = segment_id.clone();
                                menu.item(
                                    PopupMenuItem::new(command_label(command, cx))
                                        .icon(command_icon(command))
                                        .disabled(!enabled)
                                        .on_click(move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.run_row_command(
                                                    command,
                                                    &segment_id,
                                                    window,
                                                    cx,
                                                );
                                            })
                                            .ok();
                                        }),
                                )
                            }
                        };
                    }
                    for fix in &fixes {
                        let fix = fix.clone();
                        let model = model.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                localized(cx, "Fix: {title}", "Исправить: {title}")
                                    .replace("{title}", &fix.title),
                            )
                            .on_click(move |_, _, cx| {
                                model.update(cx, |model, cx| {
                                    model.edit(cx, |doc| {
                                        doc.apply_transaction(
                                            exaudio_extension::document::DocumentTransaction {
                                                label: fix.title.clone(),
                                                expected_revision: revision,
                                                edits: fix.edits.clone(),
                                            },
                                        )
                                    })
                                });
                            }),
                        );
                    }
                    let ctx = actions::snapshot(window, cx);
                    let lang = actions::lang(cx);
                    let contributed: Vec<(String, String)> = actions::registry(cx)
                        .menu(actions::MENU_ROW_CONTEXT, &ctx)
                        .into_iter()
                        .map(|item| (item.entry.id.clone(), item.entry.title_in(lang).to_string()))
                        .collect();
                    if !contributed.is_empty() {
                        menu = menu.separator();
                    }
                    for (id, title) in contributed {
                        let segment_id = segment_id.clone();
                        menu = menu.item(PopupMenuItem::new(SharedString::from(title)).on_click(
                            move |_, window, cx| {
                                let shell = cx.global::<crate::ShellHandle>().clone();
                                let args = serde_json::json!({ "segment_id": segment_id.as_ref() });
                                shell.run(&id, args, window, cx);
                            },
                        ));
                    }
                    menu
                }),
            )
            .into_any_element()
    }
}

/// A static row: tinted in its tone, without interactions.
fn render_static_row(
    params: RowParams,
    tone: TranscriptTone,
    cx: &Context<TranscriptView>,
) -> AnyElement {
    let RowParams {
        row,
        height,
        metrics,
        ..
    } = params;
    let theme = cx.theme();
    let cells = row.visible_cells;
    let tone = tone_color(tone, theme);
    let dash = || {
        div()
            .text_xs()
            .line_height(metrics.line_height)
            .text_color(theme.muted_foreground)
            .child("–")
    };

    h_flex()
        .w_full()
        .h(height)
        .items_start()
        .py(ROW_PY)
        .overflow_hidden()
        .border_b_1()
        .border_color(theme.border)
        .bg(tone.opacity(0.06))
        .when(cells.id, |el| el.child(id_cell(row, metrics, theme)))
        .when(cells.track, |el| {
            el.child(
                div()
                    .w(TRACK_W)
                    .flex_none()
                    .px(CELL_PX)
                    .text_sm()
                    .line_height(metrics.line_height)
                    .text_color(tone)
                    .truncate()
                    .child(row.speaker_label.clone()),
            )
        })
        .when(cells.start, |el| {
            el.child(static_time(row.start_time.clone(), metrics, theme))
        })
        .when(cells.end, |el| {
            el.child(static_time(row.end_time.clone(), metrics, theme))
        })
        .when_some(row.extra_column.as_deref(), |el, column| {
            el.child(extra_cell(row, column, metrics, theme))
        })
        .when(cells.text, |el| {
            let content = match &row.styled_text {
                Some(styled) => styled_text(styled, metrics, theme).into_any_element(),
                None => div()
                    .text_sm()
                    .line_height(metrics.line_height)
                    .text_color(theme.muted_foreground)
                    .child(row.text.clone())
                    .into_any_element(),
            };
            el.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(TEXT_PX)
                    .pr(TEXT_PX + WRAP_MARGIN)
                    .py(TEXT_PY)
                    .child(content),
            )
        })
        .when(cells.linter, |el| {
            el.child(
                h_flex()
                    .w(LINT_W)
                    .flex_none()
                    .justify_center()
                    .child(dash()),
            )
        })
        .when(cells.actions, |el| {
            el.child(
                h_flex()
                    .w(ACTIONS_W)
                    .flex_none()
                    .justify_center()
                    .child(dash()),
            )
        })
        .into_any_element()
}

fn id_cell(row: &TranscriptRowView, metrics: TextMetrics, theme: &Theme) -> impl IntoElement {
    id_box(div(), metrics, theme).child(row.display_id.clone())
}

fn static_time(value: String, metrics: TextMetrics, theme: &Theme) -> impl IntoElement {
    div()
        .w(TIME_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(theme.muted_foreground)
        .child(value)
}

fn extra_cell(
    row: &TranscriptRowView,
    column: &TranscriptExtraColumn,
    metrics: TextMetrics,
    theme: &Theme,
) -> impl IntoElement {
    let color = extra_cell_color(row, theme);
    extra_box(div(), rems(column.width_rems), metrics, color)
        .children(row.extra_cell.as_ref().map(|cell| cell.text.clone()))
}

/// Text colour of `row`'s cell in the extra column.
pub(super) fn extra_cell_color(row: &TranscriptRowView, theme: &Theme) -> Hsla {
    row.extra_cell
        .as_ref()
        .map_or(theme.muted_foreground, |cell| {
            cell.color_hue.map_or(theme.foreground, |hue| {
                let lightness = if theme.is_dark() { 0.72 } else { 0.30 };
                hsla(f32::from(hue) / 360., 0.70, lightness, 1.)
            })
        })
}

fn tone_color(tone: TranscriptTone, theme: &Theme) -> Hsla {
    match tone {
        TranscriptTone::Muted => theme.muted_foreground,
        TranscriptTone::Info => theme.info,
        TranscriptTone::Success => theme.success,
        TranscriptTone::Warning => theme.warning,
        TranscriptTone::Danger => theme.danger,
    }
}

/// Whether `row` is highlighted (selected) and its background colour.
pub(super) fn row_background(
    row: &TranscriptRowView,
    doc: &EditorDoc,
    is_playing: bool,
    is_find_match: bool,
    theme: &Theme,
) -> (bool, Hsla) {
    let highlighted = row.is_selected
        || doc.session().selected_segment_id.as_deref() == Some(row.segment_id.as_str());
    let background = if highlighted && is_playing {
        theme.accent.opacity(0.9)
    } else if highlighted {
        theme.list_active
    } else if is_find_match {
        theme.warning.opacity(0.10)
    } else {
        theme.background
    };
    (highlighted, background)
}

/// Colour of the dot of the row's speaker.
pub(super) fn speaker_dot_color(row: &TranscriptRowView, doc: &EditorDoc, theme: &Theme) -> Hsla {
    let project = doc.project();
    project
        .segment(&row.segment_id)
        .and_then(|segment| {
            project
                .speakers
                .iter()
                .position(|speaker| speaker.id == segment.speaker_id)
        })
        .map_or(theme.muted_foreground, |index| speaker_color(index, theme))
}

/// The start or end value a time cell shows: the draft's text, else the row's time.
pub(super) fn time_value(
    row: &TranscriptRowView,
    doc: &EditorDoc,
    field: TimeField,
) -> SharedString {
    match (doc.draft(&row.segment_id), field) {
        (Some(draft), field) => draft.time_value(field).to_string().into(),
        (None, TimeField::Start) => row.start_time.clone().into(),
        (None, TimeField::End) => row.end_time.clone().into(),
    }
}

/// A styled text: the badge in a fixed-width column and the wrapping body beside it.
fn styled_text(
    styled: &TranscriptStyledText,
    metrics: TextMetrics,
    theme: &Theme,
) -> impl IntoElement {
    let tone = tone_color(styled.badge.tone, theme);
    let text = styled.plain_text();
    let body: AnyElement = match &styled.body {
        TranscriptStyledBody::Note(_) => div()
            .italic()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
        TranscriptStyledBody::Words(words) => {
            let mut highlights = Vec::with_capacity(words.len());
            let mut offset = 0;
            for (index, word) in words.iter().enumerate() {
                if index > 0 {
                    offset += 1;
                }
                let range = offset..offset + word.text.len();
                offset = range.end;
                let style = match word.style {
                    TranscriptWordStyle::Plain => continue,
                    TranscriptWordStyle::Inserted => HighlightStyle {
                        color: Some(theme.success),
                        background_color: Some(theme.success.opacity(0.16)),
                        ..Default::default()
                    },
                    TranscriptWordStyle::Deleted => HighlightStyle {
                        color: Some(theme.danger),
                        background_color: Some(theme.danger.opacity(0.16)),
                        strikethrough: Some(StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(theme.danger),
                        }),
                        ..Default::default()
                    },
                    TranscriptWordStyle::Muted => HighlightStyle {
                        color: Some(theme.muted_foreground),
                        ..Default::default()
                    },
                };
                highlights.push((range, style));
            }
            StyledText::new(SharedString::from(text))
                .with_highlights(highlights)
                .into_any_element()
        }
    };

    h_flex()
        .items_start()
        .text_sm()
        .line_height(metrics.line_height)
        .child(
            div().w(BADGE_W).flex_none().child(
                div()
                    .px_1p5()
                    .w(rems(BADGE_W.0 - 0.5))
                    .rounded(theme.radius)
                    .bg(tone.opacity(0.16))
                    .text_color(tone)
                    .text_xs()
                    .text_center()
                    .child(styled.badge.label.get().to_string()),
            ),
        )
        .child(div().flex_1().min_w_0().child(body))
}

/// Colour of the speaker at `index` in the project's speaker list: the first two match the
/// lane palette (so a one-lane project's first speaker shares its lane colour), later ones
/// spread round the hue circle.
fn speaker_color(index: usize, theme: &Theme) -> Hsla {
    match index {
        0 => Hsla::from(rgb(LaneColor::Blue.rgb())),
        1 => Hsla::from(rgb(LaneColor::Purple.rgb())),
        _ => {
            let hue = ((207 + 137 * index) % 360) as f32 / 360.;
            let lightness = if theme.is_dark() { 0.70 } else { 0.42 };
            hsla(hue, 0.70, lightness, 1.)
        }
    }
}

pub(super) fn lint_color(severity: Severity, theme: &Theme) -> Hsla {
    match severity {
        Severity::Hint | Severity::Info => theme.muted_foreground,
        Severity::Warning => theme.warning,
        Severity::Error => theme.danger,
    }
}

fn command_label(command: TranscriptActionCommand, cx: &App) -> &'static str {
    match command {
        TranscriptActionCommand::Edit => localized(cx, "Edit", "Изменить"),
        TranscriptActionCommand::CreateBefore => {
            localized(cx, "Add segment above", "Добавить сегмент выше")
        }
        TranscriptActionCommand::CreateAfter => {
            localized(cx, "Add segment below", "Добавить сегмент ниже")
        }
        TranscriptActionCommand::MergeAbove => {
            localized(cx, "Merge with previous", "Объединить с предыдущим")
        }
        TranscriptActionCommand::MergeBelow => {
            localized(cx, "Merge with next", "Объединить со следующим")
        }
        TranscriptActionCommand::Delete => localized(cx, "Delete", "Удалить"),
    }
}

fn command_icon(command: TranscriptActionCommand) -> Icon {
    match command {
        TranscriptActionCommand::Edit => gpui_kit::assets::IconName::SquarePen.into(),
        TranscriptActionCommand::CreateBefore | TranscriptActionCommand::CreateAfter => {
            IconName::Plus.into()
        }
        TranscriptActionCommand::MergeAbove => IconName::ArrowUp.into(),
        TranscriptActionCommand::MergeBelow => IconName::ArrowDown.into(),
        TranscriptActionCommand::Delete => IconName::Delete.into(),
    }
}

fn field_key(field: TimeField) -> &'static str {
    match field {
        TimeField::Start => "start",
        TimeField::End => "end",
    }
}

/// Byte ranges of whitespace-separated words, in order (the indexing of
/// `TranscriptWordMark::word_index`).
fn word_ranges(text: &str) -> Vec<Range<usize>> {
    text.split_whitespace()
        .map(|word| {
            let start = word.as_ptr() as usize - text.as_ptr() as usize;
            start..start + word.len()
        })
        .collect()
}
