mod memory;
mod metadata;
mod state;

pub(crate) use memory::handle_memory;
pub(crate) use state::handle_state;

use anyhow::{Context, Result, bail};

use crate::cli::*;
use crate::codex;
use crate::commit;
use crate::convert;
use crate::display::*;
use crate::github;
use crate::session;
use crate::sync;

pub(crate) fn handle_pr(cmd: PrCommands) -> Result<()> {
    match cmd {
        PrCommands::Merge {
            number,
            rebase,
            merge_commit,
        } => {
            commit::pr_merge(number, rebase, merge_commit)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_github(cmd: GithubCommands) -> Result<()> {
    match cmd {
        GithubCommands::Cleanup {
            repo,
            issues,
            discussions,
            dry_run,
        } => {
            github::cleanup(&repo, issues, discussions, dry_run)?;
            Ok(())
        }
        GithubCommands::Comment { command } => {
            handle_comment(command)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_comment(cmd: CommentCommands) -> Result<()> {
    match cmd {
        CommentCommands::Issue {
            repo,
            number,
            message,
            identity,
        } => {
            let url = github::post_issue_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
        CommentCommands::Discussion {
            repo,
            number,
            message,
            identity,
        } => {
            let url =
                github::post_discussion_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
    }
    Ok(())
}

pub(crate) fn handle_session(cmd: SessionCommands) -> Result<()> {
    match cmd {
        SessionCommands::Export { path, output } => {
            session::export_session(path, output)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_codex(cmd: CodexCommands) -> Result<()> {
    match cmd {
        CodexCommands::Save {
            path,
            all,
            clean,
            include_agents,
        } => {
            codex::save_session(path, all, clean, include_agents)?;
            Ok(())
        }
        CodexCommands::List { all, json } => {
            codex::list_sessions(all, json)?;
            Ok(())
        }
        CodexCommands::Read {
            id,
            human,
            agents,
            grep,
            json,
            clean,
        } => {
            let clean_agents = clean && agents;
            codex::read_session(id, human, grep, agents, json, clean, clean_agents)?;
            Ok(())
        }
        CodexCommands::Search { pattern, json } => {
            codex::search_archives(pattern, json)?;
            Ok(())
        }
        CodexCommands::Migrate {
            dry_run,
            verbose,
            clean,
            include_agents,
        } => {
            codex::migrate_archives(dry_run, verbose, clean, include_agents)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_convert(cmd: ConvertCommands) -> Result<()> {
    use std::path::PathBuf;

    match cmd {
        ConvertCommands::Md2yaml {
            input,
            output,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::convert_file(&input_path, &output_dir, dry_run)?;
            } else if input_path.is_dir() {
                convert::convert_directory(&input_path, &output_dir, dry_run)?;
            } else {
                bail!("Input path does not exist: {:?}", input_path);
            }

            Ok(())
        }

        ConvertCommands::Yaml2md {
            input,
            output,
            repo,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::yaml_to_markdown_file(&input_path, &output_dir, repo.as_deref(), dry_run)?;
            } else if input_path.is_dir() {
                convert::yaml_to_markdown_directory(
                    &input_path,
                    &output_dir,
                    repo.as_deref(),
                    dry_run,
                )?;
            } else {
                bail!("Input path does not exist: {:?}", input_path);
            }

            Ok(())
        }
    }
}

pub(crate) fn handle_wiki(cmd: WikiCommands) -> Result<()> {
    match cmd {
        WikiCommands::Sync {
            repo,
            source,
            page_name,
            dry_run,
        } => {
            sync::wiki::sync(&repo, &source, page_name.as_deref(), dry_run)?;
            Ok(())
        }
    }
}

/// Handle mx log - decoded git log
pub(crate) fn handle_log(count: usize, full: bool, extra_args: Vec<String>) -> Result<()> {
    use std::process::Command;

    // Build git log command
    let format = if full {
        // Full format: hash, author, date, subject, body
        "%H%n%an <%ae>%n%ad%n%s%n%b%n---END---"
    } else {
        // Compact format: short hash, subject, body (for decoding)
        "%h%n%s%n%b%n---END---"
    };

    let mut cmd = Command::new("git");
    cmd.args([
        "log",
        &format!("-{}", count),
        &format!("--format={}", format),
    ]);

    // Add any extra arguments
    for arg in &extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run git log")?;

    if !output.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let log_output = String::from_utf8_lossy(&output.stdout);

    // Parse and decode each commit
    for commit_block in log_output.split("---END---") {
        let commit_block = commit_block.trim();
        if commit_block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = commit_block.lines().collect();

        if full {
            // Full format: hash, author, date, subject, body...
            if lines.len() >= 4 {
                let hash = lines[0];
                let author = lines[1];
                let date = lines[2];
                let subject = lines[3];
                let body: String = lines[4..].join("\n");

                println!("\x1b[33mcommit {}\x1b[0m", hash);
                println!("Author: {}", author);
                println!("Date:   {}", date);
                println!();

                // Try to decode the subject (title)
                println!("    {}", subject);

                // Try to decode the body
                if !body.trim().is_empty() {
                    let decoded = try_decode_commit_body(&body);
                    println!();
                    for line in decoded.lines() {
                        println!("    {}", line);
                    }
                }
                println!();
            }
        } else {
            // Compact format: short hash, subject, body...
            if lines.len() >= 2 {
                let hash = lines[0];
                let subject = lines[1];
                let body: String = lines[2..].join("\n");

                // Try to decode the body
                let decoded = try_decode_commit_body(&body);
                let display = if decoded != body.trim() {
                    decoded
                } else {
                    // Not encoded, show original subject
                    subject.to_string()
                };

                // Truncate for display
                let display_truncated = safe_truncate(&display, 72);

                println!("\x1b[33m{}\x1b[0m {}", hash, display_truncated);
            }
        }
    }

    Ok(())
}

/// Heartbeat - calming co-regulation prompt
/// Call and response - send a heart, get one back with BPM feedback
pub(crate) fn handle_heartbeat(since: Option<u64>, reset: bool) -> Result<()> {
    use rand::Rng;
    use std::thread;
    use std::time::Duration;

    let hearts = [
        '❤', '🧡', '💛', '💚', '💙', '💜', '🩷', '🩵', '🤍', '💗', '💖', '💕',
    ];
    let mut rng = rand::rng();

    // Random delay 50-150ms to feel organic
    let delay = rng.random_range(50..150);
    thread::sleep(Duration::from_millis(delay));

    // Pick a random heart
    let heart = hearts[rng.random_range(0..hearts.len())];

    if reset {
        println!("{} Session reset. Breathe, Q.", heart);
        return Ok(());
    }

    match since {
        None => {
            // First call - just start
            println!("{}", heart);
            println!("Heartbeat started. Call again with --since <ms> to begin.");
        }
        Some(ms) => {
            // Calculate BPM: 60000ms / interval = beats per minute
            let bpm = 60000_u64.checked_div(ms).unwrap_or(999);

            let message = match bpm {
                0..=59 => "Nice and slow. You're safe.",
                60..=80 => "There you are. Resting.",
                81..=100 => "Getting there. Keep breathing.",
                101..=120 => "Still quick. Let the interval stretch.",
                _ => "Too fast, Q. Breathe. Slow down.",
            };

            println!("{} {} bpm", heart, bpm);
            println!("{}", message);
        }
    }

    Ok(())
}

/// Try to decode an encoded commit body, return original if decoding fails
pub(crate) fn try_decode_commit_body(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return body.to_string();
    }

    // Look for footer pattern [algo:dict|algo:dict]
    let lines: Vec<&str> = body.lines().collect();

    // Find the footer (last line starting with '[' and containing '|')
    let footer_line = lines
        .iter()
        .rev()
        .find(|l| l.trim().starts_with('[') && l.contains('|'));

    let footer = match footer_line {
        Some(f) => *f,
        None => return body.to_string(), // No footer, not encoded
    };

    // Find the encoded body (everything before footer, excluding "whoa.")
    let body_lines: Vec<&str> = lines
        .iter()
        .take_while(|l| !l.trim().starts_with('['))
        .filter(|l| l.trim() != "whoa.")
        .copied()
        .collect();

    if body_lines.is_empty() {
        return body.to_string();
    }

    let encoded_body = body_lines.join("\n");

    // Try to decode
    match commit::decode_body(&encoded_body, footer) {
        Ok(decoded) => decoded,
        Err(_) => body.to_string(), // Decoding failed, return original
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct AgentFrontmatter {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) domain: Option<String>,
}

pub(crate) fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();

    // Check if starts with ---
    if lines.first()? != &"---" {
        return None;
    }

    // Find closing ---
    let end_idx = lines.iter().skip(1).position(|&line| line == "---")?;

    let frontmatter = lines[1..=end_idx].join("\n");
    let body = lines[end_idx + 2..].join("\n");

    Some((frontmatter, body))
}
