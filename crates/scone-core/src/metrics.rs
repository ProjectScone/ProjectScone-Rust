//! Metrics computed from recorded evidence, and from nothing else — the
//! framework's `observability.metrics` report, for the events this
//! runtime records.
//!
//! Every figure carries its `n`, the name of its denominator and a
//! definition a reader can check against the events. A figure with no
//! evidence is null with `n = 0`; it is never a default. Coverage is
//! reported alongside: how many events were read, the earliest and the
//! latest, and whether the read stopped at its limit, because lost events
//! mean partial coverage, not zero activity.
//!
//! The Rust server records recall events and connector reports, so the
//! report holds the recall figures those support: how many, the top
//! item's similarity per embedder, the bytes a recall saved and how many
//! recalls lost a lane. Latency, lane overlap, ingest, ledger and feedback
//! figures need events this runtime does not record; they are left out,
//! not reported empty, so a missing name means "not measured here".
//!
//! Quantiles use the nearest-rank method (no interpolation): the value at
//! position `ceil(q * n)` of the sorted sample.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::{Engine, Result, SconeError, auth::ScopedSpace};

/// Events one report reads when the caller names no limit.
pub const DEFAULT_LIMIT: usize = 5000;
/// The most events one report reads; a larger limit is lowered to this.
pub const MAX_LIMIT: usize = 20_000;

/// The value at `ceil(q * n)`, 1-based, of an ascending sample.
pub fn nearest_rank(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).max(1);
    Some(sorted[rank.min(sorted.len()) - 1])
}

fn metric(
    name: &str,
    value: Value,
    n: usize,
    denominator: &str,
    unit: &str,
    definition: &str,
    caveat: Option<&str>,
) -> Value {
    json!({
        "name": name, "value": value, "n": n, "denominator": denominator,
        "unit": unit, "definition": definition, "caveat": caveat,
    })
}

/// What a report has seen so far. Events are added one at a time and not
/// kept, so a report over the largest window holds its figures, not its
/// events.
#[derive(Debug, Default)]
pub struct Tally {
    considered: usize,
    earliest: Option<String>,
    latest: Option<String>,
    failures: BTreeMap<String, usize>,
    embedders: BTreeSet<String>,
    recalls: usize,
    top_similarity: BTreeMap<String, Vec<f64>>,
    reductions: Vec<f64>,
    degraded: usize,
    degraded_recorded: usize,
    daily: BTreeMap<String, BTreeMap<String, usize>>,
}

impl Tally {
    /// Count one event. `ts` must be in the store's own form, so that
    /// comparing two as text compares them in time.
    pub fn add(&mut self, ts: &str, kind: &str, payload: &Value) {
        self.considered += 1;
        if self.earliest.as_deref().is_none_or(|e| ts < e) {
            self.earliest = Some(ts.to_owned());
        }
        if self.latest.as_deref().is_none_or(|l| ts > l) {
            self.latest = Some(ts.to_owned());
        }
        let day = ts.get(..10).unwrap_or(ts).to_owned();
        *self
            .daily
            .entry(day)
            .or_default()
            .entry(kind.to_owned())
            .or_default() += 1;
        if payload.get("error").is_some() {
            *self.failures.entry(kind.to_owned()).or_default() += 1;
            return;
        }
        let embedder = payload["embedder"].as_str().filter(|e| !e.is_empty());
        if let Some(embedder) = embedder {
            self.embedders.insert(embedder.to_owned());
        }
        if kind == "recall" {
            self.add_recall(payload, embedder);
        }
    }

    fn add_recall(&mut self, payload: &Value, embedder: Option<&str>) {
        self.recalls += 1;
        let top = payload["items"].as_array().and_then(|items| items.first());
        if let (Some(similarity), Some(embedder)) =
            (top.and_then(|t| t["similarity"].as_f64()), embedder)
        {
            self.top_similarity
                .entry(embedder.to_owned())
                .or_default()
                .push(similarity);
        }
        if let (Some(returned), Some(space)) = (
            payload["returned_bytes"].as_f64(),
            payload["space_bytes"].as_f64().filter(|b| *b > 0.0),
        ) {
            self.reductions.push(1.0 - returned / space);
        }
        // A recall recorded before this runtime kept the field says nothing
        // either way, so it is left out of n rather than counted as whole.
        let degraded = match &payload["degraded"] {
            Value::Array(lanes) => Some(!lanes.is_empty()),
            Value::Bool(flag) => Some(*flag),
            _ => None,
        };
        if let Some(degraded) = degraded {
            self.degraded_recorded += 1;
            self.degraded += usize::from(degraded);
        }
    }

    /// The report, for a window the caller asked for and a read that did
    /// or did not stop at its limit.
    pub fn report(mut self, since: Option<&str>, until: Option<&str>, truncated: bool) -> Value {
        let mut metrics = vec![metric(
            "recall.count",
            json!(self.recalls),
            self.recalls,
            "successful recalls",
            "recalls",
            "Recall events without an error field in the window.",
            None,
        )];
        for (embedder, similarities) in &mut self.top_similarity {
            similarities.sort_by(f64::total_cmp);
            metrics.push(metric(
                &format!("recall.top_similarity.{embedder}"),
                json!({
                    "p10": nearest_rank(similarities, 0.1),
                    "median": nearest_rank(similarities, 0.5),
                    "p90": nearest_rank(similarities, 0.9),
                }),
                similarities.len(),
                &format!(
                    "successful recalls with a top item that the vector lane scored, embedder {embedder}"
                ),
                "cosine",
                "Nearest-rank p10/median/p90 of the top item's cosine similarity, grouped by full embedder id.",
                Some(
                    "Uncalibrated: no confidence or quality class is derived from it. Hash embedders measure word overlap, not meaning.",
                ),
            ));
        }
        self.reductions.sort_by(f64::total_cmp);
        metrics.push(metric(
            "recall.byte_context_reduction.median",
            json!(nearest_rank(&self.reductions, 0.5)),
            self.reductions.len(),
            "successful recalls in a non-empty space",
            "share of bytes",
            "Median of 1 - returned_bytes / space_bytes, where space_bytes is every stored byte in the space.",
            Some("Bytes, not tokens, and not the whole prompt; undefined on an empty space."),
        ));
        metrics.push(metric(
            "recall.degraded_count",
            json!(self.degraded),
            self.degraded_recorded,
            "successful recalls that recorded whether a lane failed",
            "recalls",
            "Recalls that answered with one lane after the other failed.",
            Some("Recalls recorded before this runtime kept the field are not in n."),
        ));
        json!({
            "evidence": "sqlite",
            "window": {"since": since, "until": until},
            "coverage": {
                "events_considered": self.considered,
                "earliest_retained": self.earliest,
                "latest": self.latest,
                "truncated": truncated,
                "schema_versions": if self.considered > 0 { vec![1] } else { vec![] },
            },
            "failures": self.failures,
            "embedders": self.embedders,
            "metrics": metrics,
            "daily": self.daily,
        })
    }
}

/// The store's own form of a time: UTC, to the millisecond.
const STORED: &str = "%Y-%m-%dT%H:%M:%fZ";

impl Engine {
    /// `text` as an instant in the store's own form. A date alone means
    /// midnight UTC; an offset is converted. SQLite reads many other
    /// things as times ("now", a bare day number), so the text must start
    /// with a calendar date before it is asked.
    fn instant(&self, name: &str, text: &str) -> Result<String> {
        let raw = text.trim();
        let bytes = raw.as_bytes();
        let dated = bytes.len() >= 10
            && bytes[..10].iter().enumerate().all(|(at, b)| {
                if at == 4 || at == 7 {
                    *b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
            && bytes
                .get(10)
                .is_none_or(|b| matches!(b, b'T' | b't' | b' '));
        let parsed = if dated {
            self.conn
                .query_row("SELECT strftime(?1, ?2)", params![STORED, raw], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten()
        } else {
            None
        };
        parsed.ok_or_else(|| {
            SconeError::InvalidInput(format!(
                "{name} must be an RFC 3339 instant or a date, like 2026-10-31T09:00:00Z; got {text:?}"
            ))
        })
    }

    /// The metrics report over this space's retained events in
    /// `[since, until)`, newest first, reading at most `limit` of them
    /// (1 to [`MAX_LIMIT`]). `coverage.truncated` says when more were there.
    pub fn metrics(
        &self,
        space: &ScopedSpace,
        since: Option<&str>,
        until: Option<&str>,
        limit: usize,
    ) -> Result<Value> {
        let lower = since.map(|s| self.instant("since", s)).transpose()?;
        let upper = until.map(|u| self.instant("until", u)).transpose()?;
        let limit = limit.clamp(1, MAX_LIMIT);
        // Events stamped before the store kept milliseconds carry a bare
        // day; reading every stamp through one form compares them in time.
        let mut statement = self.conn.prepare(
            "SELECT strftime(?1, ts), kind, payload FROM evidence_events
             WHERE space_id = ?2
               AND (?3 IS NULL OR strftime(?1, ts) >= ?3)
               AND (?4 IS NULL OR strftime(?1, ts) < ?4)
             ORDER BY id DESC LIMIT ?5",
        )?;
        let mut rows = statement.query(params![
            STORED,
            space.id(),
            lower,
            upper,
            (limit + 1) as i64
        ])?;
        let mut tally = Tally::default();
        let mut read = 0;
        while let Some(row) = rows.next()? {
            read += 1;
            if read > limit {
                break;
            }
            let ts: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let payload: Value = serde_json::from_str(&row.get::<_, String>(2)?)
                .map_err(|_| SconeError::InvalidInput("stored event is malformed".into()))?;
            tally.add(&ts, &kind, &payload);
        }
        Ok(tally.report(since, until, read > limit))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn named<'a>(report: &'a Value, name: &str) -> &'a Value {
        report["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap_or_else(|| panic!("no metric {name} in {report}"))
    }

    #[test]
    fn nearest_rank_takes_the_value_at_the_ceiling() {
        let sample = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];
        assert_eq!(nearest_rank(&sample, 0.5), Some(0.5));
        assert_eq!(nearest_rank(&sample, 0.95), Some(1.0));
        assert_eq!(nearest_rank(&sample, 0.0), Some(0.1));
        assert_eq!(nearest_rank(&[], 0.5), None);
    }

    #[test]
    fn a_failed_event_is_a_failure_not_a_recall() {
        let mut tally = Tally::default();
        tally.add(
            "2026-10-31T09:00:00.000Z",
            "recall",
            &json!({"error": "index unavailable"}),
        );
        let report = tally.report(None, None, false);
        assert_eq!(report["failures"], json!({"recall": 1}));
        assert_eq!(named(&report, "recall.count")["value"], 0);
        assert_eq!(report["daily"], json!({"2026-10-31": {"recall": 1}}));
        assert_eq!(report["coverage"]["events_considered"], 1);
    }

    #[test]
    fn a_recall_recorded_before_the_fields_were_kept_is_not_counted_whole() {
        let mut tally = Tally::default();
        let legacy = json!({"items": [{"similarity": 0.4}], "returned_bytes": 10});
        tally.add("2026-10-30T00:00:00.000Z", "recall", &legacy);
        tally.add(
            "2026-10-31T09:00:00.000Z",
            "recall",
            &json!({"items": [{"similarity": 0.8}], "returned_bytes": 25, "space_bytes": 100,
                    "embedder": "hash-64", "degraded": ["vector"]}),
        );
        let report = tally.report(None, None, false);
        assert_eq!(named(&report, "recall.count")["n"], 2);
        let degraded = named(&report, "recall.degraded_count");
        assert_eq!(
            (degraded["value"].clone(), degraded["n"].clone()),
            (json!(1), json!(1))
        );
        let reduction = named(&report, "recall.byte_context_reduction.median");
        assert_eq!(
            (reduction["value"].clone(), reduction["n"].clone()),
            (json!(0.75), json!(1))
        );
        // The legacy recall named no embedder, so its similarity is no one's.
        let top = named(&report, "recall.top_similarity.hash-64");
        assert_eq!(
            (top["value"]["median"].clone(), top["n"].clone()),
            (json!(0.8), json!(1))
        );
        assert_eq!(report["embedders"], json!(["hash-64"]));
        assert_eq!(
            report["coverage"]["earliest_retained"],
            "2026-10-30T00:00:00.000Z"
        );
        assert_eq!(report["coverage"]["latest"], "2026-10-31T09:00:00.000Z");
    }

    #[test]
    fn no_evidence_is_null_with_n_zero() {
        let report = Tally::default().report(Some("2026-10-31"), None, false);
        let reduction = named(&report, "recall.byte_context_reduction.median");
        assert_eq!(
            (reduction["value"].clone(), reduction["n"].clone()),
            (Value::Null, json!(0))
        );
        assert_eq!(report["coverage"]["schema_versions"], json!([]));
        assert_eq!(report["coverage"]["earliest_retained"], Value::Null);
        assert_eq!(
            report["window"],
            json!({"since": "2026-10-31", "until": null})
        );
    }
}
