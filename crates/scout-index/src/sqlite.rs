use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

use crate::models::{Container, Hit, Position, Record, SearchPage, SearchQuery, TagCount};

pub fn open(index_path: &Path) -> Result<Connection> {
    let conn = Connection::open(index_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS works (
            id TEXT PRIMARY KEY,
            slug TEXT UNIQUE NOT NULL,
            title TEXT NOT NULL,
            author TEXT,
            work_type TEXT NOT NULL DEFAULT 'article',
            source_system TEXT NOT NULL,
            source_id TEXT,
            url TEXT,
            imported_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            source_data TEXT NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS highlights (
            id TEXT PRIMARY KEY,
            work_id TEXT NOT NULL REFERENCES works(id),
            text TEXT NOT NULL,
            note TEXT,
            highlighted_at TEXT,
            updated_at TEXT,
            tags TEXT NOT NULL DEFAULT '[]',
            location TEXT,
            location_type TEXT,
            annotation_color TEXT,
            annotation_type TEXT,
            format TEXT NOT NULL DEFAULT 'plain',
            source_data TEXT NOT NULL DEFAULT '{}'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
            highlight_id UNINDEXED,
            work_id UNINDEXED,
            text,
            note,
            title,
            author,
            tags,
            ocr,
            tokenize='porter unicode61'
        );

        CREATE INDEX IF NOT EXISTS idx_highlights_work ON highlights(work_id);
        CREATE INDEX IF NOT EXISTS idx_works_source ON works(source_system, source_id);
    ",
    )?;

    // Migration: add highlights.ocr_text if missing.
    let has_ocr_text: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('highlights') WHERE name='ocr_text'",
        [],
        |r| r.get(0),
    )?;
    if has_ocr_text == 0 {
        conn.execute("ALTER TABLE highlights ADD COLUMN ocr_text TEXT", [])?;
    }

    // Migration: rebuild search_index with ocr column if the pre-existing table lacks it.
    let has_ocr_col: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('search_index') WHERE name='ocr'",
        [],
        |r| r.get(0),
    )?;
    if has_ocr_col == 0 {
        conn.execute("DROP TABLE search_index", [])?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE search_index USING fts5(
                highlight_id UNINDEXED,
                work_id UNINDEXED,
                text,
                note,
                title,
                author,
                tags,
                ocr,
                tokenize='porter unicode61'
            );",
        )?;
        // Repopulate from existing highlights + works.
        let mut stmt = conn.prepare(
            "SELECT h.id, h.work_id, h.text, h.note, w.title, w.author, h.tags, h.ocr_text
             FROM highlights h JOIN works w ON w.id = h.work_id",
        )?;
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            String,
            Option<String>,
        )> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        for (id, work_id, text, note, title, author, tags, ocr_text) in rows {
            conn.execute(
                "INSERT INTO search_index (highlight_id, work_id, text, note, title, author, tags, ocr)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id,
                    work_id,
                    text,
                    note.unwrap_or_default(),
                    title,
                    author.unwrap_or_default(),
                    tags,
                    ocr_text.unwrap_or_default()
                ],
            )?;
        }
    }

    Ok(())
}

pub fn upsert_container(conn: &Connection, container: &Container) -> Result<()> {
    conn.execute(
        "INSERT INTO works (id, slug, title, author, work_type, source_system, source_id, url,
          imported_at, updated_at, source_data)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(id) DO UPDATE SET
           title=excluded.title, author=excluded.author, url=excluded.url,
           updated_at=excluded.updated_at, source_data=excluded.source_data",
        params![
            container.id,
            container.slug,
            container.title,
            container.author,
            container.kind,
            container.source_system,
            container.source_id,
            container.url,
            container.imported_at,
            container.updated_at,
            serde_json::to_string(&container.source_data).unwrap_or_default()
        ],
    )?;
    Ok(())
}

pub fn upsert_record(
    conn: &Connection,
    record: &Record,
    container_title: &str,
    container_author: Option<&str>,
) -> Result<()> {
    let tags_str = serde_json::to_string(&record.tags).unwrap_or_default();
    let source_data_str = serde_json::to_string(&record.source_data).unwrap_or_default();

    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM highlights WHERE id = ?1",
            params![record.id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if exists {
        conn.execute(
            "UPDATE highlights SET work_id=?2, text=?3, note=?4, highlighted_at=?5,
             updated_at=?6, tags=?7, location=?8, location_type=?9,
             annotation_color=?10, annotation_type=?11, format=?12, source_data=?13
             WHERE id=?1",
            params![
                record.id,
                record.container_id,
                record.text,
                record.note,
                record.created_at,
                record.updated_at,
                tags_str,
                record.location,
                record.location_type,
                record.annotation_color,
                record.annotation_type,
                record.format,
                source_data_str
            ],
        )?;
        conn.execute(
            "DELETE FROM search_index WHERE highlight_id = ?1",
            params![record.id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO highlights
             (id, work_id, text, note, highlighted_at, updated_at, tags,
              location, location_type, annotation_color, annotation_type, format, source_data)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                record.id,
                record.container_id,
                record.text,
                record.note,
                record.created_at,
                record.updated_at,
                tags_str,
                record.location,
                record.location_type,
                record.annotation_color,
                record.annotation_type,
                record.format,
                source_data_str
            ],
        )?;
    }

    let ocr_text: String = conn
        .query_row(
            "SELECT COALESCE(ocr_text,'') FROM highlights WHERE id=?1",
            params![record.id],
            |r| r.get(0),
        )
        .unwrap_or_default();

    conn.execute(
        "INSERT INTO search_index
         (highlight_id, work_id, text, note, title, author, tags, ocr)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            record.id,
            record.container_id,
            record.text,
            record.note.as_deref().unwrap_or(""),
            container_title,
            container_author.unwrap_or(""),
            record.tags.join(" "),
            ocr_text
        ],
    )?;

    Ok(())
}

/// Rebuild one record's search_index row from the DB (used after OCR writes ocr_text).
pub fn reindex_record_fts(conn: &Connection, record_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM search_index WHERE highlight_id = ?1",
        params![record_id],
    )?;
    conn.execute(
        "INSERT INTO search_index (highlight_id, work_id, text, note, title, author, tags, ocr)
         SELECT h.id, h.work_id, h.text, COALESCE(h.note,''), w.title, COALESCE(w.author,''), h.tags, COALESCE(h.ocr_text,'')
         FROM highlights h JOIN works w ON w.id = h.work_id WHERE h.id = ?1",
        params![record_id],
    )?;
    Ok(())
}

/// Image sources to OCR for a record: app-owned image records -> the local
/// asset PNG; otherwise the ![image](url) URLs embedded in the text.
pub fn ocr_sources(format: &str, asset_path: Option<&str>, text: &str) -> Vec<String> {
    if format == "image" {
        return asset_path.map(|p| vec![p.to_string()]).unwrap_or_default();
    }
    let re = regex::Regex::new(r"!\[[^\]]*\]\((https?://[^)\s]+)\)").unwrap();
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// Store OCR text for a record and refresh its search_index row.
pub fn write_ocr(conn: &Connection, id: &str, text: &str) -> Result<()> {
    conn.execute(
        "UPDATE highlights SET ocr_text=?2 WHERE id=?1",
        params![id, text],
    )?;
    reindex_record_fts(conn, id)?;
    Ok(())
}

/// (id, format, text) for records that have images but no ocr_text yet.
/// `only` optionally restricts to a set of record ids (e.g. just-imported).
pub fn ocr_pending(
    conn: &Connection,
    only: Option<&[String]>,
) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, format, text FROM highlights
         WHERE (ocr_text IS NULL OR ocr_text='')
           AND (format='image' OR text LIKE '%![image](%')",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(match only {
        Some(ids) => rows
            .into_iter()
            .filter(|(id, _, _)| ids.iter().any(|x| x == id))
            .collect(),
        None => rows,
    })
}

// The denormalised column list returned for every result, in a fixed order so
// one row mapper serves search and container-record queries alike.
const RESULT_COLS: &str = "h.id, h.work_id, w.slug, h.text, h.note, w.title, \
    w.author, w.work_type, w.source_system, w.source_id, w.url, h.highlighted_at, \
    h.tags, h.location, h.annotation_color, h.annotation_type, h.format, \
    w.source_data, h.source_data, h.ocr_text";

// Concatenated searchable text used for coverage ranking and negative scans.
const HAYSTACK: &str = "(COALESCE(h.text,'')||' '||COALESCE(h.note,'')||' '||\
    COALESCE(w.title,'')||' '||COALESCE(w.author,'')||' '||COALESCE(h.tags,'')||' '||COALESCE(h.ocr_text,''))";

const REGEX_SCAN_CAP: usize = 6000;
const REGEX_RESULT_CAP: usize = 500;

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<Hit> {
    let tags_str: String = row.get(12)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    let container_source_data: serde_json::Value = row
        .get::<_, Option<String>>(17)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let record_source_data: serde_json::Value = row
        .get::<_, Option<String>>(18)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);

    Ok(Hit {
        record_id: row.get(0)?,
        container_id: row.get(1)?,
        slug: row.get(2)?,
        text: row.get(3)?,
        note: row.get(4)?,
        title: row.get(5)?,
        author: row.get(6)?,
        kind: row.get(7)?,
        source_system: row.get(8)?,
        source_id: row.get(9)?,
        url: row.get(10)?,
        created_at: row.get(11)?,
        tags,
        location: row.get(13)?,
        annotation_color: row.get(14)?,
        annotation_type: row.get(15)?,
        format: row.get(16)?,
        container_source_data,
        record_source_data,
        ocr_text: row.get(19)?,
    })
}

fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Append filter clauses (mirrors the extension's filterSql, adapted to the
/// normalised works+highlights schema).
fn push_filters(
    q: &SearchQuery,
    where_sql: &mut String,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
) {
    let mut add = |clause: String| {
        where_sql.push_str(" AND ");
        where_sql.push_str(&clause);
    };
    if let Some(a) = q.author.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(format!("%{}%", a)));
        add(format!("w.author LIKE ?{}", params.len()));
    }
    if let Some(t) = q.title.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(format!("%{}%", t)));
        add(format!("w.title LIKE ?{}", params.len()));
    }
    if let Some(t) = q.kind.as_deref().filter(|s| !s.is_empty()) {
        // Tolerate singular/plural ("books" -> "book") by prefix match on the
        // de-pluralised value.
        let v = t.strip_suffix('s').unwrap_or(t).to_lowercase();
        params.push(Box::new(format!("{}%", v)));
        add(format!("LOWER(w.work_type) LIKE ?{}", params.len()));
    }
    if let Some(t) = q.tag.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(format!("%{}%", t)));
        add(format!("h.tags LIKE ?{}", params.len()));
    }
    if !q.tag_any.is_empty() {
        let ors: Vec<String> = q
            .tag_any
            .iter()
            .map(|t| {
                params.push(Box::new(format!("%{}%", t)));
                format!("h.tags LIKE ?{}", params.len())
            })
            .collect();
        add(format!("({})", ors.join(" OR ")));
    }
    if !q.source_any.is_empty() {
        let ors: Vec<String> = q
            .source_any
            .iter()
            .map(|s| {
                params.push(Box::new(s.to_string()));
                format!("w.source_system = ?{}", params.len())
            })
            .collect();
        add(format!("({})", ors.join(" OR ")));
    }
    if q.has_image {
        add("(h.format = 'image' OR h.text LIKE '%![image](%')".to_string());
    }
    if !q.kinds.is_empty() {
        let mut ors = Vec::new();
        for t in &q.kinds {
            let v = t.strip_suffix('s').unwrap_or(t).to_lowercase();
            params.push(Box::new(format!("{}%", v)));
            ors.push(format!("LOWER(w.work_type) LIKE ?{}", params.len()));
        }
        add(format!("({})", ors.join(" OR ")));
    }
    if let Some(s) = q.source.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(s.to_string()));
        add(format!("w.source_system = ?{}", params.len()));
    }
    if let Some(c) = q.color.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(c.to_string()));
        add(format!("h.annotation_color = ?{}", params.len()));
    }
    if let Some(a) = q.after.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(a.to_string()));
        add(format!("h.highlighted_at >= ?{}", params.len()));
    }
    if let Some(b) = q.before.as_deref().filter(|s| !s.is_empty()) {
        params.push(Box::new(b.to_string()));
        add(format!(
            "h.highlighted_at IS NOT NULL AND h.highlighted_at <> '' AND h.highlighted_at < ?{}",
            params.len()
        ));
    }
}

/// Build ORDER BY (coverage -> field -> recency) mirroring the extension.
fn build_order(q: &SearchQuery, params: &mut Vec<Box<dyn rusqlite::ToSql>>) -> String {
    match q.sort.as_str() {
        "recent" => "ORDER BY h.highlighted_at DESC".to_string(),
        "oldest" => {
            "ORDER BY (h.highlighted_at IS NULL OR h.highlighted_at='') ASC, h.highlighted_at ASC"
                .to_string()
        }
        _ => {
            let terms: Vec<&String> = q.positive_terms.iter().filter(|t| !t.is_empty()).collect();
            if terms.is_empty() {
                return "ORDER BY h.highlighted_at DESC".to_string();
            }
            let mut coverage = Vec::new();
            for t in &terms {
                params.push(Box::new(format!("%{}%", escape_like(t))));
                coverage.push(format!(
                    "(CASE WHEN {HAYSTACK} LIKE ?{} ESCAPE '\\' THEN 1 ELSE 0 END)",
                    params.len()
                ));
            }
            let mut author = Vec::new();
            for t in &terms {
                params.push(Box::new(format!("%{}%", escape_like(t))));
                author.push(format!("w.author LIKE ?{} ESCAPE '\\'", params.len()));
            }
            let mut title = Vec::new();
            for t in &terms {
                params.push(Box::new(format!("%{}%", escape_like(t))));
                title.push(format!("w.title LIKE ?{} ESCAPE '\\'", params.len()));
            }
            format!(
                "ORDER BY ({}) DESC, (CASE WHEN ({}) THEN 0 WHEN ({}) THEN 1 ELSE 2 END) ASC, h.highlighted_at DESC",
                coverage.join(" + "),
                author.join(" OR "),
                title.join(" OR ")
            )
        }
    }
}

fn compile_regexes(filters: &[crate::models::RegexFilter]) -> Vec<regex::Regex> {
    filters
        .iter()
        .filter_map(|f| {
            let case_sensitive = f.flags.contains('c');
            let mut b = regex::RegexBuilder::new(&f.source);
            b.case_insensitive(!case_sensitive);
            if f.flags.contains('m') {
                b.multi_line(true);
            }
            if f.flags.contains('s') {
                b.dot_matches_new_line(true);
            }
            b.build().ok()
        })
        .collect()
}

fn passes_negatives(r: &Hit, negatives: &[String]) -> bool {
    if negatives.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {}",
        r.text,
        r.title,
        r.author.as_deref().unwrap_or("")
    )
    .to_lowercase();
    negatives.iter().all(|n| !hay.contains(&n.to_lowercase()))
}

fn passes_regexes(r: &Hit, regexes: &[regex::Regex]) -> bool {
    if regexes.is_empty() {
        return true;
    }
    let hay = format!(
        "{}\n{}\n{}",
        r.text,
        r.note.as_deref().unwrap_or(""),
        r.title
    );
    regexes.iter().all(|re| re.is_match(&hay))
}

fn run_query(conn: &Connection, q: &SearchQuery, limit: usize, offset: usize) -> Result<Vec<Hit>> {
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut where_sql = String::new();

    let from = if q.has_positive && !q.fts.is_empty() {
        params.push(Box::new(q.fts.clone()));
        where_sql.push_str(&format!("search_index MATCH ?{}", params.len()));
        "search_index \
         JOIN highlights h ON h.id = search_index.highlight_id \
         JOIN works w ON w.id = search_index.work_id"
    } else {
        where_sql.push_str("1=1");
        "highlights h JOIN works w ON w.id = h.work_id"
    };

    push_filters(q, &mut where_sql, &mut params);
    let order = build_order(q, &mut params);

    params.push(Box::new(limit as i64));
    let limit_idx = params.len();
    params.push(Box::new(offset as i64));
    let offset_idx = params.len();

    let sql = format!(
        "SELECT {RESULT_COLS} FROM {from} WHERE {where_sql} {order} LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
    );
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(refs.as_slice(), map_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn search_query(conn: &Connection, q: &SearchQuery) -> Result<SearchPage> {
    let regexes = compile_regexes(&q.regexes);

    // Regex needs a one-shot wide scan (can't honour SQL OFFSET cleanly).
    if !regexes.is_empty() {
        if q.page > 0 {
            return Ok(SearchPage {
                rows: vec![],
                has_more: false,
            });
        }
        let candidates = run_query(conn, q, REGEX_SCAN_CAP, 0)?;
        let rows: Vec<Hit> = candidates
            .into_iter()
            .filter(|r| passes_regexes(r, &regexes) && passes_negatives(r, &q.negatives))
            .take(REGEX_RESULT_CAP)
            .collect();
        return Ok(SearchPage {
            rows,
            has_more: false,
        });
    }

    let candidates = run_query(conn, q, q.page_size, q.page * q.page_size)?;
    let has_more = candidates.len() == q.page_size;
    let rows = candidates
        .into_iter()
        .filter(|r| passes_negatives(r, &q.negatives))
        .collect();
    Ok(SearchPage { rows, has_more })
}

/// All records in one container, in reading order.
pub fn container_records(conn: &Connection, container_id: &str) -> Result<Vec<Hit>> {
    let sql = format!(
        "SELECT {RESULT_COLS} FROM highlights h JOIN works w ON w.id = h.work_id
         WHERE h.work_id = ?1
         ORDER BY CAST(NULLIF(h.location,'') AS INTEGER), h.highlighted_at"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([container_id], map_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Position of one record within its container: rank, total, max location.
pub fn record_position(
    conn: &Connection,
    container_id: &str,
    location: &str,
) -> Result<Option<Position>> {
    let loc: i64 = location.parse().unwrap_or(0);
    let row = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM highlights WHERE work_id = ?1) AS total,
           (SELECT COUNT(*) FROM highlights WHERE work_id = ?1
              AND location IS NOT NULL AND location <> ''
              AND CAST(location AS INTEGER) <= ?2) AS pos,
           (SELECT MAX(CAST(NULLIF(location,'') AS INTEGER)) FROM highlights WHERE work_id = ?1) AS maxloc",
        rusqlite::params![container_id, loc],
        |r| {
            Ok(Position {
                total: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                pos: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                max_loc: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
            })
        },
    )?;
    Ok(Some(row))
}

/// Fetch a single record by id (for the find-related window's source quote).
pub fn record_by_id(conn: &Connection, id: &str) -> Option<Hit> {
    let sql = format!(
        "SELECT {RESULT_COLS} FROM highlights h JOIN works w ON w.id = h.work_id WHERE h.id = ?1"
    );
    conn.query_row(&sql, [id], map_row).ok()
}

/// Look up a container id by its slug (bridges QMD file results to records).
pub fn container_id_by_slug(conn: &Connection, slug: &str) -> Option<String> {
    conn.query_row("SELECT id FROM works WHERE slug = ?1", [slug], |r| r.get(0))
        .ok()
}

/// Distinct tags by frequency (mirrors the extension's allTags).
pub fn list_tags(conn: &Connection) -> Result<Vec<TagCount>> {
    let mut stmt = conn.prepare(
        "SELECT lower(trim(value)) AS tag, COUNT(*) AS count
         FROM highlights, json_each(highlights.tags)
         WHERE highlights.tags IS NOT NULL AND highlights.tags NOT IN ('', '[]')
         GROUP BY tag HAVING tag <> ''
         ORDER BY count DESC, tag ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagCount {
                tag: r.get(0)?,
                count: r.get(1)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Distinct facets for the filter UI: sources and colours actually present.
pub fn facets(conn: &Connection) -> Result<(Vec<String>, Vec<String>)> {
    let mut src_stmt =
        conn.prepare("SELECT DISTINCT source_system FROM works ORDER BY source_system")?;
    let sources: Vec<String> = src_stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut col_stmt = conn.prepare(
        "SELECT annotation_color, COUNT(*) c FROM highlights
         WHERE annotation_color IS NOT NULL
         GROUP BY annotation_color ORDER BY c DESC",
    )?;
    let colors: Vec<String> = col_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    Ok((sources, colors))
}

/// Dump all containers (for JSON export).
pub fn all_containers(conn: &Connection) -> Result<Vec<Container>> {
    let mut stmt = conn.prepare(
        "SELECT id, slug, title, author, work_type, source_system, source_id, url,
                imported_at, updated_at, source_data FROM works ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let sd: String = r.get(10)?;
            Ok(Container {
                id: r.get(0)?,
                slug: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                kind: r.get(4)?,
                source_system: r.get(5)?,
                source_id: r.get(6)?,
                url: r.get(7)?,
                imported_at: r.get(8)?,
                updated_at: r.get(9)?,
                source_data: serde_json::from_str(&sd).unwrap_or(serde_json::Value::Null),
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Dump all records (for JSON export).
pub fn all_records(conn: &Connection) -> Result<Vec<Record>> {
    let mut stmt = conn.prepare(
        "SELECT id, work_id, text, note, highlighted_at, updated_at, tags, location,
                location_type, annotation_color, annotation_type, format, source_data
         FROM highlights ORDER BY work_id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let tags: String = r.get(6)?;
            let sd: String = r.get(12)?;
            Ok(Record {
                id: r.get(0)?,
                container_id: r.get(1)?,
                text: r.get(2)?,
                note: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                location: r.get(7)?,
                location_type: r.get(8)?,
                annotation_color: r.get(9)?,
                annotation_type: r.get(10)?,
                format: r.get(11)?,
                source_data: serde_json::from_str(&sd).unwrap_or(serde_json::Value::Null),
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn record_count(conn: &Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM highlights", [], |row| row.get(0))
        .unwrap_or(0)
}

pub fn container_count(conn: &Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM works", [], |row| row.get(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_old_search_index_to_include_ocr() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE search_index USING fts5(
                highlight_id UNINDEXED, work_id UNINDEXED, text, note, title, author, tags,
                tokenize='porter unicode61');",
        )
        .unwrap();
        init_schema(&conn).unwrap();
        let has_ocr: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('search_index') WHERE name='ocr'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_ocr, 1);
    }

    #[test]
    fn search_matches_text_found_only_in_ocr() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute("INSERT INTO works (id,slug,title,author,work_type,source_system,source_id,url,imported_at,updated_at,source_data) VALUES ('w1','w1','W',NULL,'article','x',NULL,NULL,'t','t','{}')", []).unwrap();
        conn.execute("INSERT INTO highlights (id,work_id,text,tags,format,source_data) VALUES ('h1','w1','a plain body','[]','plain','{}')", []).unwrap();
        conn.execute(
            "UPDATE highlights SET ocr_text='ZEBRACODE inside the screenshot' WHERE id='h1'",
            [],
        )
        .unwrap();
        reindex_record_fts(&conn, "h1").unwrap();
        let q = keyword_query("ZEBRACODE", None);
        let page = search_query(&conn, &q).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].record_id, "h1");
    }

    #[test]
    fn tag_any_matches_favorite_semantics() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute("INSERT INTO works (id,slug,title,author,work_type,source_system,source_id,url,imported_at,updated_at,source_data) VALUES ('w1','w1','W',NULL,'article','x',NULL,NULL,'t','t','{}')", []).unwrap();
        conn.execute("INSERT INTO highlights (id,work_id,text,tags,format,source_data) VALUES ('h1','w1','alpha','[\"favorite\"]','plain','{}')", []).unwrap();
        conn.execute("INSERT INTO highlights (id,work_id,text,tags,format,source_data) VALUES ('h2','w1','alpha','[]','plain','{}')", []).unwrap();
        reindex_record_fts(&conn, "h1").unwrap();
        reindex_record_fts(&conn, "h2").unwrap();
        let mut q = keyword_query("alpha", None);
        q.tag_any = vec!["favorite".into(), "Liked".into()];
        let page = search_query(&conn, &q).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].record_id, "h1");
    }

    #[test]
    fn source_any_matches_source_system_exactly() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute("INSERT INTO works (id,slug,title,author,work_type,source_system,source_id,url,imported_at,updated_at,source_data) VALUES ('w1','w1','W',NULL,'article','zotero',NULL,NULL,'t','t','{}')", []).unwrap();
        conn.execute("INSERT INTO works (id,slug,title,author,work_type,source_system,source_id,url,imported_at,updated_at,source_data) VALUES ('w2','w2','W2',NULL,'article','readwise',NULL,NULL,'t','t','{}')", []).unwrap();
        conn.execute("INSERT INTO highlights (id,work_id,text,tags,format,source_data) VALUES ('h1','w1','beta','[]','plain','{}')", []).unwrap();
        conn.execute("INSERT INTO highlights (id,work_id,text,tags,format,source_data) VALUES ('h2','w2','beta','[]','plain','{}')", []).unwrap();
        reindex_record_fts(&conn, "h1").unwrap();
        reindex_record_fts(&conn, "h2").unwrap();
        let mut q = keyword_query("beta", None);
        q.source_any = vec!["zotero".into()];
        let page = search_query(&conn, &q).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].source_system, "zotero");
    }

    fn keyword_query(token: &str, color: Option<&str>) -> SearchQuery {
        SearchQuery {
            fts: format!("\"{}\"", token),
            has_positive: true,
            positive_terms: vec![token.to_string()],
            negatives: vec![],
            regexes: vec![],
            author: None,
            title: None,
            kind: None,
            tag: None,
            tag_any: vec![],
            source_any: vec![],
            has_image: false,
            kinds: vec![],
            after: None,
            before: None,
            source: None,
            color: color.map(|c| c.to_string()),
            sort: "matches".to_string(),
            page: 0,
            page_size: 50,
        }
    }
}
