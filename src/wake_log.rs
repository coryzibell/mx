//! `mx memory wake-log`: scoring and reading the wake ritual's guess log.
//!
//! `score` embeds every pending guess row once and stores two axes per row.
//! Axis A compares the guess with the entry (`sim_phrase`, `sim_content`) and
//! with its title (`sim_title`). Axis B compares it with earlier guesses on the
//! same title (`sim_prior`) against a baseline built from other entries
//! (`sim_prior_null`), overall and split by substrate.
//!
//! The scores must never reach the model that produced the guesses. `score`
//! prints counts only, and the read commands print only to a terminal or to a
//! file named with `--out`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::cli::WakeLogCommands;
use crate::embeddings::EmbeddingProvider;
use crate::helpers::cosine_similarity;
use crate::store::{AgentContext, KnowledgeStore};
use crate::surreal_db::SurrealDatabase;
use crate::wake_guess::{BASELINE_BLOOMS, content_hash};

/// Shown in `wake-log report --help` and at the top of every report.
pub const GOODHART_TEXT: &str = "Axis A rises by construction when wake phrases are retuned \
toward logged guesses. It measures how well the phrases fit what the model says. It is a \
tuning dial for phrase authoring and must never be reported as evidence of identity or memory. \
Axis B is the only number here that phrase edits cannot raise. Axis B is evidence of stable \
reaching, not of remembering.";

/// A pending row as `score` reads it. `key` is the record id's key part, an
/// array for rows written as `wake_guess:[session_id, position]` and a string
/// for rows older binaries wrote under random ids.
#[derive(Debug, Clone)]
pub struct PendingRow {
    pub key: serde_json::Value,
    pub ts: String,
    pub session_id: String,
    pub position: i64,
    pub bloom_id: String,
    pub title_shown: String,
    pub guess: String,
    pub model_id: Option<String>,
    pub phrase_source: String,
    pub phrases: Vec<String>,
    pub chunk_index: i64,
    pub chunk_total: i64,
}

/// Which prior rows an Axis B read admits, by the `model_id` they were
/// answered under. `Same` and `Cross` carry the scored row's model.
#[derive(Debug, Clone, Copy)]
pub enum Substrate<'a> {
    All,
    Same(&'a str),
    Cross(&'a str),
}

/// The row an Axis B read is computed for: rows must belong to `agent`, be
/// scored under `embedding_model`, come from another session, and have been
/// logged strictly before `ts`.
#[derive(Debug, Clone, Copy)]
pub struct HistoryScope<'a> {
    pub agent: &'a str,
    pub embedding_model: &'a str,
    pub session_id: &'a str,
    pub ts: &'a str,
}

/// One Axis B set's values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorStats {
    pub sim_prior: Option<f64>,
    pub sim_prior_null: Option<f64>,
    pub prior_n: i64,
}

/// Everything `score` writes to one row. Kept apart from `WakeGuessRow`, which
/// is what the ritual reads back and must never carry a score.
#[derive(Debug, Clone)]
pub struct ScoredFields {
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    pub sim_phrase: Option<f64>,
    pub sim_content: Option<f64>,
    pub sim_title: Option<f64>,
    pub all: PriorStats,
    /// None when the row's `model_id` is unknown: the substrate split cannot
    /// be drawn, which is not the same as having no history.
    pub same: Option<PriorStats>,
    pub cross: Option<PriorStats>,
}

/// Was this row matched against phrases an earlier reveal of the same entry
/// had already shown? Before #469 every chunk below the authored-phrase count
/// was matched against the whole authored list, which chunk 0's reveal shows,
/// so an `authored` row past chunk 0 is exactly such a row. None is logged
/// after the fix, which never uses authored phrases past chunk 0, except a
/// row whose prompt an older mx issued and this one answered.
pub fn leaked(chunk_index: i64, phrase_source: &str) -> bool {
    chunk_index > 0 && phrase_source == "authored"
}

/// A row as the read commands show it. Never carries `embedding`.
#[derive(Debug, Clone, Serialize)]
pub struct LogRow {
    pub wake: Option<i64>,
    pub ts: String,
    pub session_id: String,
    pub position: i64,
    pub bloom_id: String,
    pub chunk_index: i64,
    pub chunk_total: i64,
    pub bloom_position: i64,
    pub bloom_total: i64,
    pub title_shown: String,
    pub guess: String,
    pub model_id: Option<String>,
    pub phrase_source: String,
    pub bucket: String,
    /// See [`leaked`]. Computed when the row is read; nothing stores it.
    pub leaked: bool,
    pub scored: bool,
    pub sim_phrase: Option<f64>,
    pub sim_content: Option<f64>,
    pub sim_title: Option<f64>,
    pub sim_prior: Option<f64>,
    pub sim_prior_null: Option<f64>,
    pub prior_n: Option<i64>,
    #[serde(skip)]
    pub sim_prior_same: Option<f64>,
    #[serde(skip)]
    pub sim_prior_null_same: Option<f64>,
    #[serde(skip)]
    pub sim_prior_cross: Option<f64>,
    #[serde(skip)]
    pub sim_prior_null_cross: Option<f64>,
}

// =============================================================================
// SCORING
// =============================================================================

/// What one `score` run did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScoreCounts {
    pub rows: usize,
    pub skipped: usize,
}

#[derive(Serialize)]
struct ScoreOutput {
    status: &'static str,
    rows: usize,
    skipped: usize,
}

/// `score`'s entire stdout. Counts only: no similarity, guess or title.
pub fn score_stdout(counts: ScoreCounts) -> String {
    serde_json::to_string(&ScoreOutput {
        status: "scored",
        rows: counts.rows,
        skipped: counts.skipped,
    })
    .expect("three plain fields serialize")
}

/// The step of a row's scoring that failed. Named in diagnostics instead of
/// the error itself, which could carry the text or a value being scored.
#[derive(Debug, Clone, Copy)]
enum Stage {
    Embed,
    Read,
    Write,
    CacheRead,
    CacheWrite,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Embed => "embedding",
            Stage::Read => "history read",
            Stage::Write => "write",
            Stage::CacheRead => "phrase cache read",
            Stage::CacheWrite => "phrase cache write",
        }
    }
}

/// A phrase as the cache keys it and the model embeds it: trimmed, every
/// whitespace run collapsed to one space. Case is kept.
pub fn normalize_phrase(phrase: &str) -> String {
    phrase.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `wake_phrase_embedding` record key of an already normalized phrase.
pub fn phrase_cache_key(embedding_model: &str, normalized: &str) -> String {
    content_hash(&format!("{embedding_model}\n{normalized}"))
}

/// Score every pending row of `agent`.
///
/// `load` builds the embedding provider. It runs at most once, and not at all
/// when nothing is pending. If it fails the error is returned and every row
/// stays pending. A row whose own scoring fails is counted in `skipped` and
/// left pending for the next run; `diag` gets one line naming the row and the
/// failed step, never its text or any value.
///
/// Rows are scored oldest first, each written before the next one's history is
/// read, so one run over many rituals stores what a run after each ritual
/// would have.
pub fn score<P, F>(
    db: &SurrealDatabase,
    agent: &str,
    load: F,
    diag: &mut dyn Write,
) -> Result<ScoreCounts>
where
    P: EmbeddingProvider,
    F: FnOnce() -> Result<P>,
{
    let pending = db.wake_log_pending(agent)?;
    let mut counts = ScoreCounts::default();
    if pending.is_empty() {
        return Ok(counts);
    }

    let provider = load().context("Failed to load the embedding model; no row was scored")?;

    for row in &pending {
        let outcome = score_row(db, agent, &provider, row, diag).and_then(|fields| {
            db.wake_log_write_score(agent, &row.key, &fields)
                .map_err(|_| Stage::Write)
        });
        match outcome {
            Ok(true) => counts.rows += 1,
            Ok(false) => {}
            Err(stage) => {
                counts.skipped += 1;
                let _ = writeln!(
                    diag,
                    "wake-log score: skipped session {} step {} ({} failed); it stays pending",
                    row.session_id,
                    row.position,
                    stage.as_str()
                );
            }
        }
    }
    Ok(counts)
}

fn score_row<P: EmbeddingProvider>(
    db: &SurrealDatabase,
    agent: &str,
    provider: &P,
    row: &PendingRow,
    diag: &mut dyn Write,
) -> std::result::Result<ScoredFields, Stage> {
    let embed = |text: &str| provider.embed(text).map_err(|_| Stage::Embed);
    let em = provider.model_id();

    let guess = embed(&row.guess)?;

    let sim_phrase = if row.phrase_source == "authored" && !row.phrases.is_empty() {
        phrase_vectors(db, provider, row, guess.len(), diag)?
            .iter()
            .map(|v| cosine_similarity(&guess, v) as f64)
            .reduce(f64::max)
    } else {
        None
    };

    let title = title_for_similarity(&row.title_shown, row.chunk_index, row.chunk_total);
    let sim_title = Some(cosine_similarity(&guess, &embed(title)?) as f64);

    let sim_content = content_similarity(db, agent, &row.bloom_id, &guess, em)?;

    let scope = HistoryScope {
        agent,
        embedding_model: em,
        session_id: &row.session_id,
        ts: &row.ts,
    };
    let all = prior_stats(db, &scope, row, &guess, Substrate::All)?;
    let (same, cross) = match row.model_id.as_deref() {
        Some(model) => (
            Some(prior_stats(
                db,
                &scope,
                row,
                &guess,
                Substrate::Same(model),
            )?),
            Some(prior_stats(
                db,
                &scope,
                row,
                &guess,
                Substrate::Cross(model),
            )?),
        ),
        None => (None, None),
    };

    Ok(ScoredFields {
        embedding: guess,
        embedding_model: em.to_string(),
        sim_phrase,
        sim_content,
        sim_title,
        all,
        same,
        cross,
    })
}

/// The vectors of a row's authored phrases, in order. Cached vectors are read
/// in one statement; only the misses are embedded, then stored. A cache error
/// never fails the row: a failed read counts as all-miss and a failed write is
/// dropped, each with one diag line that carries no text or value.
fn phrase_vectors<P: EmbeddingProvider>(
    db: &SurrealDatabase,
    provider: &P,
    row: &PendingRow,
    dims: usize,
    diag: &mut dyn Write,
) -> std::result::Result<Vec<Vec<f32>>, Stage> {
    let em = provider.model_id();
    let note = |diag: &mut dyn Write, stage: Stage| {
        let _ = writeln!(
            diag,
            "wake-log score: session {} step {} ({} failed, ignored)",
            row.session_id,
            row.position,
            stage.as_str()
        );
    };

    let phrases: Vec<(String, String)> = row
        .phrases
        .iter()
        .map(|p| normalize_phrase(p))
        .filter(|normalized| !normalized.is_empty())
        .map(|normalized| (phrase_cache_key(em, &normalized), normalized))
        .collect();
    if phrases.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = phrases.iter().map(|(k, _)| k.clone()).collect();
    let mut known = db.wake_phrase_embeddings(em, &keys).unwrap_or_else(|_| {
        note(diag, Stage::CacheRead);
        HashMap::new()
    });
    known.retain(|_, v| v.len() == dims);

    let mut misses: Vec<(String, Vec<f32>)> = Vec::new();
    for (key, normalized) in &phrases {
        if !known.contains_key(key) {
            let v = provider.embed(normalized).map_err(|_| Stage::Embed)?;
            misses.push((key.clone(), v.clone()));
            known.insert(key.clone(), v);
        }
    }
    if db.wake_phrase_embeddings_store(em, &misses).is_err() {
        note(diag, Stage::CacheWrite);
    }

    Ok(phrases.iter().map(|(key, _)| known[key].clone()).collect())
}

/// The title text `sim_title` compares against: `title_shown` without the
/// ritual's `(Part N/M)` navigation suffix, rebuilt exactly from the row's own
/// chunk fields.
pub fn title_for_similarity(title_shown: &str, chunk_index: i64, chunk_total: i64) -> &str {
    if chunk_total > 1 {
        let suffix = format!(" (Part {}/{})", chunk_index + 1, chunk_total);
        title_shown.strip_suffix(&suffix).unwrap_or(title_shown)
    } else {
        title_shown
    }
}

/// Max similarity of the guess to the entry's stored vectors under the same
/// embedding model. An entry that is gone or unreadable, or has no such
/// vector, gives None: the row is still scored, since retrying would not help.
fn content_similarity(
    db: &SurrealDatabase,
    agent: &str,
    bloom_id: &str,
    guess: &[f32],
    em: &str,
) -> std::result::Result<Option<f64>, Stage> {
    let ctx = AgentContext::for_agent(agent.to_string());
    let Some(entry) = db.get(bloom_id, &ctx).map_err(|_| Stage::Read)? else {
        return Ok(None);
    };
    let key = entry
        .id
        .strip_prefix("kn-")
        .unwrap_or(&entry.id)
        .to_string();
    let sims = db
        .wake_log_content_similarities(&key, std::slice::from_ref(&entry.id), guess, em)
        .map_err(|_| Stage::Read)?;
    Ok(sims.into_iter().reduce(f64::max))
}

fn centroid(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = vectors.first()?;
    let mut sum = vec![0.0f64; first.len()];
    for v in vectors {
        for (acc, x) in sum.iter_mut().zip(v) {
            *acc += *x as f64;
        }
    }
    let n = vectors.len() as f64;
    Some(sum.into_iter().map(|x| (x / n) as f32).collect())
}

/// One Axis B set for `row`: its prior set over the same bloom and title, and
/// the baseline over up to `BASELINE_BLOOMS` other blooms, each with its own
/// prior-set centroid built the same way.
fn prior_stats(
    db: &SurrealDatabase,
    scope: &HistoryScope<'_>,
    row: &PendingRow,
    guess: &[f32],
    substrate: Substrate<'_>,
) -> std::result::Result<PriorStats, Stage> {
    let own_key = [(row.bloom_id.clone(), row.title_shown.clone())];
    let prior = db
        .wake_log_prior_vectors(scope, &own_key, substrate)
        .map_err(|_| Stage::Read)?
        .pop()
        .unwrap_or_default();
    let sim_prior = centroid(&prior).map(|c| cosine_similarity(guess, &c) as f64);

    let mut chosen: Vec<(String, String)> = Vec::new();
    for (bloom, title) in db
        .wake_log_baseline_rows(scope, &row.bloom_id, substrate)
        .map_err(|_| Stage::Read)?
    {
        if chosen.len() == BASELINE_BLOOMS {
            break;
        }
        if !chosen.iter().any(|(b, _)| *b == bloom) {
            chosen.push((bloom, title));
        }
    }
    let baseline_sets = db
        .wake_log_prior_vectors(scope, &chosen, substrate)
        .map_err(|_| Stage::Read)?;
    let null_sims: Vec<f64> = baseline_sets
        .iter()
        .filter_map(|set| centroid(set))
        .map(|c| cosine_similarity(guess, &c) as f64)
        .collect();
    let sim_prior_null =
        (!null_sims.is_empty()).then(|| null_sims.iter().sum::<f64>() / null_sims.len() as f64);

    Ok(PriorStats {
        sim_prior,
        sim_prior_null,
        prior_n: prior.len() as i64,
    })
}

// =============================================================================
// READ COMMANDS
// =============================================================================

/// Where a read command's output goes. `File` holds the `--out` file already
/// open, so the output lands in the file that was checked.
#[derive(Debug)]
pub enum OutputTarget {
    Terminal,
    File(std::fs::File, PathBuf),
}

#[cfg(unix)]
fn same_file_as_std_stream(target: &std::fs::Metadata) -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;
    [
        std::io::stdout().as_fd().try_clone_to_owned(),
        std::io::stderr().as_fd().try_clone_to_owned(),
    ]
    .into_iter()
    .flatten()
    .any(|fd| {
        std::fs::File::from(fd)
            .metadata()
            .is_ok_and(|m| m.dev() == target.dev() && m.ino() == target.ino())
    })
}

/// The terminal-only rule. Decided before anything is read: with `--out` the
/// output goes to that file; otherwise stdout must be a terminal. An `--out`
/// path that exists and is not a regular file is refused before it is opened,
/// since opening a FIFO for writing blocks until a reader appears. The file is
/// then opened here, before the store, and checked through that handle: it
/// must be a regular file once symlinks are followed, so `/dev/stdout` and the
/// like cannot route the output back onto a pipe, and a `/proc/self/fd/N`
/// path cannot later name a file the store opened.
pub fn output_target(
    command: &str,
    stdout_is_terminal: bool,
    out: Option<PathBuf>,
) -> Result<OutputTarget> {
    match out {
        Some(path) => {
            if std::fs::metadata(&path).is_ok_and(|m| !m.is_file()) {
                bail!(
                    "wake-log {command}: --out must name a regular file, and {} is not one, \
                     so nothing was read or printed.",
                    path.display()
                );
            }
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .with_context(|| {
                    format!(
                        "wake-log {command}: cannot open --out {}, so nothing was read or printed.",
                        path.display()
                    )
                })?;
            let meta = file.metadata().with_context(|| {
                format!(
                    "wake-log {command}: cannot inspect --out {}, so nothing was read or printed.",
                    path.display()
                )
            })?;
            if !meta.is_file() {
                bail!(
                    "wake-log {command}: --out must name a regular file, and {} is not one, \
                     so nothing was read or printed.",
                    path.display()
                );
            }
            #[cfg(unix)]
            if same_file_as_std_stream(&meta) {
                bail!(
                    "wake-log {command}: --out names this process's own stdout or stderr ({}), \
                     so nothing was read or printed.",
                    path.display()
                );
            }
            Ok(OutputTarget::File(file, path))
        }
        None if stdout_is_terminal => Ok(OutputTarget::Terminal),
        None => bail!(
            "wake-log {command} is terminal-only: stdout is not a terminal, so nothing was \
             read or printed. Use --out FILE to write the output to a file instead \
             (.json for JSON, any other name for text)."
        ),
    }
}

/// The calling agent, from `MX_CURRENT_AGENT`. The log is keyed by agent, so
/// with no agent there is nothing this caller may read or score.
pub fn require_agent(agent: Option<String>) -> Result<String> {
    match agent {
        Some(a) if !a.is_empty() => Ok(a),
        _ => bail!(
            "MX_CURRENT_AGENT not set. wake-log reads and scores only the calling agent's \
             guess log, and will not guess which agent that is."
        ),
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

#[derive(Debug, Default, Serialize)]
struct SourceCounts {
    authored: usize,
    derived: usize,
    auto: usize,
    unphrased: usize,
    /// Rows counted here are not counted under their source.
    leaked: usize,
}

impl SourceCounts {
    fn bump(&mut self, row: &LogRow) {
        if leaked(row.chunk_index, &row.phrase_source) {
            self.leaked += 1;
            return;
        }
        match row.phrase_source.as_str() {
            "authored" => self.authored += 1,
            "derived" => self.derived += 1,
            "auto" => self.auto += 1,
            "unphrased" => self.unphrased += 1,
            _ => {}
        }
    }
}

#[derive(Debug, Default, Serialize)]
struct Buckets {
    unhinted: SourceCounts,
    revealed: SourceCounts,
}

#[derive(Debug, Serialize)]
struct Stat {
    median: Option<f64>,
    n: usize,
}

impl Stat {
    fn of(values: Vec<f64>) -> Self {
        Self {
            n: values.len(),
            median: median(values),
        }
    }
}

#[derive(Debug, Serialize)]
struct GapStat {
    gap_median: Option<f64>,
    sim_prior_median: Option<f64>,
    null_median: Option<f64>,
    n: usize,
}

impl GapStat {
    /// Over the scored rows that have both terms, so all three medians describe
    /// the same rows.
    fn of(pairs: Vec<(f64, f64)>) -> Self {
        Self {
            n: pairs.len(),
            gap_median: median(pairs.iter().map(|(p, n)| p - n).collect()),
            sim_prior_median: median(pairs.iter().map(|(p, _)| *p).collect()),
            null_median: median(pairs.iter().map(|(_, n)| *n).collect()),
        }
    }
}

#[derive(Debug, Serialize)]
struct AxisA {
    sim_phrase: Stat,
    sim_content: Stat,
}

#[derive(Debug, Serialize)]
struct AxisB {
    same_model: GapStat,
    cross_model: GapStat,
}

/// `report`'s content. Field order is output order, Goodhart text first.
#[derive(Debug, Serialize)]
pub struct Report {
    goodhart: &'static str,
    wake: i64,
    models: Vec<String>,
    chunks: usize,
    entries: usize,
    scored: usize,
    buckets: Buckets,
    axis_a: AxisA,
    axis_b: AxisB,
}

pub fn build_report(wake: i64, rows: &[LogRow]) -> Report {
    let mut models: Vec<String> = Vec::new();
    let mut entries: Vec<&str> = Vec::new();
    let mut buckets = Buckets::default();
    for row in rows {
        let model = row
            .model_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        if !models.contains(&model) {
            models.push(model);
        }
        if !entries.contains(&row.bloom_id.as_str()) {
            entries.push(&row.bloom_id);
        }
        match row.bucket.as_str() {
            "unhinted" => buckets.unhinted.bump(row),
            "revealed" => buckets.revealed.bump(row),
            _ => {}
        }
    }

    let scored: Vec<&LogRow> = rows.iter().filter(|r| r.scored).collect();
    let pairs = |f: fn(&LogRow) -> (Option<f64>, Option<f64>)| -> Vec<(f64, f64)> {
        scored
            .iter()
            .filter_map(|r| match f(r) {
                (Some(p), Some(n)) => Some((p, n)),
                _ => None,
            })
            .collect()
    };

    Report {
        goodhart: GOODHART_TEXT,
        wake,
        models,
        chunks: rows.len(),
        entries: entries.len(),
        scored: scored.len(),
        buckets,
        axis_a: AxisA {
            // A leaked row's sim_phrase compares the guess with phrases shown
            // one step earlier.
            sim_phrase: Stat::of(
                scored
                    .iter()
                    .filter(|r| !leaked(r.chunk_index, &r.phrase_source))
                    .filter_map(|r| r.sim_phrase)
                    .collect(),
            ),
            sim_content: Stat::of(scored.iter().filter_map(|r| r.sim_content).collect()),
        },
        axis_b: AxisB {
            same_model: GapStat::of(pairs(|r| (r.sim_prior_same, r.sim_prior_null_same))),
            cross_model: GapStat::of(pairs(|r| (r.sim_prior_cross, r.sim_prior_null_cross))),
        },
    }
}

/// The bidi controls (CVE-2021-42574): format characters, not `Cc`, that
/// reorder how a terminal draws the text around them.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// A stored string as the text layout shows it: every control or bidi-control
/// character escaped, so an escape sequence or a newline in a guess or title reaches the
/// terminal as visible text.
fn shown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() || is_bidi_control(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

fn num(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.2}"))
}

pub fn report_text(r: &Report) -> String {
    let mut s = String::new();
    s.push_str(GOODHART_TEXT);
    s.push_str("\n\n");
    s.push_str(&format!(
        "Wake {}   model {}   {} chunks, {} entries   scored {}/{}\n\n",
        r.wake,
        if r.models.is_empty() {
            "-".to_string()
        } else {
            shown(&r.models.join(", "))
        },
        r.chunks,
        r.entries,
        r.scored,
        r.chunks
    ));
    s.push_str("Buckets            authored  derived  auto  unphrased  leaked\n");
    for (name, c) in [
        ("unhinted", &r.buckets.unhinted),
        ("revealed", &r.buckets.revealed),
    ] {
        s.push_str(&format!(
            "  {name:<8}{:>17}{:>9}{:>6}{:>11}{:>8}\n",
            c.authored, c.derived, c.auto, c.unphrased, c.leaked
        ));
    }
    s.push_str("\nAxis A (tunable; not identity evidence)\n");
    for (name, st) in [
        ("sim_phrase", &r.axis_a.sim_phrase),
        ("sim_content", &r.axis_a.sim_content),
    ] {
        s.push_str(&format!(
            "  {name:<12}  median {}   n {}\n",
            num(st.median),
            st.n
        ));
    }
    s.push_str("\nAxis B (gap = sim_prior - null)\n");
    for (name, g) in [
        ("same model", &r.axis_b.same_model),
        ("cross model", &r.axis_b.cross_model),
    ] {
        s.push_str(&format!(
            "  {name:<12}  gap median {}   sim_prior {}   null {}   n {}\n",
            num(g.gap_median),
            num(g.sim_prior_median),
            num(g.null_median),
            g.n
        ));
    }
    s
}

/// `bloom ID` and `wake N`: rows with the five issue similarity fields.
#[derive(Debug, Serialize)]
pub struct RowListing {
    goodhart: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bloom_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wake: Option<i64>,
    rows: Vec<LogRow>,
}

fn date_of(ts: &str) -> &str {
    ts.get(..10).unwrap_or(ts)
}

fn row_text(row: &LogRow) -> String {
    let wake = row.wake.map_or_else(|| "-".to_string(), |w| w.to_string());
    let chunk = if row.chunk_total > 1 {
        format!("   chunk {}/{}", row.chunk_index + 1, row.chunk_total)
    } else {
        String::new()
    };
    let scores = if row.scored {
        format!(
            "  sim_phrase {}   sim_content {}   sim_title {}   sim_prior {}   sim_prior_null {}",
            num(row.sim_phrase),
            num(row.sim_content),
            num(row.sim_title),
            num(row.sim_prior),
            num(row.sim_prior_null)
        )
    } else {
        "  pending (not scored yet)".to_string()
    };
    let leak = if leaked(row.chunk_index, &row.phrase_source) {
        "   leaked"
    } else {
        ""
    };
    format!(
        "Wake {wake}   {}   model {}   step {}   entry {}/{}{chunk}   bucket {}{leak}\n  \
         title  {}\n  guess  {}\n{scores}\n",
        date_of(&row.ts),
        shown(row.model_id.as_deref().unwrap_or("unknown")),
        row.position,
        row.bloom_position,
        row.bloom_total,
        row.bucket,
        shown(&row.title_shown),
        shown(&row.guess),
    )
}

pub fn listing_text(listing: &RowListing) -> String {
    let mut s = String::new();
    s.push_str(GOODHART_TEXT);
    s.push_str("\n\n");
    match (&listing.bloom_id, listing.wake) {
        (Some(id), _) => s.push_str(&format!(
            "Entry {}   {} rows\n",
            shown(id),
            listing.rows.len()
        )),
        (None, Some(w)) => s.push_str(&format!("Wake {w}   {} rows\n", listing.rows.len())),
        (None, None) => {}
    }
    for row in &listing.rows {
        s.push('\n');
        s.push_str(&row_text(row));
    }
    s
}

/// Deliver a read command's output: text to the terminal, or the file named
/// with `--out` (JSON when it ends in `.json`), returning only its path.
fn deliver<T: Serialize>(target: &OutputTarget, value: &T, text: String) -> Result<String> {
    match target {
        OutputTarget::Terminal => Ok(text),
        OutputTarget::File(file, path) => {
            let body = if is_json_path(path) {
                serde_json::to_string_pretty(value)? + "\n"
            } else {
                text
            };
            let mut file = file;
            file.set_len(0)
                .and_then(|()| file.write_all(body.as_bytes()))
                .with_context(|| format!("Failed to write {}", path.display()))?;
            Ok(path.display().to_string())
        }
    }
}

fn is_json_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// Run one wake-log command and return what goes on stdout.
///
/// The terminal-only rule and the agent check both run before `open_db`, so a
/// refused call reads nothing at all.
pub fn run<O, P, L>(
    command: WakeLogCommands,
    agent: Option<String>,
    stdout_is_terminal: bool,
    open_db: O,
    load: L,
    diag: &mut dyn Write,
) -> Result<String>
where
    O: FnOnce() -> Result<SurrealDatabase>,
    P: EmbeddingProvider,
    L: FnOnce() -> Result<P>,
{
    match command {
        WakeLogCommands::Score => {
            let agent = require_agent(agent)?;
            let db = open_db()?;
            let counts = score(&db, &agent, load, diag)?;
            Ok(score_stdout(counts))
        }
        WakeLogCommands::Report { wake, out } => {
            let target = output_target("report", stdout_is_terminal, out)?;
            let agent = require_agent(agent)?;
            let db = open_db()?;
            let wake = match wake {
                Some(w) => w,
                None => db.wake_log_latest_scored_wake(&agent)?.ok_or_else(|| {
                    anyhow!(
                        "no scored wake: run `mx memory wake-log score` first, or pass --wake N"
                    )
                })?,
            };
            let rows = db.wake_log_rows_for_wake(&agent, wake)?;
            let report = build_report(wake, &rows);
            let text = report_text(&report);
            deliver(&target, &report, text)
        }
        WakeLogCommands::Bloom { id, limit, out } => {
            let target = output_target("bloom", stdout_is_terminal, out)?;
            let agent = require_agent(agent)?;
            let id = crate::helpers::normalize_id(&id);
            let db = open_db()?;
            let rows = db.wake_log_rows_for_bloom(&agent, &id, limit)?;
            let listing = RowListing {
                goodhart: GOODHART_TEXT,
                bloom_id: Some(id),
                wake: None,
                rows,
            };
            let text = listing_text(&listing);
            deliver(&target, &listing, text)
        }
        WakeLogCommands::Wake { wake, out } => {
            let target = output_target("wake", stdout_is_terminal, out)?;
            let agent = require_agent(agent)?;
            let db = open_db()?;
            let rows = db.wake_log_rows_for_wake(&agent, wake)?;
            let listing = RowListing {
                goodhart: GOODHART_TEXT,
                bloom_id: None,
                wake: Some(wake),
                rows,
            };
            let text = listing_text(&listing);
            deliver(&target, &listing, text)
        }
    }
}

/// The CLI entry point for `mx memory wake-log`.
pub fn handle(command: WakeLogCommands, db_path: &Path, verbose: bool) -> Result<()> {
    use std::io::IsTerminal;

    let stdout = run(
        command,
        std::env::var("MX_CURRENT_AGENT").ok(),
        std::io::stdout().is_terminal(),
        || SurrealDatabase::open_with_verbose(db_path.with_extension("surreal"), verbose),
        crate::embeddings::TractProvider::new,
        &mut std::io::stderr(),
    )?;
    if stdout.ends_with('\n') {
        print!("{stdout}");
    } else {
        println!("{stdout}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
