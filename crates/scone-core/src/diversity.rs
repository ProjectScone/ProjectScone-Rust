//! Maximal marginal relevance over a recall's candidates — the framework's
//! diversity rule, so a weight places the same passages from either
//! runtime.
//!
//! Each place goes to the candidate with the highest relevance (its score
//! over the best one's) times `1 - weight`, less `weight` times its
//! greatest cosine to a passage already placed. Only the first
//! `2 * limit` places are filled this way, enough to survive the
//! per-episode cap; the rest keep their relevance order.

/// Candidates diversity fills places from; the rest keep relevance order
/// after them.
pub const MAX_DIVERSIFIED: usize = 200;

/// What diversity did to one recall's order.
#[derive(Debug, Clone, PartialEq)]
pub struct DiversityTrace {
    pub weight: f64,
    /// Candidates the places were filled from, after any phrases.
    pub candidates: usize,
    /// Candidates past the budget, left in relevance order after the rest.
    pub not_diversified: usize,
    /// Where the likeness came from: `index` (stored vectors), `embedded`
    /// (the candidates embedded once), or `unavailable` (neither; order kept).
    pub vectors: &'static str,
    /// Of the first `limit` candidates before the per-episode cap, how many
    /// are not the ones relevance alone put there.
    pub replaced: usize,
    pub why: String,
}

impl DiversityTrace {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "weight": self.weight, "candidates": self.candidates,
            "not_diversified": self.not_diversified, "vectors": self.vectors,
            "replaced": self.replaced, "why": self.why,
        })
    }
}

/// Refuse a weight outside 0 to 1, including one that is not a number.
pub fn check(weight: f64) -> crate::Result<()> {
    if (0.0..=1.0).contains(&weight) {
        return Ok(());
    }
    Err(crate::SconeError::InvalidInput(format!(
        "diversity is a weight from 0 to 1, not {weight}"
    )))
}

/// The order maximal marginal relevance gives candidates already in
/// relevance order: the indices of the `places` it filled, then the rest
/// as they were. `vectors` holds one vector per score, in any scale. Of
/// equal gains the earlier candidate wins, so ties keep relevance order.
pub fn mmr_order(scores: &[f32], vectors: &[Vec<f32>], weight: f64, places: usize) -> Vec<usize> {
    let units: Vec<Vec<f64>> = vectors.iter().map(|v| unit(v)).collect();
    let best = scores.iter().fold(0.0f64, |top, s| top.max(f64::from(*s)));
    let top = if best == 0.0 { 1.0 } else { best };
    // The greatest likeness to a placed passage; none placed reads as 0.
    let mut like: Vec<Option<f64>> = vec![None; scores.len()];
    let mut remaining: Vec<usize> = (0..scores.len()).collect();
    let mut placed = Vec::with_capacity(places.min(scores.len()));
    while !remaining.is_empty() && placed.len() < places {
        let gain = |i: usize| {
            (1.0 - weight) * f64::from(scores[i]) / top - weight * like[i].unwrap_or(0.0)
        };
        let mut chosen = 0;
        for position in 1..remaining.len() {
            if gain(remaining[position]) > gain(remaining[chosen]) {
                chosen = position;
            }
        }
        let chosen = remaining.remove(chosen);
        for &i in &remaining {
            let likeness = dot(&units[i], &units[chosen]);
            like[i] = Some(like[i].map_or(likeness, |l| l.max(likeness)));
        }
        placed.push(chosen);
    }
    placed.extend(remaining);
    placed
}

fn unit(vector: &[f32]) -> Vec<f64> {
    let norm = vector
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return vec![0.0; vector.len()];
    }
    vector.iter().map(|x| f64::from(*x) / norm).collect()
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_weight_relevance_order_stands() {
        let vectors = vec![vec![1.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]];
        assert_eq!(mmr_order(&[1.0, 0.9, 0.5], &vectors, 0.0, 6), [0, 1, 2]);
    }

    #[test]
    fn a_near_copy_gives_its_place_to_a_different_passage() {
        let vectors = vec![vec![1.0, 0.0], vec![0.99, 0.01], vec![0.0, 1.0]];
        assert_eq!(mmr_order(&[1.0, 0.99, 0.5], &vectors, 0.5, 6), [0, 2, 1]);
    }

    #[test]
    fn only_the_first_places_are_filled_by_likeness() {
        let vectors = vec![vec![1.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]];
        assert_eq!(mmr_order(&[1.0, 0.9, 0.5], &vectors, 0.9, 1), [0, 1, 2]);
    }

    #[test]
    fn a_weight_outside_zero_to_one_is_refused() {
        assert!(check(0.0).is_ok() && check(1.0).is_ok());
        assert!(check(1.5).is_err() && check(-0.1).is_err() && check(f64::NAN).is_err());
    }
}
