use anyhow::Result;
use slug::slugify;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Minimal read-view of a container for archive rendering.
pub trait ContainerMeta {
    fn slug(&self) -> &str;
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn author(&self) -> Option<&str>;
    fn kind(&self) -> &str;
    fn source_system(&self) -> &str;
    fn source_id(&self) -> Option<&str>;
    fn url(&self) -> Option<&str>;
    fn imported_at(&self) -> &str;
    fn updated_at(&self) -> &str;
    fn source_data_json(&self) -> String;
}

/// Minimal read-view of a record for archive rendering.
pub trait RecordMeta {
    fn id(&self) -> &str;
    fn text(&self) -> &str;
    fn note(&self) -> Option<&str>;
    fn created_at(&self) -> Option<&str>;
    fn tags(&self) -> &[String];
    fn annotation_color(&self) -> Option<&str>;
    fn annotation_type(&self) -> Option<&str>;
    fn format(&self) -> &str;
}

/// Build a stable, unique, filesystem-safe slug for a container.
/// Format: {author}-{title}-{source_id}, truncated to a safe length.
/// The source_id suffix guarantees uniqueness even when author+title collide
/// across sources or types.
pub fn make_slug(author: Option<&str>, title: &str, source_id: &str) -> String {
    let author_part = author.unwrap_or("unknown");
    let base = slugify(format!("{}-{}", author_part, title));
    // Truncate the descriptive part so the final filename stays well under
    // the 255-byte filesystem limit, then append the (numeric/short) id.
    let truncated: String = base.chars().take(120).collect();
    let truncated = truncated.trim_end_matches('-');
    let id_part = slugify(source_id);
    if id_part.is_empty() {
        truncated.to_string()
    } else {
        format!("{}-{}", truncated, id_part)
    }
}

/// Write the full body text of a container to readings/fulltext/{slug}.md.
pub fn write_fulltext(archive_path: &str, slug: &str, text: &str) -> Result<()> {
    let dir = Path::new(archive_path).join("readings").join("fulltext");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{}.md", slug)), text)?;
    Ok(())
}

/// Write a raw source export snapshot to imports/{source}-{stamp}.json.
/// Preserves provenance; the Archive is derived from these.
pub fn write_import_batch(
    archive_path: &str,
    source: &str,
    stamp: &str,
    raw_json: &str,
) -> Result<()> {
    let dir = Path::new(archive_path).join("imports");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{}-{}.json", source, stamp)), raw_json)?;
    Ok(())
}

/// Write v2 Archive Markdown files for a batch of containers and their records.
pub fn write_archive<C: ContainerMeta, R: RecordMeta>(
    archive_path: &str,
    containers: &[C],
    records_by_container: &HashMap<String, Vec<&R>>,
) -> Result<()> {
    let base = Path::new(archive_path);
    fs::create_dir_all(base.join("readings").join("works"))?;
    fs::create_dir_all(base.join("readings").join("fulltext"))?;
    fs::create_dir_all(base.join("readings").join("assets"))?;

    let works_dir = base.join("readings").join("works");

    for container in containers {
        // Flat directory: readings/works/{slug}.md
        let file_path = works_dir.join(format!("{}.md", container.slug()));
        let empty = vec![];
        let records = records_by_container.get(container.id()).unwrap_or(&empty);

        let content = render_container_file(container, records);
        fs::write(&file_path, content)?;
    }

    Ok(())
}

pub fn render_container_file<C: ContainerMeta, R: RecordMeta>(
    container: &C,
    records: &[&R],
) -> String {
    let mut out = String::new();

    // Frontmatter
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_escape(container.title())));
    if let Some(author) = container.author() {
        out.push_str(&format!("author: {}\n", yaml_escape(author)));
    }
    out.push_str(&format!("type: {}\n", container.kind()));
    out.push_str(&format!("source_system: {}\n", container.source_system()));
    if let Some(sid) = container.source_id() {
        out.push_str(&format!("source_id: \"{}\"\n", sid));
    }
    if let Some(url) = container.url() {
        out.push_str(&format!("url: {}\n", url));
    }
    out.push_str(&format!("imported_at: {}\n", container.imported_at()));
    out.push_str(&format!("updated_at: {}\n", container.updated_at()));
    out.push_str(&format!("source_data: {}\n", container.source_data_json()));
    out.push_str("---\n\n");

    // Records
    for record in records {
        out.push_str(&render_record(*record));
        out.push_str("\n---\n\n");
    }

    out
}

pub fn render_record<R: RecordMeta>(record: &R) -> String {
    let mut out = String::new();

    match record.format() {
        "image" => {
            out.push_str(&format!("![](../assets/{}.png)\n\n", record.id()));
        }
        "latex" => {
            out.push_str("```latex\n");
            out.push_str(record.text());
            out.push_str("\n```\n\n");
        }
        _ => {
            // Plain text: blockquote, wrapping long lines
            for line in record.text().lines() {
                out.push_str(&format!("> {}\n", line));
            }
            out.push('\n');
        }
    }

    // Metadata line
    let mut meta_parts = Vec::new();
    if let Some(date) = record.created_at() {
        // Trim to date portion if ISO datetime
        let date_short = date.split('T').next().unwrap_or(date);
        meta_parts.push(format!("highlighted_at: {}", date_short));
    }
    if !record.tags().is_empty() {
        meta_parts.push(format!("tags: {}", record.tags().join(", ")));
    }
    if let Some(color) = record.annotation_color() {
        meta_parts.push(format!("color: {}", color));
    }
    if let Some(atype) = record.annotation_type() {
        meta_parts.push(format!("type: {}", atype));
    }
    if record.format() != "plain" {
        meta_parts.push(format!("format: {}", record.format()));
    }
    if !meta_parts.is_empty() {
        out.push_str(&meta_parts.join(" | "));
        out.push('\n');
    }

    // Note
    if let Some(note) = record.note() {
        if !note.is_empty() {
            out.push('\n');
            out.push_str(note);
            out.push('\n');
        }
    }

    out
}

pub fn yaml_escape(s: &str) -> String {
    if s.contains(':') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestRecord {
        id: String,
        text: String,
        note: Option<String>,
        created_at: Option<String>,
        tags: Vec<String>,
        annotation_color: Option<String>,
        annotation_type: Option<String>,
        format: String,
    }

    impl RecordMeta for TestRecord {
        fn id(&self) -> &str {
            &self.id
        }

        fn text(&self) -> &str {
            &self.text
        }

        fn note(&self) -> Option<&str> {
            self.note.as_deref()
        }

        fn created_at(&self) -> Option<&str> {
            self.created_at.as_deref()
        }

        fn tags(&self) -> &[String] {
            &self.tags
        }

        fn annotation_color(&self) -> Option<&str> {
            self.annotation_color.as_deref()
        }

        fn annotation_type(&self) -> Option<&str> {
            self.annotation_type.as_deref()
        }

        fn format(&self) -> &str {
            &self.format
        }
    }

    fn sample_record(text: &str, format: &str) -> TestRecord {
        TestRecord {
            id: "h1".into(),
            text: text.into(),
            note: None,
            created_at: Some("2024-01-15T10:00:00Z".into()),
            tags: vec!["methods".into()],
            annotation_color: Some("green".into()),
            annotation_type: Some("highlight".into()),
            format: format.into(),
        }
    }

    #[test]
    fn slug_includes_source_id_for_uniqueness() {
        let a = make_slug(Some("Jane Smith"), "How LLMs Work", "123");
        assert_eq!(a, "jane-smith-how-llms-work-123");
    }

    #[test]
    fn slug_disambiguates_same_title_by_id() {
        let a = make_slug(Some("Smith"), "Same Title", "1");
        let b = make_slug(Some("Smith"), "Same Title", "2");
        assert_ne!(a, b);
    }

    #[test]
    fn slug_truncates_long_titles() {
        let long = "word ".repeat(100);
        let s = make_slug(Some("Author"), &long, "42");
        // descriptive part capped at 120 chars + "-42"
        assert!(s.len() <= 130, "slug too long: {}", s.len());
        assert!(s.ends_with("-42"));
    }

    #[test]
    fn slug_handles_missing_author() {
        let s = make_slug(None, "Title", "9");
        assert_eq!(s, "unknown-title-9");
    }

    #[test]
    fn renders_plain_record_as_blockquote() {
        let r = sample_record("Hello world", "plain");
        let out = render_record(&r);
        assert!(out.contains("> Hello world"));
        assert!(out.contains("highlighted_at: 2024-01-15"));
        assert!(out.contains("tags: methods"));
        assert!(out.contains("color: green"));
    }

    #[test]
    fn renders_latex_as_fenced_block() {
        let r = sample_record("\\frac{1}{2}", "latex");
        let out = render_record(&r);
        assert!(out.contains("```latex"));
        assert!(out.contains("\\frac{1}{2}"));
        assert!(out.contains("format: latex"));
    }

    #[test]
    fn renders_image_as_asset_reference() {
        let r = sample_record("", "image");
        let out = render_record(&r);
        assert!(out.contains("![](../assets/h1.png)"));
        assert!(out.contains("format: image"));
    }
}
