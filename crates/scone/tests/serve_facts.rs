#![allow(clippy::unwrap_used)]
//! Excluding and including a fact over HTTP: a reviewer's decision, so a
//! write key is refused, and the listing shows the reason when asked.
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use scone::serve::{Role, ServeConfig, SpaceKey, router};
use scone_core::embed::HashEmbedder;
use scone_core::llm::ExtractedFact;
use scone_core::{Engine, auth};
use serde_json::{Value, json};
use tower::ServiceExt;

fn app(dir: &std::path::Path) -> axum::Router {
    let mut engine = Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "team", true).unwrap();
    let (episode, _) = engine
        .import_episode(&space, "note", "mark drinks tea", None, None)
        .unwrap();
    let tea = ExtractedFact {
        subject: "mark".into(),
        predicate: "drinks".into(),
        object: "tea".into(),
        confidence: 0.9,
    };
    engine.apply_facts(&space, episode, &[tea]).unwrap();
    let key = |key: &str, role: Role| SpaceKey {
        key: key.into(),
        space: "team".into(),
        role,
    };
    router(
        engine,
        ServeConfig {
            keys: vec![key("sk-review", Role::Review), key("sk-write", Role::Write)],
        },
    )
}

async fn call(
    app: &axum::Router,
    key: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {key}"));
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

#[tokio::test]
async fn a_reviewer_excludes_and_includes_a_fact() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let reason = Some(json!({"reason": "a joke"}));
    let (status, _) = call(
        &app,
        "sk-write",
        "POST",
        "/v1/facts/1/exclude",
        reason.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(&app, "sk-review", "POST", "/v1/facts/1/exclude", reason).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"excluded": 1, "reason": "a joke"}));

    let (_, listed) = call(&app, "sk-review", "GET", "/v1/facts", None).await;
    assert_eq!(listed["facts"], json!([]));
    let (_, all) = call(&app, "sk-review", "GET", "/v1/facts?excluded=true", None).await;
    assert_eq!(all["facts"][0]["excluded_reason"], "a joke");
    assert_eq!(all["facts"][0]["status"], "active");
    let (_, recalled) = call(&app, "sk-review", "GET", "/v1/recall?q=mark%20tea", None).await;
    assert_eq!(recalled["facts"], json!([]));

    let (status, body) = call(&app, "sk-review", "POST", "/v1/facts/1/include", None).await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, json!({"included": 1, "changed": true}))
    );
    let (_, listed) = call(&app, "sk-review", "GET", "/v1/facts", None).await;
    assert_eq!(listed["facts"][0]["excluded_reason"], Value::Null);
}

#[tokio::test]
async fn an_exclusion_needs_a_reason_and_a_fact() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let (status, _) = call(
        &app,
        "sk-review",
        "POST",
        "/v1/facts/1/exclude",
        Some(json!({"reason": " "})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        "sk-review",
        "POST",
        "/v1/facts/42/exclude",
        Some(json!({"reason": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "sk-review", "POST", "/v1/facts/42/include", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
