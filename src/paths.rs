//! Centralized path resolution for mx.
//!
//! All hardcoded `~/.crewu/` references should go through this module.
//! Set `MX_HOME` to relocate the mx data directory without touching anything else.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Resolve the mx home directory.
///
/// Priority:
/// 1. `MX_HOME` environment variable
/// 2. `~/.crewu/` (backwards-compatible default)
pub fn mx_home() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("MX_HOME") {
        Ok(PathBuf::from(home))
    } else {
        Ok(dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".crewu"))
    }
}

/// Resolve the schemas directory: `$MX_HOME/schemas/`
pub fn schemas_dir() -> Result<PathBuf> {
    Ok(mx_home()?.join("schemas"))
}

/// Resolve the swap directory: `$MX_HOME/swap/`
pub fn swap_dir() -> Result<PathBuf> {
    Ok(mx_home()?.join("swap"))
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
pub fn codex_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MX_CODEX_PATH") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".crewu-private/logs/codex"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn mx_home_uses_env_var_when_set() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let result = mx_home().expect("mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(result, PathBuf::from("/tmp/test-mx-home"));
    }

    #[test]
    #[serial]
    fn mx_home_falls_back_to_crewu() {
        unsafe { std::env::remove_var("MX_HOME") };
        let result = mx_home().expect("mx_home() should succeed");
        assert!(result.ends_with(".crewu"));
    }

    #[test]
    #[serial]
    fn schemas_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let result = schemas_dir().expect("schemas_dir() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(result, PathBuf::from("/tmp/test-mx-home/schemas"));
    }

    #[test]
    #[serial]
    fn codex_dir_uses_env_var_when_set() {
        unsafe { std::env::set_var("MX_CODEX_PATH", "/tmp/test-codex") };
        let result = codex_dir().expect("codex_dir() should succeed");
        unsafe { std::env::remove_var("MX_CODEX_PATH") };
        assert_eq!(result, PathBuf::from("/tmp/test-codex"));
    }

    #[test]
    #[serial]
    fn codex_dir_falls_back_to_default() {
        unsafe { std::env::remove_var("MX_CODEX_PATH") };
        let result = codex_dir().expect("codex_dir() should succeed");
        assert!(result.ends_with(".crewu-private/logs/codex"));
    }
}
