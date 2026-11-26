//! Exaudio's side of `tools/eval-parity`: reads a corpus of Babel eval tasks with answer sets
//! (the file `tools/eval-parity/corpus.mjs` writes) and prints, per case, the visible
//! questions, the questions offering a comment and the `inputResponses` that would be sent,
//! or that the answers are refused.
//!
//! The answers are held as Babel's form holds them (one value list and one comment per
//! question, set together) and pruned like its visibility effect, so the comparison covers
//! the rules, not the order in which a user clicked.

use std::collections::{BTreeMap, BTreeSet};

use exaudio_babel_services::babel::eval::{
    EvalAnswer, EvalAnswers, EvalTask, check_task_answers, input_responses, prune_hidden_answers,
    validate_task, visible_questions,
};
use serde_json::{Value, json};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: eval_parity <corpus.json>");
    let corpus: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read corpus"))
            .expect("corpus is a JSON array");
    let results: Vec<Value> = corpus.iter().map(evaluate).collect();
    println!("{}", Value::Array(results));
}

fn evaluate(case: &Value) -> Value {
    let id = case["id"].clone();
    let task = EvalTask::new(case["task"].clone());
    if let Err(error) = validate_task(&task) {
        return json!({ "id": id, "visible": [], "commentable": [], "refused": true,
            "responses": null, "error": format!("task rejected: {}", error.message) });
    }
    let task_id = task.task_id().to_string();
    let mut per_question: BTreeMap<String, EvalAnswer> = BTreeMap::new();
    for (question, values) in case["answers"].as_object().into_iter().flatten() {
        let values = values
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        per_question.entry(question.clone()).or_default().values = values;
    }
    for (question, comment) in case["comments"].as_object().into_iter().flatten() {
        per_question.entry(question.clone()).or_default().comment =
            comment.as_str().map(str::to_string);
    }
    let mut answers: EvalAnswers = BTreeMap::from([(task_id, per_question)]);
    prune_hidden_answers(&task, &mut answers);

    let visible = visible_questions(&task, &answers);
    let listened: BTreeSet<String> = task
        .subjects()
        .iter()
        .filter(|subject| subject.is_audio())
        .map(|subject| subject.id.clone())
        .collect();
    let checked = check_task_answers(&task, &answers, &listened);
    json!({
        "id": id,
        "visible": visible.iter().map(|question| question.id.clone()).collect::<Vec<_>>(),
        "commentable": visible
            .iter()
            .filter(|question| question.comment_allowed)
            .map(|question| question.id.clone())
            .collect::<Vec<_>>(),
        "refused": checked.is_err(),
        "responses": if checked.is_ok() { Value::Array(input_responses(&task, &answers)) } else { Value::Null },
        "error": checked.err().map(|error| error.message),
    })
}
