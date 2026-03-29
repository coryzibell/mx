//! Centralized path resolution for mx CLI
//!
//! All paths the application needs derive from `mx_home()`. The base directory
//! is determined once per process (via `OnceLock`) using this priority:
//!
//! 1. `MX_HOME` environment variable (explicit override)
//! 2. Fallback: `~/.mx/`
//!
//! Subsystem-specific overrides (`MX_CODEX_PATH`, `MX_MEMORY_PATH`, etc.)
//! continue to work -- they take precedence over the derived path when set.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static MX_HOME: OnceLock<PathBuf> = OnceLock::new();

/// Pure resolution logic for MX_HOME. Takes the env var value as a parameter
/// so callers (especially tests) don't need to touch process state.
fn resolve_mx_home_with(env_val: Option<&str>) -> PathBuf {
    if let Some(val) = env_val
        && !val.is_empty()
    {
        return PathBuf::from(val);
    }
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".mx")
}

/// Resolve the MX_HOME base directory.
///
/// Priority: `MX_HOME` env var > `~/.mx/`
/// Result is cached for the lifetime of the process.
pub fn mx_home() -> &'static PathBuf {
    MX_HOME.get_or_init(|| resolve_mx_home_with(std::env::var("MX_HOME").ok().as_deref()))
}

/// Emit a startup note to stderr when MX_HOME is not explicitly configured.
pub fn emit_mx_home_note() {
    if std::env::var("MX_HOME").map_or(true, |v| v.is_empty()) {
        eprintln!(
            "note: Using default {}. Set MX_HOME to customize.",
            mx_home().display()
        );
    }
}

// ---------------------------------------------------------------------------
// Derived paths -- every path the codebase needs lives here
// ---------------------------------------------------------------------------

/// Schemas directory: `$MX_HOME/schemas/`
pub fn schemas_dir() -> PathBuf {
    mx_home().join("schemas")
}

/// Swap directory: `$MX_HOME/swap/`
pub fn swap_dir() -> PathBuf {
    mx_home().join("swap")
}

/// Sync cache directory for a specific repo: `$MX_HOME/cache/sync/<repo-slug>/`
pub fn sync_cache_dir(repo: &str) -> PathBuf {
    let repo_slug = repo.replace('/', "-");
    mx_home().join("cache").join("sync").join(repo_slug)
}

/// Artifacts directory: `$MX_HOME/artifacts/`
pub fn artifacts_dir() -> PathBuf {
    mx_home().join("artifacts")
}

/// Agents directory: `$MX_HOME/agents/`
pub fn agents_dir() -> PathBuf {
    mx_home().join("agents")
}

/// Pure resolution logic for codex directory. Takes the `MX_CODEX_PATH` env
/// var value as a parameter so callers (especially tests) don't need to touch
/// process state.
pub fn codex_dir_with(env_val: Option<&str>, home: &Path) -> PathBuf {
    if let Some(path) = env_val
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    home.join("codex")
}

/// Codex directory (session archives).
///
/// Override: `MX_CODEX_PATH` env var.
/// Default: `$MX_HOME/codex/`
pub fn codex_dir() -> PathBuf {
    codex_dir_with(std::env::var("MX_CODEX_PATH").ok().as_deref(), mx_home())
}

/// Doctor check: CLAUDE.md path: `$MX_HOME/CLAUDE.md`
pub fn doctor_claude_md() -> PathBuf {
    mx_home().join("CLAUDE.md")
}

/// Doctor check: identity colors path: `$MX_HOME/artifacts/etc/identity-colors.yaml`
pub fn doctor_identity_colors() -> PathBuf {
    artifacts_dir().join("etc").join("identity-colors.yaml")
}

/// Doctor check: ram directory: `$MX_HOME/ram/neo/`
pub fn doctor_ram_dir() -> PathBuf {
    mx_home().join("ram").join("neo")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Tests call the `_with` variants directly with explicit parameters,
    // avoiding any env-var mutation and running safely in parallel.

    #[test]
    fn mx_home_default_when_unset() {
        let result = resolve_mx_home_with(None);
        let expected = dirs::home_dir().unwrap().join(".mx");
        assert_eq!(result, expected);
    }

    #[test]
    fn mx_home_respects_env_var() {
        let result = resolve_mx_home_with(Some("/tmp/test-mx-home"));
        assert_eq!(result, PathBuf::from("/tmp/test-mx-home"));
    }

    #[test]
    fn mx_home_empty_env_is_default() {
        let result = resolve_mx_home_with(Some(""));
        let expected = dirs::home_dir().unwrap().join(".mx");
        assert_eq!(result, expected);
    }

    #[test]
    fn derived_dirs_under_mx_home() {
        // Test real derived-path functions against the cached mx_home().
        // Each function should return a path rooted under mx_home().
        let home = mx_home();

        let schemas = schemas_dir();
        assert!(schemas.starts_with(home), "schemas_dir not under mx_home");
        assert_eq!(schemas.file_name().unwrap(), "schemas");

        let swap = swap_dir();
        assert!(swap.starts_with(home), "swap_dir not under mx_home");
        assert_eq!(swap.file_name().unwrap(), "swap");

        let agents = agents_dir();
        assert!(agents.starts_with(home), "agents_dir not under mx_home");
        assert_eq!(agents.file_name().unwrap(), "agents");

        let artifacts = artifacts_dir();
        assert!(
            artifacts.starts_with(home),
            "artifacts_dir not under mx_home"
        );
        assert_eq!(artifacts.file_name().unwrap(), "artifacts");

        // codex_dir without override should also be under mx_home
        let codex = codex_dir_with(None, home);
        assert!(codex.starts_with(home), "codex_dir not under mx_home");
        assert_eq!(codex.file_name().unwrap(), "codex");

        let sync = sync_cache_dir("owner/repo");
        assert!(sync.starts_with(home), "sync_cache_dir not under mx_home");
    }

    #[test]
    fn codex_dir_respects_override() {
        let home = mx_home().clone();
        let result = codex_dir_with(Some("/custom/codex"), &home);
        assert_eq!(result, PathBuf::from("/custom/codex"));
    }

    #[test]
    fn codex_dir_empty_override_is_default() {
        let home = mx_home().clone();
        let result = codex_dir_with(Some(""), &home);
        assert_eq!(result, home.join("codex"));
    }

    #[test]
    fn codex_dir_none_override_is_default() {
        let home = mx_home().clone();
        let result = codex_dir_with(None, &home);
        assert_eq!(result, home.join("codex"));
    }

    #[test]
    fn swap_dir_is_under_mx_home() {
        // Use the cached mx_home (fine for this structural test)
        let swap = swap_dir();
        assert!(swap.starts_with(mx_home()));
    }

    #[test]
    fn sync_cache_dir_slugifies_repo() {
        let dir = sync_cache_dir("owner/repo");
        // Should contain "owner-repo" not "owner/repo"
        assert!(dir.to_string_lossy().contains("owner-repo"));
        assert!(dir.starts_with(mx_home()));
    }
}
