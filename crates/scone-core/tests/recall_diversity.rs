#![allow(clippy::unwrap_used)]
//! Diversity fills a recall's places by maximal marginal relevance, so
//! near-copies of one passage do not take several. It reads the stored
//! vectors, embeds the candidates when one is missing, and is refused
//! beside a reranker that would undo it, unless reranking is off for the
//! call.
use scone_core::embed::HashEmbedder;
use scone_core::rerank::FakeReranker;
use scone_core::{Engine, RecallOpts, SconeError, auth};

const COPIES: [&str; 3] = [
    "The crane survey found rust on the jib.",
    "The crane survey found rust on the jib again.",
    "Again the crane survey found rust on the jib.",
];
const OTHER: &str = "Crane survey notes: the slew ring bolts were loose.";
const QUESTION: &str = "crane survey found rust on the jib";

fn open(dir: &std::path::Path) -> (Engine, auth::ScopedSpace) {
    let mut engine = Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "default", true).unwrap();
    (engine, space)
}

fn store(dir: &std::path::Path) -> (Engine, auth::ScopedSpace) {
    let (mut engine, space) = open(dir);
    for note in COPIES.iter().chain([&OTHER]) {
        engine
            .import_episode(&space, "note", note, None, Some("2024-01-02T03:04:05.006Z"))
            .unwrap();
    }
    (engine, space)
}

fn diverse(weight: Option<f64>) -> RecallOpts {
    RecallOpts {
        limit: 2,
        diversity: weight,
        ..Default::default()
    }
}

fn top(pack: &scone_core::ContextPack) -> Vec<&str> {
    pack.items.iter().map(|i| i.text.as_str()).collect()
}

#[test]
fn a_near_copy_gives_its_place_to_a_different_passage() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    let plain = engine.recall(&space, QUESTION, &diverse(None)).unwrap();
    assert!(
        top(&plain).iter().all(|text| COPIES.contains(text)),
        "{:?}",
        top(&plain)
    );
    assert!(plain.diversity.is_none());
    let spread = engine
        .recall(&space, QUESTION, &diverse(Some(0.7)))
        .unwrap();
    assert_eq!(top(&spread)[0], top(&plain)[0]);
    assert_eq!(top(&spread)[1], OTHER);
    let trace = spread.diversity.unwrap();
    assert_eq!(
        (trace.vectors, trace.replaced, trace.candidates),
        ("index", 1, 4)
    );
    assert_eq!(
        trace.why,
        "1 of the first 2 places went to passages less like those above them"
    );
}

#[test]
fn no_weight_keeps_the_relevance_order() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    let plain = engine.recall(&space, QUESTION, &diverse(None)).unwrap();
    let zero = engine
        .recall(&space, QUESTION, &diverse(Some(0.0)))
        .unwrap();
    assert_eq!(top(&zero), top(&plain));
    assert_eq!(zero.diversity.unwrap().replaced, 0);
}

#[test]
fn a_missing_stored_vector_has_the_candidates_embedded() {
    let dir = tempfile::tempdir().unwrap();
    drop(store(dir.path()));
    let db = rusqlite::Connection::open(dir.path().join("scone.db")).unwrap();
    db.execute(
        "UPDATE chunks SET embedding = NULL WHERE id = (SELECT min(id) FROM chunks)",
        [],
    )
    .unwrap();
    drop(db);
    let (mut engine, space) = open(dir.path());
    let spread = engine
        .recall(&space, QUESTION, &diverse(Some(0.7)))
        .unwrap();
    assert_eq!(top(&spread)[1], OTHER);
    assert_eq!(spread.diversity.unwrap().vectors, "embedded");
}

#[test]
fn a_weight_outside_zero_to_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    for weight in [1.5, -0.25, f64::NAN] {
        match engine.recall(&space, QUESTION, &diverse(Some(weight))) {
            Err(SconeError::InvalidInput(message)) => assert!(message.contains("from 0 to 1")),
            other => panic!("expected a refusal for {weight}, got {other:?}"),
        }
    }
}

#[test]
fn a_reranker_is_refused_beside_it_unless_turned_off_for_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    engine.set_reranker(Some(Box::new(FakeReranker::preferring("slew ring"))));
    match engine.recall(&space, QUESTION, &diverse(Some(0.7))) {
        Err(SconeError::InvalidInput(message)) => assert!(message.contains("rerank=false")),
        other => panic!("expected a refusal, got {other:?}"),
    }
    let reranked = engine.recall(&space, QUESTION, &diverse(None)).unwrap();
    assert_eq!(top(&reranked)[0], OTHER);
    let unranked = RecallOpts {
        rerank: false,
        ..diverse(None)
    };
    let plain = engine.recall(&space, QUESTION, &unranked).unwrap();
    assert!(top(&plain).iter().all(|text| COPIES.contains(text)));
    let allowed = RecallOpts {
        rerank: false,
        ..diverse(Some(0.7))
    };
    assert!(
        engine
            .recall(&space, QUESTION, &allowed)
            .unwrap()
            .diversity
            .is_some()
    );
}

#[test]
fn the_recall_evidence_keeps_how_the_places_were_filled() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    let pack = engine
        .recall(&space, QUESTION, &diverse(Some(0.7)))
        .unwrap();
    engine
        .record_recall_evidence(&space, QUESTION, &pack)
        .unwrap();
    let events = engine
        .evidence_events(&space, None, 1, Some("recall"))
        .unwrap();
    let recorded = &events["events"][0]["payload"]["diversity"];
    assert_eq!(recorded["replaced"], 1);
    assert_eq!(recorded["vectors"], "index");
}
