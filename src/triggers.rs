//! Triggered-memory matching engine and session-scoped fired-state (Issue #246, PR3).
//!
//! Two responsibilities live here, kept deliberately separate so the matcher is a
//! pure, exhaustively testable function and the IO (file locking) is isolated:
//!
//! 1. **The matcher** (`match_entries`, `match_triggers`): given a raw message and
//!    a set of stored triggers, decide which triggers fire. The pipeline is
//!    `normalize → tokenize → stem → contiguous-sequence match` and is described
//!    on [`match_triggers`].
//!
//! 2. **Session fired-state** (`FiredStore`): a flock-guarded JSON file recording
//!    which memory IDs have already fired this session, so a triggered memory
//!    fires exactly ONCE per session (the immune-system "one-shot per encounter"
//!    property from #246). The read-modify-write is performed inside a single
//!    `flock` critical section so two concurrent `trigger-check` invocations
//!    (e.g. two hooks racing) can never both claim the same memory.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use fs2::FileExt;
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};

use crate::knowledge::normalize_trigger;

/// Default location of the session fired-state file. Overridable via the
/// `MX_TRIGGER_FIRED_PATH` env var (mirrors `MX_KV_DATA` for the KV store) so
/// tests get an isolated temp file and so a hook can scope it per session.
const DEFAULT_FIRED_PATH: &str = "/tmp/wonka-triggered-fired.json";

/// Environment override for the fired-state file path.
const FIRED_PATH_ENV: &str = "MX_TRIGGER_FIRED_PATH";

/// Resolve the fired-state file path: env override if set & non-empty, else the
/// default. Empty env value is treated as unset (same convention as `MX_KV_DATA`).
pub fn fired_path() -> PathBuf {
    match std::env::var(FIRED_PATH_ENV) {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_FIRED_PATH),
    }
}

/// Tokenize a raw string into stemmed tokens for trigger matching.
///
/// Steps, in order, so author-time and match-time agree exactly:
///   1. `normalize_trigger` — NFC canonicalize + lowercase + whitespace-collapse
///      (the single shared normalizer; see `src/knowledge.rs`). Returns no tokens
///      for empty/whitespace-only input.
///   2. Split on Unicode word boundaries: any run of non-alphanumeric characters
///      separates tokens. This is what makes matching **word-boundary** — "ai"
///      tokenizes "said" as `["said"]`, never exposing a bare "ai" token, so the
///      "ai" trigger cannot fire on "said".
///   3. Porter/Snowball English stem each token, so "diabetes" and "diabetic"
///      collapse to the same stem and a "diabetes" trigger fires on "diabetic".
fn stem_tokens(raw: &str) -> Vec<String> {
    // Normalize first; empty result -> no tokens.
    let Some(normalized) = normalize_trigger(raw) else {
        return Vec::new();
    };

    // Unicode word-boundary tokenization: split on non-alphanumeric runs.
    // `char::is_alphanumeric` is Unicode-aware, so accented letters and
    // non-Latin scripts stay inside tokens.
    let stemmer = Stemmer::create(Algorithm::English);
    normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty())
        .map(|tok| stemmer.stem(tok).into_owned())
        .collect()
}

/// Return true if `needle` appears as a contiguous subsequence of `haystack`.
///
/// Empty `needle` never matches (a trigger that tokenizes to nothing is inert).
/// Single-token needles match when that one stem appears anywhere as a whole
/// token. Multi-token (phrase) needles require the tokens to appear **adjacent
/// and in order** — this is what makes "blood sugar" fire on "his blood sugar
/// today" but NOT on "sugar in blood" (order) or "blood pressure and sugar"
/// (contiguity).
fn contains_contiguous(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Match a single message against one entry's normalized trigger list.
///
/// Returns the subset of `triggers` that fire, in their stored order (so output
/// is deterministic). `triggers` are expected already-normalized (stored that
/// way by PR1), but we re-tokenize/stem them here so author-time stemming uses
/// the exact same code path as match-time — the only safe way to guarantee they
/// line up.
pub fn match_triggers(message_tokens: &[String], triggers: &[String]) -> Vec<String> {
    triggers
        .iter()
        .filter(|trig| {
            let trig_tokens = stem_tokens(trig);
            contains_contiguous(message_tokens, &trig_tokens)
        })
        .cloned()
        .collect()
}

/// A matched entry: the entry id plus which of its triggers fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    pub id: String,
    pub triggers_matched: Vec<String>,
}

/// Run the matcher over a message and a collection of (id, triggers) pairs.
///
/// Tokenizes/stems the message ONCE, then checks every entry against the shared
/// token stream. Returns one `TriggerMatch` per entry that has at least one
/// firing trigger, preserving the input entry order.
pub fn match_entries<'a, I>(message: &str, entries: I) -> Vec<TriggerMatch>
where
    I: IntoIterator<Item = (&'a str, &'a [String])>,
{
    let message_tokens = stem_tokens(message);
    if message_tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, triggers) in entries {
        let matched = match_triggers(&message_tokens, triggers);
        if !matched.is_empty() {
            out.push(TriggerMatch {
                id: id.to_string(),
                triggers_matched: matched,
            });
        }
    }
    out
}

/// On-disk shape of the fired-state file: `{"fired":["kn-...", ...]}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct FiredState {
    #[serde(default)]
    fired: Vec<String>,
}

/// Handle to the session fired-state file. All mutation goes through
/// [`FiredStore::mark_survivors`] under an exclusive `flock`.
pub struct FiredStore {
    path: PathBuf,
}

impl FiredStore {
    /// Open the fired store at the resolved path (env override or default).
    pub fn open() -> Self {
        Self { path: fired_path() }
    }

    /// Construct against an explicit path (used by tests / callers that already
    /// resolved the path).
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Atomically read the fired set, compute survivors (the matched ids that
    /// have NOT already fired), append survivors to the fired set, persist, and
    /// return the survivors — all inside one `flock` critical section.
    ///
    /// `matched` is expected pre-sorted/pre-capped by the CALLER (resonance desc,
    /// cap 5): this function only enforces the one-shot dedup invariant, it does
    /// not reorder or cap. It preserves the caller's order in the returned
    /// survivors.
    ///
    /// Critical-section ordering (single `flock(LOCK_EX)`):
    ///   open(rw,create) → lock_exclusive → read → diff → append → rewrite →
    ///   flush+sync → unlock(drop).
    /// Because read and write share one lock, a concurrent `trigger-check` either
    /// runs entirely before or entirely after — it can never observe the file
    /// mid-update and double-fire a memory.
    pub fn mark_survivors(&self, matched: &[String]) -> Result<Vec<String>> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .with_context(|| format!("failed to open fired-state file: {}", self.path.display()))?;

        // Block until we hold the exclusive lock. On the same process this is
        // advisory; across processes flock serializes the whole RMW.
        file.lock_exclusive()
            .with_context(|| format!("failed to flock {}", self.path.display()))?;

        // Guard so the lock is always released, even on an early error return.
        let result = self.read_modify_write(&mut file, matched);
        // Best-effort unlock; drop also releases, but be explicit.
        let _ = FileExt::unlock(&file);
        result
    }

    /// The read-modify-write body, run while holding the exclusive lock.
    fn read_modify_write(
        &self,
        file: &mut std::fs::File,
        matched: &[String],
    ) -> Result<Vec<String>> {
        let mut contents = String::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_string(&mut contents)
            .with_context(|| format!("failed to read fired-state file: {}", self.path.display()))?;

        // Empty file (freshly created) -> empty state. Tolerate a corrupt/partial
        // file by treating it as empty rather than hard-failing the whole check;
        // a triggered-memory hook should degrade to "re-fire" rather than break.
        let mut state: FiredState = if contents.trim().is_empty() {
            FiredState::default()
        } else {
            serde_json::from_str(&contents).unwrap_or_default()
        };

        let already: HashSet<&String> = state.fired.iter().collect();
        let survivors: Vec<String> = matched
            .iter()
            .filter(|id| !already.contains(id))
            .cloned()
            .collect();

        if survivors.is_empty() {
            // Nothing new to record — leave the file untouched.
            return Ok(survivors);
        }

        state.fired.extend(survivors.iter().cloned());
        let serialized = serde_json::to_string(&state)?;

        // Overwrite in place: truncate to the new length then write from offset 0.
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(serialized.as_bytes()).with_context(|| {
            format!("failed to write fired-state file: {}", self.path.display())
        })?;
        file.flush()?;
        file.sync_all()?;

        Ok(survivors)
    }

    /// Read the current fired set WITHOUT marking anything. Used by `--dry-run`
    /// (report which matches would be new) and by tests. Takes a shared lock so
    /// it does not observe a half-written file.
    pub fn read_fired(&self) -> Result<HashSet<String>> {
        if !self.path.exists() {
            return Ok(HashSet::new());
        }
        let file = std::fs::File::open(&self.path)
            .with_context(|| format!("failed to open fired-state file: {}", self.path.display()))?;
        file.lock_shared()
            .with_context(|| format!("failed to flock(shared) {}", self.path.display()))?;
        let mut contents = String::new();
        let mut f = &file;
        let read = f.read_to_string(&mut contents);
        let _ = FileExt::unlock(&file);
        read.with_context(|| format!("failed to read fired-state file: {}", self.path.display()))?;
        if contents.trim().is_empty() {
            return Ok(HashSet::new());
        }
        let state: FiredState = serde_json::from_str(&contents).unwrap_or_default();
        Ok(state.fired.into_iter().collect())
    }

    /// Clear the fired-state file (for `mx memory trigger-reset` and tests).
    /// Removes the file entirely; the next `mark_survivors` recreates it. A
    /// missing file is success (idempotent).
    pub fn reset(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| {
                format!("failed to remove fired-state file: {}", self.path.display())
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        stem_tokens(s)
    }

    // ---- Word-boundary matching ----

    #[test]
    fn word_boundary_ai_does_not_fire_on_said_or_maintain() {
        let msg = toks("he said we should maintain it");
        // Single-word trigger "ai" must NOT match inside "said" or "maintain".
        assert!(match_triggers(&msg, &["ai".to_string()]).is_empty());
    }

    #[test]
    fn word_boundary_ai_fires_as_whole_token() {
        let msg = toks("the ai is helpful");
        assert_eq!(
            match_triggers(&msg, &["ai".to_string()]),
            vec!["ai".to_string()]
        );
    }

    // ---- Stemming ----

    #[test]
    fn stemming_diabetes_fires_on_diabetic() {
        let msg = toks("he is diabetic");
        assert_eq!(
            match_triggers(&msg, &["diabetes".to_string()]),
            vec!["diabetes".to_string()]
        );
    }

    #[test]
    fn stemming_run_fires_on_running() {
        let msg = toks("she is running today");
        assert_eq!(
            match_triggers(&msg, &["run".to_string()]),
            vec!["run".to_string()]
        );
    }

    // ---- Phrase (contiguous-sequence) matching ----

    #[test]
    fn phrase_fires_on_contiguous_in_order() {
        let msg = toks("what is his blood sugar today");
        assert_eq!(
            match_triggers(&msg, &["blood sugar".to_string()]),
            vec!["blood sugar".to_string()]
        );
    }

    #[test]
    fn phrase_does_not_fire_out_of_order() {
        let msg = toks("there is sugar in blood");
        assert!(match_triggers(&msg, &["blood sugar".to_string()]).is_empty());
    }

    #[test]
    fn phrase_does_not_fire_when_not_contiguous() {
        let msg = toks("blood pressure and high sugar");
        assert!(match_triggers(&msg, &["blood sugar".to_string()]).is_empty());
    }

    // ---- NFC (proves the shared normalizer carries through tokenization) ----

    #[test]
    fn nfc_precomposed_and_decomposed_cafe_match() {
        // Trigger stored precomposed, message arrives decomposed.
        let msg = toks("meet me at the cafe\u{0301} later");
        assert_eq!(
            match_triggers(&msg, &["caf\u{00e9}".to_string()]),
            vec!["caf\u{00e9}".to_string()]
        );
    }

    // ---- match_entries integration ----

    #[test]
    fn match_entries_returns_per_entry_matched_triggers() {
        let brad = vec!["brad".to_string(), "blood sugar".to_string()];
        let drew = vec!["drew".to_string()];
        let entries: Vec<(&str, &[String])> =
            vec![("kn-brad", brad.as_slice()), ("kn-drew", drew.as_slice())];
        let matches = match_entries("can you check brad's blood sugar?", entries);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "kn-brad");
        assert_eq!(
            matches[0].triggers_matched,
            vec!["brad".to_string(), "blood sugar".to_string()]
        );
    }

    #[test]
    fn match_entries_empty_message_matches_nothing() {
        let trig = vec!["brad".to_string()];
        let entries: Vec<(&str, &[String])> = vec![("kn-brad", trig.as_slice())];
        assert!(match_entries("   ", entries).is_empty());
    }

    // ---- FiredStore dedup ----

    fn temp_store() -> (tempfile::TempDir, FiredStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fired.json");
        (dir, FiredStore::at(path))
    }

    #[test]
    fn fired_store_marks_and_dedupes() {
        let (_dir, store) = temp_store();
        // First check: both survive.
        let survivors = store
            .mark_survivors(&["kn-a".to_string(), "kn-b".to_string()])
            .unwrap();
        assert_eq!(survivors, vec!["kn-a".to_string(), "kn-b".to_string()]);

        // Second check with one repeat + one new: only the new one survives.
        let survivors = store
            .mark_survivors(&["kn-a".to_string(), "kn-c".to_string()])
            .unwrap();
        assert_eq!(survivors, vec!["kn-c".to_string()]);

        // Third check, all already fired: nothing survives.
        let survivors = store
            .mark_survivors(&["kn-a".to_string(), "kn-b".to_string(), "kn-c".to_string()])
            .unwrap();
        assert!(survivors.is_empty());
    }

    #[test]
    fn fired_store_reset_clears_state() {
        let (_dir, store) = temp_store();
        store.mark_survivors(&["kn-a".to_string()]).unwrap();
        assert!(store.read_fired().unwrap().contains("kn-a"));
        store.reset().unwrap();
        assert!(store.read_fired().unwrap().is_empty());
        // After reset, the same id fires again.
        let survivors = store.mark_survivors(&["kn-a".to_string()]).unwrap();
        assert_eq!(survivors, vec!["kn-a".to_string()]);
    }

    #[test]
    fn fired_store_reset_missing_file_is_ok() {
        let (_dir, store) = temp_store();
        // Never written yet.
        store.reset().unwrap();
        assert!(store.read_fired().unwrap().is_empty());
    }

    #[test]
    fn fired_store_read_does_not_mark() {
        let (_dir, store) = temp_store();
        // read_fired on a fresh store -> empty, and does not create the file.
        assert!(store.read_fired().unwrap().is_empty());
        let survivors = store.mark_survivors(&["kn-a".to_string()]).unwrap();
        assert_eq!(survivors, vec!["kn-a".to_string()]);
    }
}
