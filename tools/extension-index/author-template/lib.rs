use exaudio_sdk::{ActionArgs, ActionHandler, Guest, host, types::*};

struct AuthorExtension;

/// Declared in `manifest.json` (`contributes.actions`); the shell shows it in the ⋯ menu
/// and the command palette before this component is loaded.
const INSERT_GREETING: &str = "@@EXTENSION_ID@@.insert-greeting";

fn refresh_diagnostics(document_id: &str) -> Result<(), String> {
    let document = host::snapshot(document_id)?;
    let items = document.segments.iter().filter(|segment| segment.text.trim().is_empty()).map(|segment| Diagnostic {
        id: format!("empty-{}", segment.id),
        range: TextRange { segment_id: segment.id.clone(), start_byte: 0, end_byte: segment.text.len() as u32 },
        severity: Severity::Hint,
        message: "This segment is empty. Use Insert greeting in the extension panel.".into(),
        fix: None,
    }).collect();
    host::publish_diagnostics(&Diagnostics { provider: "empty-segments".into(), document_id: document.id,
        revision: document.revision, items })
}

fn insert_greeting(args: ActionArgs) -> Result<(), String> {
    let id = args.document()?;
    let document = host::snapshot(id)?;
    let transaction = if let Some(segment) = document.segments.first() {
        exaudio_sdk::insert_text(&document, &segment.id, 0, "Hello from an extension! ".into(), "Insert greeting".into())?
    } else {
        let lane = document.lanes.first().ok_or("document has no lane to insert into")?;
        Transaction { document_id: document.id.clone(), expected_revision: document.revision,
            label: "Insert greeting".into(), edits: vec![Edit::InsertSegment(Segment {
                id: format!("greeting-{}", document.revision), lane: lane.id.clone(), speaker_id: lane.speaker_id.clone(),
                start_ms: 0, end_ms: 1000, text: "Hello from an extension! ".into(), metadata: Vec::new(),
            })] }
    };
    host::propose(&transaction)?;
    refresh_diagnostics(id)
}

const ACTIONS: &[(&str, ActionHandler)] = &[(INSERT_GREETING, insert_greeting)];

impl Guest for AuthorExtension {
    fn activate() -> Result<(), String> {
        exaudio_sdk::button_panel("author-panel", "Author extension", "insert", "Insert greeting")?;
        host::register(&Registration { id: "empty-segments".into(), kind: ContributionKind::Diagnostics,
            title: "Empty segments".into() })?;
        host::register(&Registration { id: "document-events".into(), kind: ContributionKind::Event,
            title: "Refresh empty-segment hints".into() })
    }

    fn deactivate() {
        // Contributions are owned by this runtime; the host removes them on deactivation.
    }

    fn on_action(id: String, args: String) -> Result<(), String> {
        exaudio_sdk::dispatch_action(ACTIONS, &id, &args)
    }

    fn on_ui_event(event: UiEvent) -> Result<(), String> {
        if event.contribution == "author-panel" && event.node == "insert" && event.kind == UiEventKind::Click {
            exaudio_sdk::run_action(INSERT_GREETING, &exaudio_sdk::serde_json::json!({ "document_id": event.document_id }))
        } else { Ok(()) }
    }

    fn on_event(event: Event) -> Result<(), String> {
        match event {
            Event::DocumentOpened(event) | Event::DocumentChanged(event) => refresh_diagnostics(&event.document_id),
            _ => Ok(()),
        }
    }
}

exaudio_sdk::bindings::export!(AuthorExtension with_types_in exaudio_sdk::bindings);
