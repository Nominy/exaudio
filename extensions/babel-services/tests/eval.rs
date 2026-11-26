//! Eval sessions: task validation, visibility, answers, Babel `inputResponses`, session
//! replies, the local draft, eval dashboard projects and clip decoding.

mod common;

use std::collections::BTreeSet;

use common::{MockServer, contains};
use exaudio_babel_services::babel::eval::{
    EvalAnswer, EvalAnswers, EvalDraftStore, EvalListened, EvalProgress, EvalQuestionType,
    EvalTask, ServeOutcome, SubmitOutcome, check_task_answers, decode_eval_clip, fetch_eval_clip,
    input_responses, prune_hidden_answers, set_answer, validate_task, visible_questions,
};
use exaudio_babel_services::babel::{self, BabelClient, BabelConfig, BabelRequest, Fixture};
use serde_json::{Value, json};

const Q_CORRECT: &str = "11111111-1111-4111-8111-111111111111";
const Q_WHY: &str = "22222222-2222-4222-8222-222222222222";
const Q_DETAIL: &str = "33333333-3333-4333-8333-333333333333";

fn naturalness() -> Value {
    json!({
        "taskId": "t",
        "format": "tts-naturalness",
        "formLinters": [],
        "subjects": [
            {"id": "a", "position": 0, "mediaUrl": "https://example.test/a.wav"},
            {"id": "b", "position": 1, "mediaUrl": "https://example.test/b.wav"}
        ],
        "questions": [{
            "id": "q", "label": "Which sounds natural?", "type": "side_by_side",
            "values": ["a_better", "tie", "b_better"], "valueLabels": ["A", "Same", "B"],
            "sequenceNumber": 1, "linters": []
        }]
    })
}

fn alpha() -> Value {
    json!({
        "taskId": "alpha-task",
        "format": "default",
        "formLinters": [],
        "subjects": [
            {"id": "audio", "label": "Recording", "mediaUrl": "https://example.test/audio.wav"},
            {"id": "written", "label": "Written form", "transcript": "04.03."},
            {"id": "spoken", "label": "Spoken form", "transcript": "четвёртое марта"}
        ],
        "questions": [
            {"id": Q_CORRECT, "label": "Read correctly?", "type": "mcq", "values": ["yes", "no"],
             "valueLabels": ["Да", "Нет"], "sequenceNumber": 1, "linters": []},
            {"id": Q_WHY, "label": "Explain", "type": "text", "sequenceNumber": 2,
             "metadata": {"showWhen": {"kind": "answer", "questionId": Q_CORRECT, "values": ["no"]}},
             "linters": []}
        ]
    })
}

fn task(raw: Value) -> EvalTask {
    let task = EvalTask::new(raw);
    validate_task(&task).expect("valid task");
    task
}

fn rejected(raw: Value) -> bool {
    validate_task(&EvalTask::new(raw)).is_err()
}

fn listened(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn visible_ids(task: &EvalTask, answers: &EvalAnswers) -> Vec<String> {
    visible_questions(task, answers)
        .into_iter()
        .map(|question| question.id.clone())
        .collect()
}

fn answer(task: &EvalTask, answers: &mut EvalAnswers, question: &str, values: &[&str]) {
    set_answer(
        task,
        answers,
        question,
        EvalAnswer::new(values.iter().copied()),
    )
    .expect("answer");
}

#[test]
fn alpha_supports_one_audio_and_labeled_script_subjects() {
    let task = task(alpha());
    assert_eq!(task.subjects().iter().filter(|s| s.is_audio()).count(), 1);
    assert_eq!(task.subjects()[1].transcript.as_deref(), Some("04.03."));
    let mut answers = EvalAnswers::new();
    answer(&task, &mut answers, Q_CORRECT, &["yes"]);
    assert_eq!(visible_ids(&task, &answers), [Q_CORRECT]);
    answer(&task, &mut answers, Q_CORRECT, &["no"]);
    assert_eq!(visible_ids(&task, &answers), [Q_CORRECT, Q_WHY]);

    let mut with_linters = alpha();
    with_linters["formLinters"] = json!([{"linterKey": "unknown"}]);
    assert!(rejected(with_linters));
    let mut question_linters = alpha();
    question_linters["questions"][0]["linters"] = json!([{"linterKey": "grammar-check"}]);
    assert!(rejected(question_linters));
}

#[test]
fn accepts_observed_naturalness_contract_and_sorts_by_position_and_sequence() {
    task(naturalness());
    let mut raw = alpha();
    raw["subjects"] = json!([
        {"id": "late", "position": 2, "transcript": "x"},
        {"id": "audio", "position": 0, "mediaUrl": "https://example.test/a.wav"},
        {"id": "mid", "position": 1, "transcript": "y"}
    ]);
    raw["questions"][0]["sequenceNumber"] = json!(5);
    let task = task(raw);
    let subjects: Vec<_> = task.subjects().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(subjects, ["audio", "mid", "late"]);
    let questions: Vec<_> = task.questions().iter().map(|q| q.id.as_str()).collect();
    assert_eq!(questions, [Q_WHY, Q_CORRECT]);
    assert_eq!(task.raw()["subjects"][0]["id"], "audio");
    assert_eq!(task.raw()["questions"][0]["id"], Q_WHY);
}

#[test]
fn unsupported_tasks_are_rejected_not_silently_submitted() {
    assert!(rejected(
        json!({"taskId":"t","format":"other","questions":[{}],"subjects":[{}]})
    ));
    let mut missing_audio = naturalness();
    missing_audio["subjects"][0]["mediaUrl"] = json!("");
    assert!(rejected(missing_audio));
    let mut one_clip = naturalness();
    one_clip["subjects"][0] = json!({"id": "a", "transcript": "script"});
    assert!(rejected(one_clip));
    let mut insecure = naturalness();
    insecure["subjects"][0]["mediaUrl"] = json!("http://example.test/a.wav");
    assert!(rejected(insecure));
    let mut duplicate = naturalness();
    duplicate["subjects"][1]["id"] = json!("a");
    assert!(rejected(duplicate));
    let mut no_transcript = alpha();
    no_transcript["subjects"][1] = json!({"id": "written", "label": "Written form"});
    assert!(rejected(no_transcript));
    let mut tracks = naturalness();
    tracks["subjects"][0]["mediaTracks"] = json!([{"url": "https://example.test/t.wav"}]);
    assert!(rejected(tracks));
    let mut taxonomy = naturalness();
    taxonomy["questions"][0]["type"] = json!("taxonomy");
    assert!(rejected(taxonomy));
    let mut unknown_type = naturalness();
    unknown_type["questions"][0]["type"] = json!("ranking");
    assert!(rejected(unknown_type));
    let mut duplicate_question = alpha();
    duplicate_question["questions"][1]["id"] = json!(Q_CORRECT);
    assert!(rejected(duplicate_question));
    let mut labels = naturalness();
    labels["questions"][0]["valueLabels"] = json!(["A", "B"]);
    assert!(rejected(labels));
    let mut no_values = naturalness();
    no_values["questions"][0]["values"] = json!([]);
    assert!(rejected(no_values));
    let mut flag_entry = naturalness();
    flag_entry["questions"][0]["metadata"] = json!({"flagEntry": {"noneValue": "none"}});
    assert!(rejected(flag_entry));
    let mut null_flag_entry = naturalness();
    null_flag_entry["questions"][0]["metadata"] = json!({"flagEntry": null});
    task(null_flag_entry);
}

#[test]
fn show_when_follows_babels_strict_schema() {
    let with_rule = |rule: Value| {
        let mut raw = alpha();
        raw["questions"][1]["metadata"] = json!({ "showWhen": rule });
        raw
    };
    assert!(rejected(with_rule(Value::Null)));
    assert!(rejected(with_rule(json!({"id": "other"}))));
    assert!(rejected(with_rule(
        json!({"kind": "answer", "questionId": Q_CORRECT, "values": ["no"], "extra": 1})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "answer", "questionId": "correct", "values": ["no"]})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "answer", "questionId": Q_CORRECT, "values": []})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "answer", "questionId": Q_CORRECT, "values": [""]})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "scenario", "field": "topic", "value": "x"})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "scenario", "field": "domain", "value": "   "})
    )));
    assert!(rejected(with_rule(
        json!({"kind": "scenario", "field": "domain", "value": "x".repeat(1024)})
    )));
    task(with_rule(
        json!({"kind": "scenario", "field": "domain", "value": format!(" {} ", "x".repeat(1023))}),
    ));
    // A referenced question that does not exist keeps the question hidden, as in Babel.
    let unknown_ref = task(with_rule(
        json!({"kind": "answer", "questionId": Q_DETAIL, "values": ["no"]}),
    ));
    assert_eq!(visible_ids(&unknown_ref, &EvalAnswers::new()), [Q_CORRECT]);
    // Babel reads showWhen only from object metadata.
    let mut array_metadata = alpha();
    array_metadata["questions"][1]["metadata"] = json!([{"showWhen": null}]);
    let array_metadata = task(array_metadata);
    assert_eq!(
        visible_ids(&array_metadata, &EvalAnswers::new()),
        [Q_CORRECT, Q_WHY]
    );
}

#[test]
fn scenario_visibility_matches_babel_normalization() {
    let scenario = |context: Value| {
        let mut raw = alpha();
        raw["scenarioContext"] = context;
        raw["questions"][1]["metadata"] =
            json!({"showWhen": {"kind": "scenario", "field": "domain", "value": " PHONE "}});
        task(raw)
    };
    let shown = |task: &EvalTask| visible_ids(task, &EvalAnswers::new()).len() == 2;
    assert!(shown(&scenario(json!({"domain": "phone"}))));
    assert!(shown(&scenario(json!({"domain": "\u{feff} Phone\n"}))));
    assert!(!shown(&scenario(json!({"domain": "dates"}))));
    assert!(!shown(&scenario(json!({"domain": "   "}))));
    assert!(!shown(&scenario(json!({"objective": "phone"}))));
    assert!(!shown(&scenario(json!({"domain": null}))));
    assert!(!shown(&scenario(Value::Null)));
    let mut numeric = alpha();
    numeric["scenarioContext"] = json!({"domain": 5});
    assert!(rejected(numeric));
}

fn chain() -> EvalTask {
    let mut raw = alpha();
    raw["questions"][1]["type"] = json!("mcq");
    raw["questions"][1]["values"] = json!(["audio", "script"]);
    raw["questions"]
        .as_array_mut()
        .expect("questions")
        .push(json!({
            "id": Q_DETAIL, "label": "Detail", "type": "text", "sequenceNumber": 3,
            "metadata": {"showWhen": {"kind": "answer", "questionId": Q_WHY, "values": ["script"]}}
        }));
    task(raw)
}

#[test]
fn answer_visibility_chains_and_hidden_answers_are_dropped() {
    let task = chain();
    let mut answers = EvalAnswers::new();
    answer(&task, &mut answers, Q_CORRECT, &["no"]);
    answer(&task, &mut answers, Q_WHY, &["script"]);
    answer(&task, &mut answers, Q_DETAIL, &["typo in the script"]);
    assert_eq!(visible_ids(&task, &answers), [Q_CORRECT, Q_WHY, Q_DETAIL]);
    assert_eq!(answers["alpha-task"].len(), 3);

    answer(&task, &mut answers, Q_CORRECT, &["yes"]);
    assert_eq!(visible_ids(&task, &answers), [Q_CORRECT]);
    assert_eq!(
        answers["alpha-task"].keys().collect::<Vec<_>>(),
        [Q_CORRECT]
    );
    // Hidden questions cannot be answered.
    assert!(set_answer(&task, &mut answers, Q_WHY, EvalAnswer::new(["audio"])).is_err());

    // An unpruned draft settles the same way.
    let mut draft = EvalAnswers::new();
    draft.entry("alpha-task".into()).or_default().extend([
        (Q_CORRECT.to_string(), EvalAnswer::new(["yes"])),
        (Q_WHY.to_string(), EvalAnswer::new(["script"])),
        (Q_DETAIL.to_string(), EvalAnswer::new(["x"])),
        ("stray".to_string(), EvalAnswer::new(["x"])),
    ]);
    assert_eq!(visible_ids(&task, &draft), [Q_CORRECT]);
    assert_eq!(
        input_responses(&task, &draft),
        [json!({"formInputId": Q_CORRECT, "value": "yes", "freetextValue": null})]
    );
    prune_hidden_answers(&task, &mut draft);
    assert_eq!(draft["alpha-task"].keys().collect::<Vec<_>>(), [Q_CORRECT]);
}

#[test]
fn answers_must_be_valid_complete_and_listened() {
    let task = task(naturalness());
    let mut answers = EvalAnswers::new();
    assert!(check_task_answers(&task, &answers, &listened(&[])).is_err());
    assert!(check_task_answers(&task, &answers, &listened(&["a", "b"])).is_err());
    answer(&task, &mut answers, "q", &["tie"]);
    assert!(check_task_answers(&task, &answers, &listened(&["a"])).is_err());
    check_task_answers(&task, &answers, &listened(&["a", "b"])).expect("complete");

    assert!(set_answer(&task, &mut answers, "q", EvalAnswer::new(["invalid"])).is_err());
    assert!(
        set_answer(
            &task,
            &mut answers,
            "q",
            EvalAnswer::new(["tie", "a_better"])
        )
        .is_err()
    );
    assert!(set_answer(&task, &mut answers, "missing", EvalAnswer::new(["tie"])).is_err());
    assert_eq!(answers["t"]["q"], EvalAnswer::new(["tie"]));

    // A stale draft value is still refused at check time.
    answers
        .get_mut("t")
        .expect("task")
        .insert("q".into(), EvalAnswer::new(["invalid"]));
    assert!(check_task_answers(&task, &answers, &listened(&["a", "b"])).is_err());
}

#[test]
fn input_responses_match_babel_build_response_per_type() {
    let raw = json!({
        "taskId": "t",
        "format": "tts-model",
        "subjects": [{"id": "a", "mediaUrl": "https://example.test/a.wav"}],
        "questions": [
            {"id": "mcq", "label": "M", "type": "mcq", "values": ["x", "y"], "sequenceNumber": 1},
            {"id": "likert", "label": "L", "type": "likert", "values": ["1", "2", "3"], "sequenceNumber": 2},
            {"id": "text", "label": "T", "type": "text", "sequenceNumber": 3},
            {"id": "dropdown", "label": "D", "type": "dropdown", "values": ["p", "q"], "sequenceNumber": 4},
            {"id": "sbs", "label": "S", "type": "side_by_side", "values": ["a_better", "b_better"], "sequenceNumber": 5},
            {"id": "multi", "label": "X", "type": "multiselect", "values": ["r", "s", "t"], "sequenceNumber": 6}
        ]
    });
    let task = task(raw);
    let mut answers = EvalAnswers::new();
    let mut set = |id: &str, answer: EvalAnswer| {
        set_answer(&task, &mut answers, id, answer).expect("answer");
    };
    set(
        "mcq",
        EvalAnswer::new(["y"]).with_comment("  sounds off \n"),
    );
    set("likert", EvalAnswer::new(["3"]).with_comment("   "));
    set("text", EvalAnswer::new(["  free, text  "]));
    set("dropdown", EvalAnswer::new(["p"]));
    set("sbs", EvalAnswer::new(["b_better"]).with_comment("B"));
    set("multi", EvalAnswer::new(["t", "r"]).with_comment("why"));
    assert_eq!(
        input_responses(&task, &answers),
        [
            json!({"formInputId": "mcq", "value": "y", "freetextValue": "sounds off"}),
            json!({"formInputId": "likert", "value": "3", "freetextValue": null}),
            json!({"formInputId": "text", "value": "  free, text  ", "freetextValue": null}),
            json!({"formInputId": "dropdown", "value": "p", "freetextValue": null}),
            json!({"formInputId": "sbs", "value": "b_better", "freetextValue": "B"}),
            json!({"formInputId": "multi", "value": "t,r", "freetextValue": "why", "selectedValues": ["t", "r"]}),
        ]
    );
    assert!(
        set_answer(&task, &mut answers, "multi", EvalAnswer::new(["r", "r"])).is_err(),
        "duplicate multiselect values"
    );
    // Clearing a text answer removes it.
    set_answer(&task, &mut answers, "text", EvalAnswer::new([""])).expect("clear");
    assert!(!answers["t"].contains_key("text"));
}

#[test]
fn comment_rules_follow_allow_comment_or_type_default() {
    let mut raw = alpha();
    raw["questions"][0]["metadata"] = json!({"allowComment": false});
    raw["questions"][1]["metadata"]["allowComment"] = json!(true);
    let task = task(raw);
    assert!(!task.questions()[0].comment_allowed);
    assert!(task.questions()[1].comment_allowed);
    let mut answers = EvalAnswers::new();
    assert!(
        set_answer(
            &task,
            &mut answers,
            Q_CORRECT,
            EvalAnswer::new(["no"]).with_comment("x")
        )
        .is_err()
    );
    answer(&task, &mut answers, Q_CORRECT, &["no"]);
    set_answer(
        &task,
        &mut answers,
        Q_WHY,
        EvalAnswer::new(["typo"]).with_comment("note"),
    )
    .expect("text with allowed comment");

    let default = self::task(alpha());
    assert!(default.questions()[0].comment_allowed);
    assert!(!default.questions()[1].comment_allowed);
    assert_eq!(default.questions()[1].kind, EvalQuestionType::Text);
}

#[test]
fn optional_questions_are_sent_only_when_answered() {
    let mut raw = naturalness();
    raw["questions"]
        .as_array_mut()
        .expect("questions")
        .push(json!({
            "id": "extra", "label": "Anything else?", "type": "mcq", "values": ["x"],
            "sequenceNumber": 2, "metadata": {"required": false}
        }));
    raw["questions"]
        .as_array_mut()
        .expect("questions")
        .push(json!({
            "id": "strict", "label": "Strict", "type": "mcq", "values": ["x"],
            "sequenceNumber": 3, "metadata": {"required": "false"}
        }));
    let task = task(raw);
    assert!(task.questions()[1].optional);
    assert!(
        !task.questions()[2].optional,
        "only required === false is optional"
    );
    let heard = listened(&["a", "b"]);
    let mut answers = EvalAnswers::new();
    answer(&task, &mut answers, "q", &["tie"]);
    answer(&task, &mut answers, "strict", &["x"]);
    check_task_answers(&task, &answers, &heard).expect("optional may stay empty");
    let ids = |answers: &EvalAnswers| {
        input_responses(&task, answers)
            .into_iter()
            .map(|response| {
                response["formInputId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&answers), ["q", "strict"]);
    // Comment only: sent with an empty value.
    set_answer(
        &task,
        &mut answers,
        "extra",
        EvalAnswer::default().with_comment("  just a note "),
    )
    .expect("comment");
    assert_eq!(
        input_responses(&task, &answers)[1],
        json!({"formInputId": "extra", "value": "", "freetextValue": "just a note"})
    );
    // A blank comment does not count as an answer.
    set_answer(
        &task,
        &mut answers,
        "extra",
        EvalAnswer::default().with_comment("   "),
    )
    .expect("blank comment");
    assert_eq!(ids(&answers), ["q", "strict"]);
    // A required question needs a value, a comment is not enough.
    set_answer(
        &task,
        &mut answers,
        "strict",
        EvalAnswer::default().with_comment("no value"),
    )
    .expect("comment only");
    assert!(check_task_answers(&task, &answers, &heard).is_err());
}

#[test]
fn progress_builders_check_tasks_and_build_payloads() {
    let first = task(naturalness());
    let mut second_raw = naturalness();
    second_raw["taskId"] = json!("t2");
    second_raw["subjects"][0]["id"] = json!("c");
    let second = task(second_raw);
    let session = exaudio_babel_services::babel::eval::EvalSession {
        session_id: "session-1".into(),
        tasks: vec![first.clone(), second.clone()],
    };
    let mut answers = EvalAnswers::new();
    answer(&first, &mut answers, "q", &["tie"]);
    let mut heard = EvalListened::new();
    heard.insert("t".into(), listened(&["a", "b"]));

    let save = EvalProgress::for_task("session-1", &first, &answers, &heard["t"]).expect("save");
    assert_eq!(
        BabelRequest::EvalSaveProgress(save).envelope(),
        json!({"operation": "evalTasks.saveProgress", "payload": {"sessionId": "session-1", "tasks": [
            {"taskId": "t", "inputResponses": [{"formInputId": "q", "value": "tie", "freetextValue": null}]}
        ]}})
    );
    let error = EvalProgress::for_session(&session, &answers, &heard).expect_err("second open");
    assert!(error.message.contains('2'));
    answer(&second, &mut answers, "q", &["a_better"]);
    heard.insert("t2".into(), listened(&["c", "b"]));
    let submit = EvalProgress::for_session(&session, &answers, &heard).expect("submit");
    assert_eq!(
        submit.payload(),
        json!({"sessionId": "session-1", "tasks": [
            {"taskId": "t", "inputResponses": [{"formInputId": "q", "value": "tie", "freetextValue": null}]},
            {"taskId": "t2", "inputResponses": [{"formInputId": "q", "value": "a_better", "freetextValue": null}]}
        ]})
    );
}

#[test]
fn eval_request_envelopes_and_relay_kinds() {
    let progress = EvalProgress {
        session_id: "s".into(),
        tasks: Vec::new(),
    };
    let cases = [
        (
            BabelRequest::EvalServeSession {
                project_id: "p".into(),
            },
            "evalServeSession",
            json!({"operation": "evalTasks.serveSessionTasks", "payload": {"projectId": "p"}}),
        ),
        (
            BabelRequest::EvalHoldSession {
                session_id: "s".into(),
            },
            "evalHoldSession",
            json!({"operation": "evalTasks.holdSession", "payload": {"sessionId": "s"}}),
        ),
        (
            BabelRequest::EvalSaveProgress(progress.clone()),
            "evalSaveProgress",
            json!({"operation": "evalTasks.saveProgress", "payload": {"sessionId": "s", "tasks": []}}),
        ),
        (
            BabelRequest::EvalSubmitSession(progress),
            "evalSubmitSession",
            json!({"operation": "evalTasks.submitSession", "payload": {"sessionId": "s", "tasks": []}}),
        ),
        (
            BabelRequest::LoadProjectInstructions {
                project_id: "p".into(),
            },
            "loadProjectInstructions",
            json!({"operation": "project.getProjectInstructions", "payload": {"projectId": "p"}}),
        ),
        (
            BabelRequest::LoadProjectRubricFlags {
                project_id: "p".into(),
            },
            "loadProjectRubricFlags",
            json!({"operation": "rubric.getProjectRubricFlags", "payload": {"projectId": "p"}}),
        ),
    ];
    for (request, kind, envelope) in cases {
        assert_eq!(request.relay_kind(), kind);
        assert_eq!(request.envelope(), envelope);
    }
}

#[tokio::test]
async fn eval_mutations_post_and_resource_queries_get() {
    let babel = MockServer::json(vec![
        r#"[{"result":{"data":{"json":{"kind":"empty","reason":"all_at_consensus"}}}}]"#,
        r#"[{"result":{"data":{"json":{"id":"i","instructions":"text"}}}}]"#,
        r#"[{"result":{"data":{"json":[]}}}]"#,
    ]);
    let config = BabelConfig {
        cookie: Some("__session=session-cookie".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    };
    let client = BabelClient::new(config);
    assert_eq!(
        client.serve_eval_session("p").await.expect("serve"),
        ServeOutcome::Empty {
            reason: "all_at_consensus".into()
        }
    );
    client
        .raw(&BabelRequest::LoadProjectInstructions {
            project_id: "p".into(),
        })
        .await
        .expect("instructions");
    client
        .raw(&BabelRequest::LoadProjectRubricFlags {
            project_id: "p".into(),
        })
        .await
        .expect("rubric");
    let requests = babel.join();
    assert!(contains(
        &requests[0],
        "POST /api/trpc/evalTasks.serveSessionTasks?batch=1"
    ));
    assert!(contains(
        &requests[0],
        r#"{"0":{"json":{"projectid":"p"}}}"#
    ));
    assert!(contains(
        &requests[1],
        "GET /api/trpc/project.getProjectInstructions?batch=1"
    ));
    assert!(contains(
        &requests[2],
        "GET /api/trpc/rubric.getProjectRubricFlags?batch=1"
    ));
}

fn fixture_client(reply: Value) -> BabelClient {
    BabelClient::new(BabelConfig {
        fixture: Some(Fixture::Json(reply.to_string())),
        ..BabelConfig::live("http://127.0.0.1:9")
    })
}

fn trpc(data: Value) -> Value {
    json!([{ "result": { "data": { "json": data } } }])
}

#[tokio::test]
async fn serve_outcomes_cover_every_session_kind() {
    let served = fixture_client(trpc(json!({
        "kind": "session", "sessionId": "session-1", "tasks": [naturalness(), {"taskId": "bad"}]
    })))
    .serve_eval_session("p")
    .await
    .expect("session");
    let ServeOutcome::Session(session) = served else {
        panic!("expected a session, got {served:?}");
    };
    assert_eq!(session.session_id, "session-1");
    assert_eq!(session.tasks.len(), 2);
    assert!(session.tasks[0].is_valid());
    assert!(validate_task(&session.tasks[1]).is_err());
    assert_eq!(session.task("bad").map(EvalTask::task_id), Some("bad"));

    for (reply, expected) in [
        (
            json!({"kind": "empty", "reason": "all_at_consensus"}),
            ServeOutcome::Empty {
                reason: "all_at_consensus".into(),
            },
        ),
        (
            json!({"kind": "topic_selection", "batchId": "b"}),
            ServeOutcome::Unsupported {
                kind: "topic_selection".into(),
            },
        ),
        (
            json!({"kind": "staged_session", "sessionId": "s"}),
            ServeOutcome::Unsupported {
                kind: "staged_session".into(),
            },
        ),
    ] {
        assert_eq!(
            fixture_client(trpc(reply))
                .serve_eval_session("p")
                .await
                .expect("outcome"),
            expected
        );
    }
    assert!(
        fixture_client(trpc(
            json!({"kind": "session", "sessionId": "s", "tasks": []})
        ))
        .serve_eval_session("p")
        .await
        .is_err()
    );
    for tasks in [
        json!([naturalness(), naturalness()]),
        json!([naturalness(), {"format": "default"}]),
        json!([{"taskId": "  "}]),
    ] {
        assert!(
            fixture_client(trpc(
                json!({"kind": "session", "sessionId": "s", "tasks": tasks})
            ))
            .serve_eval_session("p")
            .await
            .is_err(),
            "duplicate or empty task ids"
        );
    }
    let error = fixture_client(json!([{"error": {"json": {"message": "Project is paused", "data": {"code": "FORBIDDEN"}}}}]))
        .serve_eval_session("p")
        .await
        .expect_err("trpc error");
    assert!(error.message.contains("Project is paused"));
}

#[tokio::test]
async fn debug_output_never_shows_signed_media_urls() {
    let mut task = naturalness();
    task["subjects"][0]["mediaUrl"] =
        json!("https://cdn.example.test/a.wav?X-Amz-Signature=SECRETSIG123");
    let mut invalid = task.clone();
    invalid["taskId"] = json!("t2");
    invalid["format"] = json!("unknown");
    let outcome = fixture_client(trpc(json!({
        "kind": "session", "sessionId": "s", "tasks": [task, invalid]
    })))
    .serve_eval_session("p")
    .await
    .expect("session");
    let printed = format!("{outcome:?} {outcome:#?}");
    assert!(!printed.contains("SECRETSIG123"), "{printed}");
    assert!(!printed.contains("cdn.example.test"), "{printed}");
    assert!(printed.contains("<redacted>"));
}

#[tokio::test]
async fn hold_save_and_submit_replies() {
    let progress = EvalProgress {
        session_id: "s".into(),
        tasks: Vec::new(),
    };
    assert!(
        fixture_client(trpc(json!(true)))
            .hold_eval_session("s")
            .await
            .expect("hold")
    );
    assert!(
        !fixture_client(trpc(json!(false)))
            .hold_eval_session("s")
            .await
            .expect("hold")
    );
    assert!(
        fixture_client(trpc(json!({"x": 1})))
            .hold_eval_session("s")
            .await
            .is_err()
    );
    assert!(
        fixture_client(trpc(json!({"kind": "saved"})))
            .save_eval_progress(&progress)
            .await
            .expect("save")
    );
    assert!(
        !fixture_client(trpc(json!({"kind": "stale"})))
            .save_eval_progress(&progress)
            .await
            .expect("save")
    );
    for (reply, expected) in [
        (json!({"kind": "completed"}), SubmitOutcome::Completed),
        (
            json!({"kind": "incomplete", "missingTaskIds": ["t"]}),
            SubmitOutcome::Incomplete,
        ),
        (
            json!({"kind": "expired"}),
            SubmitOutcome::Other("expired".into()),
        ),
    ] {
        assert_eq!(
            fixture_client(trpc(reply))
                .submit_eval_session(&progress)
                .await
                .expect("submit"),
            expected
        );
    }
    assert!(
        fixture_client(trpc(json!({})))
            .submit_eval_session(&progress)
            .await
            .is_err()
    );
}

#[test]
fn draft_store_isolates_sessions_and_falls_back_to_backup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let drafts = dir.path().join("drafts");
    let store = EvalDraftStore::new(&drafts);
    let project = "3882e77e-3ace-4ff2-b2df-290e3439cfce";
    let load = |project: &str, session: &str| store.load(project, session).expect("readable");
    let mut first = EvalAnswers::new();
    first
        .entry("t".into())
        .or_default()
        .insert("q".into(), EvalAnswer::new(["tie"]).with_comment("note"));
    assert!(load(project, "s1").is_empty());
    store.save(project, "s1", &first).expect("save");
    assert_eq!(load(project, "s1"), first);
    assert!(load(project, "s2").is_empty(), "other session ignored");
    assert!(load("another-project", "s1").is_empty());

    let mut second = first.clone();
    second
        .get_mut("t")
        .expect("task")
        .insert("q".into(), EvalAnswer::new(["a_better"]));
    store.save(project, "s1", &second).expect("save again");
    assert_eq!(load(project, "s1"), second);

    let main = drafts.join(format!("eval-{project}.json"));
    let saved = std::fs::read_to_string(&main).expect("draft");
    assert!(!saved.contains("https://"), "no media URLs persisted");
    std::fs::write(&main, "{ truncated").expect("corrupt");
    assert_eq!(load(project, "s1"), first, "backup holds the previous save");

    // The unreadable main file is moved aside, not overwritten, and the backup survives.
    store.save(project, "s2", &second).expect("new session");
    assert_eq!(load(project, "s2"), second);
    assert!(load(project, "s1").is_empty());
    let aside: Vec<_> = std::fs::read_dir(&drafts)
        .expect("dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(&format!("eval-{project}.corrupt-")) && name.ends_with(".json")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        std::fs::read_to_string(aside[0].path()).expect("aside"),
        "{ truncated"
    );

    assert!(store.save("../escape", "s1", &first).is_err());
    assert!(load("../escape", "s1").is_empty());
}

#[test]
fn unreadable_drafts_fail_loading_and_fork_drafts_are_read() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = EvalDraftStore::new(dir.path());
    let project = "d35d73aa-4816-4571-b77d-960ddb15eeae";
    let main = dir.path().join(format!("eval-{project}.json"));
    let backup = dir.path().join(format!("eval-{project}.backup.json"));

    std::fs::write(&main, "garbage").expect("main");
    assert!(store.load(project, "s").is_err(), "unreadable, no backup");
    std::fs::write(&backup, "garbage too").expect("backup");
    assert!(store.load(project, "s").is_err(), "neither copy readable");
    std::fs::remove_file(&main).expect("remove main");
    assert!(
        store.load(project, "s").is_err(),
        "only an unreadable backup"
    );
    std::fs::write(
        &main,
        r#"{"version":2,"projectId":"x","sessionId":"s","answers":{}}"#,
    )
    .expect("future");
    assert!(store.load(project, "s").is_err(), "unknown version");

    std::fs::remove_file(&backup).expect("remove backup");
    std::fs::write(
        &main,
        r#"{"sessionId":"old","answers":{"t":{"q":"tie","why":""},"t2":{"q":"b_better"}}}"#,
    )
    .expect("fork draft");
    let mut expected = EvalAnswers::new();
    expected
        .entry("t".into())
        .or_default()
        .insert("q".into(), EvalAnswer::new(["tie"]));
    expected
        .entry("t2".into())
        .or_default()
        .insert("q".into(), EvalAnswer::new(["b_better"]));
    assert_eq!(store.load(project, "old").expect("fork draft"), expected);
    assert!(
        store
            .load(project, "new")
            .expect("other session")
            .is_empty()
    );
}

#[test]
fn eval_projects_without_queues_are_listed_but_never_claimable() {
    let dashboard = babel::parse_dashboard(
        &json!({
            "worker": { "id": "worker-1" },
            "projects": [
                {"projectId": "eval-by-kind", "name": "Some Eval", "status": "active",
                 "productKind": "eval_session"},
                {"projectId": "3882e77e-3ace-4ff2-b2df-290e3439cfce", "name": "RU-Alpha-Eval",
                 "permissionStatus": "active",
                 "workerStats": {"tasksCompletedToday": 3},
                 "project": {"id": "d35d73aa-4816-4571-b77d-960ddb15eeae"},
                 "form": {"id": "f-1", "productKind": "eval_task"}},
                {"projectId": "2c68b806-ca86-419f-83ea-2551e85e4619", "name": "RU-Naturalness-Eval2",
                 "status": "active", "chunkedQueueId": "should-not-claim"},
                {"projectId": "plain", "name": "No queues", "status": "active"},
                {"projectId": "paused-eval", "name": "Paused", "status": "paused", "productKind": "eval_task"},
                {"projectId": "tx", "name": "RU-tx", "status": "active", "chunkedQueueId": "q-1"}
            ],
            "projectConfigs": [
                {"projectId": "2c68b806-ca86-419f-83ea-2551e85e4619", "chunkedQueueId": "q-eval"}
            ]
        })
        .to_string(),
    )
    .expect("dashboard");
    let listed: Vec<_> = dashboard
        .projects
        .iter()
        .map(|project| (project.id.as_str(), project.is_evaluation()))
        .collect();
    assert_eq!(
        listed,
        [
            ("3882e77e-3ace-4ff2-b2df-290e3439cfce", true),
            ("2c68b806-ca86-419f-83ea-2551e85e4619", true),
            ("tx", false),
            ("eval-by-kind", true),
        ]
    );
    for project in dashboard.projects.iter().filter(|p| p.is_evaluation()) {
        assert!(project.chunked_queue_id.is_empty() && project.stitched_queue_id.is_empty());
    }
    assert_eq!(dashboard.projects[0].tasks_completed_today, 3);
    assert_eq!(
        dashboard.projects[3].product_kind.as_deref(),
        Some("eval_session")
    );
    assert_eq!(dashboard.projects[2].chunked_queue_id, "q-1");
}

#[tokio::test]
async fn live_dashboard_skips_queue_lookup_for_eval_projects() {
    let babel = MockServer::json(vec![
        r#"[{"result":{"data":{"json":{"id":"worker-1"}}}},{"result":{"data":{"json":[{"projectId":"project-1","name":"Project 1"},{"projectId":"3882e77e-3ace-4ff2-b2df-290e3439cfce","name":"RU-Alpha-Eval"}]}}}]"#,
        r#"[{"result":{"data":{"json":{"chunkedQueueId":"queue-1","stitchedTxQueueId":"stitched-1"}}}}]"#,
    ]);
    let client = BabelClient::new(BabelConfig {
        cookie: Some("__session=session-cookie".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    });
    let dashboard = client.load_dashboard().await.expect("dashboard");
    assert_eq!(dashboard.projects.len(), 2);
    assert_eq!(dashboard.projects[0].chunked_queue_id, "queue-1");
    assert!(dashboard.projects[1].is_evaluation());
    let requests = babel.join();
    assert!(contains(
        &requests[1],
        "GET /api/trpc/transcriptions.getProjectQueues?batch=1"
    ));
}

fn pcm16_wav_channels(channels: u16, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let align = 2 * channels;
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&(48_000u32 * u32::from(align)).to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

fn pcm16_wav(samples: &[i16]) -> Vec<u8> {
    pcm16_wav_channels(1, samples)
}

#[test]
fn clips_decode_streaming_wav_and_reject_damaged_audio() {
    let samples = [16384i16, -16384, 8192, -8192];
    let audio = decode_eval_clip(&pcm16_wav(&samples)).expect("wav");
    assert_eq!(
        audio.channel_data(0).expect("mono"),
        &[0.5, -0.5, 0.25, -0.25]
    );

    // Streaming writers leave the RIFF and data sizes at 0xFFFFFFFF.
    let mut streaming = pcm16_wav(&samples);
    streaming[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    streaming[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    let audio = decode_eval_clip(&streaming).expect("streaming wav");
    assert_eq!(
        audio.channel_data(0).expect("mono"),
        &[0.5, -0.5, 0.25, -0.25]
    );

    let mut truncated = pcm16_wav(&samples);
    truncated.truncate(truncated.len() - 3);
    assert!(decode_eval_clip(&truncated).is_err());
    assert!(decode_eval_clip(&pcm16_wav(&[])).is_err());
    assert!(decode_eval_clip(b"<html>Access denied</html>").is_err());
    // Clips are decoded within EVAL_CLIP_DECODE_LIMITS (9 channels exceed them).
    assert!(decode_eval_clip(&pcm16_wav_channels(9, &[0; 9])).is_err());
}

#[tokio::test]
async fn clip_download_requires_https() {
    assert!(fetch_eval_clip("http://example.test/a.wav").await.is_err());
    assert!(fetch_eval_clip("not a url").await.is_err());
}
