//! How many results to keep, chosen per query from where the scores fall —
//! the framework's `retrieval.cutoff`, rule for rule.
//!
//! Ranked by similarity, scores tend to fall in shelves: a steep drop after
//! the passages that match, then a long flat tail of passages that do not.
//! `gap_cutoff` cuts at the largest drop between `min_keep` and `max_keep`,
//! measured against the whole spread it looked at, less a small charge for
//! cutting late. When no drop stands out (`min_gap`) the scores give no
//! boundary and it keeps `max_keep` rather than invent one.
//!
//! Measured in the framework against a fixed k at the same mean number
//! kept: with one answer passage (SQuAD v1.1) the gold was kept 0.052 and
//! 0.042 more often (two embedders); with two, the second unlike the
//! question (HotpotQA), 0.084 to 0.107 less of the gold was kept. So it
//! suits lookups of one fact, and a two-hop question should keep a fixed k.

/// How many results to keep, and which rule decided it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cutoff {
    pub keep: usize,
    /// `gap` (the scores chose), `no-boundary` (no drop stood out, so
    /// `max_keep`), `short` (no more results than `min_keep`).
    pub rule: &'static str,
    /// The chosen drop over the spread examined; `None` unless `gap`.
    pub relative_gap: Option<f64>,
    pub why: String,
}

pub const MIN_GAP: f64 = 0.15;
pub const LATE_CHARGE: f64 = 0.05;

/// Where to cut a best-first score list. `None` when the bounds are not
/// `1 <= min_keep <= max_keep`, a score is not finite, or the scores are
/// not best first.
pub fn gap_cutoff(scores: &[f64], min_keep: usize, max_keep: usize) -> Option<Cutoff> {
    if min_keep < 1 || min_keep > max_keep {
        return None;
    }
    if scores.iter().any(|s| !s.is_finite()) || scores.windows(2).any(|w| w[1] > w[0]) {
        return None;
    }
    if scores.len() <= min_keep {
        return Some(Cutoff {
            keep: scores.len(),
            rule: "short",
            relative_gap: None,
            why: format!(
                "{} results, no more than the {min_keep} always kept",
                scores.len()
            ),
        });
    }
    let examined = &scores[..scores.len().min(max_keep + 1)];
    let spread = examined[0] - examined[examined.len() - 1];
    let last_cut = max_keep.min(scores.len() - 1);
    let allowed = max_keep.min(scores.len());
    if spread <= 0.0 {
        return Some(Cutoff {
            keep: allowed,
            rule: "no-boundary",
            relative_gap: None,
            why: format!("every score examined is equal, so the {allowed} allowed were kept"),
        });
    }
    let (mut best_position, mut best_value, mut best_gap) = (min_keep, f64::NEG_INFINITY, 0.0);
    for position in min_keep..=last_cut {
        let gap = (examined[position - 1] - examined[position]) / spread;
        let value = gap - LATE_CHARGE * position as f64 / max_keep as f64;
        if value > best_value {
            (best_position, best_value, best_gap) = (position, value, gap);
        }
    }
    if best_gap < MIN_GAP {
        return Some(Cutoff {
            keep: allowed,
            rule: "no-boundary",
            relative_gap: None,
            why: format!(
                "no drop reached {:.0}% of the spread, so the {allowed} allowed were kept",
                MIN_GAP * 100.0
            ),
        });
    }
    Some(Cutoff {
        keep: best_position,
        rule: "gap",
        relative_gap: Some(best_gap),
        why: format!(
            "cut after {best_position}: the scores dropped by {:.0}% of their spread there",
            best_gap * 100.0
        ),
    })
}

/// Where `cutoff` cut one recall, read from the vector lane's scores.
#[derive(Debug, Clone, PartialEq)]
pub struct CutoffTrace {
    /// `gap` (the scores fell, and the passages below were dropped),
    /// `no-boundary` (no drop stood out; nothing dropped), `short` (too few
    /// scores to read; nothing dropped), `unreadable` (a score was not a
    /// number; nothing dropped).
    pub rule: &'static str,
    /// Vector results above the cut, of those the lane returned.
    pub keep: usize,
    /// The lowest similarity kept; `None` unless `gap`.
    pub threshold: Option<f64>,
    /// The drop over the spread of the scores read; `None` unless `gap`.
    pub relative_gap: Option<f64>,
    /// Passages the answer would otherwise have held.
    pub dropped: usize,
    pub why: String,
}

impl CutoffTrace {
    fn nothing_cut(rule: &'static str, keep: usize, why: impl Into<String>) -> Self {
        Self {
            rule,
            keep,
            threshold: None,
            relative_gap: None,
            dropped: 0,
            why: why.into(),
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "rule": self.rule, "keep": self.keep, "threshold": self.threshold,
            "relative_gap": self.relative_gap, "dropped": self.dropped, "why": self.why,
        })
    }
}

/// Where to cut an answer of `limit` passages, read from the vector lane's
/// own scores rather than the fused order, which recency can reorder.
/// The scores are sorted, since an approximate index may hand back
/// near-ties out of order. Only a `gap` has a threshold; every other rule
/// leaves the answer as it was.
pub fn read(lane: &[f64], limit: usize) -> CutoffTrace {
    if lane.is_empty() {
        return CutoffTrace::nothing_cut("short", 0, "the vector lane returned nothing to cut");
    }
    if lane.iter().any(|score| !score.is_finite()) {
        return CutoffTrace::nothing_cut(
            "unreadable",
            0,
            "the vector lane returned a score that is not a number, so there is no drop to read; \
             nothing was cut",
        );
    }
    let mut scores = lane.to_vec();
    scores.sort_by(|a, b| b.total_cmp(a));
    let max_keep = limit.min(scores.len()).max(1);
    let Some(cut) = gap_cutoff(&scores, 1, max_keep) else {
        return CutoffTrace::nothing_cut(
            "unreadable",
            0,
            "the scores could not be read; nothing was cut",
        );
    };
    if cut.rule != "gap" {
        return CutoffTrace::nothing_cut(cut.rule, cut.keep, cut.why);
    }
    CutoffTrace {
        rule: "gap",
        keep: cut.keep,
        threshold: Some(scores[cut.keep - 1]),
        relative_gap: cut.relative_gap,
        dropped: 0,
        why: cut.why,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cuts_at_the_shelf() {
        let cut = gap_cutoff(&[0.9, 0.88, 0.86, 0.4, 0.38, 0.37], 1, 5).unwrap();
        assert_eq!((cut.keep, cut.rule), (3, "gap"));
        assert!(cut.relative_gap.unwrap() > 0.8);
    }

    #[test]
    fn a_flat_list_keeps_what_is_allowed() {
        // Ten evenly falling scores: each drop is a ninth of the spread.
        let even: Vec<f64> = (0..10).map(|i| 0.5 - 0.01 * i as f64).collect();
        let cut = gap_cutoff(&even, 1, 9).unwrap();
        assert_eq!((cut.keep, cut.rule), (9, "no-boundary"));
        let equal = gap_cutoff(&[0.5; 4], 1, 3).unwrap();
        assert_eq!((equal.keep, equal.rule), (3, "no-boundary"));
    }

    #[test]
    fn short_lists_and_bad_input() {
        assert_eq!(gap_cutoff(&[0.9], 1, 5).unwrap().rule, "short");
        assert!(gap_cutoff(&[0.1, 0.9], 1, 5).is_none());
        assert!(gap_cutoff(&[0.9, f64::NAN], 1, 5).is_none());
        assert!(gap_cutoff(&[0.9, 0.1], 0, 5).is_none());
    }

    #[test]
    fn the_lane_is_read_best_first_and_a_score_that_is_no_number_cuts_nothing() {
        let trace = read(&[0.4, 0.9, 0.38, 0.88, 0.86], 5);
        assert_eq!(
            (trace.rule, trace.keep, trace.threshold),
            ("gap", 3, Some(0.86))
        );
        let unreadable = read(&[0.9, f64::NAN, 0.4], 5);
        assert_eq!(
            (unreadable.rule, unreadable.threshold),
            ("unreadable", None)
        );
        assert_eq!(read(&[], 5).rule, "short");
        assert_eq!(read(&[0.9], 5).rule, "short");
    }

    #[test]
    fn a_late_drop_must_be_larger_to_win() {
        // Equal drops after 1 and after 4: the earlier one wins.
        let cut = gap_cutoff(&[1.0, 0.7, 0.7, 0.7, 0.4, 0.4], 1, 5).unwrap();
        assert_eq!(cut.keep, 1);
    }
}
