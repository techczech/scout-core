//! Numbered ids of the line files: `c000001` of `corrections.jsonl`,
//! `m000001` of the merge log. ONE spelling per number, decided here and
//! nowhere else.
//!
//! The id of the number `n` (1 to 999,999,999) is the prefix letter and
//! `n` written with at least six digits and no more than it needs:
//! exactly [`write`]. Any other spelling (`c1`, `c0000001`, `C000001`,
//! `c000000`, ten digits) is not a numbered id. So two ids are the same
//! number exactly when they are the same text, and an id can be looked up
//! as it is written in the file.

/// The largest number an id holds.
pub(crate) const MAX: u64 = 999_999_999;

/// The id of `n` under `prefix`: `c000001`, `c1000000`.
pub(crate) fn write(prefix: char, n: u64) -> String {
    format!("{prefix}{n:06}")
}

/// The number of `id`, when it is the id [`write`] gives a number from 1
/// to [`MAX`] under `prefix`.
pub(crate) fn number(prefix: char, id: &str) -> Option<u64> {
    let digits = id.strip_prefix(prefix)?;
    if !(6..=9).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    ((1..=MAX).contains(&n) && write(prefix, n) == id).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numbered_id_has_one_spelling_per_number() {
        for (id, n) in [
            ("c000001", 1),
            ("c000010", 10),
            ("c999999", 999_999),
            ("c1000000", 1_000_000),
            ("c12345678", 12_345_678),
            ("c999999999", MAX),
        ] {
            assert_eq!(number('c', id), Some(n), "{id}");
            assert_eq!(write('c', n), id);
            assert_eq!(number('m', &id.replace('c', "m")), Some(n), "{id}");
        }
        for bad in [
            "c1",
            "c00001",
            "c000000",
            "c0000001",
            "c00000001",
            "c000000001",
            "c0999999",
            "c01000000",
            "c1000000000",
            "c0000000001",
            "c18446744073709551616",
            "c-00001",
            "c+00001",
            "c00000a",
            "c 000001",
            "c000001 ",
            "000001",
            "C000001",
            "m000001",
            "c٠٠٠٠٠١",
            "",
            "c",
        ] {
            assert_eq!(number('c', bad), None, "{bad:?}");
        }
    }
}
