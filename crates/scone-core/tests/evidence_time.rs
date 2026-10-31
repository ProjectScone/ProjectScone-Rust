#![allow(clippy::unwrap_used)]
//! Evidence events carry the moment they were recorded, as episodes do,
//! not the day: two events on one day keep their order in time, and a
//! metrics window can start mid-day.
use scone_core::embed::HashEmbedder;
use scone_core::{Engine, auth};

#[test]
fn an_event_is_stamped_to_the_millisecond_like_an_episode() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(dir.path(), Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "team", true).unwrap();
    let (episode, _) = engine
        .import_episode(&space, "note", "the jib needs grease", None, None)
        .unwrap();
    let pack = engine.recall(&space, "jib", &Default::default()).unwrap();
    engine.record_recall_evidence(&space, "jib", &pack).unwrap();
    let events = engine
        .evidence_events(&space, None, 1, Some("recall"))
        .unwrap();
    let ts = events["events"][0]["ts"].as_str().unwrap().to_owned();
    let stored = engine.evidence_episode(&space, episode).unwrap();
    let created = stored["created_at"].as_str().unwrap();
    assert_eq!(ts.len(), created.len(), "{ts} and {created} differ in form");
    assert_eq!(&ts[19..20], ".", "{ts} has no fraction of a second");
    assert!(ts.as_str() >= created, "{ts} is before {created}");
}
