//! Babel evaluation sessions (`evalTasks.*`): task validation, question visibility, answer
//! rules, the `inputResponses` payload, clip download, and the local answer draft.
//!
//! Semantics follow Babel's dashboard bundle (the eval form, its `buildResponse` per question
//! type, the `showWhen` schema). Tasks Babel would validate with checks we cannot run (form or
//! question linters, taxonomy questions, unknown formats) are refused instead of submitted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use exaudio_audio::{AudioDecodeError, AudioError, DecodeLimits, DecodedAudio};
use exaudio_babel_domain::i18n::{localized, localized_fmt, with_detail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::BabelError;
use super::response::{first_string_by_key, trpc_data};

/// `project.productKind` values of evaluation projects.
pub const EVAL_PRODUCT_KINDS: [&str; 2] = ["eval_task", "eval_session"];

/// Evaluation projects the 0.1.16 fork observed live (RU-Naturalness-Eval,
/// RU-Naturalness-Eval2, RU-Alpha-Eval). Kept alongside [`EVAL_PRODUCT_KINDS`] because
/// whether `worker.getProjects` items carry `productKind` is unverified.
pub const KNOWN_EVAL_PROJECT_IDS: [&str; 3] = [
    "d35d73aa-4816-4571-b77d-960ddb15eeae",
    "2c68b806-ca86-419f-83ea-2551e85e4619",
    "3882e77e-3ace-4ff2-b2df-290e3439cfce",
];

/// Task formats this adapter renders; `tts-naturalness` compares exactly two clips.
pub const SUPPORTED_EVAL_FORMATS: [&str; 4] = [
    "tts-naturalness",
    "default",
    "tts-model",
    "tts-model-subtitles",
];

/// Download cap for one clip.
pub const EVAL_CLIP_MAX_BYTES: usize = 64 * 1024 * 1024;
const EVAL_CLIP_TIMEOUT: Duration = Duration::from_secs(60);
const SCENARIO_VALUE_MAX_UTF16: usize = 1023;

/// Evaluation project: an eval `productKind` or a known eval project id.
pub fn is_evaluation_project(project_id: &str, product_kind: Option<&str>) -> bool {
    product_kind.is_some_and(|kind| EVAL_PRODUCT_KINDS.contains(&kind))
        || KNOWN_EVAL_PROJECT_IDS.contains(&project_id)
}

/// Question types the adapter answers (Babel's `taxonomy` and unknown types are refused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalQuestionType {
    Mcq,
    Likert,
    Text,
    Multiselect,
    Dropdown,
    SideBySide,
}

impl EvalQuestionType {
    fn parse(kind: &str) -> Option<Self> {
        Some(match kind {
            "mcq" => Self::Mcq,
            "likert" => Self::Likert,
            "text" => Self::Text,
            "multiselect" => Self::Multiselect,
            "dropdown" => Self::Dropdown,
            "side_by_side" => Self::SideBySide,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mcq => "mcq",
            Self::Likert => "likert",
            Self::Text => "text",
            Self::Multiselect => "multiselect",
            Self::Dropdown => "dropdown",
            Self::SideBySide => "side_by_side",
        }
    }

    /// One option at most (`mcq`, `likert`, `dropdown`, `side_by_side`).
    pub fn is_single_choice(self) -> bool {
        !matches!(self, Self::Text | Self::Multiselect)
    }

    /// Babel shows a comment box by default for every type but `text`.
    fn comments_by_default(self) -> bool {
        self != Self::Text
    }
}

/// `scenarioContext` field a scenario condition reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioField {
    Objective,
    Domain,
    Subdomain,
}

impl ScenarioField {
    const ALL: [Self; 3] = [Self::Objective, Self::Domain, Self::Subdomain];

    fn parse(field: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.key() == field)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Objective => "objective",
            Self::Domain => "domain",
            Self::Subdomain => "subdomain",
        }
    }
}

/// `metadata.showWhen`: Babel's strict schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalShowWhen {
    /// Visible iff the controlling question's answer contains any of `values`.
    Answer {
        question_id: String,
        values: Vec<String>,
    },
    /// Visible iff the normalized scenario field equals the normalized `value` (trimmed).
    Scenario { field: ScenarioField, value: String },
}

#[derive(Clone, PartialEq, Eq)]
pub struct EvalSubject {
    pub id: String,
    pub label: Option<String>,
    /// HTTPS clip URL; `None` for script subjects, which carry `transcript` instead.
    /// Signed: never logged (`Debug` redacts it).
    pub media_url: Option<String>,
    pub transcript: Option<String>,
    pub emotion: Option<String>,
}

impl EvalSubject {
    pub fn is_audio(&self) -> bool {
        self.media_url.is_some()
    }
}

impl std::fmt::Debug for EvalSubject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalSubject")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("media_url", &self.media_url.as_ref().map(|_| REDACTED))
            .field("transcript", &self.transcript)
            .field("emotion", &self.emotion)
            .finish()
    }
}

const REDACTED: &str = "<redacted>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalQuestion {
    pub id: String,
    pub label: String,
    pub caption: Option<String>,
    pub kind: EvalQuestionType,
    /// Option values (empty for `text`).
    pub values: Vec<String>,
    /// Display labels parallel to `values`; empty when Babel sends none.
    pub value_labels: Vec<String>,
    /// `metadata.required === false`.
    pub optional: bool,
    /// Whether the comment box is shown (`metadata.allowComment`, else the type default).
    pub comment_allowed: bool,
    pub show_when: Option<EvalShowWhen>,
}

impl EvalQuestion {
    /// Display label of option `index` (its value when Babel sends no labels).
    pub fn option_label(&self, index: usize) -> Option<&str> {
        self.value_labels
            .get(index)
            .or_else(|| self.values.get(index))
            .map(String::as_str)
    }
}

/// One question's answer: selected values (single choice = one element, text = the typed
/// text) and the comment box text, kept as typed (trimmed when sent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvalAnswer {
    pub values: Vec<String>,
    pub comment: Option<String>,
}

impl EvalAnswer {
    pub fn new(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            values: values.into_iter().map(Into::into).collect(),
            comment: None,
        }
    }

    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Babel's `hasAnswer`: any value, or a comment that is not blank.
    pub fn has_answer(&self) -> bool {
        !self.values.is_empty() || self.trimmed_comment().is_some()
    }

    fn trimmed_comment(&self) -> Option<&str> {
        self.comment
            .as_deref()
            .map(js_trim)
            .filter(|comment| !comment.is_empty())
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty() && self.comment.as_deref().is_none_or(str::is_empty)
    }
}

/// Task id → question id → answer.
pub type EvalAnswers = BTreeMap<String, BTreeMap<String, EvalAnswer>>;

/// Task id → subject ids fully listened to in this session.
pub type EvalListened = BTreeMap<String, BTreeSet<String>>;

/// A served eval task: the raw Babel task JSON (subjects sorted by `position`, questions by
/// `sequenceNumber`) plus its typed parts. A task that fails validation keeps its raw JSON
/// and problem but exposes no subjects or questions.
#[derive(Clone, PartialEq)]
pub struct EvalTask {
    raw: Value,
    task_id: String,
    subjects: Vec<EvalSubject>,
    questions: Vec<EvalQuestion>,
    /// Normalized `scenarioContext` values in [`ScenarioField::ALL`] order.
    scenario: [Option<String>; 3],
    problem: Option<BabelError>,
}

struct ParsedTask {
    subjects: Vec<EvalSubject>,
    questions: Vec<EvalQuestion>,
    scenario: [Option<String>; 3],
}

/// The raw task carries signed media URLs, so it is left out.
impl std::fmt::Debug for EvalTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalTask")
            .field("task_id", &self.task_id)
            .field("raw", &REDACTED)
            .field("subjects", &self.subjects)
            .field("questions", &self.questions)
            .field("scenario", &self.scenario)
            .field("problem", &self.problem)
            .finish()
    }
}

impl EvalTask {
    pub fn new(mut raw: Value) -> Self {
        sort_by_number(&mut raw, "subjects", "position");
        sort_by_number(&mut raw, "questions", "sequenceNumber");
        let task_id = raw
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match parse_task(&raw) {
            Ok(parsed) => Self {
                raw,
                task_id,
                subjects: parsed.subjects,
                questions: parsed.questions,
                scenario: parsed.scenario,
                problem: None,
            },
            Err(problem) => Self {
                raw,
                task_id,
                subjects: Vec::new(),
                questions: Vec::new(),
                scenario: Default::default(),
                problem: Some(problem),
            },
        }
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn format(&self) -> &str {
        self.raw_text("format").unwrap_or_default()
    }

    /// The sorted raw task JSON (never persisted: it carries signed media URLs).
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// A top-level string of the raw task (`formDisplayName`, `writtenForm`, `spokenForm`).
    pub fn raw_text(&self, key: &str) -> Option<&str> {
        self.raw
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    }

    pub fn subjects(&self) -> &[EvalSubject] {
        &self.subjects
    }

    /// Every question in `sequenceNumber` order, visible or not.
    pub fn questions(&self) -> &[EvalQuestion] {
        &self.questions
    }

    pub fn question(&self, question_id: &str) -> Option<&EvalQuestion> {
        self.questions
            .iter()
            .find(|question| question.id == question_id)
    }

    pub fn is_valid(&self) -> bool {
        self.problem.is_none()
    }

    /// Single-pass visibility against `answers` of this task.
    fn shows(&self, question: &EvalQuestion, answers: &BTreeMap<String, EvalAnswer>) -> bool {
        match &question.show_when {
            None => true,
            Some(EvalShowWhen::Answer {
                question_id,
                values,
            }) => answers.get(question_id).is_some_and(|answer| {
                values
                    .iter()
                    .any(|value| answer.values.iter().any(|chosen| chosen == value))
            }),
            Some(EvalShowWhen::Scenario { field, value }) => {
                self.scenario[*field as usize] == normalize_scenario(Some(value.as_str()))
            }
        }
    }

    /// This task's answers with hidden answers dropped until visibility is stable, as
    /// Babel's form does after every change.
    fn settled_answers(&self, answers: &EvalAnswers) -> BTreeMap<String, EvalAnswer> {
        let mut current = answers.get(&self.task_id).cloned().unwrap_or_default();
        self.prune(&mut current);
        current
    }

    fn prune(&self, current: &mut BTreeMap<String, EvalAnswer>) {
        loop {
            let visible: BTreeSet<&str> = self
                .questions
                .iter()
                .filter(|question| self.shows(question, current))
                .map(|question| question.id.as_str())
                .collect();
            let before = current.len();
            current.retain(|id, _| visible.contains(id.as_str()));
            if current.len() == before {
                return;
            }
        }
    }
}

/// A served session.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalSession {
    pub session_id: String,
    pub tasks: Vec<EvalTask>,
}

impl EvalSession {
    pub fn task(&self, task_id: &str) -> Option<&EvalTask> {
        self.tasks.iter().find(|task| task.task_id == task_id)
    }
}

/// `evalTasks.serveSessionTasks` answer.
#[derive(Debug, Clone, PartialEq)]
pub enum ServeOutcome {
    Session(EvalSession),
    /// No tasks now (`reason`, e.g. `all_at_consensus`).
    Empty {
        reason: String,
    },
    /// `topic_selection`, `staged_session` or any other kind this adapter cannot run.
    Unsupported {
        kind: String,
    },
}

/// `evalTasks.submitSession` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    Completed,
    /// Babel still misses answers; the session stays open.
    Incomplete,
    Other(String),
}

/// One task's `inputResponses` for `saveProgress`/`submitSession`.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalTaskResponses {
    pub task_id: String,
    pub input_responses: Vec<Value>,
}

/// `saveProgress` / `submitSession` input.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalProgress {
    pub session_id: String,
    pub tasks: Vec<EvalTaskResponses>,
}

impl EvalProgress {
    /// One checked task, for `saveProgress` when the worker finishes a form.
    pub fn for_task(
        session_id: &str,
        task: &EvalTask,
        answers: &EvalAnswers,
        listened: &BTreeSet<String>,
    ) -> Result<Self, BabelError> {
        check_task_answers(task, answers, listened)?;
        Ok(Self {
            session_id: session_id.to_string(),
            tasks: vec![task_responses(task, answers)],
        })
    }

    /// Every task of the session, each checked, for `submitSession`.
    pub fn for_session(
        session: &EvalSession,
        answers: &EvalAnswers,
        listened: &EvalListened,
    ) -> Result<Self, BabelError> {
        let empty = BTreeSet::new();
        let tasks = session
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| {
                check_task_answers(task, answers, listened.get(&task.task_id).unwrap_or(&empty))
                    .map_err(|error| {
                        BabelError::new(localized_fmt(
                            "Form {number}: {error}",
                            "Форма {number}: {error}",
                            &[
                                ("number", &(index + 1).to_string()),
                                ("error", &error.message),
                            ],
                        ))
                    })?;
                Ok(task_responses(task, answers))
            })
            .collect::<Result<Vec<_>, BabelError>>()?;
        Ok(Self {
            session_id: session.session_id.clone(),
            tasks,
        })
    }

    /// `{sessionId, tasks: [{taskId, inputResponses}]}`.
    pub fn payload(&self) -> Value {
        json!({
            "sessionId": self.session_id,
            "tasks": self.tasks.iter().map(|task| json!({
                "taskId": task.task_id,
                "inputResponses": task.input_responses,
            })).collect::<Vec<_>>(),
        })
    }
}

fn task_responses(task: &EvalTask, answers: &EvalAnswers) -> EvalTaskResponses {
    EvalTaskResponses {
        task_id: task.task_id.clone(),
        input_responses: input_responses(task, answers),
    }
}

/// The task's validation problem, if Babel served something this adapter must not answer.
pub fn validate_task(task: &EvalTask) -> Result<(), BabelError> {
    task.problem.clone().map_or(Ok(()), Err)
}

/// Questions shown for the current answers, in `sequenceNumber` order.
pub fn visible_questions<'a>(task: &'a EvalTask, answers: &EvalAnswers) -> Vec<&'a EvalQuestion> {
    let settled = task.settled_answers(answers);
    task.questions
        .iter()
        .filter(|question| task.shows(question, &settled))
        .collect()
}

/// Drops answers of questions hidden by other answers, repeatedly (a hidden controller hides
/// its dependents too).
pub fn prune_hidden_answers(task: &EvalTask, answers: &mut EvalAnswers) {
    let Some(current) = answers.get_mut(&task.task_id) else {
        return;
    };
    task.prune(current);
    if current.is_empty() {
        answers.remove(&task.task_id);
    }
}

/// Records `answer` for a visible question after checking it against the question's options
/// and comment rule, then drops answers the change hid. Empty text values are cleared; an
/// answer without values or comment removes the entry.
pub fn set_answer(
    task: &EvalTask,
    answers: &mut EvalAnswers,
    question_id: &str,
    mut answer: EvalAnswer,
) -> Result<(), BabelError> {
    validate_task(task)?;
    let question = task.question(question_id).ok_or_else(|| {
        BabelError::new(localized(
            "This question is not part of the form.",
            "Этого вопроса нет в форме.",
        ))
    })?;
    let settled = task.settled_answers(answers);
    if !task.shows(question, &settled) {
        return Err(BabelError::new(localized(
            "This question is hidden by an earlier answer.",
            "Этот вопрос скрыт предыдущим ответом.",
        )));
    }
    if question.kind == EvalQuestionType::Text {
        answer.values.retain(|value| !value.is_empty());
    }
    if answer.comment.as_deref().is_some_and(str::is_empty) {
        answer.comment = None;
    }
    validate_answer(question, &answer)?;
    let mut current = settled;
    if answer.is_empty() {
        current.remove(question_id);
    } else {
        current.insert(question_id.to_string(), answer);
    }
    task.prune(&mut current);
    if current.is_empty() {
        answers.remove(&task.task_id);
    } else {
        answers.insert(task.task_id.clone(), current);
    }
    Ok(())
}

fn validate_answer(question: &EvalQuestion, answer: &EvalAnswer) -> Result<(), BabelError> {
    if answer
        .comment
        .as_deref()
        .is_some_and(|comment| !comment.is_empty())
        && !question.comment_allowed
    {
        return Err(BabelError::new(localized(
            "This question doesn't take a comment.",
            "К этому вопросу нельзя добавить комментарий.",
        )));
    }
    let valid = match question.kind {
        EvalQuestionType::Text => {
            answer.values.len() <= 1 && answer.values.iter().all(|value| !value.is_empty())
        }
        EvalQuestionType::Multiselect => {
            let mut seen = BTreeSet::new();
            answer
                .values
                .iter()
                .all(|value| question.values.contains(value) && seen.insert(value))
        }
        _ => {
            answer.values.len() <= 1
                && answer
                    .values
                    .iter()
                    .all(|value| question.values.contains(value))
        }
    };
    if valid {
        Ok(())
    } else {
        Err(BabelError::new(localized(
            "Invalid answer value.",
            "Недопустимое значение ответа.",
        )))
    }
}

/// Whether the task may be saved or submitted: it is valid, every clip in `listened`, every
/// visible answer fits its question, and every visible required question has a value.
pub fn check_task_answers(
    task: &EvalTask,
    answers: &EvalAnswers,
    listened: &BTreeSet<String>,
) -> Result<(), BabelError> {
    validate_task(task)?;
    if task
        .subjects
        .iter()
        .any(|subject| subject.is_audio() && !listened.contains(&subject.id))
    {
        return Err(BabelError::new(localized(
            "Listen to every clip fully before continuing.",
            "Прослушайте каждый клип полностью, прежде чем продолжить.",
        )));
    }
    let settled = task.settled_answers(answers);
    let visible: Vec<&EvalQuestion> = task
        .questions
        .iter()
        .filter(|question| task.shows(question, &settled))
        .collect();
    if visible.is_empty() {
        return Err(BabelError::new(localized(
            "This form has no questions to answer.",
            "В этой форме нет вопросов для ответа.",
        )));
    }
    for question in visible {
        let answer = settled.get(&question.id);
        if let Some(answer) = answer {
            validate_answer(question, answer)?;
        }
        if !question.optional && answer.is_none_or(|answer| answer.values.is_empty()) {
            return Err(BabelError::new(localized(
                "Answer every required question before continuing.",
                "Ответьте на все обязательные вопросы, прежде чем продолжить.",
            )));
        }
    }
    Ok(())
}

/// Babel's `inputResponses` for the task: every visible question that is required or
/// answered (`hasAnswer`), as `{formInputId, value: values.join(","), freetextValue}` plus
/// `selectedValues` for multiselect. Does not check the answers: use [`EvalProgress`].
pub fn input_responses(task: &EvalTask, answers: &EvalAnswers) -> Vec<Value> {
    let settled = task.settled_answers(answers);
    task.questions
        .iter()
        .filter(|question| task.shows(question, &settled))
        .filter(|question| {
            !question.optional
                || settled
                    .get(&question.id)
                    .is_some_and(EvalAnswer::has_answer)
        })
        .map(|question| build_response(question, settled.get(&question.id)))
        .collect()
}

fn build_response(question: &EvalQuestion, answer: Option<&EvalAnswer>) -> Value {
    let values = answer.map_or(&[][..], |answer| answer.values.as_slice());
    let mut response = Map::new();
    response.insert("formInputId".into(), json!(question.id));
    response.insert("value".into(), json!(values.join(",")));
    response.insert(
        "freetextValue".into(),
        json!(answer.and_then(EvalAnswer::trimmed_comment)),
    );
    if question.kind == EvalQuestionType::Multiselect {
        response.insert("selectedValues".into(), json!(values));
    }
    Value::Object(response)
}

// ---------------------------------------------------------------------------------------
// Task parsing and validation
// ---------------------------------------------------------------------------------------

fn sort_by_number(raw: &mut Value, list: &str, key: &str) {
    if let Some(items) = raw.get_mut(list).and_then(Value::as_array_mut) {
        items.sort_by(|left, right| {
            let number = |item: &Value| item.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            number(left).total_cmp(&number(right))
        });
    }
}

fn unsupported_format() -> BabelError {
    BabelError::new(localized(
        "This evaluation task has an unsupported format.",
        "Формат этого задания оценки не поддерживается.",
    ))
}

fn incomplete_clips() -> BabelError {
    BabelError::new(localized(
        "Babel returned incomplete clip data for this task.",
        "Babel вернул неполные данные клипов для этого задания.",
    ))
}

fn unsupported_question() -> BabelError {
    BabelError::new(localized(
        "This form has a question Exaudio can't answer yet.",
        "В этой форме есть вопрос, на который Exaudio пока не умеет отвечать.",
    ))
}

/// A non-empty list (or any other non-null value) where Babel expects an optional list.
fn has_entries(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Array(items)) => !items.is_empty(),
        Some(_) => true,
    }
}

fn optional_text(value: &Value, key: &str) -> Result<Option<String>, ()> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(()),
    }
}

fn string_list(value: &Value, key: &str) -> Result<Vec<String>, ()> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str() {
                Some(text) if !text.is_empty() => Ok(text.to_string()),
                _ => Err(()),
            })
            .collect(),
        Some(_) => Err(()),
    }
}

fn parse_task(raw: &Value) -> Result<ParsedTask, BabelError> {
    let task_id = raw
        .get("taskId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let format = raw
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let subjects = raw.get("subjects").and_then(Value::as_array);
    let questions = raw.get("questions").and_then(Value::as_array);
    let (Some(subjects), Some(questions)) = (subjects, questions) else {
        return Err(unsupported_format());
    };
    if task_id.trim().is_empty()
        || !SUPPORTED_EVAL_FORMATS.contains(&format)
        || subjects.is_empty()
        || questions.is_empty()
    {
        return Err(unsupported_format());
    }
    if has_entries(raw.get("formLinters")) {
        return Err(BabelError::new(localized(
            "This form needs extra checks Exaudio can't run yet.",
            "Эта форма требует дополнительных проверок, которые Exaudio пока не умеет выполнять.",
        )));
    }
    let subjects = parse_subjects(subjects)?;
    let audio_count = subjects.iter().filter(|subject| subject.is_audio()).count();
    if audio_count == 0 || (format == "tts-naturalness" && audio_count != 2) {
        return Err(BabelError::new(localized(
            "Audio clips are missing from this task.",
            "В задании не хватает аудиоклипов.",
        )));
    }
    let questions = parse_questions(questions)?;
    Ok(ParsedTask {
        subjects,
        questions,
        scenario: parse_scenario(raw.get("scenarioContext"))?,
    })
}

fn parse_subjects(subjects: &[Value]) -> Result<Vec<EvalSubject>, BabelError> {
    let mut ids = BTreeSet::new();
    subjects
        .iter()
        .map(|subject| {
            let id = subject
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(incomplete_clips)?;
            // Defect playback and multi-track media use Babel players this adapter lacks.
            let extra_media = subject.get("defectPlayback").is_some_and(|v| !v.is_null())
                || has_entries(subject.get("mediaTracks"));
            let text = |key: &str| optional_text(subject, key).map_err(|()| incomplete_clips());
            let media_url = text("mediaUrl")?;
            let transcript = text("transcript")?;
            let secure = media_url.as_deref().is_none_or(|url| {
                url::Url::parse(url)
                    .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
            });
            if !ids.insert(id)
                || extra_media
                || !secure
                || (media_url.is_none() && transcript.is_none())
            {
                return Err(incomplete_clips());
            }
            Ok(EvalSubject {
                id: id.to_string(),
                label: text("label")?,
                media_url,
                transcript,
                emotion: text("emotion")?,
            })
        })
        .collect()
}

fn parse_questions(questions: &[Value]) -> Result<Vec<EvalQuestion>, BabelError> {
    let mut ids = BTreeSet::new();
    questions
        .iter()
        .map(|question| {
            let kind = question
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if kind == "taxonomy" {
                return Err(BabelError::new(localized(
                    "Taxonomy questions aren't supported yet.",
                    "Вопросы-таксономии пока не поддерживаются.",
                )));
            }
            let kind = EvalQuestionType::parse(kind).ok_or_else(unsupported_question)?;
            let id = question
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(unsupported_question)?;
            let label = question
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
                .ok_or_else(unsupported_question)?;
            let values = string_list(question, "values").map_err(|()| unsupported_question())?;
            let value_labels =
                string_list(question, "valueLabels").map_err(|()| unsupported_question())?;
            let caption =
                optional_text(question, "caption").map_err(|()| unsupported_question())?;
            if !ids.insert(id)
                || (kind != EvalQuestionType::Text && values.is_empty())
                || (!value_labels.is_empty() && value_labels.len() != values.len())
                || has_entries(question.get("linters"))
                // Babel answers flag-entry questions with per-clip defect answers.
                || question
                    .get("metadata")
                    .and_then(|metadata| metadata.get("flagEntry"))
                    .is_some_and(|flag| !flag.is_null())
            {
                return Err(unsupported_question());
            }
            // Babel reads metadata only when it is a plain object.
            let metadata = question.get("metadata").and_then(Value::as_object);
            let show_when = match metadata.and_then(|metadata| metadata.get("showWhen")) {
                None => None,
                Some(rule) => Some(parse_show_when(rule).ok_or_else(|| {
                    BabelError::new(localized(
                        "A question in this form has an invalid visibility rule.",
                        "У вопроса в этой форме неверное правило видимости.",
                    ))
                })?),
            };
            let optional = metadata
                .and_then(|metadata| metadata.get("required"))
                .is_some_and(|required| required == &Value::Bool(false));
            let comment_allowed = metadata
                .and_then(|metadata| metadata.get("allowComment"))
                .and_then(Value::as_bool)
                .unwrap_or_else(|| kind.comments_by_default());
            Ok(EvalQuestion {
                id: id.to_string(),
                label: label.to_string(),
                caption,
                kind,
                values,
                value_labels,
                optional,
                comment_allowed,
                show_when,
            })
        })
        .collect()
}

/// Babel's strict `showWhen` union; anything else makes Babel throw.
fn parse_show_when(rule: &Value) -> Option<EvalShowWhen> {
    let rule = rule.as_object()?;
    let only = |allowed: &[&str]| rule.keys().all(|key| allowed.contains(&key.as_str()));
    match rule.get("kind")?.as_str()? {
        "answer" if only(&["kind", "questionId", "values"]) => {
            let question_id = rule.get("questionId")?.as_str()?;
            let values = rule
                .get("values")?
                .as_array()?
                .iter()
                .map(|value| value.as_str().filter(|value| !value.is_empty()))
                .map(|value| value.map(str::to_string))
                .collect::<Option<Vec<_>>>()?;
            (is_zod_uuid(question_id) && !values.is_empty()).then(|| EvalShowWhen::Answer {
                question_id: question_id.to_string(),
                values,
            })
        }
        "scenario" if only(&["kind", "field", "value"]) => {
            let field = ScenarioField::parse(rule.get("field")?.as_str()?)?;
            let value = js_trim(rule.get("value")?.as_str()?);
            (1..=SCENARIO_VALUE_MAX_UTF16)
                .contains(&value.encode_utf16().count())
                .then(|| EvalShowWhen::Scenario {
                    field,
                    value: value.to_string(),
                })
        }
        _ => None,
    }
}

fn parse_scenario(context: Option<&Value>) -> Result<[Option<String>; 3], BabelError> {
    let invalid = || {
        BabelError::new(localized(
            "Babel returned an unreadable task scenario.",
            "Babel вернул нечитаемый сценарий задания.",
        ))
    };
    let context = match context {
        None | Some(Value::Null) => return Ok(Default::default()),
        Some(Value::Object(context)) => context,
        Some(_) => return Err(invalid()),
    };
    let mut scenario: [Option<String>; 3] = Default::default();
    for (slot, field) in scenario.iter_mut().zip(ScenarioField::ALL) {
        *slot = match context.get(field.key()) {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => normalize_scenario(Some(text.as_str())),
            Some(_) => return Err(invalid()),
        };
    }
    Ok(scenario)
}

/// `x?.trim().toLowerCase() || null`.
fn normalize_scenario(value: Option<&str>) -> Option<String> {
    value
        .map(|value| js_trim(value).to_lowercase())
        .filter(|value| !value.is_empty())
}

/// JavaScript `String.prototype.trim` whitespace (adds U+FEFF, excludes U+0085).
fn is_js_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// Zod 4 `z.string().uuid()`: RFC 9562 versions 1-8 with the RFC variant, or nil/max.
fn is_zod_uuid(id: &str) -> bool {
    if id == "00000000-0000-0000-0000-000000000000" || id == "ffffffff-ffff-ffff-ffff-ffffffffffff"
    {
        return true;
    }
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            14 => (b'1'..=b'8').contains(byte),
            19 => b"89abAB".contains(byte),
            _ => byte.is_ascii_hexdigit(),
        })
}

// ---------------------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------------------

/// The tRPC data of an eval reply; an in-band tRPC error becomes the error.
fn eval_value(payload: &str) -> Result<Value, BabelError> {
    let unreadable = || {
        BabelError::new(localized(
            "Babel sent an unreadable reply.",
            "Babel прислал нечитаемый ответ.",
        ))
    };
    let trimmed = payload.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err(unreadable());
    }
    let mut value: Value = serde_json::from_str(trimmed).map_err(|_| unreadable())?;
    if let Value::Array(items) = &mut value
        && items.len() == 1
    {
        value = items.remove(0);
    }
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let detail = error
            .as_str()
            .map(str::to_string)
            .or_else(|| first_string_by_key(error, &["message"]))
            .unwrap_or_default();
        return Err(BabelError::new(with_detail(
            localized("Babel refused the request", "Babel отклонил запрос"),
            detail,
        )));
    }
    if value.get("result").is_some() {
        return trpc_data(&value).ok_or_else(unreadable);
    }
    Ok(value)
}

fn reply_kind(value: &Value) -> Option<&str> {
    value.get("kind").and_then(Value::as_str)
}

pub(super) fn parse_serve_outcome(payload: &str) -> Result<ServeOutcome, BabelError> {
    let value = eval_value(payload)?;
    let incomplete = || {
        BabelError::new(localized(
            "Babel returned an incomplete session.",
            "Babel вернул неполную сессию.",
        ))
    };
    match reply_kind(&value) {
        Some("session") => {
            let session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(incomplete)?;
            let tasks = value
                .get("tasks")
                .and_then(Value::as_array)
                .filter(|tasks| !tasks.is_empty())
                .ok_or_else(incomplete)?;
            let mut task_ids = BTreeSet::new();
            let ids_ok = tasks.iter().all(|task| {
                task.get("taskId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty() && task_ids.insert(id))
            });
            if !ids_ok {
                return Err(incomplete());
            }
            Ok(ServeOutcome::Session(EvalSession {
                session_id: session_id.to_string(),
                tasks: tasks.iter().cloned().map(EvalTask::new).collect(),
            }))
        }
        Some("empty") => Ok(ServeOutcome::Empty {
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        Some(kind) => Ok(ServeOutcome::Unsupported {
            kind: kind.to_string(),
        }),
        None => Err(incomplete()),
    }
}

pub(super) fn parse_hold(payload: &str) -> Result<bool, BabelError> {
    eval_value(payload)?.as_bool().ok_or_else(|| {
        BabelError::new(localized(
            "Babel sent an unexpected reply while keeping the session open.",
            "Babel прислал неожиданный ответ при продлении сессии.",
        ))
    })
}

/// `{kind: "saved"}` → true; any other kind → false (Babel did not confirm the save).
pub(super) fn parse_save_progress(payload: &str) -> Result<bool, BabelError> {
    Ok(reply_kind(&eval_value(payload)?) == Some("saved"))
}

pub(super) fn parse_submit_outcome(payload: &str) -> Result<SubmitOutcome, BabelError> {
    let value = eval_value(payload)?;
    match reply_kind(&value) {
        Some("completed") => Ok(SubmitOutcome::Completed),
        Some("incomplete") => Ok(SubmitOutcome::Incomplete),
        Some(kind) => Ok(SubmitOutcome::Other(kind.to_string())),
        None => Err(BabelError::new(localized(
            "Babel did not confirm the submission. Your answers are kept.",
            "Babel не подтвердил отправку. Ваши ответы сохранены.",
        ))),
    }
}

// ---------------------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------------------

/// Downloads a clip (HTTPS only, 60 s, 64 MiB) and decodes it off the async thread.
/// Error messages never include the URL (it is signed).
pub async fn fetch_eval_clip(url: &str) -> Result<DecodedAudio, BabelError> {
    let url = url::Url::parse(url)
        .ok()
        .filter(|url| url.scheme() == "https")
        .ok_or_else(|| {
            BabelError::new(localized(
                "Clip audio needs an HTTPS link.",
                "Для аудио клипа нужна HTTPS-ссылка.",
            ))
        })?;
    let bytes = tokio::time::timeout(EVAL_CLIP_TIMEOUT, download_clip(url))
        .await
        .map_err(|_| {
            BabelError::new(localized(
                "Clip download timed out. Try again.",
                "Загрузка клипа не уложилась во время. Попробуйте снова.",
            ))
        })??;
    tokio::task::spawn_blocking(move || decode_eval_clip(&bytes))
        .await
        .map_err(|error| {
            BabelError::new(with_detail(
                localized(
                    "Clip decoding failed. Your answers are kept.",
                    "Не удалось декодировать клип. Ваши ответы сохранены.",
                ),
                error,
            ))
        })?
}

/// Clip client: refuses redirects to anything but HTTPS before following them.
static CLIP_CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("clip redirect to a non-HTTPS link")
            } else if attempt.previous().len() >= 10 {
                attempt.error("too many clip redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())
});

async fn download_clip(url: url::Url) -> Result<Vec<u8>, BabelError> {
    let failed = |error: reqwest::Error| {
        BabelError::new(with_detail(
            if error.is_redirect() {
                localized(
                    "Clip audio needs an HTTPS link.",
                    "Для аудио клипа нужна HTTPS-ссылка.",
                )
            } else {
                localized(
                    "Clip download failed. Try again.",
                    "Не удалось загрузить клип. Попробуйте снова.",
                )
            },
            error.without_url(),
        ))
    };
    let too_large = || {
        BabelError::new(localized(
            "Clip audio exceeds the 64 MB limit.",
            "Аудио клипа больше 64 МБ.",
        ))
    };
    let client = CLIP_CLIENT.as_ref().map_err(|error| {
        BabelError::new(with_detail(
            localized(
                "Clip download failed. Try again.",
                "Не удалось загрузить клип. Попробуйте снова.",
            ),
            error,
        ))
    })?;
    let mut response = client.get(url).send().await.map_err(failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(BabelError::new(with_detail(
            localized(
                "The clip link is unavailable or expired. Reload the session.",
                "Ссылка на клип недоступна или устарела. Перезагрузите сессию.",
            ),
            format!("HTTP {status}"),
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > EVAL_CLIP_MAX_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        if bytes.len() + chunk.len() > EVAL_CLIP_MAX_BYTES {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Fork-observed bounds for one clip: 32 M decoded samples, 8 channels, 384 kHz.
pub const EVAL_CLIP_DECODE_LIMITS: DecodeLimits = DecodeLimits {
    max_samples: 32_000_000,
    max_channels: 8,
    max_sample_rate: 384_000,
};

/// Decodes clip bytes by content within [`EVAL_CLIP_DECODE_LIMITS`]; truncated or empty
/// audio is an error (it could never be listened to fully).
pub fn decode_eval_clip(bytes: &[u8]) -> Result<DecodedAudio, BabelError> {
    let audio = exaudio_audio::decode_audio_bytes_limited(bytes, EVAL_CLIP_DECODE_LIMITS)
        .map_err(|error| {
        let headline = match &error {
            AudioError::Decode(AudioDecodeError::Truncated(_)) => localized(
                "The clip is incomplete. Retry downloading it.",
                "Клип загружен не полностью. Загрузите его снова.",
            ),
            _ => localized(
                "Can't decode this clip (unsupported or damaged). Your answers are kept.",
                "Не удаётся декодировать клип (формат не поддерживается или файл повреждён). Ваши ответы сохранены.",
            ),
        };
        BabelError::new(with_detail(headline, error))
        })?;
    if audio.frame_count() == 0 {
        return Err(BabelError::new(localized(
            "The clip has no audio.",
            "В клипе нет звука.",
        )));
    }
    Ok(audio)
}

// ---------------------------------------------------------------------------------------
// Local draft
// ---------------------------------------------------------------------------------------

const DRAFT_VERSION: u32 = 1;

/// Local answer drafts, one file per project (`eval-<project>.json` plus
/// `eval-<project>.backup.json`). Only answers are stored, never task JSON or media URLs.
/// A main file that cannot be read is never overwritten: the next save moves it aside to
/// `eval-<project>.corrupt-<unix millis>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalDraftStore {
    dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftFile {
    version: u32,
    project_id: String,
    session_id: String,
    answers: EvalAnswers,
}

/// The 0.1.16 fork's draft: one string answer per question.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyDraftFile {
    session_id: String,
    #[serde(default)]
    answers: BTreeMap<String, BTreeMap<String, String>>,
}

enum DraftRead {
    Missing,
    Unreadable(String),
    /// `None` when the draft belongs to another project.
    Found(Option<(String, EvalAnswers)>),
}

fn read_draft(path: &std::path::Path, project_id: &str) -> DraftRead {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return DraftRead::Missing,
        Err(error) => return DraftRead::Unreadable(error.to_string()),
    };
    if let Ok(draft) = serde_json::from_slice::<DraftFile>(&bytes) {
        if draft.version != DRAFT_VERSION {
            return DraftRead::Unreadable(format!("unknown draft version {}", draft.version));
        }
        return DraftRead::Found(
            (draft.project_id == project_id).then_some((draft.session_id, draft.answers)),
        );
    }
    match serde_json::from_slice::<LegacyDraftFile>(&bytes) {
        Ok(legacy) => {
            let answers = legacy
                .answers
                .into_iter()
                .map(|(task_id, questions)| {
                    let questions = questions
                        .into_iter()
                        .filter(|(_, value)| !value.is_empty())
                        .map(|(question_id, value)| (question_id, EvalAnswer::new([value])))
                        .collect();
                    (task_id, questions)
                })
                .collect();
            DraftRead::Found(Some((legacy.session_id, answers)))
        }
        Err(error) => DraftRead::Unreadable(error.to_string()),
    }
}

impl EvalDraftStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn paths(&self, project_id: &str) -> Option<(PathBuf, PathBuf)> {
        let safe = !project_id.is_empty()
            && project_id.len() <= 128
            && project_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        safe.then(|| {
            (
                self.dir.join(format!("eval-{project_id}.json")),
                self.dir.join(format!("eval-{project_id}.backup.json")),
            )
        })
    }

    /// The saved answers of `session_id` (from the backup when the main file is missing or
    /// unreadable); empty when there is none or the draft belongs to another session or
    /// project. Fails when a draft file exists but neither copy can be read.
    pub fn load(&self, project_id: &str, session_id: &str) -> Result<EvalAnswers, BabelError> {
        let Some((main, backup)) = self.paths(project_id) else {
            return Ok(EvalAnswers::new());
        };
        let found = match (
            read_draft(&main, project_id),
            read_draft(&backup, project_id),
        ) {
            (DraftRead::Found(found), _) | (_, DraftRead::Found(found)) => found,
            (DraftRead::Missing, DraftRead::Missing) => None,
            (DraftRead::Unreadable(detail), _) | (_, DraftRead::Unreadable(detail)) => {
                return Err(BabelError::new(with_detail(
                    localized(
                        "Couldn't read the local answer draft. It won't be overwritten; this session starts without it.",
                        "Не удалось прочитать локальный черновик ответов. Он не будет перезаписан; сессия начнётся без него.",
                    ),
                    detail,
                )));
            }
        };
        Ok(found
            .filter(|(saved_session, _)| saved_session == session_id)
            .map(|(_, answers)| answers)
            .unwrap_or_default())
    }

    /// Atomically replaces the project's draft (temp file + rename), keeping the previous
    /// file as the backup; an unreadable previous file is moved aside instead.
    pub fn save(
        &self,
        project_id: &str,
        session_id: &str,
        answers: &EvalAnswers,
    ) -> Result<(), BabelError> {
        let failed = |detail: String| {
            BabelError::new(with_detail(
                localized(
                    "Couldn't save the local answer draft",
                    "Не удалось сохранить локальный черновик ответов",
                ),
                detail,
            ))
        };
        let (main, backup) = self
            .paths(project_id)
            .ok_or_else(|| failed(format!("invalid project id {project_id:?}")))?;
        std::fs::create_dir_all(&self.dir).map_err(|error| failed(error.to_string()))?;
        let draft = DraftFile {
            version: DRAFT_VERSION,
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
            answers: answers.clone(),
        };
        let bytes = serde_json::to_vec(&draft).map_err(|error| failed(error.to_string()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.dir)
            .map_err(|error| failed(error.to_string()))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| failed(error.to_string()))?;
        match read_draft(&main, project_id) {
            DraftRead::Missing => {}
            DraftRead::Found(_) => {
                std::fs::copy(&main, &backup).map_err(|error| failed(error.to_string()))?;
            }
            DraftRead::Unreadable(_) => {
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_millis());
                let aside = self
                    .dir
                    .join(format!("eval-{project_id}.corrupt-{millis}.json"));
                std::fs::rename(&main, &aside).map_err(|error| failed(error.to_string()))?;
            }
        }
        temporary
            .persist(&main)
            .map_err(|error| failed(error.error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zod_uuid_accepts_versioned_nil_and_max_only() {
        assert!(is_zod_uuid("d35d73aa-4816-4571-b77d-960ddb15eeae"));
        assert!(is_zod_uuid("D35D73AA-4816-4571-B77D-960DDB15EEAE"));
        assert!(is_zod_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(is_zod_uuid("ffffffff-ffff-ffff-ffff-ffffffffffff"));
        assert!(!is_zod_uuid("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"));
        assert!(!is_zod_uuid("d35d73aa-4816-0571-b77d-960ddb15eeae"));
        assert!(!is_zod_uuid("d35d73aa-4816-4571-c77d-960ddb15eeae"));
        assert!(!is_zod_uuid("correct"));
    }

    #[test]
    fn js_trim_matches_javascript_whitespace() {
        assert_eq!(js_trim("\u{feff} Phone \u{a0}\n"), "Phone");
        assert_eq!(js_trim("\u{85}x\u{85}"), "\u{85}x\u{85}");
        assert_eq!(normalize_scenario(Some("   ")), None);
        assert_eq!(
            normalize_scenario(Some(" PHONE ")).as_deref(),
            Some("phone")
        );
    }

    #[test]
    fn in_band_trpc_errors_surface_and_data_is_unwrapped() {
        let error = eval_value(
            r#"[{"error":{"json":{"message":"Session not found","data":{"code":"NOT_FOUND"}}}}]"#,
        )
        .expect_err("error");
        assert!(error.message.contains("Session not found"));
        assert_eq!(
            eval_value(r#"[{"result":{"data":{"json":{"kind":"saved"}}}}]"#).expect("data"),
            json!({"kind": "saved"})
        );
        assert_eq!(eval_value("true").expect("bool"), json!(true));
        assert!(eval_value("").is_err());
    }
}
