//! Headless end-to-end eval run for `tools/eval-smoke/run.mjs --headless`: the same client,
//! relay and eval code the eval screen uses, without a window. Talks to the relay at
//! `EVAL_E2E_RELAY` as the sub-account session `EVAL_E2E_TOKEN`; `EVAL_E2E_OWNER_TOKEN` is the
//! relay's owner token, used to prove the owner cannot take over the sub-account's session.
//! Prints one JSON line per step and exits non-zero at the first failed expectation.

use std::collections::{BTreeMap, BTreeSet};

use exaudio_babel_services::babel::BabelClient;
use exaudio_babel_services::babel::eval::{
    EvalAnswer, EvalAnswers, EvalDraftStore, EvalListened, EvalProgress, EvalSession, ServeOutcome,
    SubmitOutcome, check_task_answers, fetch_eval_clip, set_answer, validate_task,
};
use exaudio_babel_services::subaccounts::SubaccountSession;
use serde_json::json;

fn step(name: &str, detail: serde_json::Value) {
    println!("{}", json!({ "step": name, "detail": detail }));
}

fn fail(name: &str, message: impl std::fmt::Display) -> ! {
    println!("{}", json!({ "step": name, "failed": message.to_string() }));
    std::process::exit(1);
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail("env", format!("{name} is not set")))
}

fn client(host_url: &str, token: String, id: &str) -> BabelClient {
    BabelClient::relay(SubaccountSession {
        host_url: host_url.to_string(),
        token,
        subaccount_id: id.to_string(),
        display_name: id.to_string(),
    })
}

/// The answer the smoke fixture expects for one question, by its label.
fn answer_for(label: &str) -> Option<EvalAnswer> {
    match label {
        "Which clip sounds more natural?" => {
            Some(EvalAnswer::new(["tie"]).with_comment("  sounds equal  "))
        }
        "Was the script read correctly?" => Some(EvalAnswer::new(["no"])),
        "What was wrong?" => Some(EvalAnswer::new(["Read the date as digits"])),
        "Which issues apply?" => Some(EvalAnswer::new(["digits", "pause"])),
        _ => None,
    }
}

fn main() {
    let relay = env("EVAL_E2E_RELAY");
    let sub = client(&relay, env("EVAL_E2E_TOKEN"), "smoke-sub");
    let owner = client(&relay, env("EVAL_E2E_OWNER_TOKEN"), "owner");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async move {
        let dashboard = sub
            .load_dashboard()
            .await
            .unwrap_or_else(|error| fail("dashboard", error.message));
        let project = dashboard
            .projects
            .iter()
            .find(|project| project.is_evaluation())
            .unwrap_or_else(|| fail("dashboard", "no evaluation project listed"))
            .clone();
        step(
            "dashboard",
            json!({ "project": project.name, "productKind": project.product_kind }),
        );

        let resources = sub.load_project_resources(&project.id).await;
        if let Err(error) = resources
            .instructions
            .as_ref()
            .map(|_| ())
            .and(resources.grading.as_ref().map(|_| ()))
            .and(resources.library.as_ref().map(|_| ()))
        {
            fail("resources", &error.message);
        }
        step(
            "resources",
            json!("instructions, grading guide and library loaded"),
        );

        let session: EvalSession = match sub.serve_eval_session(&project.id).await {
            Ok(ServeOutcome::Session(session)) => session,
            Ok(other) => fail("serve", format!("expected a session, got {other:?}")),
            Err(error) => fail("serve", error.message),
        };
        for task in &session.tasks {
            validate_task(task).unwrap_or_else(|error| fail("validate", error.message));
        }
        step(
            "serve",
            json!({ "session": session.session_id, "forms": session.tasks.len() }),
        );

        // The relay binds the session to this sub-account: the owner resuming it is refused.
        match owner.serve_eval_session(&project.id).await {
            Err(error) => step("owner-takeover-refused", json!(error.message)),
            Ok(outcome) => fail(
                "owner-takeover-refused",
                format!("owner was served {outcome:?}"),
            ),
        }

        let mut answers: EvalAnswers = BTreeMap::new();
        let mut listened: EvalListened = BTreeMap::new();
        let drafts_dir = std::env::temp_dir().join(format!("eval-e2e-{}", std::process::id()));
        let drafts = EvalDraftStore::new(&drafts_dir);
        for (index, task) in session.tasks.iter().enumerate() {
            for question in task.questions() {
                if let Some(answer) = answer_for(&question.label) {
                    // Conditional questions appear once their controlling answer is set.
                    let _ = set_answer(task, &mut answers, &question.id, answer);
                }
            }
            for question in task.questions() {
                if let Some(answer) = answer_for(&question.label) {
                    set_answer(task, &mut answers, &question.id, answer).unwrap_or_else(|error| {
                        fail("answer", format!("{}: {}", question.label, error.message))
                    });
                }
            }
            if check_task_answers(task, &answers, &BTreeSet::new()).is_ok() {
                fail("listen-rule", "answers accepted before any clip was played");
            }
            let heard = listened.entry(task.task_id().to_string()).or_default();
            for subject in task.subjects().iter().filter(|subject| subject.is_audio()) {
                let url = subject.media_url.as_deref().unwrap_or_default();
                let clip = fetch_eval_clip(url)
                    .await
                    .unwrap_or_else(|error| fail("clip", error.message));
                step(
                    "clip",
                    json!({ "form": index + 1, "subject": subject.id, "seconds": clip.duration() }),
                );
                heard.insert(subject.id.clone());
            }
            drafts
                .save(&project.id, &session.session_id, &answers)
                .unwrap_or_else(|error| fail("draft", error.message));
            let restored = drafts
                .load(&project.id, &session.session_id)
                .unwrap_or_else(|error| fail("draft", error.message));
            if restored != answers {
                fail("draft", "draft did not round-trip");
            }
            let progress = EvalProgress::for_task(&session.session_id, task, &answers, heard)
                .unwrap_or_else(|error| fail("save", error.message));
            match sub.save_eval_progress(&progress).await {
                Ok(true) => step(
                    "save",
                    json!({ "form": index + 1, "payload": progress.payload() }),
                ),
                Ok(false) => fail("save", "Babel did not confirm saving"),
                Err(error) => fail("save", error.message),
            }
        }
        let _ = std::fs::remove_dir_all(&drafts_dir);

        match sub.hold_eval_session(&session.session_id).await {
            Ok(true) => step("hold", json!(true)),
            other => fail("hold", format!("{other:?}")),
        }
        let progress = EvalProgress::for_session(&session, &answers, &listened)
            .unwrap_or_else(|error| fail("submit", error.message));
        match sub.submit_eval_session(&progress).await {
            Ok(SubmitOutcome::Completed) => step("submit", json!("completed")),
            other => fail("submit", format!("{other:?}")),
        }
        match sub.serve_eval_session(&project.id).await {
            Ok(ServeOutcome::Empty { reason }) => {
                step("serve-after-submit", json!({ "empty": reason }))
            }
            other => fail("serve-after-submit", format!("{other:?}")),
        }
    });
}
