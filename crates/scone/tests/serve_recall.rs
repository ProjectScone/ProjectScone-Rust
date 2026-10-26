#![allow(clippy::unwrap_used)]
//! Recall options over HTTP: the lanes to run, the phrases a passage must
//! hold or must not, how diverse the answer is and where it may stop.
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use scone::serve::{Role, ServeConfig, SpaceKey, router};
use scone_core::Engine;
use scone_core::embed::HashEmbedder;
use serde_json::{Value, json};
use tower::ServiceExt;

const MATCHING: [&str; 3] = [
    "The crane survey found rust on the jib.",
    "The crane survey found grease missing on the jib.",
    "The crane survey found the jib bolts loose.",
];
const UNRELATED: [&str; 4] = [
    "Lunch was soup and bread.",
    "The harbour board meets on Tuesday.",
    "Invoices go to the second floor.",
    "Parking moved to the east lot.",
];

async fn app_with(notes: &[&str]) -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), Box::new(HashEmbedder::new(64))).unwrap();
    let app = router(
        engine,
        ServeConfig {
            keys: vec![SpaceKey {
                key: "sk-alice".into(),
                space: "alice".into(),
                role: Role::Full,
            }],
        },
    );
    for note in notes {
        let (status, _) = call(&app, "POST", "/v1/episodes", Some(json!({"content": note}))).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    (dir, app)
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer sk-alice");
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

fn texts(body: &Value) -> Vec<String> {
    let mut found: Vec<String> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["text"].as_str().unwrap().to_owned())
        .collect();
    found.sort_unstable();
    found
}

#[tokio::test]
async fn the_lanes_are_asked_for_and_reported() {
    let notes: Vec<&str> = MATCHING.iter().chain(&UNRELATED).copied().collect();
    let (_dir, app) = app_with(&notes).await;
    let (status, both) = call(&app, "GET", "/v1/recall?q=crane%20survey%20jib", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(both["lanes"], json!(["vector", "text"]));
    let (status, text) = call(
        &app,
        "GET",
        "/v1/recall?q=crane%20survey%20jib&lanes=text",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text["lanes"], json!(["text"]));
    let mut expected: Vec<String> = MATCHING.iter().map(|s| s.to_string()).collect();
    expected.sort_unstable();
    assert_eq!(texts(&text), expected);
    let (status, refused) = call(&app, "GET", "/v1/recall?q=crane&lanes=graph", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("no lane named graph")
    );
    let (_, events) = call(&app, "GET", "/v1/events?kind=recall&limit=1", None).await;
    assert_eq!(events["events"][0]["payload"]["lanes"], json!(["text"]));
}
