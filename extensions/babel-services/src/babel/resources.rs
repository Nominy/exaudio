//! Project resources shown beside Babel work: the project instructions ("How to do the task",
//! `project.getProjectInstructions`), the grading guide (`rubric.getProjectRubricFlags`) and the
//! project Library (`project.listProjectDocuments` → `project.getProjectDocument`).
//!
//! Everything here is read-only: nothing acknowledges instructions or documents. Markdown is
//! parsed into plain blocks for native rendering; HTML in it is never interpreted (only `<br>`
//! becomes a line break), and images or embedded media are shown as their alt text.

use std::ops::Range;

use exaudio_babel_domain::i18n::{localized, with_detail};
use serde_json::Value;

use super::BabelError;
use super::client::BabelClient;
use super::error_task::valid_project_id;
use super::request::BabelRequest;
use super::response::trpc_data;

/// `project.getProjectInstructions`: the current instructions markdown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectInstructions {
    /// Instruction version id (`id`), when published.
    pub id: Option<String>,
    /// Markdown source; empty when the project publishes none.
    pub markdown: String,
    /// `Some(false)` when the worker has not acknowledged this version on Babel.
    pub is_acknowledged: Option<bool>,
}

/// One grading-guide entry (`rubric.getProjectRubricFlags` item).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RubricFlag {
    pub name: String,
    pub friendly_name: Option<String>,
    pub review_category: Option<String>,
    /// `manual` / `automated`.
    pub kind: Option<String>,
    pub description: Option<String>,
    pub extended_description: Option<String>,
    pub improvement_tips: Option<String>,
}

impl RubricFlag {
    /// `friendlyName ?? name`, as Babel titles a flag.
    pub fn title(&self) -> &str {
        self.friendly_name.as_deref().unwrap_or(&self.name)
    }
}

/// Library document kinds Babel renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentKind {
    Markdown,
    Table,
    /// An uploaded file (PDF, video, audio, …).
    File,
    Other(String),
}

impl DocumentKind {
    fn parse(kind: &str) -> Self {
        match kind {
            "markdown" => Self::Markdown,
            "table" => Self::Table,
            "file" => Self::File,
            other => Self::Other(other.to_string()),
        }
    }
}

/// The latest published version of a library document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentVersion {
    pub version: Option<i64>,
    pub mime_type: Option<String>,
    pub byte_size: Option<u64>,
    pub file_name: Option<String>,
    pub created_at: Option<String>,
}

/// A `project.listProjectDocuments` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDocument {
    pub document_id: String,
    pub name: String,
    pub label: Option<String>,
    pub kind: DocumentKind,
    /// `none` / `acknowledge` / `watch`.
    pub requirement: Option<String>,
    pub needs_acknowledgement: bool,
    pub show_in_task_tool: bool,
    pub latest: Option<DocumentVersion>,
}

impl LibraryDocument {
    /// Name, else label, else id.
    pub fn title(&self) -> &str {
        [Some(self.name.as_str()), self.label.as_deref()]
            .into_iter()
            .flatten()
            .find(|title| !title.trim().is_empty())
            .unwrap_or(&self.document_id)
    }
}

/// A table document's column.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub key: String,
    pub label: String,
    /// Spreadsheet column width (characters), when imported from one.
    pub width: Option<f64>,
}

/// A table cell's display text and its link, if the source cell had one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableCell {
    pub text: String,
    pub link: Option<String>,
}

/// A table document: rows hold one cell per column, in column order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentTable {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<Vec<TableCell>>,
}

impl DocumentTable {
    /// Indexes of the rows containing `query` in any cell (case-insensitive); every row for a
    /// blank query.
    pub fn matching_rows(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        (0..self.rows.len())
            .filter(|&index| {
                query.is_empty()
                    || self.rows[index]
                        .iter()
                        .any(|cell| cell.text.to_lowercase().contains(&query))
            })
            .collect()
    }
}

/// What a library document contains.
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentBody {
    Markdown(String),
    Table(DocumentTable),
    /// A file or media document: only Babel's Library opens it.
    File,
    /// A body kind this app does not render.
    Unsupported(String),
}

/// A `project.getProjectDocument` result.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDocument {
    pub document: LibraryDocument,
    pub body: DocumentBody,
}

/// The three resource lists of a project; each part loads and fails on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectResources {
    pub instructions: Result<ProjectInstructions, BabelError>,
    pub grading: Result<Vec<RubricFlag>, BabelError>,
    pub library: Result<Vec<LibraryDocument>, BabelError>,
}

impl BabelClient {
    /// Instructions, grading guide and library list of `project_id`, requested concurrently.
    pub async fn load_project_resources(&self, project_id: &str) -> ProjectResources {
        if !valid_project_id(project_id) {
            let error = invalid_project();
            return ProjectResources {
                instructions: Err(error.clone()),
                grading: Err(error.clone()),
                library: Err(error),
            };
        }
        let project_id = project_id.to_string();
        let instructions = BabelRequest::LoadProjectInstructions {
            project_id: project_id.clone(),
        };
        let grading = BabelRequest::LoadProjectRubricFlags {
            project_id: project_id.clone(),
        };
        let library = BabelRequest::LoadProjectDocuments { project_id };
        let (instructions, grading, library) = tokio::join!(
            self.raw(&instructions),
            self.raw(&grading),
            self.raw(&library),
        );
        ProjectResources {
            instructions: instructions
                .and_then(|payload| parse_project_instructions(&payload))
                .map_err(|error| {
                    headed(
                        error,
                        localized(
                            "Couldn't load the instructions.",
                            "Не удалось загрузить инструкции.",
                        ),
                    )
                }),
            grading: grading
                .and_then(|payload| parse_rubric_flags(&payload))
                .map_err(|error| {
                    headed(
                        error,
                        localized(
                            "Couldn't load the grading guide.",
                            "Не удалось загрузить гайд по оценке.",
                        ),
                    )
                }),
            library: library
                .and_then(|payload| parse_library_documents(&payload))
                .map_err(|error| {
                    headed(
                        error,
                        localized(
                            "Couldn't load the library.",
                            "Не удалось загрузить библиотеку.",
                        ),
                    )
                }),
        }
    }

    /// One library document with its body.
    pub async fn load_project_document(
        &self,
        project_id: &str,
        document_id: &str,
    ) -> Result<ProjectDocument, BabelError> {
        if !valid_project_id(project_id) {
            return Err(invalid_project());
        }
        if !valid_project_id(document_id) {
            return Err(BabelError::new(localized(
                "This library document has no valid id.",
                "У документа библиотеки нет корректного ID.",
            )));
        }
        let request = BabelRequest::LoadProjectDocument {
            project_id: project_id.to_string(),
            document_id: document_id.to_string(),
        };
        self.raw(&request)
            .await
            .and_then(|payload| parse_project_document(document_id, &payload))
            .map_err(|error| {
                headed(
                    error,
                    localized(
                        "Couldn't load the document.",
                        "Не удалось загрузить документ.",
                    ),
                )
            })
    }
}

fn invalid_project() -> BabelError {
    BabelError::new(localized(
        "Open a project to see its instructions.",
        "Откройте проект, чтобы увидеть его инструкции.",
    ))
}

fn headed(error: BabelError, headline: &str) -> BabelError {
    BabelError {
        message: with_detail(headline, &error.message),
        ..error
    }
}

/// Decodes a raw or already-normalized tRPC payload; a tRPC error envelope fails.
fn payload_value(payload: &str) -> Result<Value, BabelError> {
    let trimmed = payload.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|error| {
        BabelError::new(with_detail(
            localized(
                "The server sent an unreadable reply.",
                "Сервер прислал нечитаемый ответ.",
            ),
            error,
        ))
    })?;
    let envelope = match &value {
        Value::Array(items) if items.len() == 1 => &items[0],
        other => other,
    };
    if let Some(error) = envelope.get("error").filter(|error| error.is_object()) {
        let message = ["/json/message", "/message"]
            .into_iter()
            .find_map(|pointer| error.pointer(pointer).and_then(Value::as_str))
            .filter(|message| !message.trim().is_empty())
            .unwrap_or(localized(
                "Babel returned an error.",
                "Babel вернул ошибку.",
            ));
        return Err(BabelError::new(message));
    }
    Ok(trpc_data(&value).unwrap_or(value))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

fn unexpected(what: &'static str) -> BabelError {
    BabelError::new(what)
}

/// Parses a `project.getProjectInstructions` payload; `null` means none are published.
pub fn parse_project_instructions(payload: &str) -> Result<ProjectInstructions, BabelError> {
    let value = payload_value(payload)?;
    if value.is_null() {
        return Ok(ProjectInstructions::default());
    }
    if !value.is_object() {
        return Err(unexpected(localized(
            "The instructions reply has an unexpected shape.",
            "Ответ с инструкциями имеет неожиданный формат.",
        )));
    }
    Ok(ProjectInstructions {
        id: text(&value, "id"),
        markdown: value
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_acknowledged: value.get("isAcknowledged").and_then(Value::as_bool),
    })
}

/// Parses a `rubric.getProjectRubricFlags` payload; `null` means no grading guide.
pub fn parse_rubric_flags(payload: &str) -> Result<Vec<RubricFlag>, BabelError> {
    let value = payload_value(payload)?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    let flags = value.as_array().ok_or_else(|| {
        unexpected(localized(
            "The grading guide reply has an unexpected shape.",
            "Ответ с гайдом по оценке имеет неожиданный формат.",
        ))
    })?;
    Ok(flags
        .iter()
        .filter_map(|flag| {
            let name = text(flag, "name").or_else(|| text(flag, "friendlyName"))?;
            Some(RubricFlag {
                name,
                friendly_name: text(flag, "friendlyName"),
                review_category: text(flag, "reviewCategory"),
                kind: text(flag, "type"),
                description: text(flag, "description"),
                extended_description: text(flag, "extendedDescription"),
                improvement_tips: text(flag, "improvementTips"),
            })
        })
        .collect())
}

fn library_document(value: &Value) -> Option<LibraryDocument> {
    let document_id = text(value, "documentId")?;
    let latest = value
        .get("latest")
        .filter(|latest| latest.is_object())
        .map(|latest| DocumentVersion {
            version: latest.get("version").and_then(Value::as_i64),
            mime_type: text(latest, "mimeType"),
            byte_size: latest.get("byteSize").and_then(Value::as_u64),
            file_name: text(latest, "fileName"),
            created_at: text(latest, "createdAt"),
        });
    Some(LibraryDocument {
        document_id,
        name: text(value, "name").unwrap_or_default(),
        label: text(value, "label"),
        kind: DocumentKind::parse(&text(value, "kind").unwrap_or_default()),
        requirement: text(value, "requirement"),
        needs_acknowledgement: value.get("needsAcknowledgement").and_then(Value::as_bool)
            == Some(true),
        show_in_task_tool: value.get("showInTaskTool").and_then(Value::as_bool) == Some(true),
        latest,
    })
}

/// Parses a `project.listProjectDocuments` payload; entries without an id are skipped.
pub fn parse_library_documents(payload: &str) -> Result<Vec<LibraryDocument>, BabelError> {
    let value = payload_value(payload)?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    let documents = value.as_array().ok_or_else(|| {
        unexpected(localized(
            "The library reply has an unexpected shape.",
            "Ответ библиотеки имеет неожиданный формат.",
        ))
    })?;
    Ok(documents.iter().filter_map(library_document).collect())
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(_) | Value::Bool(_) => value.to_string(),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
        Value::Array(items) => items.iter().map(cell_text).collect::<Vec<_>>().join(", "),
    }
}

fn document_table(table: &Value) -> Option<DocumentTable> {
    let columns: Vec<TableColumn> = table
        .get("columns")?
        .as_array()?
        .iter()
        .filter_map(|column| {
            let key = column.get("key").and_then(Value::as_str)?.to_string();
            Some(TableColumn {
                label: text(column, "label").unwrap_or_else(|| key.clone()),
                width: column.get("width").and_then(Value::as_f64),
                key,
            })
        })
        .collect();
    let links: Vec<(usize, &str, &str)> = table
        .get("cells")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|cell| {
            Some((
                usize::try_from(cell.get("row")?.as_u64()?).ok()?,
                cell.get("column")?.as_str()?,
                cell.get("href")?.as_str()?,
            ))
        })
        .collect();
    let rows = table
        .get("rows")?
        .as_array()?
        .iter()
        .enumerate()
        .map(|(index, row)| {
            columns
                .iter()
                .map(|column| TableCell {
                    text: row.get(&column.key).map(cell_text).unwrap_or_default(),
                    link: links
                        .iter()
                        .find(|(row, key, _)| *row == index && *key == column.key)
                        .map(|(_, _, href)| href.to_string()),
                })
                .collect()
        })
        .collect();
    Some(DocumentTable { columns, rows })
}

/// Parses a `project.getProjectDocument` payload for `document_id`; a reply for another
/// document, or with no readable body, fails.
pub fn parse_project_document(
    document_id: &str,
    payload: &str,
) -> Result<ProjectDocument, BabelError> {
    let value = payload_value(payload)?;
    let not_found = || {
        BabelError::new(localized(
            "This document is no longer in the project library.",
            "Этого документа больше нет в библиотеке проекта.",
        ))
    };
    let document = library_document(&value).ok_or_else(not_found)?;
    if document.document_id != document_id {
        return Err(not_found());
    }
    let unreadable = || {
        BabelError::new(localized(
            "This document's content couldn't be read.",
            "Не удалось прочитать содержимое документа.",
        ))
    };
    let body = match value.get("body").filter(|body| body.is_object()) {
        Some(body) => match body.get("kind").and_then(Value::as_str).unwrap_or_default() {
            "markdown" => DocumentBody::Markdown(
                body.get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(unreadable)?
                    .to_string(),
            ),
            "table" => DocumentBody::Table(
                body.get("table")
                    .and_then(document_table)
                    .ok_or_else(unreadable)?,
            ),
            "file" => DocumentBody::File,
            other => DocumentBody::Unsupported(other.to_string()),
        },
        None if document.kind == DocumentKind::File => DocumentBody::File,
        None => return Err(unreadable()),
    };
    Ok(ProjectDocument { document, body })
}

/// `true` for an absolute `http`/`https` URL.
pub fn is_http_link(link: &str) -> bool {
    url::Url::parse(link.trim()).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// [`library_link`] on the Babel dashboard origin ([`crate::auth::babel_base_url`]). Use this
/// rather than a client's `base_url`, which is the relay's URL for sub-account clients.
pub fn dashboard_library_link(project_id: &str, document_id: Option<&str>) -> Option<String> {
    library_link(&crate::auth::babel_base_url(), project_id, document_id)
}

/// The Babel dashboard Library link for a project (and optionally one of its documents), or
/// `None` when `base_url` is not an http(s) dashboard or the ids are not valid.
pub fn library_link(base_url: &str, project_id: &str, document_id: Option<&str>) -> Option<String> {
    if !valid_project_id(project_id) || document_id.is_some_and(|id| !valid_project_id(id)) {
        return None;
    }
    let mut url = url::Url::parse(base_url.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.path_segments_mut()
        .ok()?
        .clear()
        .extend(["projects", project_id]);
    {
        let mut query = url.query_pairs_mut();
        query.clear().append_pair("tab", "library");
        if let Some(document_id) = document_id {
            query.append_pair("doc", document_id);
        }
    }
    url.set_fragment(None);
    Some(url.into())
}

/// Inline styling of a [`RichText`] span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineStyle {
    Strong,
    Emphasis,
    Code,
    Link,
}

/// Plain text with styled byte ranges (non-overlapping per style).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichText {
    pub text: String,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
}

/// List item marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListMarker {
    Bullet,
    /// The source number, e.g. `"3."`.
    Ordered(String),
}

/// A block of parsed markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        text: RichText,
    },
    Paragraph(RichText),
    ListItem {
        depth: usize,
        marker: ListMarker,
        text: RichText,
    },
    Quote(RichText),
    Code(String),
    /// A pipe table; `header` is empty when the source has no delimiter row.
    Table {
        header: Vec<RichText>,
        rows: Vec<Vec<RichText>>,
    },
    Rule,
}

/// Parses markdown into blocks: ATX/setext headings, paragraphs, bullet and ordered lists,
/// block quotes, fenced code, pipe tables and rules; inline strong, emphasis, code and links.
/// HTML is kept as literal text (`<br>` becomes a line break).
pub fn markdown_blocks(source: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut table: Vec<&str> = Vec::new();
    let mut list_open = false;
    let mut lines = source.lines().peekable();
    let flush_paragraph = |paragraph: &mut Vec<&str>, blocks: &mut Vec<MarkdownBlock>| {
        if !paragraph.is_empty() {
            blocks.push(MarkdownBlock::Paragraph(inline_lines(paragraph)));
            paragraph.clear();
        }
    };
    let flush_quote = |quote: &mut Vec<&str>, blocks: &mut Vec<MarkdownBlock>| {
        let mut current: Vec<&str> = Vec::new();
        for line in quote.drain(..) {
            if line.trim().is_empty() {
                if !current.is_empty() {
                    blocks.push(MarkdownBlock::Quote(inline_lines(&current)));
                    current.clear();
                }
            } else {
                current.push(line);
            }
        }
        if !current.is_empty() {
            blocks.push(MarkdownBlock::Quote(inline_lines(&current)));
        }
    };
    let flush_table = |table: &mut Vec<&str>, blocks: &mut Vec<MarkdownBlock>| {
        if !table.is_empty() {
            blocks.push(pipe_table(table));
            table.clear();
        }
    };
    while let Some(raw) = lines.next() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        let indent = indent_width(line);
        if !trimmed.starts_with('|') {
            flush_table(&mut table, &mut blocks);
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            quote.push(rest.strip_prefix(' ').unwrap_or(rest));
            continue;
        }
        flush_quote(&mut quote, &mut blocks);
        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            continue;
        }
        if let Some(fence) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            let mut code = Vec::new();
            for code_line in lines.by_ref() {
                if code_line.trim_start().starts_with(fence) {
                    break;
                }
                code.push(code_line);
            }
            blocks.push(MarkdownBlock::Code(code.join("\n")));
            continue;
        }
        if trimmed.starts_with('|') {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            table.push(trimmed);
            continue;
        }
        if let Some(level) = setext_level(trimmed).filter(|_| !paragraph.is_empty()) {
            let text = inline_lines(&paragraph);
            paragraph.clear();
            blocks.push(MarkdownBlock::Heading { level, text });
            continue;
        }
        if is_rule(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            blocks.push(MarkdownBlock::Rule);
            continue;
        }
        if let Some((level, text)) = atx_heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            list_open = false;
            blocks.push(MarkdownBlock::Heading {
                level,
                text: inline(text),
            });
            continue;
        }
        if let Some((marker, content)) = list_item(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            // A heading inside an item ("- #### Title") is an emphasized item; like
            // CommonMark, it takes no lazy continuation lines.
            let (text, continues) = match atx_heading(content) {
                Some((_, heading)) => {
                    let mut text = inline(heading);
                    text.spans
                        .retain(|(_, style)| *style != InlineStyle::Strong);
                    text.spans
                        .insert(0, (0..text.text.len(), InlineStyle::Strong));
                    (text, false)
                }
                None => (inline(content), true),
            };
            blocks.push(MarkdownBlock::ListItem {
                depth: indent / 2,
                marker,
                text,
            });
            list_open = continues;
            continue;
        }
        if list_open
            && paragraph.is_empty()
            && let Some(MarkdownBlock::ListItem { text, .. }) = blocks.last_mut()
        {
            append_inline(text, " ", inline(trimmed));
            continue;
        }
        list_open = false;
        paragraph.push(raw);
    }
    flush_table(&mut table, &mut blocks);
    flush_quote(&mut quote, &mut blocks);
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn atx_heading(line: &str) -> Option<(u8, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim();
    let text = text
        .trim_end_matches('#')
        .strip_suffix(' ')
        .map_or(text, str::trim_end);
    Some((level as u8, text))
}

fn setext_level(line: &str) -> Option<u8> {
    let line = line.trim();
    if line.len() >= 2 && line.chars().all(|c| c == '=') {
        Some(1)
    } else if line.len() >= 2 && line.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .into_iter()
            .any(|mark| compact.chars().all(|c| c == mark))
}

fn list_item(line: &str) -> Option<(ListMarker, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return Some((ListMarker::Bullet, rest.trim_start()));
        }
    }
    if matches!(line, "-" | "*" | "+") {
        return Some((ListMarker::Bullet, ""));
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &line[digits..];
        for delimiter in [". ", ") "] {
            if let Some(content) = rest.strip_prefix(delimiter) {
                let marker = format!("{}{}", &line[..digits], &delimiter[..1]);
                return Some((ListMarker::Ordered(marker), content.trim_start()));
            }
        }
    }
    None
}

fn is_delimiter_row(cells: &[&str]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            cell.contains('-') && cell.chars().all(|c| matches!(c, '-' | ':' | ' '))
        })
}

/// Splits a pipe-table row on unescaped `|` outside code spans.
fn table_cells(line: &str) -> Vec<&str> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    let mut in_code = false;
    for (index, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '`' => in_code = !in_code,
            '|' if !in_code => {
                cells.push(&line[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    let last = &line[start..];
    if !last.trim().is_empty() {
        cells.push(last);
    }
    cells
}

fn pipe_table(lines: &[&str]) -> MarkdownBlock {
    let rows: Vec<Vec<&str>> = lines.iter().map(|line| table_cells(line)).collect();
    let cells = |row: &[&str]| -> Vec<RichText> {
        row.iter()
            .map(|cell| inline(&cell.trim().replace("\\|", "|")))
            .collect()
    };
    let has_header = rows.len() >= 2 && is_delimiter_row(&rows[1]);
    let header = if has_header {
        cells(&rows[0])
    } else {
        Vec::new()
    };
    let body = rows
        .iter()
        .skip(if has_header { 2 } else { 0 })
        .filter(|row| !is_delimiter_row(row))
        .map(|row| cells(row))
        .collect();
    MarkdownBlock::Table { header, rows: body }
}

fn hard_break(line: &str) -> bool {
    line.ends_with("  ") || (line.ends_with('\\') && !line.ends_with("\\\\"))
}

/// Joins paragraph lines: soft breaks become spaces, hard breaks (two trailing spaces or a
/// trailing backslash) line breaks.
fn inline_lines(lines: &[&str]) -> RichText {
    let mut text = RichText::default();
    for (index, line) in lines.iter().enumerate() {
        let content = line.trim();
        let content = if hard_break(content) {
            &content[..content.len() - 1]
        } else {
            content
        };
        let separator = if index > 0 && hard_break(lines[index - 1]) {
            "\n"
        } else {
            " "
        };
        append_inline(&mut text, separator, inline(content));
    }
    text
}

fn append_inline(target: &mut RichText, separator: &str, next: RichText) {
    if !target.text.is_empty() && !next.text.is_empty() {
        target.text.push_str(separator);
    }
    let offset = target.text.len();
    target.text.push_str(&next.text);
    target.spans.extend(
        next.spans
            .into_iter()
            .map(|(range, style)| (range.start + offset..range.end + offset, style)),
    );
}

const ESCAPABLE: &str = "\\`*_{}[]()#+-.!|<>~@\"'";

/// Parses inline markdown: `**strong**`/`__strong__`, `*emphasis*`, `` `code` ``,
/// `[text](url)`, `![alt](url)`, `<https://autolink>`, backslash escapes and `<br>`.
pub fn inline(source: &str) -> RichText {
    let mut out = RichText::default();
    let mut open: Vec<(InlineStyle, usize, &str)> = Vec::new();
    let mut rest = source;
    while let Some(c) = rest.chars().next() {
        // Escapes.
        if c == '\\'
            && let Some(next) = rest[1..].chars().next()
            && ESCAPABLE.contains(next)
        {
            out.text.push(next);
            rest = &rest[1 + next.len_utf8()..];
            continue;
        }
        // Code spans.
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            let code = &rest[1..1 + end];
            let start = out.text.len();
            out.text.push_str(code);
            out.spans.push((start..out.text.len(), InlineStyle::Code));
            rest = &rest[end + 2..];
            continue;
        }
        // Line breaks; any other HTML stays literal.
        if c == '<' {
            let lower = rest.get(..6).unwrap_or(rest).to_ascii_lowercase();
            if let Some(len) = ["<br/>", "<br />", "<br>"]
                .into_iter()
                .find(|tag| lower.starts_with(tag))
                .map(str::len)
            {
                out.text.push('\n');
                rest = &rest[len..];
                continue;
            }
            if let Some(end) = rest.find('>') {
                let target = &rest[1..end];
                if is_http_link(target) && !target.contains(char::is_whitespace) {
                    let start = out.text.len();
                    out.text.push_str(target);
                    out.spans.push((start..out.text.len(), InlineStyle::Link));
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        // Images and links.
        let image = c == '!' && rest[1..].starts_with('[');
        if (c == '[' || image)
            && let Some((label, target, consumed)) = link_parts(&rest[usize::from(image)..])
        {
            let label = inline(label);
            if image {
                let alt = if label.text.trim().is_empty() {
                    localized("image", "изображение").to_string()
                } else {
                    label.text
                };
                out.text.push_str(&format!("[{alt}]"));
            } else {
                let start = out.text.len();
                let shown = if label.text.is_empty() {
                    target.to_string()
                } else {
                    label.text.clone()
                };
                append_inline(
                    &mut out,
                    "",
                    RichText {
                        text: shown,
                        spans: label.spans,
                    },
                );
                out.spans.push((start..out.text.len(), InlineStyle::Link));
                if is_http_link(target)
                    && label.text.trim() != target.trim()
                    && !label.text.is_empty()
                {
                    out.text.push_str(&format!(" ({})", target.trim()));
                }
            }
            rest = &rest[usize::from(image) + consumed..];
            continue;
        }
        // Strong / emphasis delimiters.
        let delimiter = ["**", "__", "*"]
            .into_iter()
            .find(|delimiter| rest.starts_with(delimiter));
        if let Some(delimiter) = delimiter {
            let style = if delimiter.len() == 2 {
                InlineStyle::Strong
            } else {
                InlineStyle::Emphasis
            };
            if let Some(index) = open.iter().rposition(|(_, _, d)| *d == delimiter) {
                let (style, start, _) = open.remove(index);
                if out.text.len() > start {
                    out.spans.push((start..out.text.len(), style));
                }
                rest = &rest[delimiter.len()..];
                continue;
            }
            let after = &rest[delimiter.len()..];
            let mut after_chars = after.chars();
            let opens = after_chars.next().is_some_and(|c| !c.is_whitespace())
                && after_chars.as_str().contains(delimiter)
                && (delimiter != "__" || !preceded_by_word(&out.text));
            if opens {
                open.push((style, out.text.len(), delimiter));
                rest = after;
                continue;
            }
        }
        out.text.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out.spans.sort_by_key(|(range, _)| range.start);
    out
}

fn preceded_by_word(text: &str) -> bool {
    text.chars().last().is_some_and(char::is_alphanumeric)
}

/// `[label](target)` at the start of `source`: (label, target, bytes consumed).
fn link_parts(source: &str) -> Option<(&str, &str, usize)> {
    let mut depth = 0usize;
    let mut label_end = None;
    for (index, c) in source.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    label_end = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let label_end = label_end?;
    let after = &source[label_end + 1..];
    let target = after.strip_prefix('(')?;
    let close = target.find(')')?;
    let target_text = target[..close].trim();
    // `[x](url "title")`: drop the title.
    let target_text = target_text
        .split_once(char::is_whitespace)
        .map_or(target_text, |(url, _)| url);
    let target_text = target_text
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(target_text);
    Some((
        &source[1..label_end],
        target_text,
        label_end + 2 + close + 1,
    ))
}
