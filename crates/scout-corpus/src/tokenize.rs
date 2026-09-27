//! Tokeniser: Unicode word segmentation (UAX #29), lowercased.
//!
//! - Apostrophe-internal words stay whole (`don't`); normalisation has already
//!   unified curly apostrophes to `'`.
//! - Diacritics are letters (`příklad` is one token).
//! - Leading and trailing `_` are trimmed (Markdown `_emphasis_`).
//! - No lemmatisation in v1: the [`Lemmatizer`] seam has an identity
//!   implementation, reserved for Czech lemmas later.

use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

/// A lemma source. v1 ships only [`IdentityLemmatizer`].
pub trait Lemmatizer: Send + Sync {
    /// The lemma for an already-lowercased token in language `lang` (ISO 639-1
    /// when known).
    fn lemma<'a>(&self, token: &'a str, lang: Option<&str>) -> Cow<'a, str>;
}

/// Returns the token unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityLemmatizer;

impl Lemmatizer for IdentityLemmatizer {
    fn lemma<'a>(&self, token: &'a str, _lang: Option<&str>) -> Cow<'a, str> {
        Cow::Borrowed(token)
    }
}

/// One token: its lowercased form, its lemma and its byte span in the text it
/// was cut from (the normalised copy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub lemma: String,
    pub start: usize,
    pub end: usize,
}

/// Cut `text` into tokens.
pub fn tokenize(text: &str, lemmatizer: &dyn Lemmatizer, lang: Option<&str>) -> Vec<Token> {
    let mut out = Vec::new();
    for (i, w) in text.unicode_word_indices() {
        let trimmed_start = w.len() - w.trim_start_matches('_').len();
        let core = w.trim_matches('_');
        if core.is_empty() || !core.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let start = i + trimmed_start;
        let end = start + core.len();
        let lower = core.to_lowercase();
        let lemma = lemmatizer.lemma(&lower, lang).into_owned();
        out.push(Token {
            text: lower,
            lemma,
            start,
            end,
        });
    }
    out
}

/// Tokens as plain lowercased strings (identity lemmas).
pub fn words(text: &str) -> Vec<String> {
    tokenize(text, &IdentityLemmatizer, None)
        .into_iter()
        .map(|t| t.text)
        .collect()
}
