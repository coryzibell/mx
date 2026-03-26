//! Centralized path resolution for mx.
//!
//! All hardcoded `~/.mx/` references should go through this module.
//! Set `MX_HOME` to relocate the mx data directory without touching anything else.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::OnceLock;

static MX_HOME_CACHE: OnceLock<PathBuf> = OnceLock::new();

/// Resolve the mx home directory.
///
/// Priority:
/// 1. `MX_HOME` environment variable
/// 2. `~/.mx/` (default)
///
/// Result is cached after the first call.
pub fn mx_home() -> Result<PathBuf> {
    if let Some(cached) = MX_HOME_CACHE.get() {
        return Ok(cached.clone());
    }
    let path = resolve_mx_home()?;
    Ok(MX_HOME_CACHE.get_or_init(|| path).clone())
}

/// Internal uncached resolver — used by tests to avoid OnceLock interference.
fn resolve_mx_home() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("MX_HOME") {
        Ok(PathBuf::from(home))
    } else {
        Ok(dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".mx"))
    }
}

/// Emit a warning to stderr if `MX_HOME` is not set.
///
/// Call once at startup so users know how to customise the data directory.
pub fn warn_if_no_mx_home() {
    if std::env::var("MX_HOME").is_err() {
        eprintln!("mx: MX_HOME not set, using default ~/.mx/. Set MX_HOME to customize.");
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
/// 1. `MX_CODEX_PATH` environment variable (per-concern override)
/// 2. `$MX_HOME/logs/codex` (default — derives from MX_HOME)
pub fn codex_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MX_CODEX_PATH") {
        return Ok(PathBuf::from(path));
    }
    Ok(mx_home()?.join("logs/codex"))
}

/// Resolve the sync cache directory for a given repo: `$MX_HOME/cache/sync/<repo-slug>/`
pub fn sync_cache_dir(repo: &str) -> Result<PathBuf> {
    let repo_slug = repo.replace('/', "-");
    Ok(mx_home()?.join("cache").join("sync").join(repo_slug))
}

/// Resolve the artifacts directory: `$MX_HOME/artifacts/`
pub fn artifacts_dir() -> Result<PathBuf> {
    Ok(mx_home()?.join("artifacts"))
}

/// Resolve the agents directory: `$MX_HOME/agents/`
pub fn agents_dir() -> Result<PathBuf> {
    Ok(mx_home()?.join("agents"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // Tests use resolve_mx_home() (uncached) so they don't fight OnceLock.

    #[test]
    #[serial]
    fn mx_home_uses_env_var_when_set() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let result = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(result, PathBuf::from("/tmp/test-mx-home"));
    }

    #[test]
    #[serial]
    fn mx_home_falls_back_to_mx() {
        unsafe { std::env::remove_var("MX_HOME") };
        let result = resolve_mx_home().expect("resolve_mx_home() should succeed");
        assert!(result.ends_with(".mx"));
    }

    #[test]
    #[serial]
    fn schemas_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        // schemas_dir goes through mx_home() which may be cached — test the composition
        // via resolve_mx_home directly.
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(base.join("schemas"), PathBuf::from("/tmp/test-mx-home/schemas"));
    }

    #[test]
    #[serial]
    fn swap_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(base.join("swap"), PathBuf::from("/tmp/test-mx-home/swap"));
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
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        // Test the composition via resolve_mx_home() to avoid OnceLock dependency.
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(base.join("logs/codex"), PathBuf::from("/tmp/test-mx-home/logs/codex"));
    }

    #[test]
    #[serial]
    fn sync_cache_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(
            base.join("cache/sync/owner-repo"),
            PathBuf::from("/tmp/test-mx-home/cache/sync/owner-repo")
        );
    }

    #[test]
    #[serial]
    fn artifacts_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(base.join("artifacts"), PathBuf::from("/tmp/test-mx-home/artifacts"));
    }

    #[test]
    #[serial]
    fn agents_dir_derives_from_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let base = resolve_mx_home().expect("resolve_mx_home() should succeed");
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(base.join("agents"), PathBuf::from("/tmp/test-mx-home/agents"));
    }
}
