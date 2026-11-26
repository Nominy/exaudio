//! Babel Eval sessions relayed through `/api/subaccounts/babel-service` (request kinds
//! `evalServeSession`, `evalHoldSession`, `evalSaveProgress`, `evalSubmitSession`).
//!
//! Eval calls run one at a time. A served session is bound (persistently) to its caller — the
//! owner, authenticated with the configured owner token, or one sub-account — and its project,
//! so only that caller can hold, save, or submit it, and only for the session's own tasks.

use super::*;

const SERVE: &str = "evalTasks.serveSessionTasks";
const HOLD: &str = "evalTasks.holdSession";
const SAVE: &str = "evalTasks.saveProgress";
const SUBMIT: &str = "evalTasks.submitSession";
const OWNER_CALLER: &str = "owner";

/// Who works on a served eval session, its project, and the task ids Babel served for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EvalBinding {
    caller: String,
    project_id: String,
    task_ids: BTreeSet<String>,
}

/// The only operation each eval request kind may carry.
pub(super) fn operation_for_kind(request_kind: &str) -> Option<&'static str> {
    match request_kind {
        "evalServeSession" => Some(SERVE),
        "evalHoldSession" => Some(HOLD),
        "evalSaveProgress" => Some(SAVE),
        "evalSubmitSession" => Some(SUBMIT),
        _ => None,
    }
}

/// Eval operations are only accepted under their eval request kind.
pub(super) fn mentions_eval_operation(envelope: &Value) -> bool {
    envelope
        .get("operation")
        .and_then(Value::as_str)
        .is_some_and(|operation| operation.to_ascii_lowercase().contains("evaltasks."))
}

pub(super) async fn handle(
    state: &RelayState,
    headers: &HeaderMap,
    operation: &'static str,
    envelope: Value,
) -> Result<Json<Value>, RelayError> {
    if field(&envelope, "operation") != operation {
        return Err(RelayError::forbidden("Unsupported evaluation operation."));
    }
    if envelope.get("batchInput").is_some() {
        return Err(RelayError::bad_request(
            "Evaluation batches are not supported.",
        ));
    }
    let payload = envelope
        .get("payload")
        .filter(|payload| payload.is_object())
        .ok_or_else(|| RelayError::bad_request("Missing evaluation payload."))?;
    let session_id = field(payload, "sessionId").to_string();

    let _turn = state.eval_lock.lock().await;
    let (caller, subaccount_id, project_id) = {
        let store = state.store.lock().await;
        let (caller, subaccount) = if is_owner(headers, &state.config) {
            (OWNER_CALLER.to_string(), None)
        } else {
            let (subaccount, _) = authenticate_subaccount(headers, &store)?;
            (format!("sub:{}", subaccount.id), Some(subaccount))
        };
        let binding = if operation == SERVE {
            None
        } else {
            Some(
                store
                    .eval_sessions
                    .get(&session_id)
                    .filter(|_| !session_id.is_empty())
                    .ok_or_else(|| {
                        RelayError::new(
                            StatusCode::CONFLICT,
                            "Load the evaluation session before continuing.",
                        )
                    })?,
            )
        };
        let project_id = match binding {
            Some(binding) => binding.project_id.clone(),
            None => {
                let project_id = field(payload, "projectId");
                if project_id.trim().is_empty() {
                    return Err(RelayError::bad_request("Project ID is required."));
                }
                project_id.to_string()
            }
        };
        if let Some(subaccount) = subaccount
            && !subaccount_may_access_project(subaccount, &project_id)
        {
            return Err(RelayError::forbidden(
                "Sub-account is not assigned to this project.",
            ));
        }
        if let Some(binding) = binding {
            authorize_binding(binding, &caller, payload, operation)?;
        }
        (
            caller,
            subaccount.map(|subaccount| subaccount.id.clone()),
            project_id,
        )
    };

    let response = state.execute_envelope(envelope).await?;
    let mut store = state.store.lock().await;
    // A sub-account revoked or unassigned while the call was upstream gets neither a binding
    // nor the reply.
    let still_authorized = subaccount_id.as_deref().is_none_or(|id| {
        store.subaccounts.iter().any(|subaccount| {
            subaccount.id == id
                && !subaccount.revoked
                && subaccount_may_access_project(subaccount, &project_id)
        })
    });
    // Upstream errors carry no data and leave every binding unchanged.
    let data = response_data(&response).unwrap_or(Value::Null);
    match (operation, field(&data, "kind")) {
        (SERVE, "session") if still_authorized => {
            let (served_id, binding) = served_binding(&data, caller, project_id)?;
            if let Some(existing) = store.eval_sessions.get(&served_id)
                && (existing.caller != binding.caller || existing.project_id != binding.project_id)
            {
                return Err(RelayError::new(
                    StatusCode::CONFLICT,
                    "Babel resumed an evaluation session another client is working on. Finish that session first.",
                ));
            }
            store.eval_sessions.insert(served_id, binding);
            state.save(&mut store).await?;
        }
        // Babel finished the session either way; its binding is spent.
        (SUBMIT, "completed") => {
            store.eval_sessions.remove(&session_id);
            state.save(&mut store).await?;
        }
        _ => {}
    }
    if !still_authorized {
        return Err(RelayError::forbidden(
            "Sub-account lost access to this project.",
        ));
    }
    Ok(Json(json!({ "payload": response })))
}

/// The owner only when an owner token is configured and presented; a relay without one never
/// grants owner privilege on eval sessions.
fn is_owner(headers: &HeaderMap, config: &RelayConfig) -> bool {
    config.owner_token.is_some() && require_owner(headers, config).is_ok()
}

fn authorize_binding(
    binding: &EvalBinding,
    caller: &str,
    payload: &Value,
    operation: &str,
) -> Result<(), RelayError> {
    if binding.caller != caller {
        return Err(RelayError::new(
            StatusCode::CONFLICT,
            "This evaluation session belongs to another client.",
        ));
    }
    if operation != SAVE && operation != SUBMIT {
        return Ok(());
    }
    let tasks = payload
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| RelayError::bad_request("Evaluation tasks are required."))?;
    let mut seen = BTreeSet::new();
    for task in tasks {
        let task_id = field(task, "taskId");
        if !binding.task_ids.contains(task_id) || !seen.insert(task_id) {
            return Err(RelayError::forbidden(
                "Unknown or duplicate evaluation task.",
            ));
        }
    }
    if operation == SUBMIT && seen.len() != binding.task_ids.len() {
        return Err(RelayError::bad_request(
            "Submit requires every task in the session.",
        ));
    }
    Ok(())
}

/// Session id and binding of a served `{kind:"session", sessionId, tasks[]}`.
fn served_binding(
    data: &Value,
    caller: String,
    project_id: String,
) -> Result<(String, EvalBinding), RelayError> {
    let session_id = field(data, "sessionId");
    let tasks = data
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| RelayError::bad_gateway("Evaluation session has no tasks."))?;
    let task_ids: BTreeSet<String> = tasks
        .iter()
        .map(|task| field(task, "taskId").to_string())
        .collect();
    if session_id.is_empty()
        || task_ids.is_empty()
        || task_ids.contains("")
        || task_ids.len() != tasks.len()
    {
        return Err(RelayError::bad_gateway(
            "Invalid evaluation session identifiers.",
        ));
    }
    Ok((
        session_id.to_string(),
        EvalBinding {
            caller,
            project_id,
            task_ids,
        },
    ))
}

/// tRPC result data of an upstream reply (batch, wrapped, or bare `{kind, ...}`); `None` for
/// errors and unreadable replies.
fn response_data(response: &str) -> Option<Value> {
    let value =
        serde_json::from_str::<Value>(response.trim_start_matches('\u{feff}').trim()).ok()?;
    let item = match value {
        Value::Array(items) => items.into_iter().next()?,
        other => other,
    };
    ["/result/data/json", "/data/json", "/result/data", "/data"]
        .into_iter()
        .find_map(|pointer| item.pointer(pointer).cloned())
        .or_else(|| item.get("kind").is_some().then_some(item))
}

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;

    fn binding() -> EvalBinding {
        EvalBinding {
            caller: "sub:one".to_string(),
            project_id: "project".to_string(),
            task_ids: BTreeSet::from(["a".to_string(), "b".to_string()]),
        }
    }

    #[test]
    fn rejects_other_callers_and_unknown_or_duplicate_tasks() {
        let binding = binding();
        let error = authorize_binding(&binding, "owner", &json!({}), HOLD).unwrap_err();
        assert_eq!(error.status, StatusCode::CONFLICT);
        for tasks in [
            json!([{ "taskId": "unknown" }]),
            json!([{ "taskId": "a" }, { "taskId": "a" }]),
            json!([{ "other": "a" }]),
        ] {
            let error = authorize_binding(&binding, "sub:one", &json!({ "tasks": tasks }), SAVE)
                .unwrap_err();
            assert_eq!(error.status, StatusCode::FORBIDDEN, "{tasks}");
        }
        assert!(authorize_binding(&binding, "sub:one", &json!({}), SAVE).is_err());
        assert!(authorize_binding(&binding, "sub:one", &json!({}), HOLD).is_ok());
    }

    #[test]
    fn progress_can_be_partial_but_submit_must_cover_the_session() {
        let binding = binding();
        let partial = json!({ "tasks": [{ "taskId": "a" }] });
        assert!(authorize_binding(&binding, "sub:one", &partial, SAVE).is_ok());
        assert!(authorize_binding(&binding, "sub:one", &partial, SUBMIT).is_err());
        let full = json!({ "tasks": [{ "taskId": "a" }, { "taskId": "b" }] });
        assert!(authorize_binding(&binding, "sub:one", &full, SUBMIT).is_ok());
    }

    #[test]
    fn eval_operations_are_tied_to_their_kinds() {
        assert_eq!(operation_for_kind("evalServeSession"), Some(SERVE));
        assert_eq!(operation_for_kind("evalSubmitSession"), Some(SUBMIT));
        assert_eq!(operation_for_kind("loadWorkerProjects"), None);
        assert!(mentions_eval_operation(
            &json!({ "operation": "worker.get,evalTasks.abandonSession" })
        ));
        assert!(!mentions_eval_operation(
            &json!({ "operation": "project.getProjectInstructions" })
        ));
    }

    #[test]
    fn response_data_reads_trpc_shapes_and_skips_errors() {
        for response in [
            r#"[{"result":{"data":{"json":{"kind":"saved"}}}}]"#,
            r#"{"result":{"data":{"json":{"kind":"saved"}}}}"#,
            "\u{feff}{\"kind\":\"saved\"}",
        ] {
            assert_eq!(
                response_data(response),
                Some(json!({ "kind": "saved" })),
                "{response}"
            );
        }
        assert_eq!(
            response_data(r#"[{"error":{"json":{"message":"nope"}}}]"#),
            None
        );
        assert_eq!(response_data("not json"), None);
    }

    struct Harness {
        _temp: tempfile::TempDir,
        base_url: String,
        server: tokio::task::JoinHandle<()>,
        calls: Arc<AtomicUsize>,
        store_path: PathBuf,
    }

    /// A relay whose upstream serves session `s` (tasks `a`, `b`), saves, and completes, with
    /// sub-account `one` (token `sub-one`, project `project`) and `two` (token `sub-two`,
    /// unrestricted).
    async fn harness() -> Harness {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        let mut store = RelayStore::default();
        for (id, projects) in [("one", vec!["project".to_string()]), ("two", Vec::new())] {
            store.subaccounts.push(SubaccountRecord {
                id: id.to_string(),
                label: id.to_string(),
                code: format!("code-{id}"),
                revoked: false,
                allowed_project_ids: projects,
                analytics: AnalyticsRecord::default(),
                active_claims: BTreeMap::new(),
            });
            store.sessions.push(SessionRecord {
                token: format!("sub-{id}"),
                subaccount_id: id.to_string(),
                created_at: 1,
                last_seen_at: 1,
            });
        }
        store.save(&store_path).await.expect("seed store");
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let state = crate::tests::test_state(
            store_path.clone(),
            Arc::new(AtomicI64::new(1)),
            Arc::new(move |envelope| {
                counted.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    let data = match field(&envelope, "operation") {
                        SERVE => json!({
                            "kind": "session",
                            "sessionId": "s",
                            "tasks": [{ "taskId": "a" }, { "taskId": "b" }],
                        }),
                        HOLD => json!(true),
                        SAVE => json!({ "kind": "saved" }),
                        SUBMIT => json!({ "kind": "completed" }),
                        other => {
                            return Err(RelayError::bad_gateway(format!("unexpected {other}")));
                        }
                    };
                    Ok(json!([{ "result": { "data": { "json": data } } }]).to_string())
                })
            }),
            0,
        )
        .await;
        let (base_url, server) = crate::tests::spawn_server(state).await;
        Harness {
            _temp: temp,
            base_url,
            server,
            calls,
            store_path,
        }
    }

    impl Harness {
        async fn send(&self, token: &str, kind: &str, envelope: Value) -> reqwest::Response {
            reqwest::Client::new()
                .post(format!("{}/api/subaccounts/babel-service", self.base_url))
                .bearer_auth(token)
                .json(&json!({ "requestKind": kind, "envelope": envelope }))
                .send()
                .await
                .expect("relay response")
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        async fn bindings(&self) -> BTreeMap<String, EvalBinding> {
            RelayStore::load(&self.store_path)
                .await
                .expect("store")
                .eval_sessions
        }
    }

    fn serve(project_id: &str) -> Value {
        json!({ "operation": SERVE, "payload": { "projectId": project_id } })
    }

    fn submit(tasks: &[&str]) -> Value {
        json!({
            "operation": SUBMIT,
            "payload": {
                "sessionId": "s",
                "tasks": tasks.iter().map(|id| json!({ "taskId": id, "inputResponses": [] })).collect::<Vec<_>>(),
            },
        })
    }

    #[tokio::test]
    async fn session_lifecycle_checks_assignment_before_upstream_and_keeps_its_owner() {
        let relay = harness().await;

        let unassigned = relay
            .send("sub-one", "evalServeSession", serve("other"))
            .await;
        assert_eq!(unassigned.status(), StatusCode::FORBIDDEN);
        let unknown = relay
            .send("sub-one", "evalSubmitSession", submit(&["a", "b"]))
            .await;
        assert_eq!(unknown.status(), StatusCode::CONFLICT);
        assert_eq!(relay.calls(), 0, "rejected before any upstream call");

        let served = relay
            .send("sub-one", "evalServeSession", serve("project"))
            .await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(relay.calls(), 1);
        assert_eq!(relay.bindings().await["s"].caller, "sub:one");

        // Babel resuming `s` for the owner or another sub-account does not steal it.
        let resumed = relay
            .send("owner-token", "evalServeSession", serve("project"))
            .await;
        assert_eq!(resumed.status(), StatusCode::CONFLICT);
        let stolen = relay
            .send(
                "sub-two",
                "evalHoldSession",
                json!({
                    "operation": HOLD,
                    "payload": { "sessionId": "s" },
                }),
            )
            .await;
        assert_eq!(stolen.status(), StatusCode::CONFLICT);
        assert_eq!(relay.calls(), 2, "only the resume reached upstream");
        assert_eq!(relay.bindings().await["s"].caller, "sub:one");

        let partial = relay
            .send("sub-one", "evalSaveProgress", json!({
                "operation": SAVE,
                "payload": { "sessionId": "s", "tasks": [{ "taskId": "a", "inputResponses": [] }] },
            }))
            .await;
        assert_eq!(partial.status(), StatusCode::OK);
        let incomplete = relay
            .send("sub-one", "evalSubmitSession", submit(&["a"]))
            .await;
        assert_eq!(incomplete.status(), StatusCode::BAD_REQUEST);
        assert_eq!(relay.calls(), 3);

        let completed = relay
            .send("sub-one", "evalSubmitSession", submit(&["a", "b"]))
            .await;
        assert_eq!(completed.status(), StatusCode::OK);
        assert_eq!(relay.calls(), 4);
        assert!(relay.bindings().await.is_empty());
        relay.server.abort();
    }

    #[tokio::test]
    async fn owner_token_works_as_the_owner_caller() {
        let relay = harness().await;
        let served = relay
            .send("owner-token", "evalServeSession", serve("anything"))
            .await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(relay.bindings().await["s"].caller, OWNER_CALLER);

        let other = relay
            .send("sub-two", "evalSubmitSession", submit(&["a", "b"]))
            .await;
        assert_eq!(other.status(), StatusCode::CONFLICT);
        let wrong_token = relay
            .send("not-the-owner", "evalSubmitSession", submit(&["a", "b"]))
            .await;
        assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);

        let completed = relay
            .send("owner-token", "evalSubmitSession", submit(&["a", "b"]))
            .await;
        assert_eq!(completed.status(), StatusCode::OK);
        assert!(relay.bindings().await.is_empty());
        assert_eq!(relay.calls(), 2);
        relay.server.abort();
    }

    #[tokio::test]
    async fn eval_operations_need_their_own_kind_and_no_batches() {
        let relay = harness().await;
        for (kind, envelope) in [
            ("claimNextFromQueue", serve("project")),
            (
                "evalServeSession",
                json!({ "operation": SUBMIT, "payload": { "sessionId": "s", "tasks": [] } }),
            ),
            (
                "evalServeSession",
                json!({ "operation": "evalTasks.discardSession", "payload": { "sessionId": "s" } }),
            ),
        ] {
            let response = relay.send("sub-two", kind, envelope).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{kind}");
        }
        let batch = relay
            .send(
                "sub-two",
                "evalServeSession",
                json!({
                    "operation": SERVE,
                    "batchInput": { "0": { "json": { "projectId": "project" } } },
                }),
            )
            .await;
        assert_eq!(batch.status(), StatusCode::BAD_REQUEST);
        assert_eq!(relay.calls(), 0);
        relay.server.abort();
    }

    #[tokio::test]
    async fn bindings_survive_a_relay_restart() {
        let relay = harness().await;
        let served = relay
            .send("sub-one", "evalServeSession", serve("project"))
            .await;
        assert_eq!(served.status(), StatusCode::OK);
        relay.server.abort();

        let restarted = RelayState::new(RelayConfig {
            store_path: relay.store_path.clone(),
            owner_token: Some("owner-token".to_string()),
            babel_service_command: None,
            upstream_url: None,
            task_cents: 0,
            now_millis: Arc::new(|| 2),
            executor: Some(Arc::new(|_| Box::pin(async { Ok("[]".to_string()) }))),
        })
        .await
        .expect("restarted relay");
        let store = restarted.store.lock().await;
        assert_eq!(store.eval_sessions["s"], binding_for("sub:one"));
    }

    #[tokio::test]
    async fn losing_access_during_the_upstream_call_yields_no_binding_or_reply() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        let mut store = RelayStore::default();
        store.subaccounts.push(SubaccountRecord {
            id: "one".to_string(),
            label: "one".to_string(),
            code: "code-one".to_string(),
            revoked: false,
            allowed_project_ids: vec!["project".to_string()],
            analytics: AnalyticsRecord::default(),
            active_claims: BTreeMap::new(),
        });
        store.sessions.push(SessionRecord {
            token: "sub-one".to_string(),
            subaccount_id: "one".to_string(),
            created_at: 1,
            last_seen_at: 1,
        });
        store.save(&store_path).await.expect("seed store");
        // The owner unassigns the project while Babel is serving the session.
        let shared: Arc<std::sync::OnceLock<Arc<Mutex<RelayStore>>>> = Arc::default();
        let upstream_store = shared.clone();
        let state = crate::tests::test_state(
            store_path.clone(),
            Arc::new(AtomicI64::new(1)),
            Arc::new(move |_| {
                let store = upstream_store.get().cloned().expect("store");
                Box::pin(async move {
                    store.lock().await.subaccounts[0].allowed_project_ids =
                        vec!["other".to_string()];
                    Ok(json!([{ "result": { "data": { "json": {
                        "kind": "session",
                        "sessionId": "s",
                        "tasks": [{ "taskId": "a" }],
                    } } } }])
                    .to_string())
                })
            }),
            0,
        )
        .await;
        shared.set(state.store.clone()).expect("share store");
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sub-one".parse().expect("header"));

        let error = handle(&state, &headers, SERVE, serve("project"))
            .await
            .expect_err("access was lost");
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert!(state.store.lock().await.eval_sessions.is_empty());
    }

    #[tokio::test]
    async fn fork_relay_stores_keep_their_eval_sessions() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        std::fs::write(
            &store_path,
            r#"{
                "version": 1,
                "subaccounts": [],
                "sessions": [],
                "projectQueueConfigs": {},
                "evaluationSessions": {
                    "s": { "caller": "sub:one", "projectId": "project", "taskIds": ["a", "b"] }
                }
            }"#,
        )
        .expect("fork store");
        let store = RelayStore::load(&store_path)
            .await
            .expect("load fork store");
        assert_eq!(store.eval_sessions["s"], binding());
    }

    fn binding_for(caller: &str) -> EvalBinding {
        EvalBinding {
            caller: caller.to_string(),
            ..binding()
        }
    }
}
