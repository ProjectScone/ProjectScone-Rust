#![allow(clippy::unwrap_used)]
//! Required and excluded phrases are checked across the candidates before
//! the limit, so a passage that fails gives its place to one that holds,
//! and the pack says what they dropped and whether the answer came back
//! short of passages nobody read.
use scone_core::embed::HashEmbedder;
use scone_core::{Engine, RecallOpts, SconeError, auth};

const NOTES: [&str; 4] = [
    "The crane survey found rust on the jib.",
    "The crane survey found the SLEW-RING bolts loose.",
    "The crane survey found grease missing on the jib.",
    "Lunch was soup and bread.",
];

fn store(dir: &std::path::Path, notes: &[String]) -> (Engine, auth::ScopedSpace) {
    let mut engine = Engine::open(dir, Box::new(HashEmbedder::new(64))).unwrap();
    let space = auth::resolve(&mut engine, "default", true).unwrap();
    for note in notes {
        engine
            .import_episode(&space, "note", note, None, Some("2024-01-02T03:04:05.006Z"))
            .unwrap();
    }
    (engine, space)
}

fn notes() -> Vec<String> {
    NOTES.iter().map(|note| note.to_string()).collect()
}

fn phrases(require: &[&str], exclude: &[&str]) -> RecallOpts {
    RecallOpts {
        limit: 2,
        require: require.iter().map(|p| p.to_string()).collect(),
        exclude: exclude.iter().map(|p| p.to_string()).collect(),
        ..Default::default()
    }
}

fn texts(pack: &scone_core::ContextPack) -> Vec<&str> {
    let mut found: Vec<&str> = pack.items.iter().map(|i| i.text.as_str()).collect();
    found.sort_unstable();
    found
}

#[test]
fn a_required_phrase_keeps_only_passages_holding_it_as_whole_words() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes());
    let pack = engine
        .recall(&space, "crane survey", &phrases(&["slew ring"], &[]))
        .unwrap();
    assert_eq!(texts(&pack), [NOTES[1]]);
    let trace = pack.phrases.unwrap();
    assert_eq!(
        (
            trace.checked,
            trace.dropped_required,
            trace.dropped_excluded
        ),
        (4, 3, 0)
    );
    assert!(!trace.short, "every passage was a candidate: {}", trace.why);
}

#[test]
fn an_excluded_phrase_gives_its_place_to_the_next_passage() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes());
    let pack = engine
        .recall(&space, "crane survey jib", &phrases(&["crane"], &["RUST"]))
        .unwrap();
    assert_eq!(texts(&pack), [NOTES[2], NOTES[1]]);
    let trace = pack.phrases.unwrap();
    assert_eq!((trace.dropped_required, trace.dropped_excluded), (1, 1));
    assert_eq!(
        trace.why,
        "2 of the 4 candidates checked were dropped by the phrases"
    );
}

#[test]
fn without_phrases_there_is_no_trace() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes());
    let pack = engine
        .recall(&space, "crane", &RecallOpts::default())
        .unwrap();
    assert!(pack.phrases.is_none());
}

#[test]
fn an_answer_cut_short_by_a_full_window_says_so() {
    // More notes than a lane's candidate window, so passages past it are
    // never read, and only one holds the phrase.
    let mut many: Vec<String> = (0..70).map(|i| format!("crane survey item {i}")).collect();
    many.push("crane survey of the slew ring".into());
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &many);
    let pack = engine
        .recall(&space, "crane survey", &phrases(&["slew ring"], &[]))
        .unwrap();
    let trace = pack.phrases.unwrap();
    assert!(trace.checked >= 50, "checked {}", trace.checked);
    assert_eq!(pack.items.len(), trace.checked - trace.dropped_required);
    assert!(trace.short, "{}", trace.why);
    assert!(
        trace
            .why
            .ends_with("passages beyond the candidate window were not checked")
    );
}

#[test]
fn phrases_that_cannot_match_or_contradict_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes());
    for (opts, says) in [
        (phrases(&["--"], &[]), "no word to match"),
        (
            phrases(&["Slew ring"], &["slew-ring"]),
            "both required and excluded",
        ),
    ] {
        match engine.recall(&space, "crane", &opts) {
            Err(SconeError::InvalidInput(message)) => assert!(message.contains(says), "{message}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn the_recall_evidence_keeps_what_the_phrases_did() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, space) = store(dir.path(), &notes());
    let pack = engine
        .recall(&space, "crane survey", &phrases(&["slew ring"], &[]))
        .unwrap();
    engine
        .record_recall_evidence(&space, "crane survey", &pack)
        .unwrap();
    let events = engine
        .evidence_events(&space, None, 1, Some("recall"))
        .unwrap();
    let recorded = &events["events"][0]["payload"]["phrases"];
    assert_eq!(recorded["dropped_required"], 3);
    assert_eq!(recorded["required"][0], "slew ring");
}
