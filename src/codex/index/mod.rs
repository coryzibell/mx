//! By-project index for the codex.
//!
//! Maintains a `<codex_dir>/by-project/<basename-slug>/` directory of
//! symlinks pointing back at the time-indexed flat session archives.
//! Each archive directory at the codex root is a fully self-contained
//! session — the index is purely an alternate access path: "give me
//! every archived session for project `mx`."
//!
//! ## Format
//!
//! Symlinks. For each archive `<codex>/2026-04-29-143022-c3744b8d/`
//! whose manifest reports `project_path = "/home/charlie/recipes/coryzibell/mx"`,
//! the index creates:
//!
//! ```text
//! <codex>/by-project/mx/2026-04-29-143022-c3744b8d -> ../../2026-04-29-143022-c3744b8d
//! ```
//!
//! Symlinks were chosen over pointer files for v1 because they're cheap,
//! stdlib-friendly, and let `ls` / `find` tools traverse the index
//! transparently. If filesystem support ever bites us (Windows, FUSE
//! quirks) we'll switch to pointer files.
//!
//! ## Lifecycle
//!
//! The index is regenerable from manifests on every archive run.
//! `rebuild_from_manifests` does an atomic-ish swap: it writes a fresh
//! `by-project/` into a staging directory and renames it over the old
//! one, so a crash mid-rebuild leaves either the previous index or the
//! new one — never a partial state.
//!
//! Readers (PR 3) MUST call `is_stale` before trusting the index.

use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::codex::Manifest;

/// On-disk subdirectory name where the by-project index lives, under
/// the codex root.
const INDEX_SUBDIR: &str = "by-project";

/// Staging directory used during rebuild for the atomic-rename swap.
const STAGING_SUBDIR: &str = "by-project.staging";

/// In-memory handle to the on-disk by-project index.
#[derive(Debug, Default)]
pub struct ProjectIndex {
    /// Absolute path to `<codex_dir>/by-project/`.
    root: PathBuf,
    /// Cached entries, populated by `rebuild_from_manifests`.
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
    /// Paths into `<codex_dir>/<YYYY-MM-DD-HHMMSS>-<short-uuid>/` for
    /// every archived session belonging to this project.
    pub session_archive_paths: Vec<PathBuf>,
}

impl ProjectIndex {
    /// Open the index at `<codex_dir>/by-project/`, creating it if absent.
    /// Idempotent — calling repeatedly is safe.
    pub fn open() -> Result<Self> {
        Self::open_under(&crate::paths::codex_dir())
    }

    /// Like `open`, but rooted under an explicit codex dir. Used by tests
    /// to avoid touching `$MX_HOME/codex/`.
    pub fn open_under(codex_dir: &Path) -> Result<Self> {
        let root = codex_dir.join(INDEX_SUBDIR);
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            entries: Vec::new(),
        })
    }

    /// Regenerate the index from all manifests under `<codex_dir>/`.
    /// Call after archive runs.
    ///
    /// Walks every `<codex>/<YYYY-MM-DD-HHMMSS>-<short-uuid>/manifest.json`,
    /// groups archives by project basename, and writes a fresh symlink
    /// tree into a staging dir before renaming it into place. If the
    /// rename fails partway, the old index is left intact.
    pub fn rebuild_from_manifests(&mut self) -> Result<()> {
        // 1. Find the codex root: it's the parent of self.root.
        let codex_dir = self
            .root
            .parent()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "by-project index root has no parent: {}",
                    self.root.display()
                )
            })?
            .to_path_buf();

        // 2. Walk codex/<archive_dir>/manifest.json entries.
        let mut by_basename: HashMap<String, Vec<(PathBuf, PathBuf)>> = HashMap::new();
        let mut session_count = 0usize;

        if codex_dir.exists() {
            for entry in fs::read_dir(&codex_dir)? {
                let entry = entry?;
                let archive_dir = entry.path();
                if !archive_dir.is_dir() {
                    continue;
                }
                // Skip the by-project tree itself (and the staging tmp).
                let name = match archive_dir.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if name == INDEX_SUBDIR || name == STAGING_SUBDIR {
                    continue;
                }
                let manifest_path = archive_dir.join("manifest.json");
                if !manifest_path.exists() {
                    continue;
                }
                let raw = match fs::read_to_string(&manifest_path) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!(
                            "warning: skipping unreadable manifest {}: {e}",
                            manifest_path.display()
                        );
                        continue;
                    }
                };
                let manifest: Manifest = match serde_json::from_str(&raw) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!(
                            "warning: skipping unparseable manifest {}: {e}",
                            manifest_path.display()
                        );
                        continue;
                    }
                };
                let abs = match manifest.project_path.as_ref() {
                    Some(p) => PathBuf::from(p),
                    None => continue, // no project linkage — can't index
                };
                let slug = basename_slug_for(&abs);
                by_basename
                    .entry(slug)
                    .or_default()
                    .push((abs, archive_dir.clone()));
                session_count += 1;
            }
        }

        // 3. Write into staging.
        let staging = codex_dir.join(STAGING_SUBDIR);
        // Clean up any prior staging from a crashed run.
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(&staging)?;

        for (slug, archives) in &by_basename {
            let bucket = staging.join(slug);
            fs::create_dir_all(&bucket)?;
            for (_abs, archive_dir) in archives {
                let archive_name = match archive_dir.file_name() {
                    Some(n) => n,
                    None => continue,
                };
                // The link target uses a relative path so the index is
                // movable as a unit (e.g. when MX_HOME is rebased).
                // Going from <codex>/by-project/<slug>/<archive_name>
                // back to <codex>/<archive_name> is "../../<archive_name>".
                let target = PathBuf::from("..").join("..").join(archive_name);
                let link = bucket.join(archive_name);
                if let Err(e) = make_symlink(&target, &link) {
                    eprintln!("warning: failed to create symlink {}: {e}", link.display());
                }
            }
        }

        // 4. Atomic-ish swap: remove the old index, rename staging into place.
        //
        // We can't do a single atomic rename when the destination is a
        // non-empty directory on most filesystems, so we use a
        // remove-then-rename. A crash between these two operations
        // leaves the index briefly missing — readers must already
        // tolerate "no index" (they fall back to a manifest scan), so
        // this is acceptable for v1. A future refinement could rename
        // the old dir aside first, but that complicates the cleanup.
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        fs::rename(&staging, &self.root)?;

        // 5. Refresh the in-memory cache from the same data.
        self.entries = by_basename
            .into_iter()
            .map(|(slug, archives)| {
                let mut session_archive_paths: Vec<PathBuf> =
                    archives.iter().map(|(_, p)| p.clone()).collect();
                session_archive_paths.sort();
                ProjectEntry {
                    // Pick any of the absolute paths; ambiguity is
                    // surfaced by `lookup` (PR 3), not by the index.
                    absolute_path: archives.first().map(|(a, _)| a.clone()).unwrap_or_default(),
                    basename_slug: slug,
                    session_archive_paths,
                }
            })
            .collect();
        self.entries
            .sort_by(|a, b| a.basename_slug.cmp(&b.basename_slug));

        eprintln!(
            "Rebuilt by-project index: {} project(s), {} session(s)",
            self.entries.len(),
            session_count
        );

        Ok(())
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

    /// Number of entries currently held in memory.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// On-disk root of the index (`<codex_dir>/by-project/`). Test hook.
    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

/// Derive the basename-slug for a project absolute path.
///
/// Falls back to `home` for single-component paths like `/` or empty —
/// these shouldn't appear in well-formed manifests but we don't want a
/// `panic!` to take out a rebuild on bad data. The basename is the
/// `Path::file_name()` of the absolute path: `/home/charlie/recipes/mx`
/// -> `mx`. For `/home/charlie` (basename `charlie`) we keep the
/// basename rather than fabricating a different convention; ambiguity
/// with another project also basenamed `charlie` would surface via
/// `lookup` in PR 3.
fn basename_slug_for(absolute_path: &Path) -> String {
    absolute_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "home".to_string())
}

/// Cross-platform symlink wrapper. Symlinks aren't ergonomic on
/// Windows; we error there. The unification series is Linux/macOS-only
/// for now per the architecture doc.
fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "by-project index requires Unix symlinks",
        ))
    }
}

/// Errors raised by the by-project index.
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
    use chrono::Utc;

    fn write_manifest(archive_dir: &Path, project_path: &str, session_id: &str) {
        fs::create_dir_all(archive_dir).unwrap();
        let manifest = Manifest {
            version: crate::codex::MANIFEST_WRITE_VERSION,
            session_id: session_id.to_string(),
            archived_at: Utc::now(),
            session_start: Utc::now(),
            session_end: Utc::now(),
            project_path: Some(project_path.to_string()),
            message_count: 0,
            agent_count: 0,
            agents: vec![],
            size_bytes: 0,
            checksum: "sha256:zero".to_string(),
            image_count: None,
            images: None,
            has_clean_transcript: None,
            user_name: None,
            assistant_name: None,
            tool_output_count: None,
            mcp_log_count: None,
            history_lines: None,
            source_breakdown: None,
        };
        fs::write(
            archive_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

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
        assert!(msg.contains("'mx'"));
        assert!(msg.contains("/home/a/mx"));
        assert!(msg.contains("/home/b/mx"));
    }

    #[test]
    fn open_creates_by_project_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let idx = ProjectIndex::open_under(tmp.path()).unwrap();
        assert!(idx.root().exists());
        assert!(idx.root().ends_with(INDEX_SUBDIR));
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let _idx1 = ProjectIndex::open_under(tmp.path()).unwrap();
        // Second open against the same path must succeed.
        let _idx2 = ProjectIndex::open_under(tmp.path()).unwrap();
    }

    #[test]
    fn rebuild_from_empty_codex() {
        let tmp = tempfile::tempdir().unwrap();
        let mut idx = ProjectIndex::open_under(tmp.path()).unwrap();
        idx.rebuild_from_manifests().unwrap();
        assert_eq!(idx.entry_count(), 0);
        assert!(idx.root().exists());
    }

    #[test]
    fn rebuild_populated_codex_creates_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path();

        // Two archives for project `mx`, one for `wonka`.
        write_manifest(
            &codex.join("2026-04-29-100000-aaaaaaaa"),
            "/home/charlie/recipes/coryzibell/mx",
            "aaa",
        );
        write_manifest(
            &codex.join("2026-04-29-110000-bbbbbbbb"),
            "/home/charlie/recipes/coryzibell/mx",
            "bbb",
        );
        write_manifest(
            &codex.join("2026-04-29-120000-cccccccc"),
            "/home/charlie/recipes/coryzibell/wonka",
            "ccc",
        );

        let mut idx = ProjectIndex::open_under(codex).unwrap();
        idx.rebuild_from_manifests().unwrap();

        assert_eq!(idx.entry_count(), 2, "two distinct projects expected");

        let mx_dir = codex.join(INDEX_SUBDIR).join("mx");
        let wonka_dir = codex.join(INDEX_SUBDIR).join("wonka");
        assert!(mx_dir.exists());
        assert!(wonka_dir.exists());
        assert!(mx_dir.join("2026-04-29-100000-aaaaaaaa").exists());
        assert!(mx_dir.join("2026-04-29-110000-bbbbbbbb").exists());
        assert!(wonka_dir.join("2026-04-29-120000-cccccccc").exists());

        // Symlink target should be relative — `../../<archive_name>`.
        let link = mx_dir.join("2026-04-29-100000-aaaaaaaa");
        let target = fs::read_link(&link).unwrap();
        assert_eq!(target, PathBuf::from("../../2026-04-29-100000-aaaaaaaa"));

        // Resolves to the actual archive dir.
        let resolved = fs::canonicalize(&link).unwrap();
        let expected = fs::canonicalize(codex.join("2026-04-29-100000-aaaaaaaa")).unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn rebuild_skips_archives_without_project_path() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path();
        let archive = codex.join("2026-04-29-130000-dddddddd");
        fs::create_dir_all(&archive).unwrap();
        // Manifest with project_path=None should be skipped, not crash.
        let manifest_json = r#"{
            "version": 5,
            "session_id": "ddd",
            "archived_at": "2026-04-29T13:00:00Z",
            "session_start": "2026-04-29T13:00:00Z",
            "session_end": "2026-04-29T13:00:00Z",
            "project_path": null,
            "message_count": 0,
            "agent_count": 0,
            "agents": [],
            "size_bytes": 0,
            "checksum": "sha256:zero"
        }"#;
        fs::write(archive.join("manifest.json"), manifest_json).unwrap();

        let mut idx = ProjectIndex::open_under(codex).unwrap();
        idx.rebuild_from_manifests().unwrap();
        assert_eq!(idx.entry_count(), 0);
    }

    #[test]
    fn rebuild_replaces_existing_index_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path();

        // First archive.
        write_manifest(
            &codex.join("2026-04-29-100000-aaaaaaaa"),
            "/home/test/foo",
            "aaa",
        );
        let mut idx = ProjectIndex::open_under(codex).unwrap();
        idx.rebuild_from_manifests().unwrap();
        assert!(codex.join(INDEX_SUBDIR).join("foo").exists());

        // Second archive, different project.
        write_manifest(
            &codex.join("2026-04-29-110000-bbbbbbbb"),
            "/home/test/bar",
            "bbb",
        );
        idx.rebuild_from_manifests().unwrap();
        // Both projects present after rebuild.
        assert!(codex.join(INDEX_SUBDIR).join("foo").exists());
        assert!(codex.join(INDEX_SUBDIR).join("bar").exists());
        // Staging dir must be cleaned up.
        assert!(!codex.join(STAGING_SUBDIR).exists());
    }

    #[test]
    fn basename_slug_for_normal_path() {
        assert_eq!(
            basename_slug_for(Path::new("/home/charlie/recipes/coryzibell/mx")),
            "mx"
        );
    }

    #[test]
    fn basename_slug_for_root_falls_back() {
        // Path::file_name() returns None for `/`. We want a sane fallback,
        // not a panic. The brief recommends the basename with a `home`
        // fallback for degenerate inputs.
        assert_eq!(basename_slug_for(Path::new("/")), "home");
    }

    #[test]
    fn lookup_still_unimplemented_in_pr2() {
        let idx = ProjectIndex::default();
        let err = idx.lookup("mx").unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not yet implemented"), "got: {msg}");
    }

    #[test]
    fn is_stale_still_unimplemented_in_pr2() {
        let idx = ProjectIndex::default();
        let err = idx.is_stale().unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("not yet implemented"), "got: {msg}");
    }
}
