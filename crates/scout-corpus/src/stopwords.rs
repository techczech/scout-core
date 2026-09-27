//! Built-in English stopword list (the same list as `@scout/query`).

pub const ENGLISH: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did", "do",
    "does", "for", "from", "had", "has", "have", "he", "her", "here", "him", "his", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "may", "me", "might", "more", "most", "my",
    "no", "not", "of", "on", "one", "or", "our", "out", "over", "she", "should", "so", "some",
    "such", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this",
    "those", "to", "too", "up", "us", "was", "we", "were", "what", "when", "which", "while", "who",
    "why", "will", "with", "would", "you", "your",
];

pub fn is_stopword(t: &str) -> bool {
    ENGLISH.binary_search(&t).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn list_is_sorted_for_binary_search() {
        let mut v = super::ENGLISH.to_vec();
        v.sort();
        assert_eq!(v, super::ENGLISH);
    }
}
