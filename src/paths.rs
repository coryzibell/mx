//! Centralized path resolution for mx CLI
//!
//! All paths the application needs derive from `mx_home()`. The base directory
//! is determined once per process (via `OnceLock`) using this priority:
//!
//! 1. `MX_HOME` environment variable (explicit override)
//! 2. Fallback: `~/.mx/`
//!
//! Subsystem-specific overrides (`MX_CODEX_PATH`, `MX_SURREAL_ROOT`, etc.)
//! continue to work -- they take precedence over the derived path when set.
//!
//! Per-file env-var overrides (`MX_KV_SCHEMA`, `MX_KV_DATA`) survive too --
//! but they are resolved at the call site, not here, because they may include
//! `{agent}` placeholders.

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

/// Pure detection of the legacy `MX_MEMORY_PATH` env var.
///
/// Returns true when a non-empty value is present, signalling that a startup
/// note should be emitted to alert the user that the variable was renamed.
///
/// TODO(memory-path-rename-note): remove this detection after one release cycle.
pub(crate) fn legacy_memory_path_set(env_val: Option<&str>) -> bool {
    env_val.map(|v| !v.is_empty()).unwrap_or(false)
}

/// Emit a one-line stderr note when the deprecated `MX_MEMORY_PATH`
/// environment variable is set, telling the user it was renamed to
/// `MX_SURREAL_ROOT`.
///
/// This is the only startup note: the previous "Using default $MX_HOME"
/// message was removed because it fired on every invocation for users who
/// hadn't customized anything (i.e. most users) and wasn't part of the
/// decision-9 spec. A future verbose-path-debugging mode would belong on
/// its own ticket.
///
/// TODO(memory-path-rename-note): remove this detection+warning after one
/// release cycle.
pub fn emit_legacy_memory_path_note() {
    if legacy_memory_path_set(std::env::var("MX_MEMORY_PATH").ok().as_deref()) {
        eprintln!(
            "note: `MX_MEMORY_PATH` is no longer used. \
             It was renamed to `MX_SURREAL_ROOT`. Update your environment."
        );
    }
}

// ---------------------------------------------------------------------------
// Derived paths -- every path the codebase needs lives here
// ---------------------------------------------------------------------------

/// Swap directory: `$MX_HOME/swap/`
pub fn swap_dir() -> PathBuf {
    mx_home().join("swap")
}

/// Sync cache directory for a specific repo: `$MX_HOME/cache/sync/<repo-slug>/`
pub fn sync_cache_dir(repo: &str) -> PathBuf {
    let repo_slug = repo.replace('/', "-");
    mx_home().join("cache").join("sync").join(repo_slug)
}

// ---------------------------------------------------------------------------
// kv (decision 1)
// ---------------------------------------------------------------------------

/// `$MX_HOME/kv/schema/`
pub fn kv_schema_dir() -> PathBuf {
    mx_home().join("kv").join("schema")
}

/// `$MX_HOME/kv/data/`
pub fn kv_data_dir() -> PathBuf {
    mx_home().join("kv").join("data")
}

/// `$MX_HOME/kv/schema/{agent}.toml`
pub fn kv_schema_path(agent: &str) -> PathBuf {
    kv_schema_dir().join(format!("{}.toml", agent))
}

/// `$MX_HOME/kv/data/{agent}.json`
pub fn kv_data_path(agent: &str) -> PathBuf {
    kv_data_dir().join(format!("{}.json", agent))
}

/// Legacy `~/.crewu/kv/{agent}.schema.toml` -- soft fallback only.
///
/// TODO(kv-path-migration): remove after one release cycle.
pub fn legacy_crewu_kv_schema_path(agent: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".crewu")
            .join("kv")
            .join(format!("{}.schema.toml", agent))
    })
}

/// Legacy `~/.crewu/kv/{agent}.data.json` -- soft fallback only.
///
/// TODO(kv-path-migration): remove after one release cycle.
pub fn legacy_crewu_kv_data_path(agent: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".crewu")
            .join("kv")
            .join(format!("{}.data.json", agent))
    })
}

// ---------------------------------------------------------------------------
// state / tensor schemas (decision 5)
// ---------------------------------------------------------------------------

/// `$MX_HOME/state/schemas/`
pub fn state_schemas_dir() -> PathBuf {
    mx_home().join("state").join("schemas")
}

/// `$MX_HOME/state/schemas/{id}.yaml`
pub fn tensor_schema_path(id: &str) -> PathBuf {
    state_schemas_dir().join(format!("{}.yaml", id))
}

// ---------------------------------------------------------------------------
// memory (decisions 6, 7, 9)
// ---------------------------------------------------------------------------

fn surreal_root_with(env_val: Option<&str>, home: &Path) -> PathBuf {
    if let Some(path) = env_val
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    home.join("memory").join("surreal")
}

/// SurrealDB store root.
///
/// Override: `MX_SURREAL_ROOT` env var.
/// Default: `$MX_HOME/memory/surreal/`
pub fn surreal_root() -> PathBuf {
    surreal_root_with(std::env::var("MX_SURREAL_ROOT").ok().as_deref(), mx_home())
}

/// `$MX_HOME/memory/seed/agents/`
pub fn memory_seed_agents_dir() -> PathBuf {
    mx_home().join("memory").join("seed").join("agents")
}

/// `$MX_HOME/memory/seed/knowledge/`
pub fn memory_seed_knowledge_dir() -> PathBuf {
    mx_home().join("memory").join("seed").join("knowledge")
}

/// Pure resolution of the legacy agents directory layout. Takes `home`
/// explicitly so tests don't need to touch process state -- mirrors the
/// `surreal_root_with` / `fastembed_cache_dir_with` pattern.
fn legacy_agents_dir_with(home: &Path) -> PathBuf {
    home.join("agents")
}

/// Legacy `$MX_HOME/agents/` -- soft fallback only.
///
/// TODO(memory-seed-agents-migration): remove after one release cycle.
pub fn legacy_agents_dir() -> PathBuf {
    legacy_agents_dir_with(mx_home())
}

/// Legacy `$MX_HOME/memory/index.jsonl` -- soft fallback only.
///
/// TODO(memory-seed-knowledge-migration): remove after one release cycle.
pub fn legacy_memory_index_jsonl() -> PathBuf {
    mx_home().join("memory").join("index.jsonl")
}

// ---------------------------------------------------------------------------
// FastEmbed cache (decision 4)
// ---------------------------------------------------------------------------

fn fastembed_cache_dir_with(
    isolate_env: Option<&str>,
    cache_dir_fn: impl FnOnce() -> Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    if isolate_env.map(|v| !v.is_empty()).unwrap_or(false) {
        return home.join("memory").join("embed");
    }
    cache_dir_fn()
        .map(|d| d.join("fastembed"))
        .unwrap_or_else(|| PathBuf::from(".fastembed_cache"))
}

/// FastEmbed model cache directory.
///
/// - Default: `$XDG_CACHE_HOME/fastembed/` (shared across tools)
/// - If `MX_ISOLATE_FASTEMBED` is set: `$MX_HOME/memory/embed/`
pub fn fastembed_cache_dir() -> PathBuf {
    fastembed_cache_dir_with(
        std::env::var("MX_ISOLATE_FASTEMBED").ok().as_deref(),
        dirs::cache_dir,
        mx_home(),
    )
}

// ---------------------------------------------------------------------------
// Codex (existing)
// ---------------------------------------------------------------------------

/// Pure resolution logic for codex directory. Takes the `MX_CODEX_PATH` env
/// var value as a parameter so callers (especially tests) don't need to touch
/// process state.
fn codex_dir_with(env_val: Option<&str>, home: &Path) -> PathBuf {
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

// ---------------------------------------------------------------------------
// External Claude data (read-only ingest sources, owned by another tool)
// ---------------------------------------------------------------------------

/// Internal helper: the resolved `~/` for use by Claude-data path builders.
///
/// Centralizing this means the codex source-walker helpers below all share a
/// single failure mode if `dirs::home_dir()` ever returns `None` (which only
/// happens in pathologically misconfigured environments).
fn claude_home_root() -> PathBuf {
    dirs::home_dir().expect("Could not determine home directory")
}

/// `~/.claude/` -- the root of Claude's per-user data directory.
///
/// Use this as the join base when adding new Claude-owned subpaths. Prefer
/// the more specific helpers below when one already exists; reach for the
/// root only when adding a new sibling that doesn't yet have its own helper.
pub fn claude_dir() -> PathBuf {
    claude_home_root().join(".claude")
}

/// `~/.claude/projects/` -- read-only by convention.
///
/// Honors the `MX_CLAUDE_PROJECTS_DIR` environment variable as an
/// override. Tests set this to a controlled tempdir so they don't
/// depend on the developer's (or CI machine's) live `~/.claude/`
/// state. Production paths reach this through the same surface, so
/// the override is opt-in via env var only — no behavior change for
/// real users.
pub fn claude_projects_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("MX_CLAUDE_PROJECTS_DIR") {
        return PathBuf::from(override_dir);
    }
    claude_dir().join("projects")
}

/// `~/.claude.json` -- read-only by convention.
pub fn claude_config_path() -> PathBuf {
    claude_home_root().join(".claude.json")
}

/// `~/.claude/projects/<project_slug>/<session_id>/subagents/`
///
/// Claude writes subagent JSONLs (`agent-*.jsonl`) into a `subagents/`
/// directory that lives next to the parent session JSONL, nested under the
/// cwd-encoded project slug. The slug is the same encoding Claude uses
/// elsewhere (e.g. `-home-charlie-recipes-coryzibell-mx`); callers are
/// responsible for supplying it because the session UUID alone does not
/// uniquely identify the project directory.
///
/// PR 2 of the codex unification will wire this into the archive source
/// walker (replacing the open-coded join in `archive.rs::find_agent_sessions`).
pub fn claude_subagents_dir(project_slug: &str, session_id: &str) -> PathBuf {
    claude_projects_dir()
        .join(project_slug)
        .join(session_id)
        .join("subagents")
}

/// `~/.claude/sessions/` -- per-pid liveness JSONs.
///
/// Used as a liveness signal only (not ingested). The codex archive flow
/// will consult this directory to resolve "the most recent session" without
/// relying on mtime heuristics.
pub fn claude_sessions_dir() -> PathBuf {
    claude_dir().join("sessions")
}

/// `~/.claude/history.jsonl` -- the slash-command / prompt history file.
///
/// Sliced by session timestamp window into a `history/` sidecar by the codex
/// archive flow (PR 2).
pub fn claude_history_jsonl() -> PathBuf {
    claude_dir().join("history.jsonl")
}

/// `~/.cache/claude-cli-nodejs/<cwd-encoded>/` -- the *parent* directory
/// containing per-MCP-server log subdirs (`mcp-logs-<server>/`).
///
/// Returns the parent because the per-server `mcp-logs-*` subdirs are
/// enumerated at call time (their names depend on which MCP servers were
/// active for the cwd). The `cwd` argument is the same cwd-encoded slug
/// Claude uses (`-home-charlie-recipes-...`).
pub fn claude_mcp_logs_dir(cwd_encoded: &str) -> PathBuf {
    claude_home_root()
        .join(".cache")
        .join("claude-cli-nodejs")
        .join(cwd_encoded)
}

/// `/tmp/claude-<uid>/-home-<user>/<session_uuid>/tasks/`
///
/// Per-uid scratch directory for Claude task outputs. The `user` segment is
/// the encoded form of the user's home directory (matching Claude's slug
/// convention), and `session_uuid` is the full session UUID.
///
/// These files are disposable by design (cleared on reboot or `/tmp`
/// cleanup); the codex archive flow snapshots them at run time. Callers
/// must supply all three components because none can be derived from the
/// uid alone.
pub fn tmp_claude_tasks_dir(uid: u32, user_slug: &str, session_uuid: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/claude-{}", uid))
        .join(user_slug)
        .join(session_uuid)
        .join("tasks")
}

/// `~/.wonka/vault/archives/` -- the legacy vault snapshot directory.
///
/// Walked by `mx codex archive --backfill` (PR 5) to ingest the existing
/// vault snapshots into the codex. Note the literal `~/.wonka/` prefix:
/// the wonka vault is a *separate* root from `MX_HOME` and intentionally
/// does not derive from it. If the user has a wonka home override in the
/// future, this helper will need updating; for v1 the path is fixed.
pub fn wonka_vault_archives_dir() -> PathBuf {
    claude_home_root()
        .join(".wonka")
        .join("vault")
        .join("archives")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // Tests call the `_with` variants directly with explicit parameters,
    // avoiding any env-var mutation and running safely in parallel.
    // The handful of tests that DO observe `claude_projects_dir()`'s env
    // override (`MX_CLAUDE_PROJECTS_DIR`) are marked `#[serial]` so they
    // can clear the env without racing concurrent codex tests that set
    // the override to a tempdir.

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

        let swap = swap_dir();
        assert!(swap.starts_with(home), "swap_dir not under mx_home");
        assert_eq!(swap.file_name().unwrap(), "swap");

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

    // ---------------------------------------------------------------------
    // kv helpers
    // ---------------------------------------------------------------------

    #[test]
    fn kv_helpers_under_mx_home() {
        let home = mx_home();
        assert!(kv_schema_dir().starts_with(home));
        assert!(kv_data_dir().starts_with(home));
        assert!(kv_schema_path("smith").ends_with("kv/schema/smith.toml"));
        assert!(kv_data_path("smith").ends_with("kv/data/smith.json"));
    }

    // ---------------------------------------------------------------------
    // state / tensor schema helpers
    // ---------------------------------------------------------------------

    #[test]
    fn tensor_schema_path_layout() {
        let p = tensor_schema_path("crewu");
        assert!(p.ends_with("state/schemas/crewu.yaml"));
        assert!(p.starts_with(mx_home()));
    }

    // ---------------------------------------------------------------------
    // memory / surreal helpers
    // ---------------------------------------------------------------------

    #[test]
    fn surreal_root_default() {
        let home = mx_home().clone();
        let r = surreal_root_with(None, &home);
        assert_eq!(r, home.join("memory").join("surreal"));
    }

    #[test]
    fn surreal_root_respects_override() {
        let home = mx_home().clone();
        let r = surreal_root_with(Some("/custom/surreal"), &home);
        assert_eq!(r, PathBuf::from("/custom/surreal"));
    }

    #[test]
    fn surreal_root_empty_override_is_default() {
        let home = mx_home().clone();
        let r = surreal_root_with(Some(""), &home);
        assert_eq!(r, home.join("memory").join("surreal"));
    }

    #[test]
    fn memory_seed_dirs_under_mx_home() {
        let home = mx_home();
        assert!(memory_seed_agents_dir().starts_with(home));
        assert!(memory_seed_agents_dir().ends_with("memory/seed/agents"));
        assert!(memory_seed_knowledge_dir().starts_with(home));
        assert!(memory_seed_knowledge_dir().ends_with("memory/seed/knowledge"));
    }

    #[test]
    fn legacy_agents_dir_with_uses_supplied_home() {
        let home = PathBuf::from("/tmp/some-test-home");
        let r = legacy_agents_dir_with(&home);
        assert_eq!(r, home.join("agents"));
    }

    #[test]
    fn legacy_agents_dir_public_wrapper_matches_seam() {
        // The public wrapper should produce the same path as calling the
        // _with helper against the cached mx_home().
        assert_eq!(legacy_agents_dir(), legacy_agents_dir_with(mx_home()));
    }

    // ---------------------------------------------------------------------
    // fastembed cache
    // ---------------------------------------------------------------------

    #[test]
    fn fastembed_cache_default_uses_xdg() {
        let home = mx_home().clone();
        let xdg = PathBuf::from("/xdg/cache");
        let r = fastembed_cache_dir_with(None, || Some(xdg.clone()), &home);
        assert_eq!(r, xdg.join("fastembed"));
    }

    #[test]
    fn fastembed_cache_isolate_uses_mx_home() {
        let home = mx_home().clone();
        let xdg = PathBuf::from("/xdg/cache");
        let r = fastembed_cache_dir_with(Some("1"), || Some(xdg.clone()), &home);
        assert_eq!(r, home.join("memory").join("embed"));
    }

    #[test]
    fn fastembed_cache_isolate_empty_is_default() {
        let home = mx_home().clone();
        let xdg = PathBuf::from("/xdg/cache");
        let r = fastembed_cache_dir_with(Some(""), || Some(xdg.clone()), &home);
        assert_eq!(r, xdg.join("fastembed"));
    }

    #[test]
    fn fastembed_cache_no_xdg_falls_back() {
        let home = mx_home().clone();
        let r = fastembed_cache_dir_with(None, || None, &home);
        assert_eq!(r, PathBuf::from(".fastembed_cache"));
    }

    // ---------------------------------------------------------------------
    // claude external paths
    // ---------------------------------------------------------------------

    #[test]
    #[serial]
    fn claude_paths_under_home() {
        // `claude_projects_dir()` reads `MX_CLAUDE_PROJECTS_DIR` for
        // hermetic test overrides; clear it here so we observe the
        // real default.
        let prev = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        // SAFETY: process-wide env mutation, serialized via #[serial]
        // so concurrent codex tests don't race on the override.
        unsafe {
            std::env::remove_var("MX_CLAUDE_PROJECTS_DIR");
        }
        let home = dirs::home_dir().unwrap();
        assert_eq!(claude_projects_dir(), home.join(".claude").join("projects"));
        assert_eq!(claude_config_path(), home.join(".claude.json"));
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v);
            }
        }
    }

    #[test]
    fn claude_dir_is_dot_claude() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(claude_dir(), home.join(".claude"));
    }

    #[test]
    #[serial]
    fn claude_subagents_dir_layout() {
        // Calls `claude_projects_dir()` transitively, which reads
        // `MX_CLAUDE_PROJECTS_DIR`; clear it for the default-case
        // assertion here.
        let prev = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        // SAFETY: process-wide env mutation, serialized via #[serial].
        unsafe {
            std::env::remove_var("MX_CLAUDE_PROJECTS_DIR");
        }
        let home = dirs::home_dir().unwrap();
        let p = claude_subagents_dir("-home-charlie-recipes-coryzibell-mx", "abc-123");
        let expected = home
            .join(".claude")
            .join("projects")
            .join("-home-charlie-recipes-coryzibell-mx")
            .join("abc-123")
            .join("subagents");
        assert_eq!(p, expected);
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v);
            }
        }
    }

    #[test]
    fn claude_sessions_dir_layout() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(claude_sessions_dir(), home.join(".claude").join("sessions"));
    }

    #[test]
    fn claude_history_jsonl_layout() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            claude_history_jsonl(),
            home.join(".claude").join("history.jsonl")
        );
    }

    #[test]
    fn claude_mcp_logs_dir_layout() {
        let home = dirs::home_dir().unwrap();
        let p = claude_mcp_logs_dir("-home-charlie-recipes-coryzibell-mx");
        let expected = home
            .join(".cache")
            .join("claude-cli-nodejs")
            .join("-home-charlie-recipes-coryzibell-mx");
        assert_eq!(p, expected);
        // Per-server `mcp-logs-*` subdirs are walked at call time, not joined here.
        assert!(!p.to_string_lossy().contains("mcp-logs-"));
    }

    #[test]
    fn tmp_claude_tasks_dir_layout() {
        let p = tmp_claude_tasks_dir(
            1002,
            "-home-charlie",
            "c3744b8d-5719-4df2-924f-707945438494",
        );
        let expected = PathBuf::from("/tmp/claude-1002")
            .join("-home-charlie")
            .join("c3744b8d-5719-4df2-924f-707945438494")
            .join("tasks");
        assert_eq!(p, expected);
    }

    #[test]
    fn wonka_vault_archives_dir_layout() {
        // wonka_vault_archives_dir is rooted at ~/.wonka, not $MX_HOME.
        // Match against the home dir directly (per the doc comment).
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            wonka_vault_archives_dir(),
            home.join(".wonka").join("vault").join("archives")
        );
    }

    // -----------------------------------------------------------------
    // legacy_memory_path_set -- decision 9 (MX_MEMORY_PATH rename)
    // -----------------------------------------------------------------

    #[test]
    fn legacy_memory_path_unset_returns_false() {
        assert!(!legacy_memory_path_set(None));
    }

    #[test]
    fn legacy_memory_path_empty_returns_false() {
        assert!(!legacy_memory_path_set(Some("")));
    }

    #[test]
    fn legacy_memory_path_set_returns_true() {
        assert!(legacy_memory_path_set(Some("/anything")));
    }
}
