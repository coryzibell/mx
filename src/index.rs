use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use walkdir::WalkDir;

use crate::db::Database;
use crate::knowledge::KnowledgeEntry;

/// Index configuration
pub struct IndexConfig {
    pub zion_root: std::path::PathBuf,
    pub db_path: std::path::PathBuf,
    pub jsonl_path: std::path::PathBuf,
    pub excluded_dirs: Vec<String>,
}

impl Default for IndexConfig {
    fn default() -> Self {
        let home = dirs::home_dir().expect("No home directory");
        let matrix = home.join(".matrix");

        Self {
            zion_root: matrix.join("zion"),
            db_path: matrix.join("zion").join("knowledge.db"),
            jsonl_path: matrix.join("zion").join("index.jsonl"),
            excluded_dirs: vec!["future".to_string()],
        }
    }
}

/// Rebuild the entire index from zion markdown files
pub fn rebuild_index(config: &IndexConfig) -> Result<IndexStats> {
    // Ensure db directory exists
    if let Some(parent) = config.db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let db = Database::open(&config.db_path)?;
    let mut stats = IndexStats::default();

    // Walk zion directory
    for entry in WalkDir::new(&config.zion_root)
        .into_iter()
        .filter_entry(|e| !is_excluded(e, &config.excluded_dirs))
    {
        let entry = entry?;
        let path = entry.path();

        // Only process markdown files
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        // Skip index.jsonl and other non-knowledge files
        if path.file_name().and_then(|n| n.to_str()) == Some("index.jsonl") {
            continue;
        }

        match KnowledgeEntry::from_markdown(path, &config.zion_root) {
            Ok(entry) => {
                db.upsert_knowledge(&entry)?;
                stats.indexed += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse {:?}: {}", path, e);
                stats.errors += 1;
            }
        }
    }

    stats.total = db.count()?;

    Ok(stats)
}

/// Check if directory should be excluded
fn is_excluded(entry: &walkdir::DirEntry, excluded: &[String]) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }

    entry
        .file_name()
        .to_str()
        .map(|s| excluded.contains(&s.to_string()))
        .unwrap_or(false)
}

/// Export database to markdown
pub fn export_markdown(db: &Database, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "# Zion Knowledge Export\n")?;

    // Export all categories
    for category in &["pattern", "technique", "insight", "ritual", "project"] {
        let entries = db.list_by_category(category)?;
        if entries.is_empty() {
            continue;
        }

        writeln!(writer, "## {}\n", capitalize(category))?;

        for entry in entries {
            writeln!(writer, "### {}", entry.title)?;
            writeln!(writer, "**Category:** {}", entry.category)?;

            if !entry.tags.is_empty() {
                writeln!(writer, "**Tags:** {}", entry.tags.join(", "))?;
            }

            if let Some(created) = &entry.created_at {
                writeln!(writer, "**Created:** {}", created)?;
            }

            writeln!(writer)?;

            if let Some(body) = &entry.body {
                writeln!(writer, "{}", body)?;
            }

            writeln!(writer, "\n---\n")?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Export database to JSONL
pub fn export_jsonl(db: &Database, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    // Export all categories
    for category in &["pattern", "technique", "insight", "ritual", "project"] {
        for entry in db.list_by_category(category)? {
            let json = serde_json::to_string(&entry)?;
            writeln!(writer, "{}", json)?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Export database to CSV (metadata only, no body)
pub fn export_csv(db: &Database, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    // CSV header
    writeln!(writer, "id,category,title,tags,created_at,updated_at")?;

    // Export all categories
    for category in &["pattern", "technique", "insight", "ritual", "project"] {
        for entry in db.list_by_category(category)? {
            let tags = entry.tags.join(";"); // Use semicolon to avoid comma collision
            let created = entry.created_at.as_deref().unwrap_or("");
            let updated = entry.updated_at.as_deref().unwrap_or("");

            writeln!(
                writer,
                "{},{},\"{}\",\"{}\",{},{}",
                entry.id, entry.category, entry.title, tags, created, updated
            )?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Import JSONL into database
pub fn import_jsonl(db: &Database, path: &Path) -> Result<usize> {
    let file = File::open(path).with_context(|| format!("Failed to open {:?}", path))?;
    let reader = BufReader::new(file);

    let mut count = 0;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let entry: KnowledgeEntry = serde_json::from_str(&line)
            .with_context(|| format!("Failed to parse line: {}", line))?;

        db.upsert_knowledge(&entry)?;
        count += 1;
    }

    Ok(count)
}

#[derive(Debug, Default)]
pub struct IndexStats {
    pub indexed: usize,
    pub errors: usize,
    pub total: usize,
}

impl std::fmt::Display for IndexStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Indexed {} files ({} errors), {} total entries",
            self.indexed, self.errors, self.total
        )
    }
}
