//! YAML schema definitions for issues and discussions
//!
//! Supports both pull (GitHub → YAML) and push (YAML → GitHub) workflows.
//! Handles field locations at both root level and metadata.* for compatibility.

use serde::{Deserialize, Serialize};

/// Root YAML structure for an issue or discussion
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncYaml {
    #[serde(default)]
    pub metadata: Metadata,

    /// Main content body
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,

    /// Comments on the issue/discussion
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,

    // Root-level fields (alternative locations, for authored YAML)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

impl SyncYaml {
    /// Get title, preferring root level, falling back to metadata
    pub fn title(&self) -> &str {
        self.title
            .as_deref()
            .or(self.metadata.title.as_deref())
            .unwrap_or("Untitled")
    }

    /// Get body content, preferring root level, falling back to body_markdown
    pub fn body(&self) -> &str {
        self.body
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.body_markdown)
    }

    /// Get item type (issue or idea), normalizing "discussion" to "idea"
    pub fn item_type(&self) -> ItemType {
        let type_str = self
            .r#type
            .as_deref()
            .or(self.metadata.r#type.as_deref())
            .unwrap_or("issue");

        match type_str {
            "idea" | "discussion" => ItemType::Idea,
            _ => ItemType::Issue,
        }
    }

    /// Get labels, preferring root level
    pub fn labels(&self) -> &[String] {
        self.labels
            .as_deref()
            .unwrap_or_else(|| self.metadata.labels.as_slice())
    }

    /// Get assignees, preferring root level
    pub fn assignees(&self) -> &[String] {
        self.assignees
            .as_deref()
            .unwrap_or_else(|| self.metadata.assignees.as_slice())
    }

    /// Get discussion category
    pub fn category(&self) -> Option<&str> {
        self.category
            .as_deref()
            .or(self.metadata.category.as_deref())
    }

    /// Check if this has a GitHub issue number
    pub fn github_issue_number(&self) -> Option<u64> {
        self.metadata.github_issue_number
    }

    /// Check if this has a GitHub discussion ID
    pub fn github_discussion_id(&self) -> Option<&str> {
        self.metadata.github_discussion_id.as_deref()
    }

    /// Get the last synced snapshot
    pub fn last_synced(&self) -> Option<&LastSynced> {
        self.metadata.last_synced.as_ref()
    }
}

/// Item type for routing to Issues vs Discussions API
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Issue,
    Idea, // Maps to GitHub Discussion
}

/// Metadata block within YAML
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    #[serde(default)]
    pub labels: Vec<String>,

    #[serde(default)]
    pub assignees: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    // GitHub tracking IDs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_issue_number: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_discussion_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_discussion_number: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_updated_at: Option<String>,

    /// Snapshot of last synced state for three-way merge
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<LastSynced>,
}

/// Snapshot of synced state for three-way merge detection
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LastSynced {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub updated_at: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
}

impl LastSynced {
    /// Create a new snapshot
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        labels: Vec<String>,
        updated_at: impl Into<String>,
        assignees: Option<Vec<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            labels,
            updated_at: updated_at.into(),
            assignees,
        }
    }
}

/// Comment on an issue or discussion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub created_at: String,
    pub body: String,
}

/// GitHub Issue (from REST API)
#[derive(Debug, Clone)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub state: String,
    pub updated_at: String,
}

/// GitHub Discussion (from GraphQL API)
#[derive(Debug, Clone)]
pub struct GitHubDiscussion {
    pub id: String, // GraphQL node ID
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub category: String,
    pub updated_at: String,
}

/// GitHub Label
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub description: String,
}

/// Generate a slug from text for filenames
pub fn slugify(text: &str, max_len: usize) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(max_len)
        .collect()
}

/// Generate YAML filename from issue/discussion number and title
pub fn yaml_filename(number: u64, title: &str) -> String {
    let slug = slugify(title, 50);
    format!("{}-{}.yaml", number, slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World", 50), "hello-world");
        assert_eq!(slugify("Fix: Bug #123", 50), "fix-bug-123");
        assert_eq!(slugify("  Multiple   Spaces  ", 50), "multiple-spaces");
        assert_eq!(slugify("Very Long Title That Exceeds", 10), "very-long-");
    }

    #[test]
    fn test_yaml_filename() {
        assert_eq!(
            yaml_filename(11, "Port sync to pure Rust: GitHub Auth module"),
            "11-port-sync-to-pure-rust-github-auth-module.yaml"
        );
    }

    #[test]
    fn test_item_type_parsing() {
        let mut yaml = SyncYaml::default();
        assert_eq!(yaml.item_type(), ItemType::Issue);

        yaml.r#type = Some("idea".to_string());
        assert_eq!(yaml.item_type(), ItemType::Idea);

        yaml.r#type = Some("discussion".to_string());
        assert_eq!(yaml.item_type(), ItemType::Idea);

        yaml.r#type = Some("issue".to_string());
        assert_eq!(yaml.item_type(), ItemType::Issue);
    }

    #[test]
    fn test_field_resolution() {
        let yaml = SyncYaml {
            title: Some("Root Title".to_string()),
            metadata: Metadata {
                title: Some("Metadata Title".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        // Root level takes precedence
        assert_eq!(yaml.title(), "Root Title");

        // Fallback to metadata
        let yaml2 = SyncYaml {
            metadata: Metadata {
                title: Some("Metadata Title".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(yaml2.title(), "Metadata Title");
    }

    #[test]
    fn test_deserialize_yaml() {
        let yaml_str = r#"
metadata:
  title: Test Issue
  type: issue
  labels:
    - bug
    - enhancement
  assignees: []
  state: open
  github_issue_number: 42
body_markdown: |
  This is the body.
comments: []
"#;

        let parsed: SyncYaml = serde_yaml::from_str(yaml_str).unwrap();
        assert_eq!(parsed.title(), "Test Issue");
        assert_eq!(parsed.item_type(), ItemType::Issue);
        assert_eq!(parsed.labels(), &["bug", "enhancement"]);
        assert_eq!(parsed.github_issue_number(), Some(42));
        assert!(parsed.body().contains("This is the body"));
    }
}
