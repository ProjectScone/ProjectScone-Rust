#![allow(clippy::unwrap_used)]
//! Excluding a fact keeps it out of recall, the profile and the listing
//! without touching its interval or status; including it undoes that.
//! Only ledger facts can be excluded, and the reason travels with the
//! fact through an export.
use scone_core::embed::HashEmbedder;
use scone_core::llm::ExtractedFact;
use scone_core::{Engine, RecallOpts, SconeError, auth};

fn engine(dir: &std::path::Path) -> Engine {
    Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap()
}

fn fact(subject: &str, predicate: &str, object: &str, confidence: f32) -> ExtractedFact {
    ExtractedFact {
        subject: subject.into(),
        predicate: predicate.into(),
        object: object.into(),
        confidence,
    }
}

/// A space holding one active fact about mark's tea, as id 1.
fn with_a_fact(dir: &std::path::Path) -> (Engine, auth::ScopedSpace) {
    let mut e = engine(dir);
    let space = auth::resolve(&mut e, "default", true).unwrap();
    let (episode, _) = e
        .import_episode(
            &space,
            "note",
            "mark drinks tea",
            None,
            Some("2024-03-02T12:00:00.000Z"),
        )
        .unwrap();
    e.apply_facts(&space, episode, &[fact("mark", "drinks", "tea", 0.9)])
        .unwrap();
    (e, space)
}

fn recalled_facts(e: &mut Engine, space: &auth::ScopedSpace) -> Vec<i64> {
    e.recall(space, "mark tea", &RecallOpts::default())
        .unwrap()
        .facts
        .iter()
        .map(|f| f.fact_id)
        .collect()
}

#[test]
fn an_excluded_fact_is_not_used_and_still_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, space) = with_a_fact(dir.path());
    assert_eq!(recalled_facts(&mut e, &space), [1]);
    let before = e.space_revision(&space).unwrap();
    e.facts_exclude(&space, 1, "  a joke, not a habit ")
        .unwrap();
    assert!(e.space_revision(&space).unwrap() > before);

    assert!(recalled_facts(&mut e, &space).is_empty());
    assert!(e.facts_about(&space, "mark").unwrap().is_empty());
    assert!(e.profile(&space, 10).unwrap().static_facts.is_empty());
    assert!(e.facts_list(&space, true).unwrap().is_empty());
    let ledger = e.facts_ledger(&space, true, true).unwrap();
    let (held, reason) = &ledger[0];
    assert_eq!(
        (held.status.as_str(), held.valid_until.as_deref()),
        ("active", None)
    );
    assert_eq!(reason.as_deref(), Some("a joke, not a habit"));

    assert!(e.facts_include(&space, 1).unwrap());
    assert_eq!(recalled_facts(&mut e, &space), [1]);
    assert!(!e.facts_include(&space, 1).unwrap(), "already included");
}

#[test]
fn only_a_ledger_fact_with_a_reason_can_be_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, space) = with_a_fact(dir.path());
    e.set_propose_below(Some(0.7)).unwrap();
    let (episode, _) = e
        .import_episode(&space, "note", "mark may like lisbon", None, None)
        .unwrap();
    e.apply_facts(&space, episode, &[fact("mark", "likes", "lisbon", 0.5)])
        .unwrap();
    let proposed = e.facts_pending(&space).unwrap()[0].fact_id;
    let other = auth::resolve(&mut e, "other", true).unwrap();
    for (outcome, says) in [
        (e.facts_exclude(&space, proposed, "no"), "only ledger facts"),
        (e.facts_exclude(&space, 1, "   "), "needs a reason"),
    ] {
        match outcome {
            Err(SconeError::InvalidInput(message)) => assert!(message.contains(says), "{message}"),
            other => panic!("expected a refusal naming {says}, got {other:?}"),
        }
    }
    assert!(matches!(
        e.facts_exclude(&space, 999, "gone"),
        Err(SconeError::NotFound(_))
    ));
    assert!(matches!(
        e.facts_exclude(&other, 1, "not yours"),
        Err(SconeError::NotFound(_))
    ));
    assert!(matches!(
        e.facts_include(&other, 1),
        Err(SconeError::NotFound(_))
    ));
}

#[test]
fn the_reason_travels_with_the_fact_through_an_export() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, space) = with_a_fact(dir.path());
    let plain = e.export_jsonl(&space).unwrap();
    assert!(
        !plain.contains("excluded_reason"),
        "an included fact reads as before"
    );
    e.facts_exclude(&space, 1, "a joke").unwrap();
    let exported = e.export_jsonl(&space).unwrap();

    let elsewhere = tempfile::tempdir().unwrap();
    let mut copy = engine(elsewhere.path());
    let target = auth::resolve(&mut copy, "default", true).unwrap();
    copy.import_jsonl(&target, &exported).unwrap();
    let ledger = copy.facts_ledger(&target, true, true).unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].1.as_deref(), Some("a joke"));
    assert!(recalled_facts(&mut copy, &target).is_empty());
}

#[test]
fn a_file_from_before_exclusion_gains_it_on_open() {
    let dir = tempfile::tempdir().unwrap();
    drop(with_a_fact(dir.path()));
    let db = rusqlite::Connection::open(dir.path().join("scone.db")).unwrap();
    db.execute_batch(
        "ALTER TABLE facts DROP COLUMN excluded_reason;
         UPDATE meta SET value = '7' WHERE key = 'schema_version';",
    )
    .unwrap();
    drop(db);
    let mut e = engine(dir.path());
    assert_eq!(e.schema_version().unwrap(), 8);
    let space = auth::resolve(&mut e, "default", true).unwrap();
    assert_eq!(recalled_facts(&mut e, &space), [1]);
    e.facts_exclude(&space, 1, "after the upgrade").unwrap();
    assert!(recalled_facts(&mut e, &space).is_empty());
}
