//! Environment health check for mx CLI
//!
//! Validates critical files, directories, and configuration.

use anyhow::Result;
use std::io::IsTerminal;
use std::path::PathBuf;

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
        } else {
            if self.passed {
                ("✓", "", "")
            } else {
                ("✗", "", "")
            }
        };

        let mut result = format!("{}{}{} {}", color_code, mark, reset, self.name);

        if let Some(ref msg) = self.message {
            result.push_str(&format!(" ({})", msg));
        }

        result
    }
}

/// Run all environment checks
pub fn run_checks() -> Result<()> {
    let use_color = std::io::stdout().is_terminal();

    println!("Matrix Environment Check\n");

    let mut checks = Vec::new();

    // Check required files and directories
    checks.push(check_file_exists(
        "~/.matrix/CLAUDE.md",
        &home_path(".matrix/CLAUDE.md"),
    ));

    checks.push(check_file_exists(
        "~/.matrix/artifacts/etc/identity-colors.yaml",
        &home_path(".matrix/artifacts/etc/identity-colors.yaml"),
    ));

    checks.push(check_directory_exists(
        "~/.matrix/ram/neo/",
        &home_path(".matrix/ram/neo"),
    ));

    // Check GitHub token
    checks.push(check_github_token());

    // Print results
    for check in &checks {
        println!("{}", check.format_result(use_color));
    }

    // Summary
    let failed_count = checks.iter().filter(|c| !c.passed).count();

    println!();
    if failed_count == 0 {
        println!("All checks passed!");
        Ok(())
    } else {
        println!("{} issue{} found", failed_count, if failed_count == 1 { "" } else { "s" });
        std::process::exit(1);
    }
}

/// Check if a file exists
fn check_file_exists(display_name: &str, path: &PathBuf) -> Check {
    if path.exists() && path.is_file() {
        Check::new(display_name, true)
    } else {
        Check::with_message(display_name, false, "not found")
    }
}

/// Check if a directory exists
fn check_directory_exists(display_name: &str, path: &PathBuf) -> Check {
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

/// Resolve home directory path
fn home_path(relative: &str) -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(relative)
}
