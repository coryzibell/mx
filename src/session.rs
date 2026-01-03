use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
enum SessionLine {
    Human { message: HumanMessage },
    Assistant { message: AssistantMessage },
}

#[derive(Debug, Deserialize)]
struct HumanMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    ToolUse { name: String, id: String },
}

pub fn export_session(path: Option<String>, output: Option<String>) -> Result<()> {
    // Determine session file
    let session_path = if let Some(p) = path {
        PathBuf::from(p)
    } else {
        find_most_recent_session()?
    };

    if !session_path.exists() {
        anyhow::bail!("Session file not found: {:?}", session_path);
    }

    // Parse JSONL
    let content = fs::read_to_string(&session_path)
        .with_context(|| format!("Failed to read session file: {:?}", session_path))?;

    let mut messages: Vec<(String, String)> = Vec::new(); // (role, content)
    let mut tool_counts: HashMap<String, usize> = HashMap::new();

    for (line_num, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<SessionLine>(line) {
            Ok(SessionLine::Human { message }) => {
                // Filter out system reminders
                let clean_content = filter_system_reminders(&message.content);
                if !clean_content.is_empty() {
                    messages.push(("User".to_string(), clean_content));
                }
            }
            Ok(SessionLine::Assistant { message }) => {
                let mut text_parts = Vec::new();
                let mut tool_uses = Vec::new();

                for block in message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            text_parts.push(text);
                        }
                        ContentBlock::ToolUse { name, .. } => {
                            *tool_counts.entry(name.clone()).or_insert(0) += 1;
                            tool_uses.push(name);
                        }
                    }
                }

                let mut full_content = text_parts.join("\n");

                // Append tool usage indicators
                for tool in tool_uses {
                    full_content.push_str(&format!("\n\n[Tool: {}]", tool));
                }

                if !full_content.trim().is_empty() {
                    messages.push(("Assistant".to_string(), full_content));
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse line {}: {}", line_num + 1, e);
                continue;
            }
        }
    }

    // Generate markdown
    let mut markdown = String::new();
    markdown.push_str("# Session Export\n");
    markdown.push_str(&format!(
        "*Exported: {}*\n",
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    ));
    markdown.push_str(&format!("*Source: {}*\n\n", session_path.display()));

    // Tool usage summary
    if !tool_counts.is_empty() {
        markdown.push_str("## Tool Usage Summary\n");
        let mut tools: Vec<_> = tool_counts.iter().collect();
        tools.sort_by_key(|(name, _)| *name);
        for (name, count) in tools {
            markdown.push_str(&format!("- {}: {}\n", name, count));
        }
        markdown.push_str("\n---\n\n");
    }

    // Conversation
    markdown.push_str("## Conversation\n\n");
    for (role, content) in messages {
        markdown.push_str(&format!("### {}\n", role));
        markdown.push_str(&content);
        markdown.push_str("\n\n");
    }

    // Output
    if let Some(output_path) = output {
        fs::write(&output_path, markdown)
            .with_context(|| format!("Failed to write output: {}", output_path))?;
        println!("Exported to: {}", output_path);
    } else {
        print!("{}", markdown);
    }

    Ok(())
}

pub fn find_most_recent_session() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let projects_dir = home.join(".claude").join("projects");

    if !projects_dir.exists() {
        anyhow::bail!("Claude projects directory not found: {:?}", projects_dir);
    }

    let mut sessions = Vec::new();

    // Scan all project directories
    for entry in fs::read_dir(&projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        // Scan for .jsonl files
        for file_entry in fs::read_dir(&path)? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();

            // Skip if not .jsonl
            if file_path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }

            // Skip agent sessions
            if let Some(name) = file_path.file_name().and_then(|n| n.to_str())
                && name.starts_with("agent-")
            {
                continue;
            }

            // Get modification time
            if let Ok(metadata) = file_entry.metadata()
                && let Ok(modified) = metadata.modified()
            {
                sessions.push((file_path, modified));
            }
        }
    }

    if sessions.is_empty() {
        anyhow::bail!("No non-agent session files found in {:?}", projects_dir);
    }

    // Sort by modification time (most recent first)
    sessions.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(sessions[0].0.clone())
}

fn filter_system_reminders(content: &str) -> String {
    let mut result = String::new();
    let mut inside_reminder = false;

    for line in content.lines() {
        if line.trim() == "<system-reminder>" {
            inside_reminder = true;
            continue;
        }

        if line.trim() == "</system-reminder>" {
            inside_reminder = false;
            continue;
        }

        if !inside_reminder {
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}
