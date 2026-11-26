//! Babel audio-evaluation sessions (projects whose `productKind` is `eval_task` /
//! `eval_session`): serve a session (`evalTasks.serveSessionTasks`), play every clip to its
//! end, answer each form's visible questions, save each form (`saveProgress`) and submit the
//! whole session (`submitSession`), holding the session every three minutes while it is open.
//! Answers are kept in a local draft after every change, so no error ever drops them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use exaudio_audio::{AudioEngine, DecodedAudio, PlaybackEvent, PlaybackState};
use exaudio_babel_services::babel::BabelClient;
use exaudio_babel_services::babel::eval::{
    EvalAnswer, EvalAnswers, EvalDraftStore, EvalListened, EvalProgress, EvalQuestion,
    EvalQuestionType, EvalSession, EvalSubject, EvalTask, ServeOutcome, SubmitOutcome,
    fetch_eval_clip, prune_hidden_answers, set_answer, validate_task, visible_questions,
};
use exaudio_editor::edit::format::format_time;
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header};
use exaudio_shell::screens::ui::{self, Tone};
use exaudio_shell::services::{spawn_service, spawn_service_in};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::project_resources::ProjectResourcesView;
use crate::text::{failed, localized, localized_fmt};

const LANE: &str = "clip";
/// Babel releases an idle session; the dashboard holds it this often.
const HOLD_INTERVAL: Duration = Duration::from_secs(180);
const TICK: Duration = Duration::from_millis(250);
/// A longer gap between ticks means playback was paused or the machine slept: that listen
/// does not count.
const MAX_TICK_GAP: Duration = Duration::from_secs(3);

/// How the screen is opened (from the dashboard).
pub struct EvalOpen {
    /// Client for the `evalTasks.*` calls (the local relay's owner session when it runs).
    pub client: BabelClient,
    /// Client for the project's instructions, grading guide and library.
    pub resources_client: BabelClient,
    pub project_id: String,
    pub project_name: String,
}

pub enum EvalEvent {
    /// Back to the projects.
    Leave,
}

impl EventEmitter<EvalEvent> for EvalScreen {}

/// The one Babel request in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Serve,
    Hold,
    Save,
    Submit,
}

struct InFlight {
    kind: RequestKind,
    _task: Task<()>,
}

/// A subject's clip in the current form.
enum Clip {
    Loading,
    Ready(Arc<DecodedAudio>),
    /// Download or decode failed; the message (never the signed URL).
    Failed(SharedString),
}

/// Playback of one clip, watched for a complete listen.
struct Listening {
    subject_id: String,
    last_beat: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenStep {
    Playing,
    /// Playback reached the clip end without a gap.
    Listened,
    /// Ticks stopped for longer than [`MAX_TICK_GAP`] (pause, sleep, suspend).
    Interrupted,
}

impl Listening {
    fn new(subject_id: String, now: Instant) -> Self {
        Self {
            subject_id,
            last_beat: now,
        }
    }

    /// One tick: `ended` is whether playback reached the clip end since the last tick. A gap
    /// wins over the end, so a resumed machine never counts as having listened.
    fn step(&mut self, now: Instant, ended: bool) -> ListenStep {
        if now.saturating_duration_since(self.last_beat) > MAX_TICK_GAP {
            return ListenStep::Interrupted;
        }
        self.last_beat = now;
        if ended {
            ListenStep::Listened
        } else {
            ListenStep::Playing
        }
    }
}

/// "A", "B", … "Z", then "27", "28", … for the `index`-th audio subject.
fn clip_letter(index: usize) -> String {
    match u8::try_from(index) {
        Ok(index) if index < 26 => char::from(b'A' + index).to_string(),
        _ => (index + 1).to_string(),
    }
}

struct ActiveSession {
    session: EvalSession,
    index: usize,
    /// `holdSession` answered false: nothing is sent until the session is reloaded.
    expired: bool,
}

pub struct EvalScreen {
    client: BabelClient,
    project_id: String,
    project_name: String,
    drafts: EvalDraftStore,
    session: Option<ActiveSession>,
    answers: EvalAnswers,
    /// Session the `listened` marks belong to (a resumed session keeps them).
    listened_session: Option<String>,
    listened: EvalListened,
    request: Option<InFlight>,
    last_hold: Instant,
    clips: BTreeMap<String, Clip>,
    clip_task: Option<Task<()>>,
    engine: AudioEngine,
    playing: Option<Listening>,
    text_inputs: BTreeMap<String, Entity<TextareaState>>,
    comment_inputs: BTreeMap<String, Entity<TextareaState>>,
    input_subscriptions: Vec<Subscription>,
    notice: Option<SharedString>,
    error: Option<SharedString>,
    draft_error: Option<SharedString>,
    visible: bool,
    show_resources: bool,
    resources: Entity<ProjectResourcesView>,
    focus_handle: FocusHandle,
    _ticker: Task<()>,
}

impl EvalScreen {
    /// Starts hidden and idle: [`Self::show`] displays it and loads a session.
    pub fn new(open: EvalOpen, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let EvalOpen {
            client,
            resources_client,
            project_id,
            project_name,
        } = open;
        let resources = {
            let project_id = project_id.clone();
            cx.new(|cx| ProjectResourcesView::new(project_id, resources_client, window, cx))
        };
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            drafts: Self::draft_store(&client),
            client,
            project_id,
            project_name,
            session: None,
            answers: EvalAnswers::new(),
            listened_session: None,
            listened: EvalListened::new(),
            request: None,
            last_hold: Instant::now(),
            clips: BTreeMap::new(),
            clip_task: None,
            engine: AudioEngine::new(),
            playing: None,
            text_inputs: BTreeMap::new(),
            comment_inputs: BTreeMap::new(),
            input_subscriptions: Vec::new(),
            notice: None,
            error: None,
            draft_error: None,
            visible: false,
            show_resources: true,
            resources,
            focus_handle: cx.focus_handle(),
            _ticker: ticker,
        }
    }

    /// Drafts live in the account directory (a temp directory when the client has none).
    fn draft_store(client: &BabelClient) -> EvalDraftStore {
        EvalDraftStore::new(
            client
                .config()
                .data_dir
                .clone()
                .unwrap_or_else(|| std::env::temp_dir().join("exaudio-babel-eval")),
        )
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// A served session that is still held (not expired, not submitted).
    fn session_active(&self) -> bool {
        self.session.as_ref().is_some_and(|active| !active.expired)
    }

    /// Opening another project must wait: a session is open or a request/download runs.
    pub fn blocks_switch(&self) -> bool {
        self.session_active() || self.request.is_some() || self.clip_task.is_some()
    }

    /// Shown again from the dashboard: loads a session unless one is open. `client` replaces
    /// the transport only when nothing is open or in flight.
    pub fn show(&mut self, client: BabelClient, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = true;
        if !self.session_active() && self.request.is_none() {
            self.drafts = Self::draft_store(&client);
            self.client = client;
            self.load_session(window, cx);
        }
        cx.notify();
    }

    /// The worker tried to open `other` while this session is open.
    pub fn refuse_switch(&mut self, other: &str, cx: &mut Context<Self>) {
        self.error = Some(
            localized_fmt(
                cx,
                "Finish or submit this session before opening {other}.",
                "Завершите или отправьте эту сессию, прежде чем открывать {other}.",
                &[("other", other)],
            )
            .into(),
        );
        cx.notify();
    }

    /// Left for the dashboard: stops and releases the audio. The session and answers stay.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.stop_audio();
        self.engine.unload();
        self.clip_task = None;
        self.clips.retain(|_, clip| !matches!(clip, Clip::Loading));
        cx.notify();
    }

    /// Runs `on_confirmed` now, or after the worker agrees to leave an unsubmitted session.
    pub fn confirm_leave(
        &self,
        window: &mut Window,
        cx: &mut App,
        on_confirmed: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        if !self.session_active() {
            window.defer(cx, on_confirmed);
            return;
        }
        let on_confirmed = std::rc::Rc::new(std::cell::Cell::new(Some(on_confirmed)));
        window.open_dialog(cx, move |dialog, window, cx| {
            let on_confirmed = on_confirmed.clone();
            dialog.w(rems(27.5).to_pixels(window.rem_size())).child(
                v_flex()
                    .gap_3()
                    .child(dialog_header(
                        localized(
                            cx,
                            "Leave the unsubmitted session?",
                            "Выйти из неотправленной сессии?",
                        ),
                        Some(
                            localized(
                                cx,
                                "Your answers stay saved on this computer. Babel may release the session while you are away; then reload it.",
                                "Ваши ответы останутся на этом компьютере. Пока вас нет, Babel может освободить сессию — тогда загрузите её снова.",
                            )
                            .into(),
                        ),
                        cx,
                    ))
                    .child(
                        dialog_footer()
                            .child(
                                Button::new("eval-stay")
                                    .label(localized(cx, "Keep evaluating", "Продолжить оценку"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("eval-leave")
                                    .danger()
                                    .label(localized(cx, "Leave", "Выйти"))
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        if let Some(confirmed) = on_confirmed.take() {
                                            window.defer(cx, confirmed);
                                        }
                                    }),
                            ),
                    ),
            )
        });
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        self.confirm_leave(window, cx, move |_, cx| {
            this.update(cx, |_, cx| cx.emit(EvalEvent::Leave)).ok();
        });
    }

    fn current_task(&self) -> Option<&EvalTask> {
        let active = self.session.as_ref()?;
        active.session.tasks.get(active.index)
    }

    fn current_task_id(&self) -> Option<String> {
        self.current_task().map(|task| task.task_id().to_string())
    }

    /// Answer edits are refused while a session load/save/submit runs or after expiry.
    fn answers_locked(&self) -> bool {
        self.session.as_ref().is_none_or(|active| active.expired)
            || self
                .request
                .as_ref()
                .is_some_and(|request| request.kind != RequestKind::Hold)
    }

    // ---- session requests ---------------------------------------------------------------

    fn load_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.is_some() {
            return;
        }
        if self.session_active() {
            self.error = Some(
                localized(
                    cx,
                    "Finish the current session before loading another.",
                    "Завершите текущую сессию, прежде чем загружать другую.",
                )
                .into(),
            );
            cx.notify();
            return;
        }
        self.stop_audio();
        self.error = None;
        self.notice = None;
        let client = self.client.clone();
        let project_id = self.project_id.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { client.serve_eval_session(&project_id).await },
            |this, result, window, cx| {
                this.request = None;
                match result {
                    Ok(outcome) => this.session_served(outcome, window, cx),
                    Err(error) => this.error = Some(error.message.into()),
                }
                cx.notify();
            },
        );
        self.request = Some(InFlight {
            kind: RequestKind::Serve,
            _task: task,
        });
        cx.notify();
    }

    fn session_served(
        &mut self,
        outcome: ServeOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = match outcome {
            ServeOutcome::Session(session) => session,
            ServeOutcome::Empty { reason } => {
                self.notice = Some(if reason == "all_at_consensus" {
                    localized(
                        cx,
                        "No tasks right now: all current recordings already have enough evaluations.",
                        "Сейчас задач нет: все текущие записи уже получили достаточно оценок.",
                    )
                    .into()
                } else {
                    localized_fmt(
                        cx,
                        "No tasks right now ({reason}).",
                        "Сейчас задач нет ({reason}).",
                        &[("reason", &reason)],
                    )
                    .into()
                });
                return;
            }
            ServeOutcome::Unsupported { kind } => {
                self.error = Some(
                    localized_fmt(
                        cx,
                        "Babel offered a \"{kind}\" session, which Exaudio can't run. Use the Babel website for it.",
                        "Babel предложил сессию типа «{kind}», которую Exaudio не поддерживает. Выполните её на сайте Babel.",
                        &[("kind", &kind)],
                    )
                    .into(),
                );
                return;
            }
        };
        if session.tasks.is_empty() {
            self.error = Some(
                localized(
                    cx,
                    "Babel served a session without forms.",
                    "Babel выдал сессию без форм.",
                )
                .into(),
            );
            return;
        }
        if let Some((index, error)) = session
            .tasks
            .iter()
            .enumerate()
            .find_map(|(index, task)| validate_task(task).err().map(|error| (index, error)))
        {
            self.error = Some(
                localized_fmt(
                    cx,
                    "Form {number} of this session can't be answered in Exaudio: {error} Finish the session on the Babel website.",
                    "Форму {number} этой сессии нельзя заполнить в Exaudio: {error} Завершите сессию на сайте Babel.",
                    &[
                        ("number", &(index + 1).to_string()),
                        ("error", &error.message),
                    ],
                )
                .into(),
            );
            return;
        }
        let resumed = self.listened_session.as_deref() == Some(session.session_id.as_str());
        let index = if resumed {
            self.session
                .as_ref()
                .map_or(0, |active| active.index)
                .min(session.tasks.len() - 1)
        } else {
            0
        };
        if !resumed {
            self.listened.clear();
            self.listened_session = Some(session.session_id.clone());
        }
        // An unreadable draft is shown, never saved over (the store moves it aside).
        let mut answers = match self.drafts.load(&self.project_id, &session.session_id) {
            Ok(answers) => answers,
            Err(error) => {
                self.draft_error = Some(error.message.into());
                EvalAnswers::new()
            }
        };
        for task in &session.tasks {
            prune_hidden_answers(task, &mut answers);
        }
        self.answers = answers;
        self.session = Some(ActiveSession {
            session,
            index,
            expired: false,
        });
        self.last_hold = Instant::now();
        self.notice = Some(
            localized(
                cx,
                "Listen to each clip to the end, then answer the questions.",
                "Прослушайте каждый клип до конца, затем ответьте на вопросы.",
            )
            .into(),
        );
        self.enter_task(window, cx);
    }

    fn hold_due(&self, now: Instant) -> bool {
        self.visible
            && self.session_active()
            && self.request.is_none()
            && now.saturating_duration_since(self.last_hold) >= HOLD_INTERVAL
    }

    fn hold(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(active) = &self.session else {
            return;
        };
        self.last_hold = now;
        let client = self.client.clone();
        let session_id = active.session.session_id.clone();
        let task = spawn_service(
            cx,
            async move { client.hold_eval_session(&session_id).await },
            |this, result, cx| {
                this.request = None;
                match result {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Some(active) = &mut this.session {
                            active.expired = true;
                        }
                        this.stop_audio();
                        this.error = Some(
                            localized(
                                cx,
                                "The session expired. Your answers are kept on this computer; reload the session to continue.",
                                "Сессия истекла. Ваши ответы сохранены на этом компьютере; загрузите сессию снова, чтобы продолжить.",
                            )
                            .into(),
                        );
                    }
                    Err(error) => {
                        this.error = Some(
                            failed(
                                cx,
                                "Couldn't keep the session open.",
                                "Не удалось продлить сессию.",
                                error,
                            )
                            .into(),
                        )
                    }
                }
                cx.notify();
            },
        );
        self.request = Some(InFlight {
            kind: RequestKind::Hold,
            _task: task,
        });
    }

    /// Writes the draft; a failure is shown and returned.
    fn persist(&mut self) -> bool {
        let Some(active) = &self.session else {
            return true;
        };
        match self
            .drafts
            .save(&self.project_id, &active.session.session_id, &self.answers)
        {
            Ok(()) => {
                self.draft_error = None;
                true
            }
            Err(error) => {
                self.draft_error = Some(error.message.into());
                false
            }
        }
    }

    fn save_and_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.is_some() || !self.session_active() {
            return;
        }
        let Some(active) = &self.session else {
            return;
        };
        let task = &active.session.tasks[active.index];
        let empty = BTreeSet::new();
        let listened = self.listened.get(task.task_id()).unwrap_or(&empty);
        let progress =
            match EvalProgress::for_task(&active.session.session_id, task, &self.answers, listened)
            {
                Ok(progress) => progress,
                Err(error) => {
                    self.error = Some(error.message.into());
                    cx.notify();
                    return;
                }
            };
        let index = active.index;
        if !self.persist() {
            cx.notify();
            return;
        }
        self.error = None;
        self.notice = None;
        let client = self.client.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { client.save_eval_progress(&progress).await },
            move |this, result, window, cx| {
                this.request = None;
                match result {
                    Ok(true) => {
                        this.notice = Some(localized(cx, "Form saved.", "Форма сохранена.").into());
                        let next = this
                            .session
                            .as_ref()
                            .filter(|active| {
                                active.index == index && index + 1 < active.session.tasks.len()
                            })
                            .map(|_| index + 1);
                        if let Some(next) = next {
                            this.go_to(next, window, cx);
                        }
                    }
                    Ok(false) => {
                        this.error = Some(
                            localized(
                                cx,
                                "Babel didn't confirm saving. Your answers are kept on this computer.",
                                "Babel не подтвердил сохранение. Ваши ответы сохранены на этом компьютере.",
                            )
                            .into(),
                        );
                    }
                    Err(error) => this.error = Some(error.message.into()),
                }
                cx.notify();
            },
        );
        self.request = Some(InFlight {
            kind: RequestKind::Save,
            _task: task,
        });
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.is_some() || !self.session_active() {
            return;
        }
        let Some(active) = &self.session else {
            return;
        };
        let progress =
            match EvalProgress::for_session(&active.session, &self.answers, &self.listened) {
                Ok(progress) => progress,
                Err(error) => {
                    self.error = Some(error.message.into());
                    cx.notify();
                    return;
                }
            };
        if !self.persist() {
            cx.notify();
            return;
        }
        self.error = None;
        self.notice = None;
        let client = self.client.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { client.submit_eval_session(&progress).await },
            |this, result, _, cx| {
                this.request = None;
                match result {
                    Ok(SubmitOutcome::Completed) => {
                        this.clear_session();
                        this.notice = Some(
                            localized(
                                cx,
                                "Session submitted. Load the next session when you're ready.",
                                "Сессия отправлена. Загрузите следующую, когда будете готовы.",
                            )
                            .into(),
                        );
                    }
                    Ok(SubmitOutcome::Incomplete) => {
                        this.error = Some(
                            localized(
                                cx,
                                "Babel says the session is incomplete. Your answers are kept; check every form and submit again.",
                                "Babel сообщает, что сессия не завершена. Ответы сохранены; проверьте все формы и отправьте снова.",
                            )
                            .into(),
                        );
                    }
                    Ok(SubmitOutcome::Other(kind)) => {
                        this.error = Some(
                            localized_fmt(
                                cx,
                                "Babel answered \"{kind}\" instead of confirming. Your answers are kept; check the session on the Babel website before submitting again.",
                                "Babel ответил «{kind}» вместо подтверждения. Ответы сохранены; проверьте сессию на сайте Babel, прежде чем отправлять снова.",
                                &[("kind", &kind)],
                            )
                            .into(),
                        );
                    }
                    Err(error) => this.error = Some(error.message.into()),
                }
                cx.notify();
            },
        );
        self.request = Some(InFlight {
            kind: RequestKind::Submit,
            _task: task,
        });
        cx.notify();
    }

    fn clear_session(&mut self) {
        self.stop_audio();
        self.session = None;
        self.answers.clear();
        self.listened.clear();
        self.listened_session = None;
        self.clips.clear();
        self.clip_task = None;
        self.text_inputs.clear();
        self.comment_inputs.clear();
        self.input_subscriptions.clear();
    }

    // ---- navigation ---------------------------------------------------------------------

    fn go_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = &mut self.session else {
            return;
        };
        if index >= active.session.tasks.len() || index == active.index {
            return;
        }
        active.index = index;
        self.enter_task(window, cx);
    }

    /// The current form changed: drops the previous form's audio and builds its inputs.
    fn enter_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_audio();
        self.clips.clear();
        self.clip_task = None;
        self.text_inputs.clear();
        self.comment_inputs.clear();
        self.input_subscriptions.clear();
        let Some(task) = self.current_task().cloned() else {
            return;
        };
        let answers = self.answers.get(task.task_id());
        for question in task.questions() {
            let answer = answers.and_then(|answers| answers.get(&question.id));
            if question.kind == EvalQuestionType::Text {
                let value = answer
                    .and_then(|answer| answer.values.first())
                    .cloned()
                    .unwrap_or_default();
                let placeholder = localized(cx, "Your answer", "Ваш ответ");
                let input = self.new_input(value, placeholder, window, cx);
                let question_id = question.id.clone();
                self.input_subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event, window, cx| {
                        if let InputEvent::Change = event {
                            let text = input.read(cx).value().to_string();
                            this.answer(
                                &question_id,
                                |answer| {
                                    answer.values = if text.trim().is_empty() {
                                        Vec::new()
                                    } else {
                                        vec![text]
                                    };
                                },
                                window,
                                cx,
                            );
                        }
                    },
                ));
                self.text_inputs.insert(question.id.clone(), input);
            }
            if question.comment_allowed {
                let value = answer
                    .and_then(|answer| answer.comment.clone())
                    .unwrap_or_default();
                let placeholder =
                    localized(cx, "Comment (optional)", "Комментарий (необязательно)");
                let input = self.new_input(value, placeholder, window, cx);
                let question_id = question.id.clone();
                self.input_subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event, window, cx| {
                        if let InputEvent::Change = event {
                            let text = input.read(cx).value().to_string();
                            this.answer(
                                &question_id,
                                |answer| answer.comment = Some(text),
                                window,
                                cx,
                            );
                        }
                    },
                ));
                self.comment_inputs.insert(question.id.clone(), input);
            }
        }
        cx.notify();
    }

    fn new_input(
        &self,
        value: String,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .soft_wrap(true)
                .placeholder(placeholder)
                .default_value(value)
        })
    }

    // ---- answers ------------------------------------------------------------------------

    /// Changes one answer of the current form through `set_answer`, then saves the draft.
    fn answer(
        &mut self,
        question_id: &str,
        change: impl FnOnce(&mut EvalAnswer),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.answers_locked() {
            return;
        }
        let Some(active) = &self.session else {
            return;
        };
        let task = &active.session.tasks[active.index];
        let mut answer = self
            .answers
            .get(task.task_id())
            .and_then(|answers| answers.get(question_id))
            .cloned()
            .unwrap_or_default();
        change(&mut answer);
        match set_answer(task, &mut self.answers, question_id, answer) {
            Ok(()) => {
                self.error = None;
                self.persist();
                self.clear_hidden_inputs(window, cx);
            }
            Err(error) => self.error = Some(error.message.into()),
        }
        cx.notify();
    }

    /// Hidden questions lost their answers; their boxes must not show stale text when they
    /// reappear.
    fn clear_hidden_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.current_task() else {
            return;
        };
        let visible: BTreeSet<String> = visible_questions(task, &self.answers)
            .into_iter()
            .map(|question| question.id.clone())
            .collect();
        for (question_id, input) in self.text_inputs.iter().chain(&self.comment_inputs) {
            if !visible.contains(question_id) && !input.read(cx).value().is_empty() {
                input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
            }
        }
    }

    fn choose(
        &mut self,
        question: &EvalQuestion,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if question.kind == EvalQuestionType::Multiselect {
            let options = question.values.clone();
            self.answer(
                &question.id,
                move |answer| {
                    let mut chosen: BTreeSet<String> = answer.values.drain(..).collect();
                    if !chosen.remove(&value) {
                        chosen.insert(value);
                    }
                    answer.values = options
                        .into_iter()
                        .filter(|option| chosen.contains(option))
                        .collect();
                },
                window,
                cx,
            );
        } else {
            self.answer(
                &question.id,
                move |answer| answer.values = vec![value],
                window,
                cx,
            );
        }
    }

    // ---- audio --------------------------------------------------------------------------

    fn stop_audio(&mut self) {
        self.engine.pause();
        self.playing = None;
    }

    fn play(&mut self, subject_id: String, cx: &mut Context<Self>) {
        if !self.session_active() || self.clip_task.is_some() {
            return;
        }
        self.stop_audio();
        match self.clips.get(&subject_id) {
            Some(Clip::Ready(audio)) => {
                let audio = Arc::clone(audio);
                self.start_playback(subject_id, audio, cx);
            }
            Some(Clip::Loading) => {}
            None | Some(Clip::Failed(_)) => self.download(subject_id, cx),
        }
        cx.notify();
    }

    fn download(&mut self, subject_id: String, cx: &mut Context<Self>) {
        let Some(task) = self.current_task() else {
            return;
        };
        let Some(url) = task
            .subjects()
            .iter()
            .find(|subject| subject.id == subject_id)
            .and_then(|subject| subject.media_url.clone())
        else {
            return;
        };
        let task_id = task.task_id().to_string();
        self.clips.insert(subject_id.clone(), Clip::Loading);
        self.clip_task = Some(spawn_service(
            cx,
            async move { fetch_eval_clip(&url).await },
            move |this, result, cx| {
                this.clip_task = None;
                if this.current_task_id().as_deref() != Some(task_id.as_str()) {
                    return;
                }
                match result {
                    Ok(audio) => {
                        let audio = Arc::new(audio);
                        this.clips
                            .insert(subject_id.clone(), Clip::Ready(Arc::clone(&audio)));
                        if this.visible && this.session_active() {
                            this.start_playback(subject_id, audio, cx);
                        }
                    }
                    Err(error) => {
                        this.clips
                            .insert(subject_id, Clip::Failed(error.message.into()));
                    }
                }
                cx.notify();
            },
        ));
    }

    fn start_playback(&mut self, subject_id: String, audio: Arc<DecodedAudio>, cx: &App) {
        let duration = audio.duration();
        let started = self.engine.insert_lane(LANE, audio).and_then(|()| {
            self.engine.set_timeline_end(Some(duration));
            self.engine.play_from(0.0)
        });
        match started {
            Ok(_) => self.playing = Some(Listening::new(subject_id, Instant::now())),
            Err(error) => {
                self.engine.pause();
                self.error = Some(
                    failed(
                        cx,
                        "Can't play audio on this device.",
                        "Не удаётся воспроизвести аудио на этом устройстве.",
                        error,
                    )
                    .into(),
                );
            }
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if let Some(mut listening) = self.playing.take() {
            let ended = matches!(self.engine.poll(), Some(PlaybackEvent::Ended));
            match listening.step(now, ended) {
                ListenStep::Listened => {
                    if let Some(task_id) = self.current_task_id() {
                        self.listened
                            .entry(task_id)
                            .or_default()
                            .insert(listening.subject_id);
                    }
                }
                ListenStep::Interrupted => {
                    self.stop_audio();
                    self.error = Some(
                        localized(
                            cx,
                            "Playback was interrupted. Play the clip again to the end.",
                            "Воспроизведение прервалось. Прослушайте клип снова до конца.",
                        )
                        .into(),
                    );
                }
                ListenStep::Playing => {
                    if self.engine.state() != PlaybackState::Stopped {
                        self.playing = Some(listening);
                    }
                }
            }
            cx.notify();
        }
        if self.hold_due(now) {
            self.hold(now, cx);
            cx.notify();
        }
    }
}

impl Focusable for EvalScreen {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// ---- rendering ----------------------------------------------------------------------------

impl EvalScreen {
    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_3()
            .child(
                Button::new("eval-back")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .label(localized(cx, "Projects", "Проекты"))
                    .on_click(cx.listener(|this, _, window, cx| this.leave(window, cx))),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(self.project_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(localized(cx, "Audio evaluation", "Оценка аудио")),
                    ),
            )
            .child(div().flex_1())
            .child(
                Button::new("eval-resources-toggle")
                    .ghost()
                    .small()
                    .icon(IconName::FileText)
                    .label(if self.show_resources {
                        localized(cx, "Hide instructions", "Скрыть инструкции")
                    } else {
                        localized(cx, "Show instructions", "Показать инструкции")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_resources = !this.show_resources;
                        cx.notify();
                    })),
            )
    }

    fn render_messages(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.request.as_ref().map(|request| request.kind);
        let expired = self.session.as_ref().is_some_and(|active| active.expired);
        let waiting: Option<&'static str> = match busy {
            Some(RequestKind::Serve) => Some(localized(
                cx,
                "Loading a session from Babel…",
                "Загружаем сессию из Babel…",
            )),
            Some(RequestKind::Save) => Some(localized(cx, "Saving the form…", "Сохраняем форму…")),
            Some(RequestKind::Submit) => Some(localized(
                cx,
                "Submitting the session…",
                "Отправляем сессию…",
            )),
            Some(RequestKind::Hold) | None => None,
        };
        v_flex()
            .w_full()
            .gap_2()
            .when_some(waiting, |this, waiting| {
                this.child(ui::banner(
                    Tone::Info,
                    waiting,
                    None,
                    Some(Spinner::new().small().into_any_element()),
                    cx,
                ))
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(ui::banner(Tone::Danger, error, None, None, cx))
            })
            .when_some(self.draft_error.clone(), |this, error| {
                this.child(ui::banner(Tone::Warning, error, None, None, cx))
            })
            .when(expired && busy.is_none(), |this| {
                this.child(ui::banner(
                    Tone::Warning,
                    localized(cx, "The session expired", "Сессия истекла"),
                    Some(
                        localized(
                            cx,
                            "Your answers are kept on this computer.",
                            "Ваши ответы сохранены на этом компьютере.",
                        )
                        .into(),
                    ),
                    Some(
                        Button::new("eval-reload")
                            .warning()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label(localized(cx, "Reload session", "Загрузить сессию снова"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_session(window, cx)),
                            )
                            .into_any_element(),
                    ),
                    cx,
                ))
            })
            .when_some(self.notice.clone(), |this, notice| {
                this.child(ui::banner(Tone::Info, notice, None, None, cx))
            })
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> Div {
        let loading = self
            .request
            .as_ref()
            .is_some_and(|request| request.kind == RequestKind::Serve);
        v_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_4()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .child(ui::muted(
                localized(
                    cx,
                    "A session is a set of forms: listen to every clip to the end, answer each form, save it, then submit the session.",
                    "Сессия — это набор форм: прослушайте каждый клип до конца, ответьте на каждую форму, сохраните её, затем отправьте сессию.",
                ),
                cx,
            ))
            .child(
                h_flex().child(
                    Button::new("eval-load")
                        .primary()
                        .small()
                        .label(localized(cx, "Load session", "Загрузить сессию"))
                        .loading(loading)
                        .disabled(self.request.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.load_session(window, cx))),
                ),
            )
    }

    fn render_subject(
        &self,
        subject: &EvalSubject,
        audio_index: Option<usize>,
        listened: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let label: SharedString = match (&subject.label, audio_index) {
            (Some(label), _) if !label.trim().is_empty() => label.clone().into(),
            (_, Some(index)) => localized_fmt(
                cx,
                "Clip {letter}",
                "Клип {letter}",
                &[("letter", &clip_letter(index))],
            )
            .into(),
            _ => localized(cx, "Text", "Текст").into(),
        };
        let mut card = v_flex()
            .flex_1()
            .min_w(rems(16.))
            .gap_1p5()
            .px_3()
            .py_2p5()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .when(subject.is_audio() && listened, |this| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(theme.success)
                                .child(Icon::new(IconName::CircleCheck).small())
                                .child(localized(cx, "Listened", "Прослушано")),
                        )
                    }),
            );
        if let Some(emotion) = subject
            .emotion
            .as_ref()
            .filter(|text| !text.trim().is_empty())
        {
            card = card.child(ui::muted(
                localized_fmt(
                    cx,
                    "Emotion: {emotion}",
                    "Эмоция: {emotion}",
                    &[("emotion", emotion)],
                ),
                cx,
            ));
        }
        if let Some(transcript) = subject
            .transcript
            .as_ref()
            .filter(|text| !text.trim().is_empty())
        {
            card = card.child(div().text_sm().child(transcript.clone()));
        }
        if subject.is_audio() {
            card = card.child(self.render_player(subject, listened, cx));
        }
        card
    }

    fn render_player(&self, subject: &EvalSubject, listened: bool, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let playing = self
            .playing
            .as_ref()
            .is_some_and(|listening| listening.subject_id == subject.id);
        let clip = self.clips.get(&subject.id);
        let loading = matches!(clip, Some(Clip::Loading));
        let state: Option<SharedString> = match clip {
            Some(Clip::Loading) => {
                Some(localized(cx, "Loading the clip…", "Загрузка клипа…").into())
            }
            Some(Clip::Failed(error)) => Some(
                failed(
                    cx,
                    "Couldn't load the clip.",
                    "Не удалось загрузить клип.",
                    error,
                )
                .into(),
            ),
            Some(Clip::Ready(audio)) if playing => Some(
                format!(
                    "{} / {}",
                    format_time(self.engine.position().min(audio.duration())),
                    format_time(audio.duration())
                )
                .into(),
            ),
            _ if !listened => {
                Some(localized(cx, "Listen to the end", "Прослушайте до конца").into())
            }
            _ => None,
        };
        let id = subject.id.clone();
        let button_id = SharedString::from(format!("eval-play-{}", subject.id));
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(if playing {
                Button::new(button_id)
                    .small()
                    .icon(IconName::Pause)
                    .label(localized(cx, "Stop", "Стоп"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stop_audio();
                        cx.notify();
                    }))
            } else {
                Button::new(button_id)
                    .small()
                    .outline()
                    .icon(IconName::Play)
                    .label(localized(cx, "Play", "Воспроизвести"))
                    .loading(loading)
                    .disabled(!self.session_active() || self.clip_task.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| this.play(id.clone(), cx)))
            })
            .when_some(state, |this, state| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(state),
                )
            })
    }

    fn render_question(
        &self,
        number: usize,
        question: &EvalQuestion,
        answer: Option<&EvalAnswer>,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let locked = self.answers_locked();
        let mut card = v_flex()
            .w_full()
            .gap_2()
            .px_3()
            .py_2p5()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(question.label.clone()),
                    )
                    .when(question.optional, |this| {
                        this.child(ui::muted(localized(cx, "Optional", "Необязательно"), cx))
                    }),
            )
            .when_some(
                question
                    .caption
                    .clone()
                    .filter(|text| !text.trim().is_empty()),
                |this, caption| this.child(ui::muted(caption, cx)),
            );
        if question.kind == EvalQuestionType::Text {
            if let Some(input) = self.text_inputs.get(&question.id) {
                card = card.child(Textarea::new(input).small().disabled(locked));
            }
        } else {
            let mut options = h_flex().w_full().flex_wrap().gap_2();
            for (index, value) in question.values.iter().enumerate() {
                let selected = answer.is_some_and(|answer| answer.values.contains(value));
                let label = question.option_label(index).unwrap_or(value).to_string();
                let chosen = value.clone();
                let target = question.clone();
                options = options.child(
                    Button::new(SharedString::from(format!("eval-option-{number}-{index}")))
                        .small()
                        .outline()
                        .when(question.kind == EvalQuestionType::Multiselect, |this| {
                            this.icon(if selected {
                                IconName::Check
                            } else {
                                IconName::Plus
                            })
                        })
                        .label(label)
                        .selected(selected)
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose(&target, chosen.clone(), window, cx)
                        })),
                );
            }
            card = card.child(options);
            if question.kind == EvalQuestionType::Multiselect {
                card = card.child(ui::muted(
                    localized(cx, "Choose all that apply.", "Выберите все подходящие."),
                    cx,
                ));
            }
        }
        if let Some(input) = self.comment_inputs.get(&question.id) {
            card = card.child(Textarea::new(input).small().disabled(locked));
        }
        card
    }

    fn render_form(&self, cx: &mut Context<Self>) -> Option<Div> {
        let active = self.session.as_ref()?;
        let task = active.session.tasks.get(active.index)?;
        let muted = cx.theme().muted_foreground;
        let total = active.session.tasks.len();
        let last = active.index + 1 == total;
        let busy = self.request.is_some();
        let usable = !active.expired;
        let title = {
            let form = localized_fmt(
                cx,
                "Form {i} / {n}",
                "Форма {i} / {n}",
                &[
                    ("i", &(active.index + 1).to_string()),
                    ("n", &total.to_string()),
                ],
            );
            match task.raw_text("formDisplayName") {
                Some(name) => format!("{form} — {name}"),
                None => form,
            }
        };
        let empty = BTreeSet::new();
        let listened = self.listened.get(task.task_id()).unwrap_or(&empty);
        let mut subjects = h_flex().w_full().flex_wrap().gap_3().items_start();
        let mut audio_index = 0;
        for subject in task.subjects() {
            let index = subject.is_audio().then(|| {
                audio_index += 1;
                audio_index - 1
            });
            subjects = subjects.child(self.render_subject(
                subject,
                index,
                listened.contains(&subject.id),
                cx,
            ));
        }
        let mut form = v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(subjects);
        for (key, en, ru) in [
            ("writtenForm", "Written form", "Письменная форма"),
            ("spokenForm", "Spoken form", "Устная форма"),
        ] {
            if let Some(text) = task.raw_text(key) {
                form = form.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .child(localized(cx, en, ru)),
                        )
                        .child(div().text_sm().child(text.to_string())),
                );
            }
        }
        let answers = self.answers.get(task.task_id());
        for (number, question) in visible_questions(task, &self.answers)
            .into_iter()
            .enumerate()
        {
            let answer = answers.and_then(|answers| answers.get(&question.id));
            form = form.child(self.render_question(number, question, answer, cx));
        }
        let index = active.index;
        let saving = self.request.as_ref().map(|request| request.kind);
        form = form.child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("eval-previous")
                        .small()
                        .outline()
                        .icon(IconName::ArrowLeft)
                        .label(localized(cx, "Previous", "Назад"))
                        .disabled(busy || index == 0)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.request.is_none() && index > 0 {
                                this.go_to(index - 1, window, cx)
                            }
                        })),
                )
                .child(div().flex_1())
                .child({
                    let button = Button::new("eval-save")
                        .small()
                        .label(if last {
                            localized(cx, "Save", "Сохранить")
                        } else {
                            localized(cx, "Save & next", "Сохранить и далее")
                        })
                        .loading(saving == Some(RequestKind::Save))
                        .disabled(busy || !usable)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_and_next(window, cx)),
                        );
                    if last {
                        button.outline()
                    } else {
                        button.primary().icon(IconName::ChevronRight)
                    }
                })
                .when(last, |this| {
                    this.child(
                        Button::new("eval-submit")
                            .small()
                            .primary()
                            .label(localized(cx, "Submit session", "Отправить сессию"))
                            .loading(saving == Some(RequestKind::Submit))
                            .disabled(busy || !usable)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    )
                }),
        );
        Some(form)
    }
}

impl Render for EvalScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx);
        let messages = self.render_messages(cx);
        let body = match self.render_form(cx) {
            Some(form) => form,
            None => self.render_empty(cx),
        };
        let theme = cx.theme();
        let background = theme.background;
        let foreground = theme.foreground;
        let border = theme.border;
        div()
            .id("eval-screen")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(
                h_flex()
                    .size_full()
                    .items_start()
                    .child(
                        div()
                            .id("eval-main")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scrollbar()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_3()
                                    .px_6()
                                    .py_4()
                                    .child(header)
                                    .child(messages)
                                    .child(body),
                            ),
                    )
                    .when(self.show_resources, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .w(rems(30.))
                                .h_full()
                                .border_l_1()
                                .border_color(border)
                                .child(self.resources.clone()),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn clip_letters_run_a_to_z_then_numbers() {
        assert_eq!(clip_letter(0), "A");
        assert_eq!(clip_letter(1), "B");
        assert_eq!(clip_letter(25), "Z");
        assert_eq!(clip_letter(26), "27");
    }

    #[test]
    fn steady_playback_to_the_end_counts_as_listened() {
        let start = Instant::now();
        let mut listening = Listening::new("a".into(), start);
        for tick in 1..=8 {
            let now = start + TICK * tick;
            assert_eq!(listening.step(now, false), ListenStep::Playing);
        }
        assert_eq!(listening.step(start + TICK * 9, true), ListenStep::Listened);
    }

    #[test]
    fn a_gap_longer_than_three_seconds_interrupts_even_at_the_end() {
        let start = Instant::now();
        let mut listening = Listening::new("a".into(), start);
        assert_eq!(listening.step(start + TICK, false), ListenStep::Playing);
        let resumed = start + TICK + MAX_TICK_GAP + Duration::from_millis(1);
        assert_eq!(listening.step(resumed, true), ListenStep::Interrupted);
    }

    #[test]
    fn a_gap_of_exactly_three_seconds_still_counts() {
        let start = Instant::now();
        let mut listening = Listening::new("a".into(), start);
        assert_eq!(
            listening.step(start + MAX_TICK_GAP, true),
            ListenStep::Listened
        );
    }

    #[test]
    fn each_tick_restarts_the_gap_window() {
        let start = Instant::now();
        let mut listening = Listening::new("a".into(), start);
        let mut now = start;
        for _ in 0..5 {
            now += Duration::from_secs(2);
            assert_eq!(listening.step(now, false), ListenStep::Playing);
        }
        assert_eq!(
            listening.step(now + Duration::from_secs(2), true),
            ListenStep::Listened
        );
    }
}
