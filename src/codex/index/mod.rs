//! By-project index for the codex.
//!
//! Maintains a `~/.wonka/codex/by-project/<basename-slug>/` directory of
//! pointers (symlinks or files — implementer's choice; this PR stubs the
//! API only) into the time-indexed flat session storage.
//!
//! The index is regenerable from manifests on every archive run; readers
//! must stale-check before trusting it. PR 2 will integrate write paths;
//! PR 3 will integrate read paths.

use anyhow::Result;
use std::path::PathBuf;
use thiserror::Error;

/// In-memory handle to the on-disk by-project index.
///
/// Constructed via [`ProjectIndex::open`]. Stays empty in this PR; the
/// internal layout is intentionally private so PR 2 can choose between a
/// symlink-based or pointer-file-based representation without churning
/// callers.
#[derive(Debug, Default)]
pub struct ProjectIndex {
    /// Cached entries, populated by `rebuild_from_manifests`. Empty in PR 1.
    entries: Vec<ProjectEntry>,
}

/// One project's entry in the index: its absolute path on disk, its
/// basename-slug (the human-friendly key used by `--project mx`), and the
/// time-indexed codex directories that archive sessions for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    /// The absolute path of the project on disk (matches `manifest.project_path`).
    pub absolute_path: PathBuf,
    /// The basename-slug (e.g. `mx`, `wonka`) — last segment of `absolute_path`.
    pub basename_slug: String,
    /// Paths into `~/.wonka/codex/<YYYY-MM-DD-HHMMSS>-<short-uuid>/` for
    /// every archived session belonging to this project.
    pub session_archive_paths: Vec<PathBuf>,
}

impl ProjectIndex {
    /// Open the index at `~/.wonka/codex/by-project/`, creating it if absent.
    ///
    /// PR 2 will integrate this when archive writes the index. Until then,
    /// this returns [`IndexError::NotImplemented`] so a stray production
    /// caller surfaces as a typed error rather than a panic.
    pub fn open() -> Result<Self> {
        Err(IndexError::NotImplemented { method: "open" }.into())
    }

    /// Regenerate the index from all manifests under codex/. Call after archive runs.
    ///
    /// PR 2 will integrate this when archive writes the index. Until then,
    /// this returns [`IndexError::NotImplemented`].
    pub fn rebuild_from_manifests(&mut self) -> Result<()> {
        Err(IndexError::NotImplemented {
            method: "rebuild_from_manifests",
        }
        .into())
    }

    /// Look up a project by absolute path, raw slug, or basename. Returns
    /// the matched entry, or an [`IndexError::AmbiguousProject`] error if a
    /// basename matches multiple absolute paths.
    ///
    /// PR 3 will integrate this when export reads the index. Until then,
    /// this returns [`IndexError::NotImplemented`].
    pub fn lookup(&self, _query: &str) -> Result<ProjectEntry> {
        Err(IndexError::NotImplemented { method: "lookup" }.into())
    }

    /// Returns true if the on-disk index is stale relative to the manifest
    /// timestamps. Readers MUST call this before trusting the index.
    ///
    /// PR 3 will integrate this when export reads the index. Until then,
    /// this returns [`IndexError::NotImplemented`].
    pub fn is_stale(&self) -> Result<bool> {
        Err(IndexError::NotImplemented { method: "is_stale" }.into())
    }

    /// Number of entries currently held in memory. Provided as a smoke-test
    /// hook so PR 1 can assert the type is constructable without exercising
    /// the not-yet-implemented methods.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Errors raised by the by-project index.
///
/// `AmbiguousProject` and `NotImplemented` are enumerated in PR 1; PRs 2 and
/// 3 will extend this enum as concrete failure modes are wired up. The
/// variant payloads are stable and ready for `--project` disambiguation
/// messages and for surfacing stub-call sites as typed errors instead of
/// panics.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("ambiguous project query '{query}' matches multiple paths: {matches:?}")]
    AmbiguousProject {
        query: String,
        matches: Vec<PathBuf>,
    },
    #[error("ProjectIndex::{method} is not yet implemented (wired up in a later PR)")]
    NotImplemented { method: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Smoke tests: confirm the module compiles, the public types are
    // constructable, and the `unimplemented!()` methods are reachable from
    // the public API surface (i.e. no `pub` was forgotten). Actual behavior
    // lands in PRs 2 and 3.
    // ---------------------------------------------------------------------

    #[test]
    fn project_index_default_is_empty() {
        let idx = ProjectIndex::default();
        assert_eq!(idx.entry_count(), 0);
    }

    #[test]
    fn project_entry_constructable() {
        let entry = ProjectEntry {
            absolute_path: PathBuf::from("/home/charlie/recipes/coryzibell/mx"),
            basename_slug: "mx".to_string(),
            session_archive_paths: vec![PathBuf::from(
                "/home/charlie/.wonka/codex/2026-04-29-143022-c3744b8d",
            )],
        };
        assert_eq!(entry.basename_slug, "mx");
        assert_eq!(entry.session_archive_paths.len(), 1);
    }

    #[test]
    fn index_error_ambiguous_renders_query_and_matches() {
        let err = IndexError::AmbiguousProject {
            query: "mx".to_string(),
            matches: vec![PathBuf::from("/home/a/mx"), PathBuf::from("/home/b/mx")],
        };
        let msg = format!("{}", err);
        assert!(msg.contains("'mx'"), "rendered message: {}", msg);
        assert!(msg.contains("/home/a/mx"), "rendered message: {}", msg);
        assert!(msg.contains("/home/b/mx"), "rendered message: {}", msg);
    }
}
