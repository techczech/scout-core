//! The `@scout/query` stopword list (`packages/scout-query/src/stopwords.ts`).

/// Sorted, for binary search.
pub const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did", "do",
    "does", "for", "from", "had", "has", "have", "he", "her", "here", "him", "his", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "may", "me", "might", "more", "most", "my",
    "no", "not", "of", "on", "one", "or", "our", "out", "over", "she", "should", "so", "some",
    "such", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this",
    "those", "to", "too", "up", "us", "was", "we", "were", "what", "when", "which", "while", "who",
    "why", "will", "with", "would", "you", "your",
];

/// TS `isStopword`: trimmed and lowercased first.
pub fn is_stopword(term: &str) -> bool {
    STOPWORDS
        .binary_search(&term.trim().to_lowercase().as_str())
        .is_ok()
}

/// TS `withoutStopwords`: drop stop words unless that would empty the list.
pub fn without_stopwords(terms: &[String]) -> Vec<String> {
    let kept: Vec<String> = terms.iter().filter(|t| !is_stopword(t)).cloned().collect();
    if kept.is_empty() {
        terms.to_vec()
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn list_is_sorted_and_matches_the_ts_list() {
        let mut v = super::STOPWORDS.to_vec();
        v.sort();
        assert_eq!(v, super::STOPWORDS);
        let ts = include_str!("../../../packages/scout-query/src/stopwords.ts");
        let body = &ts[ts.find("new Set([").unwrap()..ts.find("]);").unwrap()];
        let mut from_ts: Vec<&str> = body.split('"').skip(1).step_by(2).collect();
        from_ts.sort();
        assert_eq!(from_ts, super::STOPWORDS);
    }
}
