// Shared helpers for sourceless imports (CSV, Kindle, JSON): deterministic
// content-hash IDs so re-importing a file upserts rather than duplicates,
// plus stable container IDs. The SHA1 input uses a unit separator between
// fields, preserving idempotency while avoiding simple concatenation collisions.

use sha1::{Digest, Sha1};

fn sha1_hex(parts: &[&str]) -> String {
    let mut hasher = Sha1::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update(b"\x1f"); // unit separator between fields
        }
        hasher.update(p.as_bytes());
    }
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Stable record ID: same content -> same ID (idempotent re-import).
pub fn record_id(source: &str, title: &str, author: &str, text: &str, location: &str) -> String {
    format!("{}-{}", source, sha1_hex(&[title, author, text, location]))
}

/// Stable container ID derived from title+author so records regroup consistently.
pub fn container_id(source: &str, title: &str, author: &str) -> String {
    format!("{}-w-{}", source, sha1_hex(&[title, author]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_deterministic() {
        let a = record_id("csv", "Book", "Author", "quote", "12");
        let b = record_id("csv", "Book", "Author", "quote", "12");
        assert_eq!(a, b);
        assert!(a.starts_with("csv-"));
    }

    #[test]
    fn ids_differ_on_content() {
        let a = record_id("csv", "Book", "Author", "quote one", "");
        let b = record_id("csv", "Book", "Author", "quote two", "");
        assert_ne!(a, b);
    }

    #[test]
    fn field_separator_prevents_collisions() {
        // "ab"+"c" must not equal "a"+"bc"
        assert_ne!(
            record_id("csv", "ab", "c", "", ""),
            record_id("csv", "a", "bc", "", "")
        );
    }
}
