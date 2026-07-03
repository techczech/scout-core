use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub author: Option<String>,
    /// Container kind ("article", "book", ...). SQL column: work_type.
    pub kind: String,
    pub source_system: String,
    pub source_id: Option<String>,
    pub url: Option<String>,
    pub imported_at: String,
    pub updated_at: String,
    pub source_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    /// SQL column: work_id.
    pub container_id: String,
    pub text: String,
    pub note: Option<String>,
    /// SQL column: highlighted_at.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub tags: Vec<String>,
    pub location: Option<String>,
    pub location_type: Option<String>,
    pub annotation_color: Option<String>,
    pub annotation_type: Option<String>,
    pub format: String,
    pub source_data: serde_json::Value,
}

/// One search hit: the denormalised record+container row, app-agnostic.
/// Apps decorate this into their own result types (e.g. Highlight Scout adds
/// citation/zotero_link/asset_path derived from source_data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub record_id: String,
    pub container_id: String,
    pub slug: String,
    pub text: String,
    pub note: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub kind: String,
    pub source_system: String,
    pub source_id: Option<String>,
    pub url: Option<String>,
    pub created_at: Option<String>,
    pub tags: Vec<String>,
    pub location: Option<String>,
    pub annotation_color: Option<String>,
    pub annotation_type: Option<String>,
    pub format: String,
    pub ocr_text: Option<String>,
    pub container_source_data: serde_json::Value,
    pub record_source_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage {
    pub rows: Vec<Hit>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexFilter {
    pub source: String,
    pub flags: String,
}

/// Structured query. Apps parse their query grammar (e.g. @scout/query) and
/// translate app-level toggles into the generic fields:
/// favorite -> tag_any, zotero -> source_any.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub fts: String,
    pub has_positive: bool,
    #[serde(default)]
    pub positive_terms: Vec<String>,
    #[serde(default)]
    pub negatives: Vec<String>,
    #[serde(default)]
    pub regexes: Vec<RegexFilter>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tag: Option<String>,
    /// OR across tags-LIKE clauses (was HS `favorite`).
    #[serde(default)]
    pub tag_any: Vec<String>,
    /// OR across exact source_system matches (was HS `zotero`).
    #[serde(default)]
    pub source_any: Vec<String>,
    #[serde(default)]
    pub has_image: bool,
    /// Combinable kind filter (OR across the list). Empty = no filter.
    #[serde(default)]
    pub kinds: Vec<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub source: Option<String>,
    pub color: Option<String>,
    pub sort: String,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub pos: i64,
    pub total: i64,
    pub max_loc: i64,
}
