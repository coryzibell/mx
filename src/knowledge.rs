use anyhow::{Context, Result};
use base_d::{DictionaryRegistry, HashAlgorithm, encode, hash};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A knowledge entry from Zion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub category_id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub applicability: Vec<String>,
    #[serde(default)]
    pub source_project_id: Option<String>,
    #[serde(default)]
    pub source_agent_id: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,

    // Provenance metadata - tracks where knowledge came from
    /// Source type: manual, ram, cache, agent_session
    #[serde(default)]
    pub source_type_id: Option<String>,
    /// Entry type: primary (original), summary, synthesis
    #[serde(default)]
    pub entry_type_id: Option<String>,
    /// Session ID if absorbed from RAM
    #[serde(default)]
    pub session_id: Option<String>,
    /// Ephemeral hint - session-based knowledge that may be pruned
    #[serde(default)]
    pub ephemeral: bool,
}

/// Custom deserializer for applicability - accepts string or array
fn deserialize_applicability<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_yaml::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => Ok(vec![s]),
        Value::Sequence(seq) => seq
            .into_iter()
            .map(|v| match v {
                Value::String(s) => Ok(s),
                _ => Err(D::Error::custom("Expected string in applicability array")),
            })
            .collect(),
        _ => Ok(vec![]),
    }
}

/// Frontmatter parsed from markdown
#[derive(Debug, Default, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_applicability")]
    pub applicability: Vec<String>,
    #[serde(default)]
    pub source_project: Option<String>,
    #[serde(default)]
    pub source_agent: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
}

impl KnowledgeEntry {
    /// Generate a hash-based ID from path and title
    pub fn generate_id(path: &str, title: &str) -> String {
        let input = format!("{}:{}", path, title);
        let hex = Self::blake3_hex(input.as_bytes());
        format!("kn-{}", &hex[..8])
    }

    /// Compute content hash for change detection
    pub fn compute_hash(content: &str) -> String {
        Self::blake3_hex(content.as_bytes())
    }

    /// Hash data with blake3 and encode as lowercase hex
    fn blake3_hex(data: &[u8]) -> String {
        let hash_bytes = hash(data, HashAlgorithm::Blake3);
        let registry = DictionaryRegistry::load_default().expect("base-d dictionaries");
        let dict = registry.dictionary("base16").expect("base16 dictionary");
        encode(&hash_bytes, &dict).to_lowercase()
    }

    /// Parse a markdown file into a knowledge entry
    pub fn from_markdown(path: &Path, zion_root: &Path) -> Result<Self> {
        let content =
            fs::read_to_string(path).with_context(|| format!("Failed to read {:?}", path))?;

        let (frontmatter, body) = parse_frontmatter(&content)?;

        // Derive category from path if not in frontmatter
        let relative = path.strip_prefix(zion_root).unwrap_or(path);
        let category_id = frontmatter.category.clone().unwrap_or_else(|| {
            relative
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        // Extract title from frontmatter or first heading
        let title = frontmatter.title.clone().unwrap_or_else(|| {
            extract_first_heading(&body).unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Untitled")
                    .to_string()
            })
        });

        // Extract summary (first paragraph after heading)
        let summary = extract_summary(&body);

        // Generate ID if not provided
        let path_str = relative.to_string_lossy().to_string();
        let id = frontmatter
            .id
            .unwrap_or_else(|| Self::generate_id(&path_str, &title));

        let now = chrono::Utc::now().to_rfc3339();

        Ok(Self {
            id,
            category_id,
            title,
            body: Some(body),
            summary,
            applicability: frontmatter.applicability,
            source_project_id: frontmatter.source_project,
            source_agent_id: frontmatter.source_agent,
            file_path: Some(path_str),
            tags: frontmatter.tags,
            created_at: frontmatter.created.or_else(|| Some(now.clone())),
            updated_at: Some(now),
            content_hash: Some(Self::compute_hash(&content)),
            // Markdown files are manual, primary knowledge
            source_type_id: Some("manual".to_string()),
            entry_type_id: Some("primary".to_string()),
            session_id: None,
            ephemeral: false,
        })
    }
}

/// Parse YAML frontmatter from markdown content
fn parse_frontmatter(content: &str) -> Result<(Frontmatter, String)> {
    let content = content.trim_start();

    if !content.starts_with("---") {
        return Ok((Frontmatter::default(), content.to_string()));
    }

    let rest = &content[3..];
    let end = rest.find("\n---").or_else(|| rest.find("\r\n---"));

    match end {
        Some(pos) => {
            let yaml = &rest[..pos];
            let body = rest[pos + 4..].trim_start_matches(['\n', '\r']).to_string();

            let frontmatter: Frontmatter = serde_yaml::from_str(yaml).unwrap_or_default();

            Ok((frontmatter, body))
        }
        None => Ok((Frontmatter::default(), content.to_string())),
    }
}

/// Extract the first markdown heading
fn extract_first_heading(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return Some(trimmed.trim_start_matches('#').trim().to_string());
        }
    }
    None
}

/// Extract summary (first non-empty paragraph after any heading)
fn extract_summary(content: &str) -> Option<String> {
    let mut in_paragraph = false;
    let mut paragraph = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip headings
        if trimmed.starts_with('#') {
            continue;
        }

        // Empty line ends paragraph
        if trimmed.is_empty() {
            if in_paragraph && !paragraph.is_empty() {
                return Some(paragraph.trim().to_string());
            }
            in_paragraph = false;
            paragraph.clear();
            continue;
        }

        // Skip code blocks, lists, blockquotes for summary
        if trimmed.starts_with("```")
            || trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('>')
            || trimmed.starts_with('|')
        {
            continue;
        }

        in_paragraph = true;
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(trimmed);
    }

    if !paragraph.is_empty() {
        Some(paragraph.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
id: test-123
title: Test Entry
tags: [rust, testing]
applicability: [cross-platform, rust]
---

# Content Here

This is the body."#;

        let (fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.id, Some("test-123".to_string()));
        assert_eq!(fm.title, Some("Test Entry".to_string()));
        assert_eq!(fm.tags, vec!["rust", "testing"]);
        assert_eq!(fm.applicability, vec!["cross-platform", "rust"]);
        assert!(body.contains("# Content Here"));
    }

    #[test]
    fn test_parse_frontmatter_applicability_string() {
        let content = r#"---
title: Test
applicability: rust
---
Body"#;

        let (fm, _) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.applicability, vec!["rust"]);
    }

    #[test]
    fn test_parse_frontmatter_applicability_array() {
        let content = r#"---
title: Test
applicability:
  - rust
  - async
  - cli
---
Body"#;

        let (fm, _) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.applicability, vec!["rust", "async", "cli"]);
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "# Just Content\n\nNo frontmatter here.";
        let (fm, body) = parse_frontmatter(content).unwrap();
        assert!(fm.id.is_none());
        assert!(body.contains("# Just Content"));
    }

    #[test]
    fn test_extract_heading() {
        assert_eq!(
            extract_first_heading("# Hello World"),
            Some("Hello World".to_string())
        );
        assert_eq!(
            extract_first_heading("## Subheading"),
            Some("Subheading".to_string())
        );
    }

    #[test]
    fn test_generate_id() {
        let id = KnowledgeEntry::generate_id("pattern/test.md", "Test Pattern");
        assert!(id.starts_with("kn-"));
        assert_eq!(id.len(), 11); // "kn-" + 8 hex chars
    }
}
