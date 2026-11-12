//! Babel "add region" dialog state: the editor's add-region dialog (lane, start/end, text) plus
//! the annotation mode (intensity + optional hierarchical categories), its validation and the
//! create payload with the segment metadata.

use std::collections::BTreeMap;

use exaudio_core::Project;
use serde_json::{Value, json};

use crate::i18n::{is_russian, localized};
use exaudio_editor::edit::create_region::{
    CreateRegionDialog, CreateRegionError, CreateRegionPayload,
};

const BABEL_TRANSCRIPTION_TYPE: &str = "transcription";

const BACKGROUND_NOISE_TYPE: &str = "background_noise";
/// Projects whose annotation regions use primary/secondary category labels.
pub const BACKGROUND_NOISE_ANNOTATION_PROJECT_ID: &str = "4c0db8c4-699e-4014-9dbf-27f178c3da8e";
pub const BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID: &str = "7cb04a85-c9f3-4d73-afb9-34156777948f";
/// Category value that requires a free-text description.
pub const OTHER_CATEGORY: &str = "other";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRegionMode {
    Transcription,
    Annotation,
}

/// The open Babel "add region" dialog: the editor's dialog plus the annotation fields.
#[derive(Debug, Clone, PartialEq)]
pub struct BabelCreateRegionDialog {
    pub mode: CreateRegionMode,
    /// Lane, start/end with their drafts and errors, and the transcription text.
    pub region: CreateRegionDialog,
    pub intensity: Option<u8>,
    pub uses_hierarchical_labels: bool,
    pub primary_selection: String,
    pub secondary_selection: String,
    pub custom_description: String,
}

/// Validated "create" request.
#[derive(Debug, Clone, PartialEq)]
pub struct BabelCreateRegionPayload {
    /// Lane, range and row text; an annotation region carries its category label as text.
    pub region: CreateRegionPayload,
    pub metadata: BTreeMap<String, Value>,
}

/// Why the dialog cannot be submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BabelCreateRegionError {
    Region(CreateRegionError),
    IncompleteAnnotation,
}

impl From<CreateRegionError> for BabelCreateRegionError {
    fn from(error: CreateRegionError) -> Self {
        Self::Region(error)
    }
}

impl std::fmt::Display for BabelCreateRegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Region(CreateRegionError::Time(error)) => std::fmt::Display::fmt(error, f),
            Self::Region(CreateRegionError::UnknownLane) => {
                f.write_str(localized("Choose a speaker.", "Выберите спикера."))
            }
            Self::IncompleteAnnotation => f.write_str(localized(
                "Choose an intensity and both categories.",
                "Выберите интенсивность и обе категории.",
            )),
        }
    }
}

impl std::error::Error for BabelCreateRegionError {}

impl BabelCreateRegionDialog {
    /// Transcription-mode dialog.
    pub fn new(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self::with_mode(
            CreateRegionMode::Transcription,
            selected_lane_id,
            start_seconds,
            end_seconds,
            false,
        )
    }

    /// Annotation-mode dialog (hierarchical labels on).
    pub fn new_annotation(selected_lane_id: String, start_seconds: f64, end_seconds: f64) -> Self {
        Self::with_mode(
            CreateRegionMode::Annotation,
            selected_lane_id,
            start_seconds,
            end_seconds,
            true,
        )
    }

    /// Dialog whose mode and labels follow the project (annotation projects get annotation mode).
    pub fn new_for_project(
        project: &Project,
        selected_lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Self {
        if project_uses_annotation_mode(project) {
            Self::with_mode(
                CreateRegionMode::Annotation,
                selected_lane_id,
                start_seconds,
                end_seconds,
                project_uses_hierarchical_annotation_labels(project),
            )
        } else {
            Self::new(selected_lane_id, start_seconds, end_seconds)
        }
    }

    fn with_mode(
        mode: CreateRegionMode,
        selected_lane_id: String,
        start_seconds: f64,
        end_seconds: f64,
        uses_hierarchical_labels: bool,
    ) -> Self {
        Self {
            mode,
            region: CreateRegionDialog::new(selected_lane_id, start_seconds, end_seconds),
            intensity: None,
            uses_hierarchical_labels,
            primary_selection: String::new(),
            secondary_selection: String::new(),
            custom_description: String::new(),
        }
    }

    /// Changing the primary category clears the secondary choice and description.
    pub fn set_primary_category(&mut self, value: String) {
        self.region.active_time_field = None;
        self.primary_selection = value;
        self.secondary_selection.clear();
        self.custom_description.clear();
    }

    /// Choosing "other" clears the description; other choices keep it.
    pub fn set_secondary_category(&mut self, value: String) {
        self.region.active_time_field = None;
        if value == OTHER_CATEGORY {
            self.custom_description.clear();
        }
        self.secondary_selection = value;
    }

    pub fn set_custom_description(&mut self, value: String) {
        self.region.active_time_field = None;
        self.custom_description = value;
    }

    /// Accepts an intensity option label (`"2 - Noticeable..."`) or a bare number.
    pub fn set_intensity_label(&mut self, value: &str) {
        self.region.active_time_field = None;
        self.intensity = intensity_value_from_label(value);
    }

    /// Whether the Create button is enabled.
    pub fn is_create_enabled(&self) -> bool {
        self.region.is_create_enabled()
            && match self.mode {
                CreateRegionMode::Transcription => true,
                CreateRegionMode::Annotation => self.annotation_fields_are_complete(),
            }
    }

    /// Validates the region against the project, then the annotation fields, and builds the
    /// create payload.
    pub fn validate_for_submit(
        &mut self,
        project: &Project,
    ) -> Result<BabelCreateRegionPayload, BabelCreateRegionError> {
        let mut region = self.region.validate_for_submit(project)?;
        if self.mode == CreateRegionMode::Transcription {
            return Ok(BabelCreateRegionPayload {
                region,
                metadata: BTreeMap::new(),
            });
        }
        if !self.annotation_fields_are_complete() {
            return Err(BabelCreateRegionError::IncompleteAnnotation);
        }
        let (text, metadata) = self.annotation_text_and_metadata();
        region.text = text;
        Ok(BabelCreateRegionPayload { region, metadata })
    }

    fn annotation_fields_are_complete(&self) -> bool {
        if self.intensity.is_none() {
            return false;
        }
        if !self.uses_hierarchical_labels {
            return true;
        }
        if self.primary_selection.is_empty() {
            return false;
        }
        if self.primary_selection == OTHER_CATEGORY || self.secondary_selection == OTHER_CATEGORY {
            return !self.custom_description.trim().is_empty();
        }
        !self.secondary_selection.is_empty()
    }

    /// Row text and segment metadata of an annotation region.
    fn annotation_text_and_metadata(&self) -> (String, BTreeMap<String, Value>) {
        let mut metadata = BTreeMap::new();
        metadata.insert("type".to_string(), json!(BACKGROUND_NOISE_TYPE));
        metadata.insert("intensity".to_string(), json!(self.intensity));
        if !self.uses_hierarchical_labels {
            return (String::new(), metadata);
        }
        let content = annotation_content(
            &self.primary_selection,
            &self.secondary_selection,
            &self.custom_description,
        );
        metadata.insert("content".to_string(), content.clone());
        (annotation_display_text(&content), metadata)
    }
}

/// Primary annotation categories.
pub fn primary_category_options() -> &'static [&'static str] {
    &[
        "speech-sounds (optional)",
        "non-speech-sounds",
        "distorted-speech",
        "background-noise",
        "background-speech",
        "plosive",
        "system-noise",
        OTHER_CATEGORY,
    ]
}

/// Secondary options for `primary`, ending with "other" when non-empty.
pub fn secondary_category_options(primary: &str) -> Vec<&'static str> {
    let mut options = secondary_category_options_for(primary);
    if !options.is_empty() {
        options.push(OTHER_CATEGORY);
    }
    options
}

/// Intensity option labels (`1`..`4`) in the current UI language.
pub fn intensity_options() -> &'static [&'static str] {
    const EN: [&str; 4] = [
        "1 – Barely noticeable",
        "2 – Noticeable but not disruptive",
        "3 – Disruptive",
        "4 – Severe",
    ];
    const RU: [&str; 4] = [
        "1 – Едва заметно",
        "2 – Заметно, но не мешает",
        "3 – Мешает",
        "4 – Сильно мешает",
    ];
    if is_russian() { &RU } else { &EN }
}

/// Display label for a stored primary or secondary category value; unknown values (custom
/// descriptions) are shown as they are.
pub fn category_label(value: &str) -> &str {
    let (en, ru) = match value {
        "speech-sounds (optional)" => ("Speech sounds (optional)", "Речевые звуки (необязательно)"),
        "non-speech-sounds" => ("Non-speech sounds", "Неречевые звуки"),
        "distorted-speech" => ("Distorted speech", "Искажённая речь"),
        "background-noise" => ("Background noise", "Фоновый шум"),
        "background-speech" | "background speech" => ("Background speech", "Фоновая речь"),
        "plosive" => ("Plosive", "Взрывной звук"),
        "system-noise" => ("System noise", "Системный шум"),
        OTHER_CATEGORY => ("Other", "Другое"),
        "main speech" => ("Main speech", "Основная речь"),
        "interjection (um, uh, mhm, ah, ya)" => (
            "Interjection (um, uh, mhm, ah, ya)",
            "Междометие (эм, э-э, мгм, ах, ага)",
        ),
        "cough" => ("Cough", "Кашель"),
        "sneeze" => ("Sneeze", "Чихание"),
        "sigh" => ("Sigh", "Вздох"),
        "laugh" => ("Laugh", "Смех"),
        "breath" => ("Breath", "Дыхание"),
        "throat clear" => ("Throat clear", "Прочистка горла"),
        "mouth click" => ("Mouth click", "Щелчок губ"),
        "audio smear" => ("Audio smear", "Размытый звук"),
        "muffle" => ("Muffle", "Приглушение"),
        "echo (channel bleed)" => ("Echo (channel bleed)", "Эхо (просачивание канала)"),
        "dropout/network" => ("Dropout or network loss", "Пропадание звука (сеть)"),
        "rustle" => ("Rustle", "Шуршание"),
        "mic bump" => ("Mic bump", "Удар по микрофону"),
        "tap" => ("Tap", "Стук"),
        "click" => ("Click", "Щелчок"),
        "keyboard" => ("Keyboard", "Клавиатура"),
        "wind" => ("Wind", "Ветер"),
        "animal" => ("Animal", "Животное"),
        "other environmental noise" => ("Other environmental noise", "Другой шум окружения"),
        "static" => ("Static", "Статика"),
        "static-burst" => ("Static burst", "Всплеск статики"),
        _ => return value,
    };
    localized(en, ru)
}

/// Parses an intensity option label or bare number (`"0"` is accepted).
pub fn intensity_value_from_label(value: &str) -> Option<u8> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(value) = value.parse::<u8>() {
        return Some(value);
    }
    value
        .chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .and_then(|v| u8::try_from(v).ok())
}

/// Option label for an intensity value.
pub fn intensity_label(value: Option<u8>) -> Option<&'static str> {
    let value = value?.to_string();
    intensity_options()
        .iter()
        .copied()
        .find(|option| option.starts_with(&value))
}

/// Annotation projects: `studio.babel.mode = annotation` or any non-transcription segment.
pub fn project_uses_annotation_mode(project: &Project) -> bool {
    project
        .metadata
        .get("studio.babel.mode")
        .is_some_and(|mode| mode == "annotation")
        || project.segments.iter().any(|segment| {
            segment
                .metadata
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != BABEL_TRANSCRIPTION_TYPE)
        })
}

pub fn project_uses_hierarchical_annotation_labels(project: &Project) -> bool {
    matches!(
        project.id.as_str(),
        BACKGROUND_NOISE_ANNOTATION_PROJECT_ID | BACKGROUND_NOISE_ANNOTATION_FTE_PROJECT_ID
    )
}

/// Secondary categories for `primary`, without the trailing "other".
pub fn secondary_category_options_for(primary: &str) -> Vec<&'static str> {
    match primary {
        "speech-sounds (optional)" => vec!["main speech", "interjection (um, uh, mhm, ah, ya)"],
        "non-speech-sounds" => vec![
            "cough",
            "sneeze",
            "sigh",
            "laugh",
            "breath",
            "throat clear",
            "mouth click",
        ],
        "distorted-speech" => vec![
            "audio smear",
            "muffle",
            "echo (channel bleed)",
            "dropout/network",
        ],
        "background-noise" => vec![
            "rustle",
            "mic bump",
            "tap",
            "click",
            "keyboard",
            "wind",
            "animal",
            "other environmental noise",
        ],
        "background-speech" => vec!["background speech"],
        "plosive" => vec!["plosive"],
        "system-noise" => vec!["static", "static-burst"],
        _ => Vec::new(),
    }
}

/// Babel `content` JSON (`primaryLabel`/`secondaryLabel`); "other" uses the description.
pub fn annotation_content(primary: &str, secondary: &str, custom_description: &str) -> Value {
    let secondary_label = if primary == OTHER_CATEGORY || secondary == OTHER_CATEGORY {
        let custom = custom_description.trim();
        if custom.is_empty() {
            Value::Null
        } else {
            json!(custom)
        }
    } else if secondary.is_empty() {
        Value::Null
    } else {
        json!(secondary)
    };
    json!({
        "primaryLabel": primary,
        "secondaryLabel": secondary_label,
    })
}

/// Row text for annotation content: `"primary / secondary"` or `"primary"`.
pub fn annotation_display_text(content: &Value) -> String {
    let primary = content.get("primaryLabel").and_then(Value::as_str);
    let secondary = content.get("secondaryLabel").and_then(Value::as_str);
    match (primary, secondary) {
        (Some(primary), Some(secondary)) if !secondary.is_empty() => {
            format!("{primary} / {secondary}")
        }
        (Some(primary), _) => primary.to_string(),
        _ => String::new(),
    }
}
