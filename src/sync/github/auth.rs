//! GitHub authentication - reads token from ~/.claude.json
//!
//! Token location: projects.<path>.mcpServers.github.env.GITHUB_PERSONAL_ACCESS_TOKEN

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Claude configuration file structure (partial)
#[derive(Debug, Deserialize)]
struct ClaudeConfig {
    #[serde(default)]
    projects: HashMap<String, ProjectConfig>,
}

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServer>,
}

#[derive(Debug, Deserialize)]
struct McpServer {
    #[serde(default)]
    env: HashMap<String, String>,
}

/// Path to Claude configuration file
fn claude_config_path() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".claude.json")
}

/// Read GitHub token from ~/.claude.json
///
/// Searches through all projects for a GitHub MCP server configuration
/// and extracts the GITHUB_PERSONAL_ACCESS_TOKEN.
///
/// # Errors
///
/// Returns an error if:
/// - ~/.claude.json doesn't exist
/// - File cannot be parsed as JSON
/// - No GitHub token is found in any project
pub fn get_github_token() -> Result<String> {
    let config_path = claude_config_path();

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let config: ClaudeConfig = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;

    // Search through all projects for a GitHub token
    for project_config in config.projects.values() {
        if let Some(github_server) = project_config.mcp_servers.get("github")
            && let Some(token) = github_server.env.get("GITHUB_PERSONAL_ACCESS_TOKEN")
            && !token.is_empty()
        {
            return Ok(token.clone());
        }
    }

    anyhow::bail!(
        "GitHub token not found in {}\n\
         Expected path: projects.<project>.mcpServers.github.env.GITHUB_PERSONAL_ACCESS_TOKEN",
        config_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path() {
        let path = claude_config_path();
        assert!(path.ends_with(".claude.json"));
    }

    // Note: Integration test for get_github_token() requires actual ~/.claude.json
    // Run manually: cargo test -- --ignored
    #[test]
    #[ignore]
    fn test_get_token_from_config() {
        let token = get_github_token().expect("Should find token");
        assert!(!token.is_empty());
        // Tokens typically start with ghp_ or gho_
        assert!(
            token.starts_with("ghp_")
                || token.starts_with("gho_")
                || token.starts_with("github_pat_")
        );
    }
}
