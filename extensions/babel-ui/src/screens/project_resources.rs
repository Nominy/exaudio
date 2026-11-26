//! Project resources beside Babel work: "How to do the task" (project instructions), "Grading
//! guide" (rubric flags) and "Library" (project documents; a document loads when opened).
//!
//! Read-only: nothing is acknowledged here, Babel's website does that. Markdown renders as
//! native text (HTML is never interpreted), wide tables scroll horizontally, file and media
//! documents point to Babel's Library, and only http(s) links can be copied.

use std::time::Duration;

use exaudio_babel_services::babel::resources::{
    DocumentBody, DocumentKind, DocumentTable, InlineStyle, LibraryDocument, ListMarker,
    MarkdownBlock, ProjectDocument, ProjectInstructions, ProjectResources, RichText, RubricFlag,
    dashboard_library_link, is_http_link, markdown_blocks,
};
use exaudio_babel_services::babel::{BabelClient, BabelError};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::text::{count, localized, localized_fmt};
use exaudio_shell::screens::ui::{self, Tone};
use exaudio_shell::services::spawn_service_in;

/// Table document rows per page (Babel pages its library tables too).
const TABLE_PAGE_ROWS: usize = 50;
const COPY_FEEDBACK: Duration = Duration::from_secs(2);

/// The view's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceTab {
    #[default]
    HowTo,
    Grading,
    Library,
}

impl ResourceTab {
    const ALL: [Self; 3] = [Self::HowTo, Self::Grading, Self::Library];

    fn id(self) -> &'static str {
        match self {
            Self::HowTo => "resources-tab-how-to",
            Self::Grading => "resources-tab-grading",
            Self::Library => "resources-tab-library",
        }
    }

    fn label(self, cx: &App) -> &'static str {
        match self {
            Self::HowTo => localized(cx, "How to do the task", "Как выполнять задание"),
            Self::Grading => localized(cx, "Grading guide", "Гайд по оценке"),
            Self::Library => localized(cx, "Library", "Библиотека"),
        }
    }
}

/// A grading-guide entry with its markdown parsed.
struct FlagView {
    flag: RubricFlag,
    description: Vec<MarkdownBlock>,
    extended: Vec<MarkdownBlock>,
    tips: Vec<MarkdownBlock>,
}

/// Loaded resources with markdown parsed off the UI thread.
struct Loaded {
    instructions: Result<(ProjectInstructions, Vec<MarkdownBlock>), BabelError>,
    grading: Result<Vec<FlagView>, BabelError>,
    library: Result<Vec<LibraryDocument>, BabelError>,
}

impl Loaded {
    fn new(resources: ProjectResources) -> Self {
        let blocks =
            |source: &Option<String>| source.as_deref().map(markdown_blocks).unwrap_or_default();
        Self {
            instructions: resources.instructions.map(|instructions| {
                let blocks = markdown_blocks(&instructions.markdown);
                (instructions, blocks)
            }),
            grading: resources.grading.map(|flags| {
                flags
                    .into_iter()
                    .map(|flag| FlagView {
                        description: blocks(&flag.description),
                        extended: blocks(&flag.extended_description),
                        tips: blocks(&flag.improvement_tips),
                        flag,
                    })
                    .collect()
            }),
            library: resources.library,
        }
    }
}

struct OpenDocument {
    document: ProjectDocument,
    /// Parsed markdown of a markdown document.
    blocks: Vec<MarkdownBlock>,
}

enum DocumentState {
    Loading,
    Ready(Box<OpenDocument>),
    Failed(BabelError),
}

/// Instructions / grading guide / library of one Babel project.
///
/// Mount with `cx.new(|cx| ProjectResourcesView::new(project_id, client, window, cx))` inside a
/// container that gives it a height; the view fills it and scrolls its own body. Loading
/// starts immediately; [`Self::set_project`] switches projects and [`Self::refresh`] reloads.
pub struct ProjectResourcesView {
    project_id: String,
    client: BabelClient,
    tab: ResourceTab,
    loaded: Option<Loaded>,
    loading: bool,
    /// Bumped per resources load; stale completions compare it.
    generation: u64,
    load_task: Option<Task<()>>,
    selected: Option<String>,
    document: Option<DocumentState>,
    document_generation: u64,
    document_task: Option<Task<()>>,
    table_filter: Entity<InputState>,
    table_page: usize,
    /// Key of the copy button that just copied, for its "Copied" state.
    copied: Option<SharedString>,
    copy_reset: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectResourcesView {
    pub fn new(
        project_id: impl Into<String>,
        client: BabelClient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "Search this table",
                "Поиск по таблице",
            ))
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&table_filter, window, |this, _, event, _, cx| {
                    if let InputEvent::Change = event {
                        this.table_page = 0;
                        cx.notify();
                    }
                }),
            ];
        let mut view = Self {
            project_id: project_id.into(),
            client,
            tab: ResourceTab::default(),
            loaded: None,
            loading: false,
            generation: 0,
            load_task: None,
            selected: None,
            document: None,
            document_generation: 0,
            document_task: None,
            table_filter,
            table_page: 0,
            copied: None,
            copy_reset: None,
            _subscriptions: subscriptions,
        };
        view.load(window, cx);
        view
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn tab(&self) -> ResourceTab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: ResourceTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// Shows another project (reloads only when the project changes); `client` replaces the
    /// transport either way.
    pub fn set_project(
        &mut self,
        project_id: impl Into<String>,
        client: BabelClient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_id = project_id.into();
        self.client = client;
        if project_id == self.project_id {
            return;
        }
        self.project_id = project_id;
        self.loaded = None;
        self.close_document(window, cx);
        self.load(window, cx);
    }

    /// Reloads the three lists and the open document.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load(window, cx);
        if let Some(document_id) = self.selected.clone() {
            self.open_document(document_id, window, cx);
        }
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let client = self.client.clone();
        let project_id = self.project_id.clone();
        self.load_task = Some(spawn_service_in(
            window,
            cx,
            async move { Loaded::new(client.load_project_resources(&project_id).await) },
            move |this, loaded, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                let selected_gone = match (&this.selected, &loaded.library) {
                    (Some(selected), Ok(library)) => {
                        !library.iter().any(|doc| &doc.document_id == selected)
                    }
                    _ => false,
                };
                if selected_gone {
                    this.close_document(window, cx);
                }
                this.loaded = Some(loaded);
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn close_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.document_generation += 1;
        self.selected = None;
        self.document = None;
        self.document_task = None;
        self.reset_table(window, cx);
    }

    fn reset_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.table_page = 0;
        if !self.table_filter.read(cx).value().is_empty() {
            self.table_filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn open_document(&mut self, document_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(document_id.as_str()) {
            self.reset_table(window, cx);
        }
        self.document_generation += 1;
        let generation = self.document_generation;
        self.selected = Some(document_id.clone());
        self.document = Some(DocumentState::Loading);
        let client = self.client.clone();
        let project_id = self.project_id.clone();
        self.document_task = Some(spawn_service_in(
            window,
            cx,
            async move {
                client
                    .load_project_document(&project_id, &document_id)
                    .await
                    .map(|document| {
                        let blocks = match &document.body {
                            DocumentBody::Markdown(source) => markdown_blocks(source),
                            _ => Vec::new(),
                        };
                        Box::new(OpenDocument { document, blocks })
                    })
            },
            move |this, result, _, cx| {
                if this.document_generation != generation {
                    return;
                }
                this.document = Some(match result {
                    Ok(document) => DocumentState::Ready(document),
                    Err(error) => DocumentState::Failed(error),
                });
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn copy_link(&mut self, key: SharedString, link: String, cx: &mut Context<Self>) {
        if !is_http_link(&link) {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        self.copied = Some(key);
        self.copy_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPY_FEEDBACK).await;
            this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// A copy-link button, or `None` unless `link` is http(s).
    fn copy_button(
        &self,
        key: SharedString,
        label: Option<&'static str>,
        link: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<Button> {
        let link = link.filter(|link| is_http_link(link))?;
        let copied = self.copied.as_ref() == Some(&key);
        let tooltip = localized(cx, "Copy link", "Копировать ссылку");
        let button = Button::new(key.clone())
            .ghost()
            .xsmall()
            .icon(if copied {
                IconName::Check
            } else {
                IconName::Copy
            })
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.copy_link(key.clone(), link.clone(), cx);
            }));
        Some(match label {
            Some(_) if copied => button.label(localized(cx, "Copied", "Скопировано")),
            Some(label) => button.label(label),
            None => button,
        })
    }

    /// Library links always point at the Babel dashboard, whatever the transport (a
    /// sub-account client's `base_url` is the relay).
    fn library_link(&self, document_id: Option<&str>) -> Option<String> {
        dashboard_library_link(&self.project_id, document_id)
    }

    fn error_banner(
        &self,
        error: &BabelError,
        retry: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let body = error.signed_out.then(|| {
            SharedString::from(localized(
                cx,
                "Sign in to Babel again, then try again.",
                "Войдите в Babel снова и повторите попытку.",
            ))
        });
        let retry = Button::new(id)
            .small()
            .outline()
            .label(localized(cx, "Try again", "Повторить"))
            .disabled(self.loading)
            .on_click(cx.listener(move |this, _, window, cx| retry(this, window, cx)));
        ui::banner(
            Tone::Danger,
            error.message.clone(),
            body,
            Some(retry.into_any_element()),
            cx,
        )
    }

    fn render_instructions(&self, loaded: &Loaded, cx: &mut Context<Self>) -> AnyElement {
        match &loaded.instructions {
            Err(error) => self
                .error_banner(error, Self::refresh, "resources-instructions-retry", cx)
                .into_any_element(),
            Ok((instructions, _)) if instructions.markdown.trim().is_empty() => ui::muted(
                localized(
                    cx,
                    "This project has no published instructions.",
                    "У этого проекта нет опубликованных инструкций.",
                ),
                cx,
            )
            .into_any_element(),
            Ok((instructions, blocks)) => {
                let cx: &App = cx;
                v_flex()
                .w_full()
                .gap_3()
                .when(instructions.is_acknowledged == Some(false), |this| {
                    this.child(ui::banner(
                        Tone::Info,
                        localized(cx, "Updated instructions", "Инструкции обновлены"),
                        Some(
                            localized(
                                cx,
                                "Read the current version below. Acknowledge it on the Babel website.",
                                "Прочитайте текущую версию ниже. Подтвердить ознакомление можно на сайте Babel.",
                            )
                            .into(),
                        ),
                        None,
                        cx,
                    ))
                })
                .child(markdown(blocks, "instructions", cx))
                .into_any_element()
            }
        }
    }

    fn render_grading(&self, loaded: &Loaded, cx: &mut Context<Self>) -> AnyElement {
        let flags = match &loaded.grading {
            Err(error) => {
                return self
                    .error_banner(error, Self::refresh, "resources-grading-retry", cx)
                    .into_any_element();
            }
            Ok(flags) if flags.is_empty() => {
                return ui::muted(
                    localized(
                        cx,
                        "This project has no grading guide.",
                        "У этого проекта нет гайда по оценке.",
                    ),
                    cx,
                )
                .into_any_element();
            }
            Ok(flags) => flags,
        };
        let cx: &App = cx;
        let theme = cx.theme();
        let tips_caption = localized(cx, "How to improve", "Как улучшить");
        v_flex()
            .w_full()
            .gap_4()
            .children(flags.iter().enumerate().map(|(index, view)| {
                let meta = [
                    view.flag.kind.as_deref(),
                    view.flag.review_category.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                v_flex()
                    .w_full()
                    .gap_2()
                    .p_3()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(view.flag.title().to_string()),
                            )
                            .when(!meta.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(meta),
                                )
                            }),
                    )
                    .child(markdown(&view.description, &format!("flag-{index}-d"), cx))
                    .when(!view.extended.is_empty(), |this| {
                        this.child(markdown(&view.extended, &format!("flag-{index}-e"), cx))
                    })
                    .when(!view.tips.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.muted_foreground)
                                .child(tips_caption),
                        )
                        .child(markdown(
                            &view.tips,
                            &format!("flag-{index}-t"),
                            cx,
                        ))
                    })
            }))
            .into_any_element()
    }

    fn render_library(&self, loaded: &Loaded, cx: &mut Context<Self>) -> AnyElement {
        let documents = match &loaded.library {
            Err(error) => {
                return self
                    .error_banner(error, Self::refresh, "resources-library-retry", cx)
                    .into_any_element();
            }
            Ok(documents) if documents.is_empty() => {
                return ui::muted(
                    localized(
                        cx,
                        "This project's library is empty.",
                        "Библиотека этого проекта пуста.",
                    ),
                    cx,
                )
                .into_any_element();
            }
            Ok(documents) => documents,
        };
        let summary = count(
            documents.len() as u64,
            ("document", "documents"),
            ("документ", "документа", "документов"),
        );
        let library_copy = self.copy_button(
            "resources-copy-library".into(),
            Some(localized(
                cx,
                "Copy Library link",
                "Копировать ссылку на библиотеку",
            )),
            self.library_link(None),
            cx,
        );
        let rows: Vec<Div> = documents
            .iter()
            .enumerate()
            .map(|(index, document)| self.library_row(index, document, cx))
            .collect();
        let open = self.render_document(cx);
        let border = cx.theme().border;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(ui::muted(summary, cx))
                    .child(div().flex_1())
                    .children(library_copy),
            )
            .child(v_flex().w_full().gap_1().children(rows))
            .when_some(open, |this, open| {
                this.child(
                    v_flex()
                        .w_full()
                        .pt_3()
                        .border_t_1()
                        .border_color(border)
                        .child(open),
                )
            })
            .into_any_element()
    }

    fn library_row(&self, index: usize, document: &LibraryDocument, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let selected = self.selected.as_deref() == Some(document.document_id.as_str());
        let kind = match &document.kind {
            DocumentKind::Markdown => localized(cx, "Document", "Документ"),
            DocumentKind::Table => localized(cx, "Table", "Таблица"),
            DocumentKind::File => localized(cx, "File", "Файл"),
            DocumentKind::Other(_) => localized(cx, "Other", "Другое"),
        };
        let warning = theme.warning;
        let muted = theme.muted_foreground;
        let needs_review = document
            .needs_acknowledgement
            .then(|| localized(cx, "Updated, review it", "Обновлён, ознакомьтесь"));
        let id = document.document_id.clone();
        h_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new(("resources-document", index))
                    .ghost()
                    .small()
                    .icon(IconName::FileText)
                    .label(document.title().to_string())
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_document(id.clone(), window, cx);
                    })),
            )
            .child(div().text_xs().text_color(muted).child(kind))
            .children(needs_review.map(|text| div().text_xs().text_color(warning).child(text)))
    }

    fn render_document(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.document.as_ref()?;
        let selected = self.selected.clone()?;
        Some(match state {
            DocumentState::Loading => ui::muted(
                localized(cx, "Loading the document…", "Загрузка документа…"),
                cx,
            )
            .into_any_element(),
            DocumentState::Failed(error) => self
                .error_banner(
                    error,
                    move |this, window, cx| this.open_document(selected.clone(), window, cx),
                    "resources-document-retry",
                    cx,
                )
                .into_any_element(),
            DocumentState::Ready(open) => {
                let document = &open.document.document;
                let body = match &open.document.body {
                    DocumentBody::Markdown(_) => {
                        markdown(&open.blocks, "document", cx).into_any_element()
                    }
                    DocumentBody::Table(table) => self.render_table(table, cx).into_any_element(),
                    DocumentBody::File | DocumentBody::Unsupported(_) => self
                        .render_file(document, &open.document.body, cx)
                        .into_any_element(),
                };
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(document.title().to_string()),
                    )
                    .when(document.needs_acknowledgement, |this| {
                        this.child(ui::banner(
                            Tone::Warning,
                            localized(cx, "Updated document", "Документ обновлён"),
                            Some(
                                localized(
                                    cx,
                                    "Review it before working. Acknowledge it on the Babel website.",
                                    "Ознакомьтесь с ним перед работой. Подтвердить ознакомление можно на сайте Babel.",
                                )
                                .into(),
                            ),
                            None,
                            cx,
                        ))
                    })
                    .child(body)
                    .into_any_element()
            }
        })
    }

    fn render_file(
        &self,
        document: &LibraryDocument,
        body: &DocumentBody,
        cx: &mut Context<Self>,
    ) -> Div {
        let note = match body {
            DocumentBody::Unsupported(_) => localized(
                cx,
                "This kind of document isn't shown here. Open it in the project's Library on Babel.",
                "Такие документы здесь не показываются. Откройте его в библиотеке проекта на Babel.",
            ),
            _ => localized(
                cx,
                "This is a file or media document. Open it in the project's Library on Babel.",
                "Это файл или медиадокумент. Откройте его в библиотеке проекта на Babel.",
            ),
        };
        let latest = document.latest.clone().unwrap_or_default();
        let size = latest.byte_size.map(|bytes| {
            if bytes >= 1024 * 1024 {
                format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
            } else {
                format!("{:.0} KB", (bytes as f64 / 1024.0).ceil())
            }
        });
        let copy = self.copy_button(
            "resources-copy-document".into(),
            Some(localized(
                cx,
                "Copy Library link",
                "Копировать ссылку на библиотеку",
            )),
            self.library_link(Some(&document.document_id)),
            cx,
        );
        v_flex()
            .w_full()
            .gap_2()
            .child(ui::banner(Tone::Info, note, None, None, cx))
            .children(
                latest
                    .file_name
                    .map(|name| ui::detail_row(localized(cx, "File", "Файл"), name, cx)),
            )
            .children(
                latest
                    .mime_type
                    .map(|mime| ui::detail_row(localized(cx, "Type", "Тип"), mime, cx)),
            )
            .children(size.map(|size| ui::detail_row(localized(cx, "Size", "Размер"), size, cx)))
            .children(copy.map(|copy| h_flex().child(copy)))
    }

    fn render_table(&self, table: &DocumentTable, cx: &mut Context<Self>) -> Div {
        let query = self.table_filter.read(cx).value().to_string();
        let matching = table.matching_rows(&query);
        let pages = matching.len().div_ceil(TABLE_PAGE_ROWS).max(1);
        let page = self.table_page.min(pages - 1);
        let shown = &matching[(page * TABLE_PAGE_ROWS).min(matching.len())
            ..((page + 1) * TABLE_PAGE_ROWS).min(matching.len())];
        let widths: Vec<f32> = table
            .columns
            .iter()
            .map(|column| {
                column
                    .width
                    .map_or(12., |width| (width as f32 * 0.5).clamp(6., 24.))
            })
            .collect();
        let total: f32 = widths.iter().sum();
        let summary = count(
            matching.len() as u64,
            ("row", "rows"),
            ("строка", "строки", "строк"),
        );
        let page_label = localized_fmt(
            cx,
            "Page {page} of {pages}",
            "Страница {page} из {pages}",
            &[
                ("page", &(page + 1).to_string()),
                ("pages", &pages.to_string()),
            ],
        );
        let theme = cx.theme();
        let border = theme.border;
        let header = h_flex()
            .items_start()
            .bg(theme.group_box)
            .font_weight(FontWeight::SEMIBOLD)
            .children(table.columns.iter().zip(&widths).map(|(column, width)| {
                div()
                    .flex_none()
                    .w(rems(*width))
                    .px_2()
                    .py_1()
                    .child(column.label.clone())
            }));
        let rows: Vec<Div> = shown
            .iter()
            .map(|&row_index| {
                let cells: Vec<Div> = table.rows[row_index]
                    .iter()
                    .zip(&widths)
                    .enumerate()
                    .map(|(column, (cell, width))| {
                        let copy = self.copy_button(
                            format!("resources-cell-link-{row_index}-{column}").into(),
                            None,
                            cell.link.clone(),
                            cx,
                        );
                        h_flex()
                            .flex_none()
                            .w(rems(*width))
                            .items_start()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .child(div().flex_1().min_w_0().child(cell.text.clone()))
                            .children(copy)
                    })
                    .collect();
                h_flex()
                    .items_start()
                    .border_t_1()
                    .border_color(border)
                    .children(cells)
            })
            .collect();
        let grid = v_flex()
            .flex_none()
            .w(rems(total))
            .text_xs()
            .border_1()
            .border_color(border)
            .rounded(cx.theme().radius)
            .child(header)
            .children(rows);
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .w(rems(16.))
                            .child(Input::new(&self.table_filter).small()),
                    )
                    .child(ui::muted(summary, cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("resources-table-previous")
                            .ghost()
                            .xsmall()
                            .label(localized(cx, "Previous", "Назад"))
                            .disabled(page == 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.table_page = page.saturating_sub(1);
                                cx.notify();
                            })),
                    )
                    .child(ui::muted(page_label, cx))
                    .child(
                        Button::new("resources-table-next")
                            .ghost()
                            .xsmall()
                            .label(localized(cx, "Next", "Далее"))
                            .disabled(page + 1 >= pages)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.table_page = page + 1;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .child(grid)
                    .overflow_x_scrollbar()
                    .id("resources-document-table"),
            )
    }
}

impl Render for ProjectResourcesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let tabs = ButtonGroup::new("resources-tabs")
            .outline()
            .small()
            .children(ResourceTab::ALL.map(|item| {
                Button::new(item.id())
                    .label(item.label(cx))
                    .selected(item == tab)
            }))
            .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                if let Some(tab) = clicked
                    .first()
                    .and_then(|index| ResourceTab::ALL.get(*index))
                {
                    this.set_tab(*tab, cx);
                }
            }));
        let refresh = Button::new("resources-refresh")
            .ghost()
            .small()
            .icon(IconName::RefreshCw)
            .label(localized(cx, "Refresh", "Обновить"))
            .tooltip(localized(
                cx,
                "Reload the instructions, grading guide and library",
                "Перезагрузить инструкции, гайд по оценке и библиотеку",
            ))
            .loading(self.loading)
            .disabled(self.loading)
            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx)));
        let body = match &self.loaded {
            None => ui::muted(
                localized(
                    cx,
                    "Loading project resources…",
                    "Загрузка материалов проекта…",
                ),
                cx,
            )
            .into_any_element(),
            Some(loaded) => match tab {
                ResourceTab::HowTo => self.render_instructions(loaded, cx),
                ResourceTab::Grading => self.render_grading(loaded, cx),
                ResourceTab::Library => self.render_library(loaded, cx),
            },
        };
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .child(tabs)
                    .child(div().flex_1())
                    .child(refresh),
            )
            .child(
                div()
                    .id("resources-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(div().w_full().pr_2().pb_2().child(body)),
            )
    }
}

/// Text with strong / emphasis / code / link styling.
fn rich(text: &RichText, cx: &App) -> StyledText {
    let theme = cx.theme();
    let mut bounds: Vec<usize> = text
        .spans
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = bounds
        .windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            let mut style = HighlightStyle::default();
            let mut styled = false;
            for (span, kind) in &text.spans {
                if span.start > range.start || range.end > span.end {
                    continue;
                }
                styled = true;
                match kind {
                    InlineStyle::Strong => style.font_weight = Some(FontWeight::SEMIBOLD),
                    InlineStyle::Emphasis => style.font_style = Some(FontStyle::Italic),
                    InlineStyle::Code => style.background_color = Some(theme.muted),
                    InlineStyle::Link => {
                        style.color = Some(theme.link);
                        style.underline = Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(theme.link),
                            wavy: false,
                        });
                    }
                }
            }
            (styled && !range.is_empty()).then_some((range, style))
        })
        .collect();
    StyledText::new(SharedString::from(text.text.clone())).with_highlights(highlights)
}

/// Native rendering of parsed markdown; `prefix` keeps table scroll ids unique.
fn markdown(blocks: &[MarkdownBlock], prefix: &str, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w_full()
        .gap_2()
        .text_sm()
        .children(blocks.iter().enumerate().map(|(index, block)| {
            match block {
                MarkdownBlock::Heading { level, text } => div()
                    .pt(if *level <= 2 { px(6.) } else { px(2.) })
                    .font_weight(FontWeight::SEMIBOLD)
                    .map(|this| match level {
                        1 => this.text_lg(),
                        2 => this.text_base(),
                        _ => this.text_sm(),
                    })
                    .child(rich(text, cx))
                    .into_any_element(),
                MarkdownBlock::Paragraph(text) => div().child(rich(text, cx)).into_any_element(),
                MarkdownBlock::ListItem {
                    depth,
                    marker,
                    text,
                } => h_flex()
                    .items_start()
                    .gap_2()
                    .pl(rems(0.25 + *depth as f32))
                    .child(div().flex_none().text_color(theme.muted_foreground).child(
                        match marker {
                            ListMarker::Bullet => SharedString::from("•"),
                            ListMarker::Ordered(marker) => SharedString::from(marker.clone()),
                        },
                    ))
                    .child(div().flex_1().min_w_0().child(rich(text, cx)))
                    .into_any_element(),
                MarkdownBlock::Quote(text) => div()
                    .pl_3()
                    .border_l_2()
                    .border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .child(rich(text, cx))
                    .into_any_element(),
                MarkdownBlock::Code(code) => div()
                    .w_full()
                    .p_2()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .child(code.clone())
                    .into_any_element(),
                MarkdownBlock::Rule => div().w_full().h_px().bg(theme.border).into_any_element(),
                MarkdownBlock::Table { header, rows } => {
                    markdown_table(format!("{prefix}-table-{index}"), header, rows, cx)
                        .into_any_element()
                }
            }
        }))
}

fn markdown_table(
    id: String,
    header: &[RichText],
    rows: &[Vec<RichText>],
    cx: &App,
) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let columns = rows
        .iter()
        .map(Vec::len)
        .chain([header.len()])
        .max()
        .unwrap_or(0);
    let widths: Vec<f32> = (0..columns)
        .map(|column| {
            let chars = header
                .get(column)
                .into_iter()
                .chain(rows.iter().filter_map(|row| row.get(column)))
                .map(|cell| cell.text.chars().count())
                .max()
                .unwrap_or(0);
            (chars as f32 * 0.5 + 1.).clamp(5., 22.)
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let line = |cells: &[RichText]| {
        h_flex()
            .items_start()
            .children(widths.iter().enumerate().map(|(column, width)| {
                div()
                    .flex_none()
                    .w(rems(*width))
                    .px_2()
                    .py_1()
                    .children(cells.get(column).map(|cell| rich(cell, cx)))
            }))
    };
    let grid = v_flex()
        .flex_none()
        .w(rems(total))
        .text_xs()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .when(!header.is_empty(), |this| {
            this.child(
                line(header)
                    .bg(theme.group_box)
                    .font_weight(FontWeight::SEMIBOLD),
            )
        })
        .children(
            rows.iter()
                .map(|row| line(row).border_t_1().border_color(theme.border)),
        );
    div()
        .w_full()
        .child(grid)
        .overflow_x_scrollbar()
        .id(SharedString::from(id))
}
