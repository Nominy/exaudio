//! Editor hotkeys: semantic [`EditorAction`]s with their default bindings as gpui keystroke
//! strings (`"secondary-z"`, `"alt-shift-up"`, `"space"`) and the focus context each binding
//! lives in. The shell declares them as `exaudio.core.*` actions; routing, user overrides and
//! conflicts live in `exaudio_extension::actions`.
//!
//! `secondary-` is gpui's platform modifier alias (cmd on macOS, ctrl elsewhere).

pub use exaudio_extension::actions::KeyChord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::structure::AuditionKey;
use crate::i18n::Text;

/// Where a binding is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Everywhere, including while a text input has focus.
    Global,
    /// Only while a transcript row's text editor has focus.
    TranscriptText,
    /// Only while no text input has focus (timeline / transcript grid / workspace).
    Timeline,
    /// Like [`KeyContext::Timeline`], but only while a cut-preview overlay exists; these
    /// bindings take precedence over timeline bindings on the same keystroke.
    CutPreview,
    /// Wherever [`KeyContext::Timeline`] or [`KeyContext::TranscriptText`] applies (not in
    /// dialog inputs); those contexts take precedence on the same keystroke.
    Transcript,
    /// Only while a non-transcript input has focus (add-region dialog fields, find bar,
    /// lane zoom input).
    Dialog,
}

impl KeyContext {
    /// Whether bindings in `self` and `other` can fire for the same key press.
    pub fn overlaps(self, other: KeyContext) -> bool {
        use KeyContext::*;
        match (self, other) {
            (Global, _) | (_, Global) => true,
            (Transcript, Dialog) | (Dialog, Transcript) => false,
            (Transcript, _) | (_, Transcript) => true,
            (CutPreview, Timeline) | (Timeline, CutPreview) => false,
            (left, right) => left == right,
        }
    }
}

macro_rules! editor_actions {
    ($($variant:ident),* $(,)?) => {
        /// Every keyboard-triggerable editor action.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum EditorAction { $($variant),* }

        impl EditorAction {
            pub const ALL: &'static [EditorAction] = &[$(EditorAction::$variant),*];
        }
    };
}

editor_actions! {
    PlayPause,
    PreviousSegment,
    NextSegment,
    FocusPreviousSegment,
    FocusNextSegment,
    Undo,
    Redo,
    OpenFind,
    FindNext,
    FindPrevious,
    DeleteSelectedSegment,
    BackToProjects,
    ToggleOverlay,
    SwitchGhostCursorLane,
    CreateRegionSubmit,
    CreateRegionEscape,
    ResetSpeakerWorkflow,
    SwitchSpeakerWorkflowLane1,
    SwitchSpeakerWorkflowLane2,
    SwapSplitPanes,
    RewindPlaybackOneSecond,
    MoveTextBeforeCaretToPreviousSegment,
    MoveTextAfterCaretToNextSegment,
    IncreasePlaybackSpeed,
    DecreasePlaybackSpeed,
    MergeSelectedSegmentAbove,
    MergeSelectedSegmentBelow,
    TrimCurrentSegmentToAudio,
    TrimAllVisibleSegmentsToAudio,
    AutoInsertSegmentAtCaret,
    AutoSegmentVisibleSilences,
    SplitAtPlayhead,
    CommitCutPreviewSmartSplit,
    CommitCutPreviewPlainSplit,
    LoopCutPreviewOverlay,
    EscapePressed,
    ActivateDefaultSurfaceAction,
    TextUndo,
    TextRedo,
    TranscriptEscape,
    CompletionPrevious,
    CompletionNext,
    CompletionAccept,
    CompletionDismiss,
}

/// Static description of an action: hotkeys-dialog section/description, context and defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub action: EditorAction,
    pub section: Text,
    pub description: Text,
    pub context: KeyContext,
    pub defaults: &'static [&'static str],
    /// Whether the user may rebind it (fixed text-editing keys are not configurable).
    pub configurable: bool,
}

const fn def(
    action: EditorAction,
    section: Text,
    description: Text,
    context: KeyContext,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        action,
        section,
        description,
        context,
        defaults,
        configurable: true,
    }
}

const fn fixed(
    action: EditorAction,
    description: Text,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        action,
        section: TEXT_EDITING,
        description,
        context: KeyContext::TranscriptText,
        defaults,
        configurable: false,
    }
}

const fn t(en: &'static str, ru: &'static str) -> Text {
    Text::new(en, ru)
}

use EditorAction as A;
use KeyContext::{CutPreview, Dialog, Global, Timeline, Transcript, TranscriptText};

const GLOBAL: Text = t("General", "Общие");
const TIMELINE: Text = t("Timeline", "Таймлайн");
const NAV: Text = t("Transcript navigation", "Навигация по расшифровке");
const DIALOG: Text = t("Dialogs", "Диалоги");
const HELPER: Text = t("Helper editing", "Редактирование Helper");
const TIMELINE_EDITING: Text = t("Timeline editing", "Редактирование таймлайна");
const CUT: Text = t("Cut selection", "Выделение для разреза");
const TEXT_EDITING: Text = t("Text editing", "Редактирование текста");

/// All actions in hotkeys-dialog order.
pub const ACTION_DEFS: &[ActionDef] = &[
    def(
        A::PlayPause,
        GLOBAL,
        t("Play / pause", "Воспроизвести / пауза"),
        Timeline,
        &["space"],
    ),
    def(
        A::PreviousSegment,
        GLOBAL,
        t("Go to previous segment", "К предыдущему сегменту"),
        Timeline,
        &["left"],
    ),
    def(
        A::NextSegment,
        GLOBAL,
        t("Go to next segment", "К следующему сегменту"),
        Timeline,
        &["right"],
    ),
    def(
        A::Undo,
        GLOBAL,
        t("Undo", "Отменить"),
        Timeline,
        &["secondary-z"],
    ),
    def(
        A::Redo,
        GLOBAL,
        t("Redo", "Повторить"),
        Timeline,
        &["secondary-shift-z", "secondary-y"],
    ),
    def(
        A::OpenFind,
        GLOBAL,
        t("Find in transcript", "Поиск в расшифровке"),
        Global,
        &["secondary-f"],
    ),
    def(
        A::FindNext,
        GLOBAL,
        t("Next match", "Следующее совпадение"),
        Global,
        &["f3", "secondary-g"],
    ),
    def(
        A::FindPrevious,
        GLOBAL,
        t("Previous match", "Предыдущее совпадение"),
        Global,
        &["shift-f3", "secondary-shift-g"],
    ),
    def(
        A::DeleteSelectedSegment,
        GLOBAL,
        t("Delete the selected segment", "Удалить выбранный сегмент"),
        Timeline,
        &["d"],
    ),
    def(
        A::BackToProjects,
        GLOBAL,
        t("Back to Start", "На начальный экран"),
        Timeline,
        &["alt-left"],
    ),
    def(
        A::ToggleOverlay,
        TIMELINE,
        t(
            "Show or hide the 1-second overlay",
            "Показать или скрыть секундную сетку",
        ),
        Timeline,
        &["t"],
    ),
    def(
        A::SwitchGhostCursorLane,
        NAV,
        t(
            "Switch the ghost cursor to the other track",
            "Переключить призрачный курсор на другую дорожку",
        ),
        Transcript,
        &["tab"],
    ),
    def(
        A::FocusPreviousSegment,
        NAV,
        t(
            "Edit the previous segment's text",
            "Редактировать текст предыдущего сегмента",
        ),
        Timeline,
        &["shift-left"],
    ),
    def(
        A::FocusNextSegment,
        NAV,
        t(
            "Edit the next segment's text",
            "Редактировать текст следующего сегмента",
        ),
        Timeline,
        &["shift-right"],
    ),
    def(
        A::CreateRegionSubmit,
        DIALOG,
        t("Add the new segment", "Добавить новый сегмент"),
        Dialog,
        &["secondary-enter"],
    ),
    def(
        A::CreateRegionEscape,
        DIALOG,
        t("Cancel the new segment", "Отменить новый сегмент"),
        Dialog,
        &["escape"],
    ),
    def(
        A::EscapePressed,
        DIALOG,
        t(
            "Close the dialog or clear the cut selection",
            "Закрыть диалог или снять выделение",
        ),
        Timeline,
        &["escape"],
    ),
    def(
        A::ActivateDefaultSurfaceAction,
        DIALOG,
        t("Confirm", "Подтвердить"),
        Timeline,
        &["enter"],
    ),
    def(
        A::ResetSpeakerWorkflow,
        HELPER,
        t("Reset the speaker tracks", "Сбросить дорожки спикеров"),
        Global,
        &["alt-`"],
    ),
    def(
        A::SwitchSpeakerWorkflowLane1,
        HELPER,
        t(
            "Switch to speaker track 1",
            "Переключиться на дорожку спикера 1",
        ),
        Global,
        &["alt-1"],
    ),
    def(
        A::SwitchSpeakerWorkflowLane2,
        HELPER,
        t(
            "Switch to speaker track 2",
            "Переключиться на дорожку спикера 2",
        ),
        Global,
        &["alt-2"],
    ),
    // Helper `split.swapPanes`: works from the row being edited too (Helper claims Ctrl+I
    // before the rich-text italic handler).
    def(
        A::SwapSplitPanes,
        HELPER,
        t("Swap the speaker panes", "Поменять панели спикеров местами"),
        Global,
        &["secondary-i"],
    ),
    def(
        A::RewindPlaybackOneSecond,
        HELPER,
        t("Rewind 1 second", "Назад на 1 секунду"),
        Global,
        &["alt-x"],
    ),
    def(
        A::MoveTextBeforeCaretToPreviousSegment,
        HELPER,
        t(
            "Move text before the cursor to the previous segment",
            "Перенести текст до курсора в предыдущий сегмент",
        ),
        TranscriptText,
        &["alt-["],
    ),
    def(
        A::MoveTextAfterCaretToNextSegment,
        HELPER,
        t(
            "Move text after the cursor to the next segment",
            "Перенести текст после курсора в следующий сегмент",
        ),
        TranscriptText,
        &["alt-]"],
    ),
    def(
        A::IncreasePlaybackSpeed,
        TIMELINE_EDITING,
        t("Faster", "Быстрее"),
        Timeline,
        &["shift-1"],
    ),
    def(
        A::DecreasePlaybackSpeed,
        TIMELINE_EDITING,
        t("Slower", "Медленнее"),
        Timeline,
        &["shift-2"],
    ),
    def(
        A::MergeSelectedSegmentAbove,
        TIMELINE_EDITING,
        t(
            "Merge with previous segment",
            "Объединить с предыдущим сегментом",
        ),
        Timeline,
        &["alt-shift-up"],
    ),
    def(
        A::MergeSelectedSegmentBelow,
        TIMELINE_EDITING,
        t(
            "Merge with next segment",
            "Объединить со следующим сегментом",
        ),
        Timeline,
        &["alt-shift-down"],
    ),
    def(
        A::TrimCurrentSegmentToAudio,
        TIMELINE_EDITING,
        t(
            "Trim the segment to the speech in it",
            "Обрезать сегмент по речи",
        ),
        Timeline,
        &["alt-r"],
    ),
    def(
        A::TrimAllVisibleSegmentsToAudio,
        TIMELINE_EDITING,
        t(
            "Trim all visible segments to their speech",
            "Обрезать все видимые сегменты по речи",
        ),
        Timeline,
        &["alt-shift-r"],
    ),
    def(
        A::AutoInsertSegmentAtCaret,
        TIMELINE_EDITING,
        t(
            "Add a segment around the nearest speech",
            "Добавить сегмент вокруг ближайшей речи",
        ),
        Timeline,
        &["alt-c"],
    ),
    def(
        A::AutoSegmentVisibleSilences,
        TIMELINE_EDITING,
        t(
            "Split visible segments at silences",
            "Разделить видимые сегменты по паузам",
        ),
        Timeline,
        &["alt-shift-s"],
    ),
    // Timeline-context `s` yields to the cut-preview `s` while a cut selection exists.
    def(
        A::SplitAtPlayhead,
        TIMELINE_EDITING,
        t(
            "Split the selected segment at the playhead",
            "Разделить выбранный сегмент по курсору воспроизведения",
        ),
        Timeline,
        &["s"],
    ),
    def(
        A::CommitCutPreviewSmartSplit,
        CUT,
        t(
            "Split here, splitting the text by words",
            "Разрезать здесь, разделив текст по словам",
        ),
        CutPreview,
        &["s"],
    ),
    def(
        A::CommitCutPreviewPlainSplit,
        CUT,
        t(
            "Split here, keeping the text",
            "Разрезать здесь, сохранив текст",
        ),
        CutPreview,
        &["shift-s"],
    ),
    def(
        A::LoopCutPreviewOverlay,
        CUT,
        t(
            "Loop the selection (again to stop)",
            "Повторять выделение (ещё раз — стоп)",
        ),
        CutPreview,
        &["l"],
    ),
    fixed(
        A::TextUndo,
        t("Undo text edit", "Отменить правку текста"),
        &["secondary-z"],
    ),
    fixed(
        A::TextRedo,
        t("Redo text edit", "Повторить правку текста"),
        &["secondary-shift-z", "secondary-y"],
    ),
    fixed(
        A::TranscriptEscape,
        t(
            "Leave the segment and play / pause",
            "Выйти из сегмента и воспроизвести / пауза",
        ),
        &["escape"],
    ),
    fixed(
        A::CompletionPrevious,
        t("Previous suggestion", "Предыдущая подсказка"),
        &["up"],
    ),
    fixed(
        A::CompletionNext,
        t("Next suggestion", "Следующая подсказка"),
        &["down"],
    ),
    fixed(
        A::CompletionAccept,
        t("Accept suggestion", "Принять подсказку"),
        &["enter", "tab"],
    ),
    fixed(
        A::CompletionDismiss,
        t("Hide suggestions", "Скрыть подсказки"),
        &["escape"],
    ),
];

impl EditorAction {
    /// Persisted id (`"playPause"`, `"deleteSelectedSegment"`, ...).
    pub fn id(self) -> String {
        match serde_json::to_value(self) {
            Ok(Value::String(id)) => id,
            _ => format!("{self:?}"),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        serde_json::from_value(Value::String(id.to_string())).ok()
    }

    pub fn def(self) -> &'static ActionDef {
        ACTION_DEFS
            .iter()
            .find(|def| def.action == self)
            .expect("every EditorAction has an ActionDef")
    }

    /// Completion keys only apply while the completion list is open.
    pub fn requires_completions(self) -> bool {
        matches!(
            self,
            A::CompletionPrevious | A::CompletionNext | A::CompletionAccept | A::CompletionDismiss
        )
    }
}

/// Held boundary-audition key for an unmodified Q/W/E/R press outside text inputs.
pub fn audition_key_for(keystroke: &str) -> Option<AuditionKey> {
    let chord = KeyChord::parse(keystroke)?;
    if chord.secondary || chord.ctrl || chord.alt || chord.platform {
        return None;
    }
    match chord.key.as_str() {
        "q" => Some(AuditionKey::BeforeStart),
        "w" => Some(AuditionKey::AfterStart),
        "e" => Some(AuditionKey::BeforeEnd),
        "r" => Some(AuditionKey::AfterEnd),
        _ => None,
    }
}
