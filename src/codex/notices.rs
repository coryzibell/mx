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

    /// Regression guard: the warning must reference the canonical CLI verb
    /// (`mx codex archive --backfill`). C1 from PR 272 review caught a
    /// mismatch where the string said one thing and the CLI said another.
    /// Now that the verb IS `archive`, we just assert consistency.
    #[test]
    fn warning_text_uses_archive_subcommand() {
        // Build a vault with one snapshot so the warning fires.
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("real-vault");
        fs::create_dir_all(vault.join("session-20260311-202812-631046")).unwrap();
        assert!(vault_has_snapshots(&vault));

        // Format the message exactly as `warn_if_vault_present` would.
        // We don't go through `warn_if_vault_present` itself because its
        // `OnceLock` makes the test order-sensitive across the suite --
        // this is an equivalent literal-string assertion.
        let msg = format!(
            "note: {} contains historical session data not in the codex.\n      \
             Run `mx codex archive --backfill` to ingest, then remove the vault directory.",
            vault.display()
        );
        assert!(
            msg.contains("mx codex archive --backfill"),
            "vault-warning string must reference `mx codex archive --backfill`: {msg}"
        );

        // Assert the function body uses the canonical verb.
        let src = include_str!("notices.rs");
        let func_start = src.find("pub(crate) fn warn_if_vault_present").unwrap();
        let func_end = src[func_start..].find("\n}\n").unwrap() + func_start;
        let func_body = &src[func_start..func_end];
        assert!(
            func_body.contains("mx codex archive --backfill"),
            "warn_if_vault_present must emit `mx codex archive --backfill`"
        );
        // The old verb should no longer appear in the function body.
        assert!(
            !func_body.contains("mx codex save"),
            "warn_if_vault_present must not reference deprecated `mx codex save`"
        );
    }
}
