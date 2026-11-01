//! Whether a passage holds a phrase, by whole words, whatever its case and
//! punctuation — the framework's `retrieval.phrases` reading, so a phrase
//! required over HTTP keeps the same passages from either runtime.
//!
//! A phrase and a passage are both read as words: letters and digits after
//! NFKC normalisation and full case folding, anything else a boundary. A
//! phrase matches where its words stand together, in order, as whole words,
//! so "slew ring" matches "SLEW-RING" and "art" does not match "party".
//! Scripts written without spaces between words have no boundaries to find
//! without a dictionary, so a phrase in one matches inside a run of it.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::error::{Result, SconeError};

/// Phrases one recall may require and exclude together.
pub const MAX_PHRASES: usize = 20;
/// Characters in one phrase.
pub const MAX_PHRASE_CHARS: usize = 200;

/// Blocks of scripts written without spaces between words: Thai, Lao,
/// Myanmar, Khmer, Hangul, the CJK radicals, ideographic marks and
/// numerals, kana, bopomofo and the ideographs.
fn unspaced(c: char) -> bool {
    matches!(c,
        '\u{0e00}'..='\u{0eff}' | '\u{1000}'..='\u{109f}' | '\u{1780}'..='\u{17ff}'
        | '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' | '\u{a960}'..='\u{a97f}'
        | '\u{ac00}'..='\u{d7ff}' | '\u{2e80}'..='\u{2fdf}' | '\u{3005}'..='\u{3007}'
        | '\u{3021}'..='\u{3029}' | '\u{3031}'..='\u{3035}' | '\u{303b}' | '\u{303c}'
        | '\u{3040}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{3100}'..='\u{312f}'
        | '\u{31a0}'..='\u{31bf}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{3134f}')
}

/// A letter or digit, as the framework's `[^\W_]` reads one: marks are
/// boundaries there, so they are here too.
fn word_char(c: char) -> bool {
    (c.is_alphabetic() || c.is_numeric()) && !is_combining_mark(c)
}

/// `text` as its words, normalised and joined by single spaces.
pub fn words(text: &str) -> String {
    let folded = caseless::default_case_fold_str(&text.nfkc().collect::<String>());
    folded
        .split(|c: char| !word_char(c))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn within(words: &str, wanted: &str) -> bool {
    if wanted.is_empty() {
        return false;
    }
    if wanted.chars().any(unspaced) {
        return words.replace(' ', "").contains(&wanted.replace(' ', ""));
    }
    format!(" {words} ").contains(&format!(" {wanted} "))
}

/// Whether `text` holds `phrase` as whole words in order.
pub fn holds(text: &str, phrase: &str) -> bool {
    within(&words(text), &words(phrase))
}

/// Refuse phrases that cannot be matched, are too many or too long, or
/// that are both required and excluded, so nothing could be returned.
pub fn check(require: &[String], exclude: &[String]) -> Result<()> {
    for (name, phrases) in [("require", require), ("exclude", exclude)] {
        for phrase in phrases {
            let length = phrase.chars().count();
            if length > MAX_PHRASE_CHARS {
                return Err(SconeError::InvalidInput(format!(
                    "a phrase is at most {MAX_PHRASE_CHARS} characters; one in {name} is {length}"
                )));
            }
            if words(phrase).is_empty() {
                return Err(SconeError::InvalidInput(format!(
                    "the phrase {phrase:?} in {name} has no word to match"
                )));
            }
        }
    }
    if require.len() + exclude.len() > MAX_PHRASES {
        return Err(SconeError::InvalidInput(format!(
            "at most {MAX_PHRASES} phrases may be required and excluded together"
        )));
    }
    let required: std::collections::BTreeSet<String> = require.iter().map(|p| words(p)).collect();
    let both: Vec<String> = exclude
        .iter()
        .map(|p| words(p))
        .filter(|p| required.contains(p))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if !both.is_empty() {
        return Err(SconeError::InvalidInput(format!(
            "{} is both required and excluded, so nothing could be returned",
            both.join(", ")
        )));
    }
    Ok(())
}

/// Why a passage was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Required,
    Excluded,
}

/// Required and excluded phrases, normalised once, to check many passages against.
pub struct Phrases {
    require: Vec<String>,
    exclude: Vec<String>,
}

impl Phrases {
    pub fn new(require: &[String], exclude: &[String]) -> Self {
        Self {
            require: require.iter().map(|p| words(p)).collect(),
            exclude: exclude.iter().map(|p| words(p)).collect(),
        }
    }

    /// `None` when `text` holds every required phrase and no excluded one,
    /// otherwise the rule that dropped it. The passage is normalised once.
    pub fn check(&self, text: &str) -> Option<Dropped> {
        let words = words(text);
        if !self.require.iter().all(|p| within(&words, p)) {
            return Some(Dropped::Required);
        }
        if self.exclude.iter().any(|p| within(&words, p)) {
            return Some(Dropped::Excluded);
        }
        None
    }
}

/// What required and excluded phrases did to one recall's passages.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhraseTrace {
    pub required: Vec<String>,
    pub excluded: Vec<String>,
    /// Candidates the phrases were checked against, before the limit.
    pub checked: usize,
    pub dropped_required: usize,
    pub dropped_excluded: usize,
    /// Fewer passages came back than the limit after phrases dropped some,
    /// and a lane had filled its candidate window: passages beyond it were
    /// never read.
    pub short: bool,
    pub why: String,
}

impl PhraseTrace {
    /// Say whether the answer came back short after the phrases dropped
    /// passages. It is short only when a lane filled its candidate window:
    /// otherwise every passage was a candidate, and nothing went unchecked.
    pub fn finish(&mut self, returned: usize, limit: usize, window_full: bool) {
        let dropped = self.dropped_required + self.dropped_excluded;
        self.short = returned < limit && dropped > 0 && window_full;
        self.why = format!(
            "{dropped} of the {} candidates checked were dropped by the phrases",
            self.checked
        );
        if self.short {
            self.why.push_str(&format!(
                "; {returned} of {limit} came back, and passages beyond the candidate window were not checked"
            ));
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "required": self.required, "excluded": self.excluded, "checked": self.checked,
            "dropped_required": self.dropped_required, "dropped_excluded": self.dropped_excluded,
            "short": self.short, "why": self.why,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_words_whatever_the_case_and_punctuation() {
        assert!(holds("Check the SLEW-RING bolts.", "slew ring"));
        assert!(!holds("We had a party.", "art"));
        assert!(holds("Die Straße ist nass.", "STRASSE"));
        assert!(holds("ﬁnal report", "final"));
    }

    #[test]
    fn unspaced_scripts_match_inside_a_run() {
        assert!(holds("東京都の天気", "東京"));
        assert!(!holds("大阪の天気", "東京"));
    }

    #[test]
    fn refuses_what_cannot_be_matched_or_contradicts() {
        let none: Vec<String> = Vec::new();
        assert!(check(&["--".into()], &none).is_err());
        assert!(check(&["Slew ring".into()], &["slew-ring".into()]).is_err());
        assert!(check(&["x".repeat(201)], &none).is_err());
        assert!(check(&vec!["a".to_owned(); 21], &none).is_err());
        assert!(check(&["slew ring".into()], &["rust".into()]).is_ok());
    }

    #[test]
    fn required_comes_before_excluded() {
        let phrases = Phrases::new(&["jib".into()], &["rust".into()]);
        assert_eq!(phrases.check("rust on the jib"), Some(Dropped::Excluded));
        assert_eq!(phrases.check("rust on the boom"), Some(Dropped::Required));
        assert_eq!(phrases.check("grease on the jib"), None);
    }
}
