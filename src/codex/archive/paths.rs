//! Archive-folder naming utilities.
//!
//! These helpers concern themselves with the *names* of the per-session
//! directories under `~/.wonka/codex/`, not with codex source paths
//! (which live in `crate::paths`). They handle the
//! `<YYYY-MM-DD-HHMMSS>-<short-uuid>[.N]` convention and its `.N`
//! collision suffix.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Pick a non-colliding archive directory under `codex_dir`.
///
/// If `codex_dir/base_name` does not exist, return it. Otherwise scan
/// siblings for the highest existing `.N` suffix and return
/// `base_name.{N+1}`.
pub(crate) fn determine_archive_dir(codex_dir: &Path, base_name: &str) -> Result<PathBuf> {
    let base_dir = codex_dir.join(base_name);

    if !base_dir.exists() {
        return Ok(base_dir);
    }

    // Find highest incremental number
    let mut max_incremental = 0;
    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with(base_name)
            && let Some(suffix) = name_str.strip_prefix(base_name)
            && let Some(num_str) = suffix.strip_prefix('.')
            && let Ok(num) = num_str.parse::<u32>()
        {
            max_incremental = max_incremental.max(num);
        }
    }

    Ok(codex_dir.join(format!("{}.{}", base_name, max_incremental + 1)))
}

/// Decompose an archive directory name into `(short_id, incremental)`.
///
/// Examples:
/// - `2026-01-03-141500-abc12345`   -> `("abc12345", 0)`
/// - `2026-01-03-141500-abc12345.2` -> `("abc12345", 2)`
pub(crate) fn parse_archive_name(name: &str) -> (String, u32) {
    if let Some(dot_pos) = name.rfind('.')
        && let Ok(num) = name[dot_pos + 1..].parse::<u32>()
    {
        let base = &name[..dot_pos];
        let short_id = extract_short_id(base);
        return (short_id, num);
    }

    (extract_short_id(name), 0)
}

/// Pull the short-id segment from an archive directory name.
///
/// Precondition: `name` is the post-`.N` base from `parse_archive_name`,
/// e.g. `2026-01-03-141500-abc12345`. The short id is the trailing `-`
/// segment.
///
/// S5: we now also validate the trailing segment is non-empty and made
/// of alphanumerics (the short uuids Claude writes are 8 hex chars; we
/// accept the broader alnum class so the parser tolerates future ID
/// schemes). On invalid input the function falls back to the original
/// `name` — preserving the previous behavior for malformed archive
/// directories so a single bad name doesn't break a `list` traversal.
fn extract_short_id(name: &str) -> String {
    let candidate = name.split('-').next_back().unwrap_or(name);
    if candidate.is_empty() || !candidate.chars().all(|c| c.is_ascii_alphanumeric()) {
        return name.to_string();
    }
    candidate.to_string()
}

/// Strip the `.N` incremental suffix from an archive directory name, if any.
pub(crate) fn get_base_archive_name(name: &str) -> String {
    if let Some(dot_pos) = name.rfind('.')
        && name[dot_pos + 1..].parse::<u32>().is_ok()
    {
        return name[..dot_pos].to_string();
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_archive_name_no_suffix() {
        let (short, incr) = parse_archive_name("2026-01-03-141500-abc12345");
        assert_eq!(short, "abc12345");
        assert_eq!(incr, 0);
    }

    #[test]
    fn parse_archive_name_with_suffix() {
        let (short, incr) = parse_archive_name("2026-01-03-141500-abc12345.7");
        assert_eq!(short, "abc12345");
        assert_eq!(incr, 7);
    }

    #[test]
    fn get_base_archive_name_strips_suffix() {
        assert_eq!(
            get_base_archive_name("2026-01-03-141500-abc12345.7"),
            "2026-01-03-141500-abc12345"
        );
        assert_eq!(
            get_base_archive_name("2026-01-03-141500-abc12345"),
            "2026-01-03-141500-abc12345"
        );
    }

    #[test]
    fn extract_short_id_validates_trailing_segment() {
        // Happy path: hex-shaped id.
        assert_eq!(extract_short_id("2026-01-03-141500-abc12345"), "abc12345");
        // Empty trailing segment (name ends in '-') falls back to the
        // whole name rather than yielding an empty string.
        assert_eq!(extract_short_id("2026-01-03-141500-"), "2026-01-03-141500-");
        // Non-alnum trailing segment also falls back.
        assert_eq!(
            extract_short_id("2026-01-03-141500-bad/path"),
            "2026-01-03-141500-bad/path"
        );
        // No '-' at all: trailing segment IS the name; if alnum, return as-is.
        assert_eq!(extract_short_id("abc12345"), "abc12345");
    }

    #[test]
    fn determine_archive_dir_picks_first_free() {
        let tmp = tempfile::tempdir().unwrap();
        let base = "2026-04-29-120000-deadbeef";

        // No existing dir → returns base
        let p = determine_archive_dir(tmp.path(), base).unwrap();
        assert_eq!(p, tmp.path().join(base));

        // Create base, then expect .1
        fs::create_dir(tmp.path().join(base)).unwrap();
        let p = determine_archive_dir(tmp.path(), base).unwrap();
        assert_eq!(p, tmp.path().join(format!("{}.1", base)));

        // Create .1 → expect .2
        fs::create_dir(tmp.path().join(format!("{}.1", base))).unwrap();
        let p = determine_archive_dir(tmp.path(), base).unwrap();
        assert_eq!(p, tmp.path().join(format!("{}.2", base)));
    }
}
