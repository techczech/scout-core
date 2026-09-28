//! Cleaning report (`scout clean-report`): per normalisation stage, how many
//! passages it changed, with before/after samples.
//!
//! Reads the source files through the adapters (read-only, invariant 1), so
//! it sees passages the index skips because boilerplate emptied them. No
//! index is needed.
//!
//! - A stage *changed* a passage when its output text differs from its input.
//! - "before" is what the index would count with that one stage left out
//!   ([`normalize::normalize_without`]); "after" is what it counts. Both are
//!   token lists of one source line; `original` is that line as written.
//! - Samples are grouped by their token change (tokens removed and added), so
//!   one systematic defect ("amp amp amp") shows once with its line count.
//!   Groups order by occurrences desc, then change; each group's sample is its
//!   first line in corpus order (path, line).

use crate::adapter;
use crate::markdown;
use crate::normalize::{self, Stage};
use crate::registry::CorpusConfig;
use crate::tokenize::words;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

/// Options for [`clean_report`].
#[derive(Debug, Clone)]
pub struct CleanOptions {
    /// Samples per stage.
    pub samples: usize,
    /// Candidate boilerplate regexes, counted as if they were in the registry
    /// (the bench's "try a rule" before "apply and rebuild").
    pub preview_rules: Vec<String>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        CleanOptions {
            samples: 5,
            preview_rules: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CleanReport {
    pub schema_version: u32,
    pub corpus: String,
    pub documents: usize,
    pub passages: usize,
    pub lines: usize,
    pub samples_per_rule: usize,
    pub rules: Vec<RuleReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuleReport {
    /// `boilerplate`, `entities`, `tags`, `emphasis`, `links_urls`,
    /// `apostrophes`, `nfc`.
    pub rule: String,
    pub label: String,
    /// Passages whose text the stage changed.
    pub passages: usize,
    /// Of those, passages whose counted tokens differ without the stage.
    pub token_passages: usize,
    /// Source lines whose counted tokens differ without the stage.
    pub token_lines: usize,
    /// Boilerplate only: one entry per regex, registry rules first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<Vec<PatternReport>>,
    pub samples: Vec<CleanSample>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PatternReport {
    /// 1-based position among the corpus rules (registry, then preview).
    pub n: usize,
    pub regex: String,
    /// `registry` or `preview`.
    pub origin: String,
    /// Lines this rule dropped (a line is credited to the first rule matching it).
    pub lines: usize,
    pub passages: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CleanSample {
    pub passage_id: String,
    pub rel_path: String,
    pub path: String,
    pub line: usize,
    /// The source line as written.
    pub original: String,
    /// Tokens counted without this stage.
    pub before: Vec<String>,
    /// Tokens counted.
    pub after: Vec<String>,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    /// Lines with the same token change.
    pub occurrences: usize,
    /// Boilerplate: the rule (`n`) that dropped the line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<usize>,
}

#[derive(Default)]
struct Acc {
    passages: usize,
    token_passages: usize,
    token_lines: usize,
    groups: BTreeMap<String, (usize, CleanSample)>,
}

/// Multiset difference `a - b`, in `a`'s order.
fn minus(a: &[String], b: &[String]) -> Vec<String> {
    let mut left: BTreeMap<&str, usize> = BTreeMap::new();
    for t in b {
        *left.entry(t.as_str()).or_default() += 1;
    }
    let mut out = Vec::new();
    for t in a {
        match left.get_mut(t.as_str()) {
            Some(n) if *n > 0 => *n -= 1,
            _ => out.push(t.clone()),
        }
    }
    out
}

/// The cleaning report for one corpus.
pub fn clean_report(cfg: &CorpusConfig, opts: &CleanOptions) -> Result<CleanReport> {
    let mut sources: Vec<(String, &str)> = cfg
        .boilerplate
        .iter()
        .map(|r| (r.clone(), "registry"))
        .collect();
    sources.extend(opts.preview_rules.iter().map(|r| (r.clone(), "preview")));
    let all: Vec<String> = sources.iter().map(|(r, _)| r.clone()).collect();
    let rules = normalize::compile_rules(&all)?;

    let mut acc: BTreeMap<Stage, Acc> = Stage::ALL.iter().map(|s| (*s, Acc::default())).collect();
    let mut pattern_lines = vec![0usize; rules.len()];
    let mut pattern_passages = vec![0usize; rules.len()];
    let (mut documents, mut passages, mut lines) = (0, 0, 0);

    for f in markdown::discover(cfg)? {
        let text = markdown::read_source(&f.abs_path)?;
        for d in adapter::records(cfg, &f.rel_path, &text) {
            documents += 1;
            for p in &d.passages {
                passages += 1;
                lines += p.original.lines().count();
                let (full, stages) = normalize::normalize_traced(&p.original, &rules);
                if stages.is_empty() {
                    continue;
                }
                let counted = words(&full.text);
                for stage in stages {
                    let a = acc.get_mut(&stage).unwrap();
                    a.passages += 1;
                    let without = normalize::normalize_without(&p.original, &rules, stage);
                    if words(&without.text) != counted {
                        a.token_passages += 1;
                    }
                    let mut hit = vec![false; rules.len()];
                    for (i, line) in p.original.lines().enumerate() {
                        let pattern = if stage == Stage::Boilerplate {
                            let k = normalize::boilerplate_rule(line, &rules);
                            if let Some(k) = k {
                                pattern_lines[k] += 1;
                                hit[k] = true;
                            }
                            k
                        } else {
                            None
                        };
                        let after = words(&normalize::normalize(line, &rules).text);
                        let before = words(&normalize::normalize_without(line, &rules, stage).text);
                        if before == after {
                            continue;
                        }
                        a.token_lines += 1;
                        let removed = minus(&before, &after);
                        let added = minus(&after, &before);
                        if removed.is_empty() && added.is_empty() {
                            continue;
                        }
                        let key = format!("-{}|+{}", removed.join(" "), added.join(" "));
                        let n = p.line_start + i;
                        a.groups
                            .entry(key)
                            .or_insert_with(|| {
                                (
                                    0,
                                    CleanSample {
                                        passage_id: format!(
                                            "{}:{}:{}",
                                            cfg.id, d.key, p.line_start
                                        ),
                                        rel_path: d.key.clone(),
                                        path: cfg.source_path(&d.key).display().to_string(),
                                        line: n,
                                        original: line.to_string(),
                                        before,
                                        after,
                                        removed,
                                        added,
                                        occurrences: 0,
                                        pattern: pattern.map(|k| k + 1),
                                    },
                                )
                            })
                            .0 += 1;
                    }
                    for (k, h) in hit.iter().enumerate() {
                        if *h {
                            pattern_passages[k] += 1;
                        }
                    }
                }
            }
        }
    }

    let rules_out = Stage::ALL
        .iter()
        .map(|stage| {
            let a = acc.remove(stage).unwrap();
            let mut groups: Vec<(usize, String, CleanSample)> =
                a.groups.into_iter().map(|(k, (n, s))| (n, k, s)).collect();
            groups.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
            let samples = groups
                .into_iter()
                .take(opts.samples)
                .map(|(n, _, mut s)| {
                    s.occurrences = n;
                    s
                })
                .collect();
            let patterns = (*stage == Stage::Boilerplate).then(|| {
                sources
                    .iter()
                    .enumerate()
                    .map(|(k, (regex, origin))| PatternReport {
                        n: k + 1,
                        regex: regex.clone(),
                        origin: origin.to_string(),
                        lines: pattern_lines[k],
                        passages: pattern_passages[k],
                    })
                    .collect()
            });
            RuleReport {
                rule: stage.id().into(),
                label: stage.label().into(),
                passages: a.passages,
                token_passages: a.token_passages,
                token_lines: a.token_lines,
                patterns,
                samples,
            }
        })
        .collect();

    Ok(CleanReport {
        schema_version: SCHEMA_VERSION,
        corpus: cfg.id.clone(),
        documents,
        passages,
        lines,
        samples_per_rule: opts.samples,
        rules: rules_out,
    })
}
