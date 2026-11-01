#![allow(clippy::unwrap_used)]
//! A recall that stops where the vector scores fall away.
//!
//! Three notes answer the question and the rest share none of its words,
//! so their cosines sit far below. The hash embedder splits on whitespace
//! alone, so the notes hold the question's words without punctuation. With `cutoff` the answer holds the
//! three and says where it cut; without it, the limit is filled with the
//! rest. A list with no drop is left alone, and settings the cut was not
//! measured in are refused.
use scone_core::embed::HashEmbedder;
use scone_core::rerank::FakeReranker;
use scone_core::{Engine, RecallOpts, SconeError, auth};

const MATCHING: [&str; 3] = [
    "The crane survey found rust on the jib today",
    "The crane survey found grease missing on the jib today",
    "The crane survey found the jib bolts loose.",
];
const UNRELATED: [&str; 4] = [
    "Lunch was soup and bread.",
    "The harbour board meets on Tuesday.",
    "Invoices go to the second floor.",
    "Parking moved to the east lot.",
];
const QUESTION: &str = "crane survey jib";

fn store(dir: &std::path::Path, notes: &[&str]) -> (Engine, auth::ScopedSpace) {
    let mut engine = Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "default", true).unwrap();
    for note in notes {
        engine
            .import_episode(&space, "note", note, None, Some("2024-01-02T03:04:05.006Z"))
            .unwrap();
    }
    (engine, space)
}

fn everything() -> Vec<&'static str> {
    MATCHING.iter().chain(&UNRELATED).copied().collect()
}

fn vector(limit: usize, cutoff: bool) -> RecallOpts {
    RecallOpts {
        limit,
        lanes: vec!["vector".into()],
        cutoff,
        ..Default::default()
    }
}

#[test]
fn the_answer_stops_at_the_drop_and_says_where() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &everything());
    let whole = engine.recall(&space, QUESTION, &vector(5, false)).unwrap();
    let cut = engine.recall(&space, QUESTION, &vector(5, true)).unwrap();
    assert_eq!(whole.items.len(), 5);
    assert!(whole.cutoff.is_none());
    let mut kept: Vec<&str> = cut.items.iter().map(|i| i.text.as_str()).collect();
    kept.sort_unstable();
    let mut expected = MATCHING.to_vec();
    expected.sort_unstable();
    assert_eq!(kept, expected);
    let trace = cut.cutoff.unwrap();
    assert_eq!((trace.rule, trace.keep, trace.dropped), ("gap", 3, 2));
    let threshold = trace.threshold.unwrap();
    assert!(
        cut.items
            .iter()
            .all(|i| f64::from(i.similarity.unwrap()) >= threshold)
    );
    assert!(trace.why.ends_with("; 2 passage(s) below it were dropped"));
}

#[test]
fn a_list_with_no_drop_is_left_as_it_was() {
    // Each note shares the same three words with the question.
    let notes = [
        "crane survey jib alpha.",
        "crane survey jib bravo.",
        "crane survey jib charlie.",
        "crane survey jib delta.",
    ];
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes);
    let whole = engine.recall(&space, QUESTION, &vector(3, false)).unwrap();
    let cut = engine.recall(&space, QUESTION, &vector(3, true)).unwrap();
    let ids =
        |pack: &scone_core::ContextPack| pack.items.iter().map(|i| i.chunk_id).collect::<Vec<_>>();
    assert_eq!(ids(&cut), ids(&whole));
    let trace = cut.cutoff.unwrap();
    assert_eq!(
        (trace.rule, trace.dropped, cut.items.len()),
        ("no-boundary", 0, 3)
    );
}

#[test]
fn the_cut_is_recorded_with_the_recall() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &everything());
    let cut = engine.recall(&space, QUESTION, &vector(5, true)).unwrap();
    engine
        .record_recall_evidence(&space, QUESTION, &cut)
        .unwrap();
    let events = engine
        .evidence_events(&space, None, 1, Some("recall"))
        .unwrap();
    assert_eq!(events["events"][0]["payload"]["cutoff"]["dropped"], 2);
}

#[test]
fn settings_it_was_not_measured_in_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &MATCHING);
    let both = RecallOpts {
        cutoff: true,
        ..Default::default()
    };
    let text = RecallOpts {
        lanes: vec!["text".into()],
        ..both.clone()
    };
    let decomposed = RecallOpts {
        decompose: true,
        ..vector(5, true)
    };
    for (opts, says) in [(both, "lanes"), (text, "lanes"), (decomposed, "decompose")] {
        match engine.recall(&space, QUESTION, &opts) {
            Err(SconeError::InvalidInput(message)) => assert!(message.contains(says), "{message}"),
            other => panic!("expected a refusal naming {says}, got {other:?}"),
        }
    }
}

#[test]
fn a_reranker_is_refused_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &MATCHING);
    engine.set_reranker(Some(Box::new(FakeReranker::preferring("rust"))));
    match engine.recall(&space, QUESTION, &vector(5, true)) {
        Err(SconeError::InvalidInput(message)) => assert!(message.contains("rerank=false")),
        other => panic!("expected a refusal, got {other:?}"),
    }
    let allowed = RecallOpts {
        rerank: false,
        ..vector(5, true)
    };
    assert!(
        engine
            .recall(&space, QUESTION, &allowed)
            .unwrap()
            .cutoff
            .is_some()
    );
}
