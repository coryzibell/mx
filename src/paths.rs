//! Centralized path resolution for mx.
//!
//! All hardcoded `~/.crewu/` references should go through this module.
//! Set `MX_HOME` to relocate the mx data directory without touching anything else.

use std::path::PathBuf;

/// Resolve the mx home directory.
///
/// Priority:
/// 1. `MX_HOME` environment variable
/// 2. `~/.crewu/` (backwards-compatible default)
pub fn mx_home() -> PathBuf {
    if let Ok(home) = std::env::var("MX_HOME") {
        PathBuf::from(home)
    } else {
        dirs::home_dir()
            .expect("Could not determine home directory")
            .join(".crewu")
    }
}

/// Resolve the schemas directory: `$MX_HOME/schemas/`
pub fn schemas_dir() -> PathBuf {
    mx_home().join("schemas")
}

/// Resolve the swap directory: `$MX_HOME/swap/`
pub fn swap_dir() -> PathBuf {
    mx_home().join("swap")
}

/// Resolve the codex directory.
///
/// Priority:
/// 1. `MX_CODEX_PATH` environment variable (existing per-concern override)
/// 2. `~/.crewu-private/logs/codex` (default — note: separate sibling dir, not under MX_HOME)
///
/// The private directory is intentionally not derived from `MX_HOME` here because
/// `MX_CODEX_PATH` already covers the relocation use-case and the derivation adds
/// complexity without meaningful benefit.
pub fn codex_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MX_CODEX_PATH") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".crewu-private/logs/codex")
}
