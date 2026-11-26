//! Project resources (instructions, grading guide, library) parsed from synthetic fixtures
//! shaped like Babel's tRPC replies (no real Babel project content).

use exaudio_babel_services::auth::babel_base_url;
use exaudio_babel_services::babel::resources::{
    DocumentBody, DocumentKind, InlineStyle, ListMarker, MarkdownBlock, dashboard_library_link,
    inline, is_http_link, library_link, markdown_blocks, parse_library_documents,
    parse_project_document, parse_project_instructions, parse_rubric_flags,
};
use exaudio_babel_services::babel::{BabelClient, BabelConfig, Fixture};
use exaudio_babel_services::subaccounts::SubaccountSession;

const INSTRUCTIONS: &str = include_str!("fixtures/resources/instructions.json");
const GRADING: &str = include_str!("fixtures/resources/grading.json");
const DOCUMENTS: &str = include_str!("fixtures/resources/documents.json");
const STYLE_GUIDE: &str = include_str!("fixtures/resources/style-guide.json");
const STYLE_GUIDE_ID: &str = "00000000-0000-7000-8000-00000000d002";
const PROJECT_ID: &str = "00000000-0000-4000-8000-0000000000b1";

fn fixture_client(json: &str) -> BabelClient {
    BabelClient::new(BabelConfig {
        fixture: Some(Fixture::Json(json.to_string())),
        ..BabelConfig::live("http://127.0.0.1:9")
    })
}

#[test]
fn resources_instructions_fixture_parses_with_tables() {
    let instructions = parse_project_instructions(INSTRUCTIONS).expect("instructions");
    assert_eq!(
        instructions.id.as_deref(),
        Some("00000000-0000-4000-8000-0000000000a1")
    );
    assert_eq!(instructions.is_acknowledged, Some(false));
    assert!(instructions.markdown.starts_with("## 🔍"));

    let blocks = markdown_blocks(&instructions.markdown);
    assert!(matches!(
        &blocks[0],
        MarkdownBlock::Heading { level: 2, text } if text.text == "🔍 About this sample project"
    ));
    let tables: Vec<_> = blocks
        .iter()
        .filter_map(|block| match block {
            MarkdownBlock::Table { header, rows } => Some((header, rows)),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 3);
    let (header, rows) = tables[0];
    let header: Vec<&str> = header.iter().map(|cell| cell.text.as_str()).collect();
    assert_eq!(header, ["Spoken form", "Audio", "Answer", "Why"]);
    assert!(rows.iter().all(|row| row.len() == 4));
    assert_eq!(rows[0][2].text, "✅ YES");
    // `\@` escapes are unescaped, never shown.
    let (_, rows) = tables[1];
    assert!(
        rows.iter()
            .any(|row| row[0].text == "Write to sam.lee@example.com")
    );
    let (header, rows) = tables[2];
    assert_eq!(header.len(), 2);
    assert_eq!(rows[0][1].text, "✅ ДА");
    assert!(blocks.iter().any(|block| matches!(
        block,
        MarkdownBlock::Quote(text) if text.text.starts_with("💡")
    )));
    // "- #### Title" items are emphasized list items; the next line is its own paragraph.
    let item = blocks
        .iter()
        .position(|block| {
            matches!(
                block,
                MarkdownBlock::ListItem { marker: ListMarker::Bullet, text, .. }
                    if text.text.starts_with("Letters read one by one")
                        && text.spans.first().is_some_and(|(range, style)| {
                            *style == InlineStyle::Strong && range.len() == text.text.len()
                        })
            )
        })
        .expect("heading item");
    assert!(matches!(
        &blocks[item + 1],
        MarkdownBlock::Paragraph(text) if text.text.starts_with("\"A B C\"")
    ));
}

#[test]
fn resources_grading_fixture_parses_every_flag() {
    let flags = parse_rubric_flags(GRADING).expect("grading");
    assert_eq!(flags.len(), 3);
    assert_eq!(flags[0].name, "sample-clarity");
    assert_eq!(flags[0].title(), "Sample Clarity");
    assert_eq!(flags[0].kind.as_deref(), Some("manual"));
    assert_eq!(flags[0].review_category.as_deref(), Some("content"));
    assert_eq!(flags[1].kind.as_deref(), Some("automated"));
    assert_eq!(flags[1].review_category.as_deref(), Some("audio"));
    assert!(flags.iter().all(|flag| flag.description.is_some()));
    assert!(flags.iter().all(|flag| flag.extended_description.is_none()));
    // Descriptions are markdown: paragraphs, inline styles and lists.
    let blocks = markdown_blocks(flags[1].description.as_deref().unwrap_or_default());
    assert!(matches!(&blocks[0], MarkdownBlock::Paragraph(text) if text.spans.len() == 2));
    assert_eq!(
        blocks
            .iter()
            .filter(|block| matches!(block, MarkdownBlock::ListItem { .. }))
            .count(),
        2
    );
}

#[test]
fn resources_library_fixture_lists_file_and_table_documents() {
    let documents = parse_library_documents(DOCUMENTS).expect("documents");
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].title(), "Sample Handbook");
    assert_eq!(documents[0].kind, DocumentKind::File);
    assert!(!documents[0].needs_acknowledgement);
    let latest = documents[0].latest.as_ref().expect("latest");
    assert_eq!(latest.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(latest.file_name.as_deref(), Some("sample-handbook.pdf"));
    assert_eq!(latest.byte_size, Some(120_000));
    assert_eq!(documents[1].document_id, STYLE_GUIDE_ID);
    assert_eq!(documents[1].kind, DocumentKind::Table);
    assert!(documents[1].needs_acknowledgement);
    assert_eq!(documents[1].requirement.as_deref(), Some("acknowledge"));
}

#[test]
fn resources_style_guide_fixture_is_a_table_with_links() {
    let document = parse_project_document(STYLE_GUIDE_ID, STYLE_GUIDE).expect("style guide");
    assert_eq!(document.document.title(), "Sample Reading Guide");
    assert!(document.document.needs_acknowledgement);
    let DocumentBody::Table(table) = document.body else {
        panic!("expected a table");
    };
    let labels: Vec<&str> = table.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Category",
            "Subcategory",
            "Case",
            "Written form example",
            "Spoken form example"
        ]
    );
    assert_eq!(table.columns[4].width, Some(45.0));
    assert_eq!(table.rows.len(), 8);
    assert!(table.rows.iter().all(|row| row.len() == 5));
    assert_eq!(table.rows[0][3].text, "+1 555 0100");
    assert_eq!(
        table.rows[5][3].link.as_deref(),
        Some("http://example.com/terms")
    );
    assert!(table.rows[0][3].link.is_none());
    let phones = table.matching_rows("PHONE NUMBERS");
    assert_eq!(phones, [0, 1, 2]);
    assert_eq!(table.matching_rows("марта"), [4]);
    assert_eq!(table.matching_rows("  ").len(), 8);
}

#[test]
fn resources_document_for_another_id_or_without_body_fails() {
    assert!(parse_project_document("other-document", STYLE_GUIDE).is_err());
    let no_body = r#"{"documentId":"doc-1","name":"Guide","kind":"markdown"}"#;
    assert!(parse_project_document("doc-1", no_body).is_err());
    let file = r#"{"documentId":"doc-2","name":"Video","kind":"file","body":{"kind":"file","url":"https://signed.example/v.mp4"}}"#;
    assert_eq!(
        parse_project_document("doc-2", file).expect("file").body,
        DocumentBody::File
    );
    let markdown = r##"[{"result":{"data":{"json":{"documentId":"doc-3","name":"Rules","kind":"markdown","body":{"kind":"markdown","content":"# Rules","mediaUrls":{}}}}}}]"##;
    assert_eq!(
        parse_project_document("doc-3", markdown)
            .expect("markdown")
            .body,
        DocumentBody::Markdown("# Rules".into())
    );
}

#[test]
fn resources_empty_and_error_replies_degrade() {
    assert_eq!(
        parse_project_instructions("null").expect("none").markdown,
        ""
    );
    assert!(parse_rubric_flags("").expect("none").is_empty());
    assert!(parse_library_documents("[]").expect("none").is_empty());
    let error = r#"[{"error":{"json":{"message":"FORBIDDEN","code":-32603}}}]"#;
    assert_eq!(
        parse_rubric_flags(error).expect_err("error").message,
        "FORBIDDEN"
    );
    assert!(parse_project_instructions("[1,2]").is_err());
    assert!(parse_library_documents("{\"x\":1}").is_err());
    assert!(parse_rubric_flags("not json").is_err());
}

#[test]
fn resources_markdown_keeps_html_as_text() {
    let blocks = markdown_blocks(
        "## Правила\n\n| Written | Spoken |\n| --- | --- |\n| 04.03 | четвёртое марта |\n\n<script>alert(1)</script> line<br>next",
    );
    assert!(
        matches!(&blocks[0], MarkdownBlock::Heading { level: 2, text } if text.text == "Правила")
    );
    assert!(
        matches!(&blocks[1], MarkdownBlock::Table { header, rows } if header.len() == 2 && rows.len() == 1)
    );
    assert!(matches!(
        &blocks[2],
        MarkdownBlock::Paragraph(text) if text.text == "<script>alert(1)</script> line\nnext"
    ));
}

#[test]
fn resources_inline_styles_and_links() {
    let text = inline(
        "**Bold** and *it* `code` [doc](https://x.example/a) [rel](/projects/p) ![photo](i.png) 2 * 3 dialogue_quality",
    );
    assert_eq!(
        text.text,
        "Bold and it code doc (https://x.example/a) rel [photo] 2 * 3 dialogue_quality"
    );
    let styled = |style| {
        text.spans
            .iter()
            .filter(|(_, s)| *s == style)
            .map(|(range, _)| &text.text[range.clone()])
            .collect::<Vec<_>>()
    };
    assert_eq!(styled(InlineStyle::Strong), ["Bold"]);
    assert_eq!(styled(InlineStyle::Emphasis), ["it"]);
    assert_eq!(styled(InlineStyle::Code), ["code"]);
    assert_eq!(styled(InlineStyle::Link), ["doc", "rel"]);
}

#[test]
fn resources_library_links_are_http_only() {
    assert_eq!(
        library_link(
            "https://dashboard.babel.audio",
            PROJECT_ID,
            Some(STYLE_GUIDE_ID)
        )
        .as_deref(),
        Some(
            "https://dashboard.babel.audio/projects/00000000-0000-4000-8000-0000000000b1?tab=library&doc=00000000-0000-7000-8000-00000000d002"
        )
    );
    assert!(library_link("file:///C:/babel", PROJECT_ID, None).is_none());
    assert!(library_link("https://dashboard.babel.audio", "bad id", None).is_none());
    assert!(is_http_link("http://example.com/legal/terms"));
    assert!(!is_http_link("javascript:alert(1)"));
    assert!(!is_http_link("/projects/p?tab=library"));
}

#[test]
fn resources_library_links_use_the_dashboard_origin_not_the_relay() {
    // A sub-account client's `base_url` is its relay; Library links never use it.
    let relay = BabelConfig::relay(SubaccountSession {
        host_url: "http://192.168.1.20:8787".to_string(),
        token: "token".to_string(),
        subaccount_id: "sub-1".to_string(),
        display_name: "Sub".to_string(),
    });
    assert_eq!(relay.base_url, "http://192.168.1.20:8787");
    let link = dashboard_library_link(PROJECT_ID, None).expect("dashboard link");
    assert!(!link.contains("192.168.1.20"), "{link}");
    assert_eq!(
        Some(link),
        library_link(&babel_base_url(), PROJECT_ID, None)
    );
}

#[tokio::test]
async fn resources_each_part_fails_on_its_own() {
    // Every request gets the instructions reply: only that part parses.
    let resources = fixture_client(INSTRUCTIONS)
        .load_project_resources(PROJECT_ID)
        .await;
    assert!(resources.instructions.is_ok());
    assert!(resources.grading.is_err());
    assert!(resources.library.is_err());

    let resources = fixture_client(GRADING)
        .load_project_resources(PROJECT_ID)
        .await;
    assert!(resources.instructions.is_err());
    assert_eq!(resources.grading.expect("grading").len(), 3);

    let resources = fixture_client(DOCUMENTS)
        .load_project_resources(PROJECT_ID)
        .await;
    assert_eq!(resources.library.expect("library").len(), 2);

    let document = fixture_client(STYLE_GUIDE)
        .load_project_document(PROJECT_ID, STYLE_GUIDE_ID)
        .await
        .expect("document");
    assert!(matches!(document.body, DocumentBody::Table(_)));
}

#[tokio::test]
async fn resources_unreachable_babel_and_bad_ids_fail_every_part() {
    let offline = BabelClient::new(BabelConfig {
        cookie: Some("__session=test".to_string()),
        ..BabelConfig::live("http://127.0.0.1:9")
    });
    let resources = offline.load_project_resources(PROJECT_ID).await;
    assert!(resources.instructions.is_err());
    assert!(resources.grading.is_err());
    assert!(resources.library.is_err());
    assert!(
        offline
            .load_project_document(PROJECT_ID, STYLE_GUIDE_ID)
            .await
            .is_err()
    );

    let resources = fixture_client(INSTRUCTIONS)
        .load_project_resources("not a project")
        .await;
    assert!(resources.instructions.is_err() && resources.grading.is_err());
    assert!(
        fixture_client(STYLE_GUIDE)
            .load_project_document(PROJECT_ID, "")
            .await
            .is_err()
    );
}
