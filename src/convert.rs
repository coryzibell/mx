//! Conversion utilities for transforming between formats

use anyhow::{Context, Result};
use chrono::DateTime;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use crate::sync::yaml::schema::{SyncYaml, slugify};

/// Parse markdown file to YAML structure
pub fn parse_markdown(content: &str) -> Result<SyncYaml> {
    // Check for YAML frontmatter
    if content.starts_with("---\n") || content.starts_with("---\r\n") {
        return parse_markdown_with_frontmatter(content);
    }

    let lines: Vec<&str> = content.lines().collect();

    // Extract title (first # heading)
    let title = lines
        .iter()
        .find(|line| line.trim().starts_with("# "))
        .map(|line| line.trim_start_matches("# ").trim())
        .unwrap_or("Untitled");

    // Extract Type and Labels using regex - compile once
    let type_re = Regex::new(r#"\*\*Type:\*\*\s*`([^`]+)`"#)?;
    let labels_re = Regex::new(r#"\*\*Labels:\*\*\s*(.+)"#)?;
    let label_re = Regex::new(r"`([^`]+)`")?;

    let mut item_type = "issue".to_string();
    let mut labels = Vec::new();

    for line in &lines {
        if let Some(caps) = type_re.captures(line) {
            item_type = caps[1].to_string();
        }

        if let Some(caps) = labels_re.captures(line) {
            // Parse labels: `label1`, `label2`
            let label_text = &caps[1];
            for cap in label_re.captures_iter(label_text) {
                labels.push(cap[1].to_string());
            }
        }
    }

    // Extract body - everything after metadata lines
    let body_start = find_body_start(&lines);
    let body = if body_start < lines.len() {
        lines[body_start..].join("\n").trim().to_string()
    } else {
        String::new()
    };

    let mut yaml = SyncYaml::default();
    yaml.metadata.title = Some(title.to_string());
    yaml.metadata.r#type = Some(item_type);
    yaml.metadata.labels = labels;
    yaml.body_markdown = body;

    Ok(yaml)
}

/// Parse markdown with YAML frontmatter
fn parse_markdown_with_frontmatter(content: &str) -> Result<SyncYaml> {
    // Find end of frontmatter
    let rest = &content[4..]; // Skip opening "---\n"
    let end_pos = rest
        .find("\n---")
        .with_context(|| "Invalid frontmatter: missing closing ---")?;

    let frontmatter = &rest[..end_pos];
    let body_start = end_pos + 5; // Skip "\n---\n"
    let body = if body_start < rest.len() {
        rest[body_start..].trim().to_string()
    } else {
        String::new()
    };

    // Parse frontmatter as YAML
    let fm: serde_yaml::Value =
        serde_yaml::from_str(frontmatter).with_context(|| "Failed to parse frontmatter")?;

    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled")
        .to_string();

    let item_type = fm
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("issue")
        .to_string();

    let labels: Vec<String> = fm
        .get("labels")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let mut yaml = SyncYaml::default();
    yaml.metadata.title = Some(title);
    yaml.metadata.r#type = Some(item_type);
    yaml.metadata.labels = labels;
    yaml.body_markdown = body;

    // Also grab priority if present
    if let Some(priority) = fm.get("priority").and_then(|v| v.as_str()) {
        yaml.metadata.labels.push(format!("priority:{}", priority));
    }

    Ok(yaml)
}

/// Find where body content starts (after title and metadata)
fn find_body_start(lines: &[&str]) -> usize {
    let mut idx = 0;

    // Skip title
    while idx < lines.len() {
        if lines[idx].trim().starts_with("# ") {
            idx += 1;
            break;
        }
        idx += 1;
    }

    // Skip blank lines and metadata lines
    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() || line.starts_with("**Type:**") || line.starts_with("**Labels:**") {
            idx += 1;
        } else {
            break;
        }
    }

    idx
}

/// Convert markdown file to YAML
pub fn convert_file(input: &Path, output_dir: &Path, dry_run: bool) -> Result<PathBuf> {
    let content =
        fs::read_to_string(input).with_context(|| format!("Failed to read file: {:?}", input))?;

    let yaml_data = parse_markdown(&content)?;

    // Preserve input filename, just change extension
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");
    let filename = format!("{}.yaml", stem);
    let output_path = output_dir.join(&filename);

    if dry_run {
        println!("Would create: {:?}", output_path);
        if let Some(title) = &yaml_data.metadata.title {
            println!("  Title: {}", title);
        }
        if let Some(item_type) = &yaml_data.metadata.r#type {
            println!("  Type: {}", item_type);
        }
        if !yaml_data.metadata.labels.is_empty() {
            println!("  Labels: {}", yaml_data.metadata.labels.join(", "));
        }
    } else {
        fs::create_dir_all(output_dir)
            .with_context(|| format!("Failed to create directory: {:?}", output_dir))?;

        let yaml_str = serde_yaml::to_string(&yaml_data)?;
        fs::write(&output_path, yaml_str)
            .with_context(|| format!("Failed to write file: {:?}", output_path))?;

        println!("Created: {:?}", output_path);
    }

    Ok(output_path)
}

/// Convert directory of markdown files
pub fn convert_directory(input_dir: &Path, output_dir: &Path, dry_run: bool) -> Result<()> {
    let entries = fs::read_dir(input_dir)
        .with_context(|| format!("Failed to read directory: {:?}", input_dir))?;

    let mut count = 0;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            convert_file(&path, output_dir, dry_run)?;
            count += 1;
        }
    }

    if dry_run {
        println!("\nWould convert {} files", count);
    } else {
        println!("\nConverted {} files", count);
    }

    Ok(())
}

/// Convert YAML file to markdown
pub fn yaml_to_markdown_file(
    input: &Path,
    output_dir: &Path,
    repo: Option<&str>,
    dry_run: bool,
) -> Result<PathBuf> {
    let content =
        fs::read_to_string(input).with_context(|| format!("Failed to read file: {:?}", input))?;

    let yaml_data: SyncYaml = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse YAML: {:?}", input))?;

    // Infer repo from parent directory name if not provided
    let repo_string = match repo {
        Some(r) => r.to_string(),
        None => infer_repo_from_path(input)?,
    };

    let markdown = generate_markdown(&yaml_data, &repo_string)?;

    // Generate output filename
    let filename = generate_markdown_filename(&yaml_data, input)?;
    let output_path = output_dir.join(&filename);

    if dry_run {
        println!("Would create: {:?}", output_path);
        println!("  Title: {}", yaml_data.title());
        if let Some(num) = yaml_data.github_issue_number() {
            println!("  Issue #: {}", num);
        } else if let Some(id) = yaml_data.github_discussion_id() {
            println!("  Discussion ID: {}", id);
        }
    } else {
        fs::create_dir_all(output_dir)
            .with_context(|| format!("Failed to create directory: {:?}", output_dir))?;

        fs::write(&output_path, markdown)
            .with_context(|| format!("Failed to write file: {:?}", output_path))?;

        println!("Created: {:?}", output_path);
    }

    Ok(output_path)
}

/// Convert directory of YAML files to markdown
pub fn yaml_to_markdown_directory(
    input_dir: &Path,
    output_dir: &Path,
    repo: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let entries = fs::read_dir(input_dir)
        .with_context(|| format!("Failed to read directory: {:?}", input_dir))?;

    let mut count = 0;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            yaml_to_markdown_file(&path, output_dir, repo, dry_run)?;
            count += 1;
        }
    }

    if dry_run {
        println!("\nWould convert {} files", count);
    } else {
        println!("\nConverted {} files", count);
    }

    Ok(())
}

/// Generate markdown content from YAML (with frontmatter for clean roundtrips)
fn generate_markdown(yaml: &SyncYaml, repo: &str) -> Result<String> {
    let mut md = String::new();

    // YAML frontmatter
    md.push_str("---\n");
    md.push_str(&format!(
        "title: \"{}\"\n",
        yaml.title().replace('"', "\\\"")
    ));

    if let Some(item_type) = yaml.r#type.as_deref().or(yaml.metadata.r#type.as_deref()) {
        md.push_str(&format!("type: {}\n", item_type));
    }

    let labels = yaml.labels();
    if !labels.is_empty() {
        md.push_str("labels:\n");
        for label in labels {
            md.push_str(&format!("  - {}\n", label));
        }
    }

    if let Some(state) = &yaml.metadata.state {
        md.push_str(&format!("state: {}\n", state));
    }

    // GitHub metadata
    if let Some(number) = yaml.github_issue_number() {
        md.push_str(&format!("github_issue: {}\n", number));
        md.push_str(&format!("github_repo: {}\n", repo));
    } else if let Some(disc_num) = yaml.metadata.github_discussion_number {
        md.push_str(&format!("github_discussion: {}\n", disc_num));
        md.push_str(&format!("github_repo: {}\n", repo));
    }

    if let Some(updated_at) = &yaml.metadata.github_updated_at {
        md.push_str(&format!("updated_at: {}\n", updated_at));
    }

    md.push_str("---\n\n");

    // Body
    let body = yaml.body();
    if !body.is_empty() {
        md.push_str(body);
        md.push_str("\n\n");
    }

    // Comments
    if !yaml.comments.is_empty() {
        md.push_str("---\n\n## Comments\n\n");

        for comment in &yaml.comments {
            if let Ok(date_str) = format_date_short(&comment.created_at) {
                md.push_str(&format!("### {} ({})\n", comment.author, date_str));
            } else {
                md.push_str(&format!("### {}\n", comment.author));
            }
            md.push_str(&format!("{}\n\n", comment.body));
        }
    }

    Ok(md)
}

/// Generate markdown filename from YAML data
fn generate_markdown_filename(yaml: &SyncYaml, original_path: &Path) -> Result<String> {
    if let Some(number) = yaml.github_issue_number() {
        let slug = slugify(yaml.title(), 50);
        Ok(format!("{}-{}.md", number, slug))
    } else if let Some(_disc_id) = yaml.github_discussion_id() {
        let slug = slugify(yaml.title(), 50);
        if let Some(num) = yaml.metadata.github_discussion_number {
            Ok(format!("disc-{}-{}.md", num, slug))
        } else {
            Ok(format!("disc-{}.md", slug))
        }
    } else {
        // Use original filename base
        let stem = original_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed");
        Ok(format!("{}.md", stem))
    }
}

/// Infer repository from directory name (e.g., "coryzibell-mx" -> "coryzibell/mx")
fn infer_repo_from_path(path: &Path) -> Result<String> {
    let parent = path
        .parent()
        .with_context(|| "Cannot infer repo: no parent directory")?;

    let dir_name = parent
        .file_name()
        .and_then(|s| s.to_str())
        .with_context(|| "Cannot infer repo: invalid directory name")?;

    // Try to parse "owner-repo" format
    if let Some(pos) = dir_name.find('-') {
        let owner = &dir_name[..pos];
        let repo = &dir_name[pos + 1..];
        Ok(format!("{}/{}", owner, repo))
    } else {
        Ok(format!("unknown/{}", dir_name))
    }
}

/// Format ISO8601 date to human-readable format
fn format_date(iso_date: &str) -> Result<String> {
    let dt = DateTime::parse_from_rfc3339(iso_date)
        .with_context(|| format!("Invalid date format: {}", iso_date))?;
    Ok(dt.format("%B %d, %Y").to_string())
}

/// Format ISO8601 date to short human-readable format
fn format_date_short(iso_date: &str) -> Result<String> {
    let dt = DateTime::parse_from_rfc3339(iso_date)
        .with_context(|| format!("Invalid date format: {}", iso_date))?;
    Ok(dt.format("%b %d, %Y").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_markdown() {
        let content = r#"# Issue Title Here

**Type:** `issue`
**Labels:** `enhancement`, `identity:smith`

## Context

Some context here...

## Problem

Description of the problem...
"#;

        let yaml = parse_markdown(content).unwrap();
        assert_eq!(yaml.metadata.title.as_deref(), Some("Issue Title Here"));
        assert_eq!(yaml.metadata.r#type.as_deref(), Some("issue"));
        assert_eq!(yaml.metadata.labels, vec!["enhancement", "identity:smith"]);
        assert!(yaml.body_markdown.contains("## Context"));
        assert!(yaml.body_markdown.contains("## Problem"));
    }

    #[test]
    fn test_find_body_start() {
        let lines = vec![
            "# Title",
            "",
            "**Type:** `issue`",
            "**Labels:** `bug`",
            "",
            "## Body",
            "Content",
        ];

        let start = find_body_start(&lines);
        assert_eq!(lines[start], "## Body");
    }

    #[test]
    fn test_parse_markdown_with_frontmatter() {
        let content = r#"---
title: "chore(deps): update zero-code dependencies"
type: issue
labels:
  - dependencies
  - identity:smith
priority: P2
---

## Context

Some context here...

## Problem

Description of the problem...
"#;

        let yaml = parse_markdown(content).unwrap();
        assert_eq!(
            yaml.metadata.title.as_deref(),
            Some("chore(deps): update zero-code dependencies")
        );
        assert_eq!(yaml.metadata.r#type.as_deref(), Some("issue"));
        assert!(yaml.metadata.labels.contains(&"dependencies".to_string()));
        assert!(yaml.metadata.labels.contains(&"identity:smith".to_string()));
        assert!(yaml.metadata.labels.contains(&"priority:P2".to_string()));
        assert!(yaml.body_markdown.contains("## Context"));
        assert!(yaml.body_markdown.contains("## Problem"));
    }
}
