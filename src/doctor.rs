//! Environment health check for mx CLI
//!
//! Validates critical files, directories, and configuration.

use anyhow::Result;
use std::io::IsTerminal;

use crate::sync::github::auth::get_github_token;

/// Check result with colorization support
#[derive(Debug)]
struct Check {
    name: String,
    passed: bool,
    message: Option<String>,
}

impl Check {
    fn new(name: impl Into<String>, passed: bool) -> Self {
        Self {
            name: name.into(),
            passed,
            message: None,
        }
    }

    fn with_message(name: impl Into<String>, passed: bool, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed,
            message: Some(message.into()),
        }
    }

    fn format_result(&self, use_color: bool) -> String {
        let (mark, color_code, reset) = if use_color {
            if self.passed {
                ("✓", "\x1b[32m", "\x1b[0m") // Green checkmark
            } else {
                ("✗", "\x1b[31m", "\x1b[0m") // Red X
            }
        } else if self.passed {
            ("✓", "", "")
        } else {
            ("✗", "", "")
        };

        let mut result = format!("{}{}{} {}", color_code, mark, reset, self.name);

        if let Some(ref msg) = self.message {
            result.push_str(&format!(" ({})", msg));
        }

        result
    }
}

/// Run all environment checks
pub fn run_checks(json: bool) -> Result<()> {
    let use_color = std::io::stdout().is_terminal();

    let home = crate::paths::mx_home()?;
    let checks = vec![
        check_file_exists("$MX_HOME/CLAUDE.md", &home.join("CLAUDE.md")),
        check_file_exists(
            "$MX_HOME/artifacts/etc/identity-colors.yaml",
            &home.join("artifacts/etc/identity-colors.yaml"),
        ),
        check_directory_exists("$MX_HOME/ram/neo/", &home.join("ram/neo")),
        check_github_token(),
    ];

    let failed_count = checks.iter().filter(|c| !c.passed).count();

    if json {
        let json_checks: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "passed": c.passed,
                    "message": c.message,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "checks": json_checks,
                "passed": failed_count == 0,
                "failed_count": failed_count,
            }))?
        );
    } else {
        println!("Environment Health Check\n");

        // Print results
        for check in &checks {
            println!("{}", check.format_result(use_color));
        }

        // Summary
        println!();
        if failed_count == 0 {
            println!("All checks passed!");
        } else {
            println!(
                "{} issue{} found",
                failed_count,
                if failed_count == 1 { "" } else { "s" }
            );
        }
    }

    if failed_count > 0 {
        anyhow::bail!("{} check(s) failed", failed_count);
    }

    Ok(())
}

/// Check if a file exists
fn check_file_exists(display_name: &str, path: &std::path::Path) -> Check {
    if path.exists() && path.is_file() {
        Check::new(display_name, true)
    } else {
        Check::with_message(display_name, false, "not found")
    }
}

/// Check if a directory exists
fn check_directory_exists(display_name: &str, path: &std::path::Path) -> Check {
    if path.exists() && path.is_dir() {
        Check::new(display_name, true)
    } else {
        Check::with_message(display_name, false, "not found")
    }
}

/// Check GitHub token validity
fn check_github_token() -> Check {
    match get_github_token() {
        Ok(token) => {
            // Validate token format
            if token.starts_with("ghp_") || token.starts_with("github_pat_") {
                Check::new("GitHub token (valid format)", true)
            } else {
                Check::with_message(
                    "GitHub token (invalid format)",
                    false,
                    "missing ghp_ or github_pat_ prefix",
                )
            }
        }
        Err(_) => Check::with_message("GitHub token", false, "not found in ~/.claude.json"),
    }
}

