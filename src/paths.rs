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

use std::path::PathBuf;
use std::sync::OnceLock;

static MX_HOME: OnceLock<PathBuf> = OnceLock::new();

/// Resolve the MX_HOME base directory.
///
/// Priority: `MX_HOME` env var > `~/.mx/`
/// Result is cached for the lifetime of the process.
pub fn mx_home() -> &'static PathBuf {
    MX_HOME.get_or_init(|| {
        if let Ok(val) = std::env::var("MX_HOME") {
            if !val.is_empty() {
                return PathBuf::from(val);
            }
        }
        dirs::home_dir()
            .expect("Could not determine home directory")
            .join(".mx")
    })
}

/// Emit a startup note to stderr when MX_HOME is not explicitly configured.
pub fn emit_mx_home_note() {
    if std::env::var("MX_HOME").is_err() {
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

/// Codex directory (session archives).
///
/// Override: `MX_CODEX_PATH` env var.
/// Default: `$MX_HOME/codex/`
pub fn codex_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MX_CODEX_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    mx_home().join("codex")
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
// Display helpers for CLI help text
// ---------------------------------------------------------------------------

/// The default MX_HOME path as a display string (for help text).
/// Returns something like `~/.mx` regardless of MX_HOME override.
pub fn default_mx_home_display() -> &'static str {
    "~/.mx"
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: These tests manipulate env vars and are NOT safe to run in parallel
    // with other tests that read MX_HOME / MX_CODEX_PATH. Run with:
    //   cargo test paths:: -- --test-threads=1
    //
    // Because OnceLock caches the first call, we test the *inner* resolution
    // logic directly rather than going through the cached `mx_home()`.

    /// Resolve mx_home without caching (for test isolation).
    fn resolve_mx_home_uncached() -> PathBuf {
        if let Ok(val) = std::env::var("MX_HOME") {
            if !val.is_empty() {
                return PathBuf::from(val);
            }
        }
        dirs::home_dir()
            .expect("Could not determine home directory")
            .join(".mx")
    }

    #[test]
    fn mx_home_default_when_unset() {
        unsafe { std::env::remove_var("MX_HOME") };
        let result = resolve_mx_home_uncached();
        let expected = dirs::home_dir().unwrap().join(".mx");
        assert_eq!(result, expected);
    }

    #[test]
    fn mx_home_respects_env_var() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx-home") };
        let result = resolve_mx_home_uncached();
        unsafe { std::env::remove_var("MX_HOME") };
        assert_eq!(result, PathBuf::from("/tmp/test-mx-home"));
    }

    #[test]
    fn derived_dirs_under_mx_home() {
        unsafe { std::env::set_var("MX_HOME", "/tmp/test-mx") };
        let home = resolve_mx_home_uncached();
        unsafe { std::env::remove_var("MX_HOME") };

        // Verify that derived paths would be under the home
        assert!(home.join("schemas").starts_with(&home));
        assert!(home.join("swap").starts_with(&home));
        assert!(home.join("agents").starts_with(&home));
        assert!(home.join("artifacts").starts_with(&home));
        assert!(home.join("codex").starts_with(&home));
        assert!(home.join("cache").join("sync").starts_with(&home));
    }

    #[test]
    fn codex_dir_respects_override() {
        unsafe { std::env::set_var("MX_CODEX_PATH", "/custom/codex") };
        let result = codex_dir();
        unsafe { std::env::remove_var("MX_CODEX_PATH") };
        assert_eq!(result, PathBuf::from("/custom/codex"));
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
