//! Would the reader read the folder as meant with one merge undone?
//!
//! The question `unmerge` asks before it writes, and `scout events
//! merges` and `scout events` ask of every merge in effect. Asked the
//! long way ([`Snapshot::verify`]) it reads the whole record set again
//! for each merge. Here the set is gone over once ([`Folder::of`]) and
//! each merge is answered from what the undo can change
//! ([`Folder::without_merge`]): the records merged into `a`, and the
//! monthly ids they list.
//!
//! Why that is enough. Clearing `a.merged_into` changes where a record
//! resolves for exactly the records whose chain passes `a` (`a` and what
//! is merged into it, directly or through a chain): they now end at `a`;
//! every other record ends where it did. A monthly id goes to the first
//! record, in id order, that lists it, and each later record that lists
//! it and ends elsewhere is warned about; so the holder and the warnings
//! of a monthly id can change only where one of those records lists it.
//! Nothing else the reader says depends on a `merged_into`: no file is
//! skipped or read differently, and no `merged_into` is left naming no
//! record or closing a cycle by taking one away.
//!
//! The answer, refusal text included, is the long way's: the tests below
//! ask both of generated folders and compare them word for word.

use super::store::{moved_fault, new_warning, refuse, Snapshot};
use crate::events::{member_conflict, EventRecord};
use crate::normalize::fold;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

/// One read of the records folder, gone over once.
pub(super) struct Folder<'s> {
    pub snap: &'s Snapshot,
    /// Folded record id → the records merged directly into it.
    merged_here: BTreeMap<String, Vec<&'s str>>,
    /// Folded monthly id → each listing of it, in the reader's order
    /// (records in id order, a record's members in their order): the
    /// record's folded id, the member's place in the record and the
    /// member as written.
    listed: BTreeMap<String, Vec<(&'s str, usize, &'s str)>>,
    /// The reader's warnings about the folder as it is.
    known: BTreeSet<&'s str>,
}

impl<'s> Folder<'s> {
    pub fn of(snap: &'s Snapshot) -> Folder<'s> {
        let records = &snap.set.records;
        let mut merged_here: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        let mut listed: BTreeMap<String, Vec<(&str, usize, &str)>> = BTreeMap::new();
        for (k, r) in records {
            if let Some(into) = &r.merged_into {
                merged_here.entry(fold(into)).or_default().push(k);
            }
            for (at, m) in r.members.iter().enumerate() {
                let mk = fold(m);
                // The reader passes over an empty member and one that is
                // a record id.
                if !mk.is_empty() && !records.contains_key(&mk) {
                    listed.entry(mk).or_default().push((k, at, m));
                }
            }
        }
        Folder {
            snap,
            merged_here,
            listed,
            known: snap.set.warnings.iter().map(String::as_str).collect(),
        }
    }

    /// The folded ids of `a` and of every record merged into it, directly
    /// or through a chain.
    fn with_merged_in(&self, a: &EventRecord) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut todo = vec![fold(&a.id)];
        while let Some(k) = todo.pop() {
            if let Some(more) = self.merged_here.get(&k) {
                todo.extend(more.iter().map(|m| m.to_string()));
            }
            out.insert(k);
        }
        out
    }

    /// Record `a` merged into nothing, every other record as it is: the
    /// reader must warn about nothing it does not warn about now, and
    /// `a`, the records merged into it and each monthly id they hold must
    /// resolve to `a`, with nothing else moved. The refusal is
    /// [`Snapshot::verify`]'s.
    pub fn without_merge(&self, a: &EventRecord) -> Result<()> {
        let set = &self.snap.set;
        let moved = self.with_merged_in(a);
        // Where a record ends once `a` is merged into nothing.
        let ends_at = |k: &str| -> Option<&String> {
            if moved.contains(k) {
                Some(&a.id)
            } else {
                set.aliases.get(k)
            }
        };
        // The event `a` is part of now: what resolves there and is `a`'s
        // or merged into `a` is meant to resolve to `a`.
        let event_now = set.aliases.get(&fold(&a.id));
        let meant = |k: &str| -> Option<&String> {
            let have = set.aliases.get(k)?;
            Some(if Some(have) == event_now { &a.id } else { have })
        };
        let monthly: BTreeSet<&String> = moved
            .iter()
            .filter_map(|k| set.records.get(k))
            .flat_map(|r| &r.members)
            .filter_map(|m| self.listed.get_key_value(&fold(m)).map(|(mk, _)| mk))
            .collect();

        // (record, the member's place in it) → the warning: the reader's
        // order, records in id order and each one's members in theirs.
        let mut warned: BTreeMap<(&str, usize), String> = BTreeMap::new();
        // Folded id → (resolves to, is meant to), in the table's order.
        let mut table: BTreeMap<&str, (Option<&String>, Option<&String>)> = BTreeMap::new();
        for k in &moved {
            table.insert(k, (ends_at(k), meant(k)));
        }
        for mk in monthly {
            let mut holder: Option<&String> = None;
            for (rk, at, m) in &self.listed[mk] {
                let Some(last) = ends_at(rk) else { continue };
                match holder {
                    None => holder = Some(last),
                    Some(have) if have == last => {}
                    Some(have) => {
                        let id = &set.records[*rk].id;
                        warned.insert((rk, *at), member_conflict(id, m, have));
                    }
                }
            }
            table.insert(mk, (holder, meant(mk)));
        }
        let mut faults: Vec<String> = warned
            .into_values()
            .filter(|w| !self.known.contains(w.as_str()))
            .map(|w| new_warning(&w))
            .collect();
        faults.extend(
            table
                .into_iter()
                .filter(|(_, (got, want))| got != want)
                .map(|(k, (got, want))| moved_fault(k, got, want)),
        );
        refuse(faults)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reader's id table as it is meant to be once `a` is merged into
    /// nothing: `a`, every record merged into `a` (directly or through a
    /// chain) and each monthly id those records hold now resolve to `a`;
    /// nothing else moves.
    fn table_without_merge(snap: &Snapshot, a: &EventRecord) -> BTreeMap<String, String> {
        let records = &snap.set.records;
        let a_key = fold(&a.id);
        let reaches_a = |r: &EventRecord| {
            let mut cur = r;
            // A chain is at most as long as the set.
            for _ in 0..=records.len() {
                if fold(&cur.id) == a_key {
                    return true;
                }
                match cur
                    .merged_into
                    .as_deref()
                    .and_then(|t| records.get(&fold(t)))
                {
                    Some(next) => cur = next,
                    None => return false,
                }
            }
            false
        };
        let mut expected = snap.set.aliases.clone();
        let holder = expected.get(&a_key).cloned();
        for (k, r) in records.iter().filter(|(_, r)| reaches_a(r)) {
            // A member that is a record id is not a monthly id (the reader
            // passes over it).
            let members = r.members.iter().map(|m| fold(m));
            let keys =
                std::iter::once(k.clone()).chain(members.filter(|m| !records.contains_key(m)));
            for key in keys {
                if let Some(have) = expected.get_mut(&key) {
                    if Some(&*have) == holder.as_ref() {
                        *have = a.id.clone();
                    }
                }
            }
        }
        expected
    }

    /// The long way: the whole folder read again with `a` unmerged.
    fn long_way(snap: &Snapshot, a: &EventRecord) -> Result<()> {
        let mut new_a = a.clone();
        new_a.merged_into = None;
        snap.verify(&[&new_a], &table_without_merge(snap, a))
    }

    /// A small generator (no dependency): the next number below `n`.
    struct Gen(u64);
    impl Gen {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    /// Up to nine records. Each may be merged into an earlier one (so no
    /// cycle, and chains of any depth), and lists up to three members
    /// drawn from a few monthly ids in two spellings, a record id and an
    /// empty one, so records of one event and of different events list
    /// the same monthly id, twice in one record too.
    fn folder(g: &mut Gen) -> Vec<EventRecord> {
        const MONTHLY: [&str; 7] = ["m/1", "M/1", "m/2", "m/3", "M/3 ", "m/4", ""];
        let n = 2 + g.below(8) as usize;
        // Ids in an order that is not the order of making, so "earlier"
        // and "first in id order" differ.
        let id = |i: usize| format!("r{}", (i * 7 + 3) % 10);
        (0..n)
            .map(|i| {
                let members = (0..g.below(4))
                    .map(|_| match g.below(12) {
                        11 => id(g.below(n as u64) as usize),
                        m => MONTHLY[m as usize % MONTHLY.len()].to_string(),
                    })
                    .collect();
                let merged_into = (i > 0 && g.below(3) > 0).then(|| {
                    let into = id(g.below(i as u64) as usize);
                    // The reader folds ids.
                    if g.below(4) == 0 {
                        into.to_uppercase()
                    } else {
                        into
                    }
                });
                EventRecord {
                    id: id(i),
                    name: format!("N{i}"),
                    members,
                    merged_into,
                    ..EventRecord::default()
                }
            })
            .collect()
    }

    #[test]
    fn the_answer_for_one_merge_is_the_whole_folders_word_for_word() {
        let mut g = Gen(20261012);
        let (mut asked, mut refused, mut accepted, mut moved, mut warned) = (0, 0, 0, 0, 0);
        for _ in 0..4000 {
            let snap = Snapshot::of_records(folder(&mut g));
            assert!(snap.set.unmerged.is_empty() && snap.set.refused.is_empty());
            let fast = Folder::of(&snap);
            for a in snap
                .set
                .records
                .values()
                .filter(|r| r.merged_into.is_some())
            {
                let say = |r: Result<()>| r.err().map(|e| format!("{e:#}"));
                let (long, short) = (say(long_way(&snap, a)), say(fast.without_merge(a)));
                assert_eq!(
                    short,
                    long,
                    "unmerging {} in {:#?}",
                    a.id,
                    snap.set.records.values().collect::<Vec<_>>()
                );
                asked += 1;
                match &long {
                    None => accepted += 1,
                    Some(why) => {
                        refused += 1;
                        warned += usize::from(why.contains("the reader would warn"));
                        moved += usize::from(why.contains("would be held by"));
                    }
                }
            }
        }
        eprintln!(
            "{asked} merges asked both ways: {accepted} accepted, {refused} refused ({warned} with a new warning, {moved} with an id that would not resolve as meant)"
        );
        // The folders made must show every kind of answer, many times.
        assert!(
            accepted > 1000 && warned > 500 && moved > 500,
            "{accepted} {warned} {moved}"
        );
    }
}
