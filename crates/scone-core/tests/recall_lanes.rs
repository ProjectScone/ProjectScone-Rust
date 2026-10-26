#![allow(clippy::unwrap_used)]
//! A recall runs the lanes it is asked for and says which answered. Text
//! alone makes no embedding call; vector alone returns only what the
//! vector lane found; a lane recall does not have is refused.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use scone_core::embed::{EmbeddingProvider, HashEmbedder};
use scone_core::{Engine, RecallOpts, SconeError, auth};

/// The hash embedder, counting the texts it is asked to embed.
struct Counting {
    inner: HashEmbedder,
    texts: Arc<AtomicUsize>,
}

impl EmbeddingProvider for Counting {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn embed(&self, texts: &[&str]) -> scone_core::Result<Vec<Vec<f32>>> {
        self.texts.fetch_add(texts.len(), Ordering::SeqCst);
        self.inner.embed(texts)
    }
}

const NOTES: [&str; 4] = [
    "The crane survey found rust on the jib.",
    "The crane survey found the jib bolts loose.",
    "Lunch was soup and bread.",
    "The harbour board meets on Tuesday.",
];

fn store(dir: &std::path::Path) -> (Engine, auth::ScopedSpace, Arc<AtomicUsize>) {
    let texts = Arc::new(AtomicUsize::new(0));
    let embedder = Counting {
        inner: HashEmbedder::new(64),
        texts: texts.clone(),
    };
    let mut engine = Engine::open(dir, Box::new(embedder)).unwrap();
    let space = auth::resolve(&mut engine, "default", true).unwrap();
    for note in NOTES {
        engine
            .import_episode(&space, "note", note, None, Some("2024-01-02T03:04:05.006Z"))
            .unwrap();
    }
    (engine, space, texts)
}

fn lanes(names: &[&str]) -> RecallOpts {
    RecallOpts {
        lanes: names.iter().map(|lane| lane.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn both_lanes_answer_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, _) = store(dir.path());
    let pack = engine
        .recall(&space, "crane survey jib", &RecallOpts::default())
        .unwrap();
    assert_eq!(pack.lanes, ["vector", "text"]);
}

#[test]
fn text_alone_makes_no_embedding_call_and_finds_only_its_words() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, texts) = store(dir.path());
    engine
        .recall(&space, "warm up", &RecallOpts::default())
        .unwrap();
    let before = texts.load(Ordering::SeqCst);
    let pack = engine
        .recall(&space, "crane survey jib", &lanes(&["text"]))
        .unwrap();
    assert_eq!(texts.load(Ordering::SeqCst), before);
    assert_eq!(pack.lanes, ["text"]);
    let mut found: Vec<&str> = pack.items.iter().map(|i| i.text.as_str()).collect();
    found.sort_unstable();
    assert_eq!(found, [NOTES[0], NOTES[1]]);
    assert!(pack.items.iter().all(|i| i.similarity.is_none()));
}

#[test]
fn vector_alone_returns_only_what_the_vector_lane_found() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, _) = store(dir.path());
    let pack = engine
        .recall(&space, "crane survey jib", &lanes(&["vector"]))
        .unwrap();
    assert_eq!(pack.lanes, ["vector"]);
    assert_eq!(pack.items.len(), NOTES.len());
    assert!(pack.items.iter().all(|i| i.similarity.is_some()));
}

#[test]
fn lanes_come_back_in_their_own_order() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, _) = store(dir.path());
    let pack = engine
        .recall(&space, "crane", &lanes(&["text", "vector", "text"]))
        .unwrap();
    assert_eq!(pack.lanes, ["vector", "text"]);
}

#[test]
fn an_unknown_or_empty_lane_list_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, _) = store(dir.path());
    for (asked, says) in [
        (&["graph", "text"][..], "no lane named graph"),
        (&[][..], "at least one lane"),
    ] {
        match engine.recall(&space, "crane", &lanes(asked)) {
            Err(SconeError::InvalidInput(message)) => assert!(message.contains(says), "{message}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn a_text_lane_that_fails_is_not_said_to_answer() {
    // A field the index does not have is a query the text lane cannot run.
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space, _) = store(dir.path());
    let both = engine
        .recall(&space, "nosuchfield:crane", &RecallOpts::default())
        .unwrap();
    assert_eq!(both.lanes, ["vector"]);
    assert!(both.degraded.iter().any(|d| d.starts_with("fts:")));
    match engine.recall(&space, "nosuchfield:crane", &lanes(&["text"])) {
        Err(SconeError::InvalidInput(message)) => assert!(message.contains("text, failed")),
        other => panic!("expected the only lane's failure, got {other:?}"),
    }
}
