//! Operator-facing notices emitted at the start of every `mx codex *`
//! invocation.
//!
//! Today there's exactly one notice — the vault-present nag — but this
//! module is the right home for any future "before you do anything else,
//! the codex would like to point out..." messages so the dispatch sites
//! stay tidy.

use std::path::Path;
use std::sync::OnceLock;

/// Emit the vault-present warning at most once per process. Idempotent
/// across repeated calls — the `OnceLock` guarantees a single fire even
/// if `mx codex archive` and `mx codex export` both run in the same
/// process.
///
/// Skipped when:
/// - the vault directory does not exist (clean machines stay silent), or
/// - the vault directory exists but contains no `session-*` snapshots.
///
/// Callers that are already in the act of backfilling pass
/// `suppress = true` so the operator doesn't get nagged about the very
/// thing they're fixing.
pub(crate) fn warn_if_vault_present(suppress: bool) {
    if suppress {
        return;
    }
    static FIRED: OnceLock<()> = OnceLock::new();
    if FIRED.get().is_some() {
        return;
    }
    let vault = crate::paths::wonka_vault_archives_dir();
    if !vault_has_snapshots(&vault) {
        return;
    }
    eprintln!(
        "note: {} contains historical session data not in the codex.\n      \
         Run `mx codex archive --backfill` to ingest, then remove the vault directory.",
        vault.display()
    );
    let _ = FIRED.set(());
}

/// True iff `vault_path` exists and has at least one `session-*`
/// subdirectory. Anything else (missing dir, empty dir, dir with only
/// non-snapshot junk) returns false — those states represent "no work to
/// surface" and shouldn't generate noise.
fn vault_has_snapshots(vault_path: &Path) -> bool {
    let entries = match std::fs::read_dir(vault_path) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        match entry.file_name().to_str() {
            Some(name) if name.starts_with("session-") => return true,
            _ => continue,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn vault_missing_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        assert!(!vault_has_snapshots(&bogus));
    }

    #[test]
    fn vault_present_but_empty_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("empty-vault");
        fs::create_dir_all(&vault).unwrap();
        assert!(!vault_has_snapshots(&vault));
    }

    #[test]
    fn vault_with_only_junk_files_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("junk-vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("README.txt"), "hi").unwrap();
        // A directory that doesn't match the session-* prefix.
        fs::create_dir_all(vault.join("other-dir")).unwrap();
        assert!(!vault_has_snapshots(&vault));
    }

    #[test]
    fn vault_with_one_snapshot_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("real-vault");
        fs::create_dir_all(vault.join("session-20260311-202812-631046")).unwrap();
        assert!(vault_has_snapshots(&vault));
    }
}
