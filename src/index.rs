use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use walkdir::WalkDir;

use crate::knowledge::KnowledgeEntry;
use crate::store::KnowledgeStore;

/// Index configuration
pub struct IndexConfig {
    pub memory_root: std::path::PathBuf,
    pub db_path: std::path::PathBuf,
    pub jsonl_path: std::path::PathBuf,
    pub excluded_dirs: Vec<String>,
}

impl Default for IndexConfig {
    fn default() -> Self {
        let home = dirs::home_dir().expect("No home directory");
        let base = std::env::var("MX_MEMORY_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| home.join(".crystal-tokyo"));

        Self {
            memory_root: base.join("memory"),
            db_path: base.join("memory").join("knowledge.db"),
            jsonl_path: base.join("memory").join("index.jsonl"),
            excluded_dirs: vec!["future".to_string()],
        }
    }
}

/// Rebuild the entire index from memory markdown files
pub fn rebuild_index(config: &IndexConfig) -> Result<IndexStats> {
    // Ensure db directory exists
    if let Some(parent) = config.db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let db = crate::store::create_store(&config.db_path)?;
    let mut stats = IndexStats::default();

    // Walk memory directory
    for entry in WalkDir::new(&config.memory_root)
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

        match KnowledgeEntry::from_markdown(path, &config.memory_root) {
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

/// Export database to markdown directory structure
pub fn export_markdown(db: &dyn KnowledgeStore, dir_path: &Path) -> Result<()> {
    // Create base directory
    fs::create_dir_all(dir_path)
        .with_context(|| format!("Failed to create directory {:?}", dir_path))?;

    // Export all categories dynamically
    // Use public_only context to export all non-private entries
    let ctx = crate::store::AgentContext::public_only();
    let filter = crate::store::KnowledgeFilter::default();
    let categories = db.list_categories()?;
    for category in categories {
        let entries = db.list_by_category(&category.id, &ctx, &filter)?;
        if entries.is_empty() {
            continue;
        }

        // Create category subdirectory
        let category_dir = dir_path.join(&category.id);
        fs::create_dir_all(&category_dir)
            .with_context(|| format!("Failed to create category dir {:?}", category_dir))?;

        for entry in entries {
            // Generate filename from title
            let filename = slugify(&entry.title);
            let file_path = category_dir.join(format!("{}.md", filename));

            // Handle filename collisions
            let final_path = get_unique_path(&file_path)?;

            // Write entry to individual file
            let file = File::create(&final_path)
                .with_context(|| format!("Failed to create {:?}", final_path))?;
            let mut writer = BufWriter::new(file);

            // Write frontmatter
            writeln!(writer, "---")?;
            writeln!(writer, "id: {}", entry.id)?;
            writeln!(writer, "title: {}", entry.title)?;
            writeln!(writer, "category: {}", entry.category_id)?;

            if !entry.tags.is_empty() {
                writeln!(writer, "tags: [{}]", entry.tags.join(", "))?;
            }

            if !entry.applicability.is_empty() {
                if entry.applicability.len() == 1 {
                    writeln!(writer, "applicability: {}", entry.applicability[0])?;
                } else {
                    writeln!(writer, "applicability:")?;
                    for app in &entry.applicability {
                        writeln!(writer, "  - {}", app)?;
                    }
                }
            }

            if let Some(created) = &entry.created_at {
                writeln!(writer, "created: {}", created)?;
            }

            if let Some(updated) = &entry.updated_at {
                writeln!(writer, "updated: {}", updated)?;
            }

            if let Some(source_project) = &entry.source_project_id {
                writeln!(writer, "source_project: {}", source_project)?;
            }

            if let Some(source_agent) = &entry.source_agent_id {
                writeln!(writer, "source_agent: {}", source_agent)?;
            }

            // Only write resonance if it's meaningful (non-zero)
            if entry.resonance > 0 {
                writeln!(writer, "resonance: {}", entry.resonance)?;
            }

            if let Some(ref resonance_type) = entry.resonance_type {
                writeln!(writer, "resonance_type: {}", resonance_type)?;
            }

            if let Some(ref wake_phrase) = entry.wake_phrase {
                // Quote it because wake phrases may contain special YAML characters
                writeln!(writer, "wake_phrase: \"{}\"", wake_phrase.replace("\"", "\\\""))?;
            }

            writeln!(writer, "---\n")?;

            // Write body
            if let Some(body) = &entry.body {
                writeln!(writer, "{}", body)?;
            }

            writer.flush()?;
        }
    }

    Ok(())
}

/// Slugify a string for use as a filename
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' => c,
            ' ' | '-' | '_' => '-',
            _ => '_',
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Get unique path by appending -1, -2, etc. if file exists
fn get_unique_path(path: &Path) -> Result<std::path::PathBuf> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid file stem")?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path.parent().context("No parent directory")?;

    for i in 1..1000 {
        let new_name = if ext.is_empty() {
            format!("{}-{}", stem, i)
        } else {
            format!("{}-{}.{}", stem, i, ext)
        };

        let new_path = parent.join(new_name);
        if !new_path.exists() {
            return Ok(new_path);
        }
    }

    anyhow::bail!("Could not find unique filename for {:?}", path)
}

/// Export database to JSONL
pub fn export_jsonl(db: &dyn KnowledgeStore, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    // Export all categories dynamically
    // Use public_only context to export all non-private entries
    let ctx = crate::store::AgentContext::public_only();
    let filter = crate::store::KnowledgeFilter::default();
    let categories = db.list_categories()?;
    for category in categories {
        for entry in db.list_by_category(&category.id, &ctx, &filter)? {
            let json = serde_json::to_string(&entry)?;
            writeln!(writer, "{}", json)?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Export database to CSV (metadata only, no body)
pub fn export_csv(db: &dyn KnowledgeStore, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    // CSV header (v3 schema field names)
    writeln!(
        writer,
        "id,category_id,title,tags,applicability,source_project_id,created_at,updated_at"
    )?;

    // Export all categories dynamically
    // Use public_only context to export all non-private entries
    let ctx = crate::store::AgentContext::public_only();
    let filter = crate::store::KnowledgeFilter::default();
    let categories = db.list_categories()?;
    for category in categories {
        for entry in db.list_by_category(&category.id, &ctx, &filter)? {
            let tags = entry.tags.join(";"); // Use semicolon to avoid comma collision
            let applicability = entry.applicability.join(";");
            let source_project = entry.source_project_id.as_deref().unwrap_or("");
            let created = entry.created_at.as_deref().unwrap_or("");
            let updated = entry.updated_at.as_deref().unwrap_or("");

            writeln!(
                writer,
                "{},{},\"{}\",\"{}\",\"{}\",{},{},{}",
                entry.id,
                entry.category_id,
                entry.title,
                tags,
                applicability,
                source_project,
                created,
                updated
            )?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Import JSONL into database
pub fn import_jsonl(db: &dyn KnowledgeStore, path: &Path) -> Result<usize> {
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
