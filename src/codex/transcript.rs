use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use super::{ArchiveEntry, Manifest};

static SYSTEM_REMINDER_RE: OnceLock<Regex> = OnceLock::new();
static USER_NAME: OnceLock<String> = OnceLock::new();
static ASSISTANT_NAME: OnceLock<String> = OnceLock::new();

/// Pure resolution logic for user display name. Takes the env var value as a
/// parameter so callers (especially tests) don't need to touch process state.
fn resolve_user_name_with(env_val: Option<&str>) -> String {
    if let Some(name) = env_val
        && !name.is_empty()
    {
        return name.to_string();
    }
    // Fallback: try git config user.name
    if let Ok(output) = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        && output.status.success()
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "User".to_string()
}

/// Resolve the user display name (uncached). Used as the initializer for the
/// OnceLock cache. Reads MX_USER_NAME from the environment and delegates to
/// the pure `resolve_user_name_with`.
fn resolve_user_name_inner() -> String {
    resolve_user_name_with(std::env::var("MX_USER_NAME").ok().as_deref())
}

/// Resolve the user display name for transcripts.
/// Priority: MX_USER_NAME env var > git config user.name > "User"
/// Result is cached for the lifetime of the process via OnceLock.
pub(super) fn resolve_user_name() -> String {
    USER_NAME.get_or_init(resolve_user_name_inner).clone()
}

/// Pure resolution logic for assistant display name. Takes the env var value
/// as a parameter so callers (especially tests) don't need to touch process
/// state.
fn resolve_assistant_name_with(env_val: Option<&str>) -> String {
    if let Some(name) = env_val
        && !name.is_empty()
    {
        return name.to_string();
    }
    "Orchestrator".to_string()
}

/// Resolve the assistant display name (uncached). Used as the initializer for
/// the OnceLock cache. Reads MX_ASSISTANT_NAME from the environment and
/// delegates to the pure `resolve_assistant_name_with`.
fn resolve_assistant_name_inner() -> String {
    resolve_assistant_name_with(std::env::var("MX_ASSISTANT_NAME").ok().as_deref())
}

/// Resolve the assistant display name for transcripts.
/// Priority: MX_ASSISTANT_NAME env var > "Orchestrator"
/// Result is cached for the lifetime of the process via OnceLock.
pub(super) fn resolve_assistant_name() -> String {
    ASSISTANT_NAME
        .get_or_init(resolve_assistant_name_inner)
        .clone()
}

fn system_reminder_re() -> &'static Regex {
    SYSTEM_REMINDER_RE
        .get_or_init(|| Regex::new(r"(?s)<system-reminder>.*?</system-reminder>").unwrap())
}

/// Strip <system-reminder>...</system-reminder> blocks from a string
fn strip_system_reminders(content: &str) -> String {
    system_reminder_re().replace_all(content, "").to_string()
}

/// Generate a clean markdown transcript from JSONL session content
pub(super) fn generate_clean_transcript(
    session_content: &str,
    user_name: &str,
    assistant_name: &str,
) -> Result<String> {
    let mut output = String::new();
    let user_prefix = format!("**{}:**", user_name);
    let assistant_prefix = format!("**{}:**", assistant_name);

    for line in session_content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines
        };

        let msg_type = match msg["type"].as_str() {
            Some(t) => t,
            None => continue,
        };

        match msg_type {
            "user" => {
                let content = &msg["message"]["content"];
                if let Some(text) = content.as_str() {
                    // String content: strip system reminders, skip if empty
                    let stripped = strip_system_reminders(text);
                    let trimmed = stripped.trim();
                    if !trimmed.is_empty() {
                        output.push_str(&format!("{} {}\n\n", user_prefix, trimmed));
                    }
                }
                // Array content (tool results): skip
            }
            "assistant" => {
                if let Some(blocks) = msg["message"]["content"].as_array() {
                    let mut text_parts = Vec::new();
                    for block in blocks {
                        if block["type"].as_str() == Some("text")
                            && let Some(text) = block["text"].as_str()
                        {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                text_parts.push(trimmed.to_string());
                            }
                        }
                    }
                    let joined = text_parts.join("\n\n");
                    if !joined.is_empty() {
                        output.push_str(&format!("{} {}\n\n", assistant_prefix, joined));
                    }
                }
            }
            _ => {} // skip summary, tool results, etc.
        }
    }

    Ok(output)
}

/// Generate a clean transcript from JSONL, including agent sub-session transcripts.
/// Each agent's transcript is appended with a separator and heading.
pub(super) fn generate_clean_transcript_with_agents(
    session_content: &str,
    agent_sessions: &[(String, String)], // (agent_name, jsonl_content)
    user_name: &str,
    assistant_name: &str,
) -> Result<String> {
    let mut output = generate_clean_transcript(session_content, user_name, assistant_name)?;

    for (agent_name, agent_content) in agent_sessions {
        let agent_transcript = generate_clean_transcript(agent_content, user_name, assistant_name)?;
        if !agent_transcript.is_empty() {
            output.push_str(&format!(
                "\n---\n\n## Agent: {}\n\n{}",
                agent_name, agent_transcript
            ));
        }
    }

    Ok(output)
}

/// Build a mapping from agentId -> subagent_type by scanning the parent session JSONL
/// for Agent tool_use calls that contain a "subagent_type" field in their input.
pub(super) fn build_agent_type_map(session_content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in session_content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Look for assistant messages with tool_use blocks named "Agent"
        if msg["type"].as_str() != Some("assistant") {
            continue;
        }
        if let Some(blocks) = msg["message"]["content"].as_array() {
            for block in blocks {
                if block["type"].as_str() == Some("tool_use")
                    && block["name"].as_str() == Some("Agent")
                    && let Some(input) = block["input"].as_object()
                    && let Some(subagent_type) = input.get("subagent_type").and_then(|v| v.as_str())
                {
                    // The agentId may appear in the tool result as the tool_use id,
                    // but more commonly is extracted from the agent filename.
                    // The id field of the tool_use block is the tool_use_id.
                    if let Some(tool_use_id) = block["id"].as_str() {
                        map.insert(tool_use_id.to_string(), subagent_type.to_string());
                    }
                }
            }
        }
    }
    map
}

/// Resolve an agent name using the agent type map if possible, falling back to the hex ID.
/// Matches by checking if the tool_use_id ends with the hex_id extracted from the agent
/// filename (e.g., tool_use_id "toolu_abc123" ends with hex_id "abc123"). When multiple
/// entries match, the longest matching tool_use_id wins to avoid false positives from
/// short hex IDs.
pub(super) fn resolve_agent_display_name(
    path: &Path,
    agent_type_map: &HashMap<String, String>,
) -> String {
    let hex_id = agent_name_from_path(path);
    if hex_id.is_empty() {
        return hex_id;
    }
    // Find the best match: tool_use_id that ends with the hex_id.
    // Pick the longest tool_use_id among matches to avoid ambiguity with short hex IDs.
    let best_match = agent_type_map
        .iter()
        .filter(|(tool_use_id, _)| tool_use_id.ends_with(&hex_id))
        .max_by_key(|(tool_use_id, _)| tool_use_id.len());
    if let Some((_, subagent_type)) = best_match {
        return subagent_type.clone();
    }
    hex_id
}

/// Extract an agent name from its filename (e.g. "agent-abc12345.jsonl" -> "abc12345")
pub(super) fn agent_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .strip_prefix("agent-")
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
        })
        .to_string()
}

/// Generate clean transcripts for archives that have session.jsonl but no conversation.md
pub(super) fn migrate_clean_transcripts(
    codex_dir: &Path,
    archives: Vec<ArchiveEntry>,
    dry_run: bool,
    verbose: bool,
    include_agents: bool,
) -> Result<()> {
    let mut needs_transcript = Vec::new();

    for archive in archives {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");
        let transcript_file = archive_dir.join("conversation.md");

        if transcript_file.exists() {
            // Already has a clean transcript — skip
            if verbose {
                println!(
                    "  Skipping {} (already has conversation.md)",
                    archive.short_id
                );
            }
            continue;
        }

        if !session_file.exists() {
            // Clean-only archive or missing JSONL — can't generate
            if verbose {
                println!(
                    "  Skipping {} (no session.jsonl to generate from)",
                    archive.short_id
                );
            }
            continue;
        }

        needs_transcript.push(archive);
    }

    if needs_transcript.is_empty() {
        println!("All archives already have clean transcripts (or have no session.jsonl).");
        return Ok(());
    }

    println!(
        "Found {} archive(s) needing clean transcript",
        needs_transcript.len()
    );

    if dry_run {
        println!("\n[DRY RUN MODE - No changes will be made]\n");
        for archive in &needs_transcript {
            println!("  Would generate conversation.md for {}", archive.short_id);
        }
        return Ok(());
    }

    let mut generated = 0;

    for archive in &needs_transcript {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");
        let transcript_file = archive_dir.join("conversation.md");
        let manifest_path = archive_dir.join("manifest.json");

        let session_content = fs::read_to_string(&session_file)?;
        let transcript = if include_agents {
            let agents_dir = archive_dir.join("agents");
            let mut agent_sessions = Vec::new();
            let agent_type_map = build_agent_type_map(&session_content);
            if agents_dir.exists() {
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        let agent_name = resolve_agent_display_name(&path, &agent_type_map);
                        let agent_content = fs::read_to_string(&path)?;
                        agent_sessions.push((agent_name, agent_content));
                    }
                }
                agent_sessions.sort_by(|a, b| a.0.cmp(&b.0));
            }
            let user_name = resolve_user_name();
            let assistant_name = resolve_assistant_name();
            generate_clean_transcript_with_agents(
                &session_content,
                &agent_sessions,
                &user_name,
                &assistant_name,
            )?
        } else {
            let user_name = resolve_user_name();
            let assistant_name = resolve_assistant_name();
            generate_clean_transcript(&session_content, &user_name, &assistant_name)?
        };

        fs::write(&transcript_file, &transcript)?;

        // Update manifest to record has_clean_transcript
        if manifest_path.exists() {
            let manifest_content = fs::read_to_string(&manifest_path)?;
            if let Ok(mut manifest) = serde_json::from_str::<Manifest>(&manifest_content) {
                manifest.has_clean_transcript = Some(true);
                manifest.user_name = Some(resolve_user_name());
                manifest.assistant_name = Some(resolve_assistant_name());
                let updated = serde_json::to_string_pretty(&manifest)?;
                fs::write(&manifest_path, updated)?;
            }
        }

        if verbose {
            println!("  Generated conversation.md for {}", archive.short_id);
        }

        generated += 1;
    }

    println!("\n--- Migration Summary ---");
    println!("Clean transcripts generated: {}", generated);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---------------------------------------------------------------------------
    // strip_system_reminders
    // ---------------------------------------------------------------------------

    #[test]
    fn strip_single_reminder() {
        let input = "before<system-reminder>secret</system-reminder>after";
        assert_eq!(strip_system_reminders(input), "beforeafter");
    }

    #[test]
    fn strip_multiple_reminders() {
        let input =
            "a<system-reminder>one</system-reminder>b<system-reminder>two</system-reminder>c";
        assert_eq!(strip_system_reminders(input), "abc");
    }

    #[test]
    fn strip_no_reminders_unchanged() {
        let input = "just plain text with no reminders";
        assert_eq!(strip_system_reminders(input), input);
    }

    #[test]
    fn strip_multiline_reminder() {
        // The regex uses (?s) so . matches newlines
        let input = "start<system-reminder>\nline one\nline two\n</system-reminder>end";
        assert_eq!(strip_system_reminders(input), "startend");
    }

    #[test]
    fn strip_adjacent_reminders() {
        // Two reminders with no gap between them
        let input =
            "<system-reminder>first</system-reminder><system-reminder>second</system-reminder>";
        assert_eq!(strip_system_reminders(input), "");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — helper
    // ---------------------------------------------------------------------------

    /// Build a JSONL line for a user message with string content.
    fn user_str(text: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"content":{}}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    /// Build a JSONL line for a user message with array content (tool results).
    fn user_array() -> &'static str {
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"x","content":"some output"}]}}"#
    }

    /// Build a JSONL line for an assistant message with a single text block.
    fn assistant_text(text: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}}]}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    /// Build a JSONL line for an assistant message with a tool_use block only.
    fn assistant_tool_use() -> &'static str {
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#
    }

    /// Build a JSONL line for an assistant message with both text and tool_use blocks.
    fn assistant_mixed(text: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}},{{"type":"tool_use","id":"t2","name":"Read","input":{{}}}}]}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — user string content
    // ---------------------------------------------------------------------------

    #[test]
    fn user_string_gets_user_prefix() {
        let jsonl = user_str("Hello there");
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**User:** Hello there\n\n");
    }

    #[test]
    fn user_string_content_is_trimmed() {
        let jsonl = user_str("  spaced out  ");
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**User:** spaced out\n\n");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — user array content dropped
    // ---------------------------------------------------------------------------

    #[test]
    fn user_array_content_is_dropped() {
        let result = generate_clean_transcript(user_array(), "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — assistant text blocks
    // ---------------------------------------------------------------------------

    #[test]
    fn assistant_text_gets_assistant_prefix() {
        let jsonl = assistant_text("Here is my answer.");
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**Orchestrator:** Here is my answer.\n\n");
    }

    #[test]
    fn assistant_multiple_text_blocks_joined() {
        // Two text blocks in one assistant message → joined with \n\n, single prefix
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Part one."},{"type":"text","text":"Part two."}]}}"#;
        let result = generate_clean_transcript(jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**Orchestrator:** Part one.\n\nPart two.\n\n");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — assistant tool_use blocks dropped
    // ---------------------------------------------------------------------------

    #[test]
    fn assistant_tool_use_only_is_dropped() {
        let result =
            generate_clean_transcript(assistant_tool_use(), "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn assistant_mixed_keeps_only_text() {
        // text + tool_use block → only text survives, tool_use dropped
        let jsonl = assistant_mixed("Thinking out loud.");
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**Orchestrator:** Thinking out loud.\n\n");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — system reminder stripping
    // ---------------------------------------------------------------------------

    #[test]
    fn system_reminder_stripped_from_user_content() {
        let text = "real question<system-reminder>ignore me</system-reminder>";
        let jsonl = user_str(text);
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**User:** real question\n\n");
    }

    #[test]
    fn user_content_only_reminder_is_dropped() {
        // After stripping the reminder, content is empty → entire message dropped
        let text = "<system-reminder>only this</system-reminder>";
        let jsonl = user_str(text);
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn user_content_whitespace_only_after_strip_is_dropped() {
        let text = "  <system-reminder>noise</system-reminder>  ";
        let jsonl = user_str(text);
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — non-user/non-assistant types dropped
    // ---------------------------------------------------------------------------

    #[test]
    fn file_history_snapshot_type_dropped() {
        let jsonl = r#"{"type":"file-history-snapshot","message":{"content":"snapshot data"}}"#;
        let result = generate_clean_transcript(jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn unknown_type_dropped() {
        let jsonl = r#"{"type":"summary","data":"session summary here"}"#;
        let result = generate_clean_transcript(jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — empty / malformed input
    // ---------------------------------------------------------------------------

    #[test]
    fn empty_input_produces_empty_output() {
        let result = generate_clean_transcript("", "User", "Orchestrator").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn blank_lines_skipped() {
        let jsonl = format!("\n\n{}\n\n", user_str("hi"));
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**User:** hi\n\n");
    }

    #[test]
    fn malformed_jsonl_line_skipped() {
        // A bad line followed by a valid line — bad line is silently skipped
        let jsonl = format!("NOT JSON\n{}", user_str("valid"));
        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();
        assert_eq!(result, "**User:** valid\n\n");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript — mixed realistic session
    // ---------------------------------------------------------------------------

    #[test]
    fn realistic_mixed_session() {
        // A realistic interleaving: user string, user array (tool result), assistant
        // with tool_use only, assistant with text, unknown type, user with reminder.
        let lines = [
            user_str("Can you list the files?"),
            user_array().to_string(),
            assistant_tool_use().to_string(),
            assistant_text("Here are the files: foo.rs, bar.rs."),
            r#"{"type":"file-history-snapshot","content":"snap"}"#.to_string(),
            user_str("Thanks<system-reminder>sys note</system-reminder>, what about tests?"),
            assistant_mixed("I see test coverage is low."),
        ];
        let jsonl = lines.join("\n");

        let result = generate_clean_transcript(&jsonl, "User", "Orchestrator").unwrap();

        // Expected: user string → User, tool result → dropped, assistant tool_use → dropped,
        // assistant text → Orchestrator, snapshot → dropped, user with reminder stripped → User,
        // assistant mixed → Orchestrator (text only)
        let expected = concat!(
            "**User:** Can you list the files?\n\n",
            "**Orchestrator:** Here are the files: foo.rs, bar.rs.\n\n",
            "**User:** Thanks, what about tests?\n\n",
            "**Orchestrator:** I see test coverage is low.\n\n",
        );
        assert_eq!(result, expected);
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript_with_agents
    // ---------------------------------------------------------------------------

    #[test]
    fn agents_empty_list_same_as_plain() {
        let main_session = user_str("Hello");
        let with_agents =
            generate_clean_transcript_with_agents(&main_session, &[], "User", "Orchestrator")
                .unwrap();
        let plain = generate_clean_transcript(&main_session, "User", "Orchestrator").unwrap();
        assert_eq!(with_agents, plain);
    }

    #[test]
    fn agents_single_agent_appended() {
        let main_jsonl = user_str("Main question");
        let agent_jsonl = format!(
            "{}\n{}",
            user_str("Agent task"),
            assistant_text("Agent did it.")
        );

        let result = generate_clean_transcript_with_agents(
            &main_jsonl,
            &[("worker-1".to_string(), agent_jsonl)],
            "User",
            "Orchestrator",
        )
        .unwrap();

        let expected = concat!(
            "**User:** Main question\n\n",
            "\n---\n\n## Agent: worker-1\n\n",
            "**User:** Agent task\n\n",
            "**Orchestrator:** Agent did it.\n\n",
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn agents_multiple_agents_sorted_by_name() {
        let main_jsonl = user_str("Start");
        let agent_b = assistant_text("From agent B.");
        let agent_a = assistant_text("From agent A.");

        // Pass out of order; function receives pre-sorted in production,
        // but generate_clean_transcript_with_agents itself just appends in order
        let result = generate_clean_transcript_with_agents(
            &main_jsonl,
            &[
                ("alpha".to_string(), agent_a),
                ("beta".to_string(), agent_b),
            ],
            "User",
            "Orchestrator",
        )
        .unwrap();

        // Should have agent alpha before beta (passed in that order)
        assert!(result.contains("## Agent: alpha"));
        assert!(result.contains("## Agent: beta"));
        let alpha_pos = result.find("## Agent: alpha").unwrap();
        let beta_pos = result.find("## Agent: beta").unwrap();
        assert!(alpha_pos < beta_pos, "alpha should appear before beta");
    }

    #[test]
    fn agents_empty_agent_content_skipped() {
        let main_jsonl = user_str("Main");
        // Agent JSONL has no user/assistant messages (only tool results)
        let agent_jsonl = user_array().to_string();

        let result = generate_clean_transcript_with_agents(
            &main_jsonl,
            &[("empty-agent".to_string(), agent_jsonl)],
            "User",
            "Orchestrator",
        )
        .unwrap();

        // Should NOT contain agent section since transcript was empty
        assert!(!result.contains("## Agent:"));
        assert_eq!(result, "**User:** Main\n\n");
    }

    #[test]
    fn agents_transcript_has_separator() {
        let main_jsonl = user_str("Question");
        let agent_jsonl = assistant_text("Answer from agent.");

        let result = generate_clean_transcript_with_agents(
            &main_jsonl,
            &[("sub-1".to_string(), agent_jsonl)],
            "User",
            "Orchestrator",
        )
        .unwrap();

        // Verify separator format
        assert!(result.contains("\n---\n\n## Agent: sub-1\n\n"));
    }

    // ---------------------------------------------------------------------------
    // agent_name_from_path
    // ---------------------------------------------------------------------------

    #[test]
    fn agent_name_strips_prefix() {
        let path = PathBuf::from("/some/dir/agent-abc12345.jsonl");
        assert_eq!(agent_name_from_path(&path), "abc12345");
    }

    #[test]
    fn agent_name_no_prefix_returns_stem() {
        let path = PathBuf::from("/some/dir/custom-session.jsonl");
        assert_eq!(agent_name_from_path(&path), "custom-session");
    }

    #[test]
    fn agent_name_agent_prefix_only() {
        // Edge case: filename is exactly "agent-.jsonl"
        let path = PathBuf::from("/some/dir/agent-.jsonl");
        assert_eq!(agent_name_from_path(&path), "");
    }
    // ---------------------------------------------------------------------------
    // resolve_user_name / resolve_assistant_name
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_user_name_from_env() {
        let name = resolve_user_name_with(Some("TestHuman"));
        assert_eq!(name, "TestHuman");
    }

    #[test]
    fn resolve_user_name_fallback_without_env() {
        let name = resolve_user_name_with(None);
        // Should be either git user.name or "User" -- both are valid
        assert!(!name.is_empty());
    }

    #[test]
    fn resolve_user_name_empty_env_is_fallback() {
        let name = resolve_user_name_with(Some(""));
        // Empty string should behave like None -- fall back to git or "User"
        assert!(!name.is_empty());
    }

    #[test]
    fn resolve_assistant_name_from_env() {
        let name = resolve_assistant_name_with(Some("Opus"));
        assert_eq!(name, "Opus");
    }

    #[test]
    fn resolve_assistant_name_default() {
        let name = resolve_assistant_name_with(None);
        assert_eq!(name, "Orchestrator");
    }

    #[test]
    fn resolve_assistant_name_empty_env_is_default() {
        let name = resolve_assistant_name_with(Some(""));
        assert_eq!(name, "Orchestrator");
    }

    // ---------------------------------------------------------------------------
    // generate_clean_transcript with custom names
    // ---------------------------------------------------------------------------

    #[test]
    fn custom_speaker_names_in_transcript() {
        let jsonl = format!("{}\n{}", user_str("Hello"), assistant_text("Hi there."));
        let result = generate_clean_transcript(&jsonl, "Alice", "Bot").unwrap();
        assert_eq!(result, "**Alice:** Hello\n\n**Bot:** Hi there.\n\n");
    }

    // ---------------------------------------------------------------------------
    // build_agent_type_map
    // ---------------------------------------------------------------------------

    #[test]
    fn agent_type_map_from_parent_jsonl() {
        // Mock a parent session JSONL with an Agent tool_use call
        let parent_jsonl = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_abc123","name":"Agent","input":{"subagent_type":"Turnkey Whistledown","task":"build it"}}]}}"#;
        let map = build_agent_type_map(parent_jsonl);
        assert_eq!(
            map.get("toolu_abc123").map(|s| s.as_str()),
            Some("Turnkey Whistledown")
        );
    }

    #[test]
    fn agent_type_map_empty_for_no_agents() {
        let parent_jsonl = r#"{"type":"user","message":{"content":"hello"}}"#;
        let map = build_agent_type_map(parent_jsonl);
        assert!(map.is_empty());
    }

    #[test]
    fn agent_type_map_multiple_agents() {
        let line1 = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_aaa","name":"Agent","input":{"subagent_type":"Builder","task":"build"}},{"type":"tool_use","id":"toolu_bbb","name":"Agent","input":{"subagent_type":"Tester","task":"test"}}]}}"#;
        let map = build_agent_type_map(line1);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("toolu_aaa").map(|s| s.as_str()), Some("Builder"));
        assert_eq!(map.get("toolu_bbb").map(|s| s.as_str()), Some("Tester"));
    }

    #[test]
    fn resolve_agent_display_name_with_map() {
        let mut map = HashMap::new();
        map.insert("toolu_abc123".to_string(), "Whistledown".to_string());
        // Agent file would be named agent-abc123.jsonl
        let path = PathBuf::from("/some/dir/agent-abc123.jsonl");
        // The hex_id is "abc123", which is contained in "toolu_abc123"
        let name = resolve_agent_display_name(&path, &map);
        assert_eq!(name, "Whistledown");
    }

    #[test]
    fn resolve_agent_display_name_fallback() {
        let map = HashMap::new();
        let path = PathBuf::from("/some/dir/agent-def456.jsonl");
        let name = resolve_agent_display_name(&path, &map);
        assert_eq!(name, "def456");
    }

    // ---------------------------------------------------------------------------
    // W4: backwards-compat & edge-case tests
    // ---------------------------------------------------------------------------

    #[test]
    fn resolve_agent_display_name_empty_hex_id() {
        // S2 guard: an empty hex_id (from "agent-.jsonl") should be returned as-is,
        // not match every tool_use_id via "anything".contains("").
        let mut map = HashMap::new();
        map.insert("toolu_xyz789".to_string(), "ShouldNotMatch".to_string());
        let path = PathBuf::from("/some/dir/agent-.jsonl");
        let name = resolve_agent_display_name(&path, &map);
        assert_eq!(name, "");
    }

    #[test]
    fn build_agent_type_map_missing_subagent_type() {
        // Agent tool_use block without subagent_type field should be silently skipped.
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_notype","name":"Agent","input":{"task":"do something"}}]}}"#;
        let map = build_agent_type_map(jsonl);
        assert!(
            map.is_empty(),
            "map should be empty when subagent_type is missing"
        );
    }
}
