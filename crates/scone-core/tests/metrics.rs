#![allow(clippy::unwrap_used)]
//! The metrics report over a store's own recall events: what it counts,
//! where a window starts and stops, and what it says when its limit bites.
use scone_core::auth::{self, ScopedSpace};
use scone_core::embed::HashEmbedder;
use scone_core::metrics::DEFAULT_LIMIT;
use scone_core::{Engine, RecallOpts, SconeError};
use serde_json::{Value, json};

fn store(dir: &std::path::Path) -> (Engine, ScopedSpace) {
    let mut engine = Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "team", true).unwrap();
    for note in [
        "the jib needs grease",
        "the boom needs paint",
        "the mast is sound",
    ] {
        engine
            .import_episode(&space, "note", note, None, None)
            .unwrap();
    }
    (engine, space)
}

fn recall(engine: &mut Engine, space: &ScopedSpace, question: &str) {
    // One passage of three, so the recall leaves bytes out.
    let one = RecallOpts {
        limit: 1,
        ..Default::default()
    };
    let pack = engine.recall(space, question, &one).unwrap();
    engine
        .record_recall_evidence(space, question, &pack)
        .unwrap();
}

fn named<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no metric {name} in {report}"))
}

/// Restamp the recall events, oldest first, as if recorded at these times.
fn stamp(dir: &std::path::Path, stamps: &[&str]) {
    let conn = rusqlite::Connection::open(dir.join("scone.db")).unwrap();
    for (at, ts) in stamps.iter().enumerate() {
        conn.execute(
            "UPDATE evidence_events SET ts = ?1 WHERE id = (SELECT id FROM evidence_events ORDER BY id LIMIT 1 OFFSET ?2)",
            rusqlite::params![ts, at as i64],
        )
        .unwrap();
    }
}

#[test]
fn a_report_counts_the_recalls_this_space_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    recall(&mut engine, &space, "jib grease");
    recall(&mut engine, &space, "boom paint");
    let report = engine.metrics(&space, None, None, DEFAULT_LIMIT).unwrap();

    let count = named(&report, "recall.count");
    assert_eq!(
        (count["value"].clone(), count["n"].clone()),
        (json!(2), json!(2))
    );
    let embedder = engine.embedder_id().to_owned();
    assert_eq!(report["embedders"], json!([embedder]));
    assert_eq!(
        named(&report, &format!("recall.top_similarity.{embedder}"))["n"],
        2
    );
    let reduction = named(&report, "recall.byte_context_reduction.median");
    assert_eq!(reduction["n"], 2);
    let saved = reduction["value"].as_f64().unwrap();
    assert!(saved > 0.0 && saved < 1.0, "{saved}");
    let degraded = named(&report, "recall.degraded_count");
    assert_eq!(
        (degraded["value"].clone(), degraded["n"].clone()),
        (json!(0), json!(2))
    );
    assert_eq!(report["coverage"]["events_considered"], 2);
    assert_eq!(report["coverage"]["truncated"], false);
    assert_eq!(report["coverage"]["schema_versions"], json!([1]));
    let days = report["daily"].as_object().unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!(days.values().next().unwrap(), &json!({"recall": 2}));

    let other = auth::resolve(&mut engine, "other", true).unwrap();
    let elsewhere = engine.metrics(&other, None, None, DEFAULT_LIMIT).unwrap();
    assert_eq!(elsewhere["coverage"]["events_considered"], 0);
}

#[test]
fn a_limit_that_bites_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    for question in ["jib", "boom", "mast"] {
        recall(&mut engine, &space, question);
    }
    let read = |limit: usize| {
        let report = engine.metrics(&space, None, None, limit).unwrap();
        (
            report["coverage"]["events_considered"].as_u64().unwrap(),
            report["coverage"]["truncated"].as_bool().unwrap(),
        )
    };
    assert_eq!(read(2), (2, true));
    // Exactly as many as there are is not a truncated read.
    assert_eq!(read(3), (3, false));
    // No limit reads nothing: zero is raised to one, and says it stopped.
    assert_eq!(read(0), (1, true));
}

#[test]
fn the_window_is_half_open_and_read_in_time_not_text() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path());
    recall(&mut engine, &space, "jib");
    recall(&mut engine, &space, "boom");
    // The first as a store stamped it before it kept milliseconds.
    stamp(
        dir.path(),
        &["2026-10-31T00:00:00Z", "2026-10-31T09:00:00.000Z"],
    );
    let considered = |since: Option<&str>, until: Option<&str>| {
        engine.metrics(&space, since, until, DEFAULT_LIMIT).unwrap()["coverage"]
            ["events_considered"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(considered(Some("2026-10-31"), None), 2);
    // As text, "...00:00:00Z" sorts after "...00:00:00.500Z"; in time it
    // is half a second before it.
    assert_eq!(considered(Some("2026-10-31T00:00:00.500Z"), None), 1);
    assert_eq!(considered(None, Some("2026-10-31T09:00:00Z")), 1);
    assert_eq!(considered(None, Some("2026-10-31T09:00:00.001Z")), 2);
    // An offset is the same instant: 04:00 at -05:00 is 09:00 UTC.
    assert_eq!(considered(None, Some("2026-10-31T04:00:00-05:00")), 1);
    let report = engine
        .metrics(
            &space,
            Some("2026-10-31T00:00:00.500Z"),
            None,
            DEFAULT_LIMIT,
        )
        .unwrap();
    assert_eq!(
        report["coverage"]["earliest_retained"],
        "2026-10-31T09:00:00.000Z"
    );
    assert_eq!(
        report["window"],
        json!({"since": "2026-10-31T00:00:00.500Z", "until": null})
    );

    for bad in [
        "yesterday",
        "now",
        "2460000.5",
        "2026-13-45",
        "2026-10-31X09",
    ] {
        assert!(
            matches!(
                engine.metrics(&space, Some(bad), None, DEFAULT_LIMIT),
                Err(SconeError::InvalidInput(_))
            ),
            "{bad} was read as a time"
        );
    }
}
