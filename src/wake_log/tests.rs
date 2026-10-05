//! `wake-log` against the embedded test store. Every title, entry and guess
//! here is invented. The embedding provider is a deterministic fake, so no
//! model is loaded except by the ignored timing test.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::knowledge::KnowledgeEntry;
use crate::surreal_db::SurrealDatabase;

// =============================================================================
// FIXTURES
// =============================================================================

const AGENT: &str = "agent-a";
const OTHER_AGENT: &str = "agent-b";
const FAKE_MODEL: &str = "test/fake-embed";
const DIM: usize = 16;

/// Bag-of-words hashing into `DIM` buckets: texts that share words point the
/// same way, so the fixtures produce distinct, meaningful similarities.
fn fake_vec(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    for word in text.split_whitespace() {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in word.to_lowercase().bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        v[(h % DIM as u64) as usize] += 1.0 + ((h >> 8) % 3) as f32;
    }
    if v.iter().all(|x| *x == 0.0) {
        v[0] = 1.0;
    }
    v
}

struct FakeProvider {
    fail_on: Option<String>,
}

impl FakeProvider {
    fn ok() -> Self {
        Self { fail_on: None }
    }
}

impl EmbeddingProvider for FakeProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if self.fail_on.as_deref() == Some(text) {
            bail!("fake provider refuses this text");
        }
        Ok(fake_vec(text))
    }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
    fn dimensions(&self) -> usize {
        DIM
    }
    fn model_id(&self) -> &str {
        FAKE_MODEL
    }
}

fn run_score(db: &SurrealDatabase, agent: &str) -> ScoreCounts {
    let mut diag = Vec::new();
    score(db, agent, || Ok(FakeProvider::ok()), &mut diag).unwrap()
}

fn lit(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn ts_at(sec: u32) -> String {
    format!(
        "2026-01-01T{:02}:{:02}:{:02}Z",
        sec / 3600,
        (sec / 60) % 60,
        sec % 60
    )
}

/// One invented guess row, inserted with an explicit `ts`.
#[derive(Clone)]
struct Fx {
    agent: String,
    wake: Option<i64>,
    session: String,
    position: i64,
    sec: u32,
    bloom: String,
    title: String,
    guess: String,
    model: Option<String>,
    source: String,
    phrases: Vec<String>,
    chunk_index: i64,
    chunk_total: i64,
    bucket: String,
    /// Insert as already scored under this embedding model, with this vector.
    prescored: Option<(String, Vec<f32>)>,
}

fn fx(session: &str, position: i64, sec: u32, bloom: &str, title: &str, guess: &str) -> Fx {
    Fx {
        agent: AGENT.to_string(),
        wake: Some(1),
        session: session.to_string(),
        position,
        sec,
        bloom: bloom.to_string(),
        title: title.to_string(),
        guess: guess.to_string(),
        model: Some("model-a".to_string()),
        source: "authored".to_string(),
        phrases: vec!["lantern by the quay".to_string()],
        chunk_index: 0,
        chunk_total: 1,
        bucket: "revealed".to_string(),
        prescored: None,
    }
}

impl Fx {
    fn agent(mut self, a: &str) -> Self {
        self.agent = a.to_string();
        self
    }
    fn wake(mut self, w: Option<i64>) -> Self {
        self.wake = w;
        self
    }
    fn model(mut self, m: Option<&str>) -> Self {
        self.model = m.map(str::to_string);
        self
    }
    fn source(mut self, s: &str, phrases: &[&str]) -> Self {
        self.source = s.to_string();
        self.phrases = phrases.iter().map(|p| p.to_string()).collect();
        self
    }
    fn chunk(mut self, index: i64, total: i64) -> Self {
        self.chunk_index = index;
        self.chunk_total = total;
        self
    }
    fn bucket(mut self, b: &str) -> Self {
        self.bucket = b.to_string();
        self
    }
    fn prescored(mut self, em: &str) -> Self {
        let v = fake_vec(&self.guess);
        self.prescored = Some((em.to_string(), v));
        self
    }
}

fn insert(db: &SurrealDatabase, f: &Fx) {
    let phrases: Vec<String> = f.phrases.iter().map(|p| lit(p)).collect();
    let scored = match &f.prescored {
        Some((em, v)) => format!(
            ", embedding = [{}], embedding_model = {}, scored_at = time::now()",
            v.iter()
                .map(|x| format!("{x:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            lit(em)
        ),
        None => String::new(),
    };
    let sql = format!(
        "CREATE type::thing('wake_guess', [{session}, {position}]) SET
            agent = {agent}, wake = {wake}, session_id = {session},
            ts = <datetime>{ts}, bloom_id = {bloom}, chunk_index = {ci}, chunk_total = {ct},
            position = {position}, bloom_position = 1, bloom_total = 3,
            title_shown = {title}, guess = {guess}, model_id = {model},
            phrase_source = {source}, phrases = [{phrases}], match_kind = 'none',
            match_index = NONE, bucket = {bucket}, content_hash = 'abc'{scored}
        RETURN NONE",
        session = lit(&f.session),
        position = f.position,
        agent = lit(&f.agent),
        wake = f.wake.map_or("NONE".to_string(), |w| w.to_string()),
        ts = lit(&ts_at(f.sec)),
        bloom = lit(&f.bloom),
        ci = f.chunk_index,
        ct = f.chunk_total,
        title = lit(&f.title),
        guess = lit(&f.guess),
        model = f.model.as_deref().map_or("NONE".to_string(), lit),
        source = lit(&f.source),
        phrases = phrases.join(", "),
        bucket = lit(&f.bucket),
    );
    db.query_json_for_test(&sql).unwrap();
}

const STORED_FIELDS: &str = "embedding_model, scored_at != NONE AS scored, sim_phrase,
    sim_content, sim_title, sim_prior, sim_prior_null, prior_n, sim_prior_same,
    sim_prior_null_same, prior_n_same, sim_prior_cross, sim_prior_null_cross, prior_n_cross";

fn stored(db: &SurrealDatabase, session: &str, position: i64) -> serde_json::Value {
    let rows = db
        .query_json_for_test(&format!(
            "SELECT {STORED_FIELDS} FROM wake_guess
            WHERE session_id = {} AND position = {position}",
            lit(session)
        ))
        .unwrap();
    assert_eq!(rows.len(), 1, "one row at {session}/{position}");
    rows.into_iter().next().unwrap()
}

fn all_stored(db: &SurrealDatabase) -> Vec<serde_json::Value> {
    db.query_json_for_test(&format!(
        "SELECT session_id, position, {STORED_FIELDS} FROM wake_guess
        ORDER BY session_id, position"
    ))
    .unwrap()
}

fn is_none(v: &serde_json::Value, key: &str) -> bool {
    v.get(key).is_none_or(|x| x.is_null())
}

fn cos64(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    dot / (na * nb)
}

fn cos(a: &[f32], b: &[f32]) -> f64 {
    cosine_similarity(a, b) as f64
}

fn centroid_of(guesses: &[&str]) -> Vec<f32> {
    let vs: Vec<Vec<f32>> = guesses.iter().map(|g| fake_vec(g)).collect();
    centroid(&vs).unwrap()
}

fn entry(id: &str, title: &str, owner: Option<&str>, visibility: &str) -> KnowledgeEntry {
    let now = chrono::Utc::now().to_rfc3339();
    KnowledgeEntry {
        id: id.to_string(),
        category_id: "bloom".to_string(),
        title: title.to_string(),
        body: Some("An invented body about lanterns and harbors.".to_string()),
        summary: None,
        applicability: vec![],
        source_project_id: None,
        source_agent_id: None,
        file_path: None,
        tags: vec![],
        created_at: Some(now.clone()),
        updated_at: Some(now.clone()),
        content_hash: Some("fixture-hash".to_string()),
        source_type_id: Some("manual".to_string()),
        entry_type_id: Some("primary".to_string()),
        session_id: None,
        ephemeral: false,
        content_type_id: Some("text".to_string()),
        owner: owner.map(str::to_string),
        visibility: visibility.to_string(),
        resonance: 9,
        resonance_type: Some("foundational".to_string()),
        last_activated: Some(now),
        activation_count: 0,
        decay_rate: 0.0,
        anchors: vec![],
        wake_phrases: vec![],
        triggers: vec![],
        wake_order: None,
        wake_phrase: None,
        embedding: None,
        embedding_model: None,
        embedded_at: None,
        chunk_count: 0,
        format: "markdown".to_string(),
        effective_resonance: None,
    }
}

fn no_read(calls: &Cell<usize>) -> impl FnOnce() -> Result<SurrealDatabase> + '_ {
    move || {
        calls.set(calls.get() + 1);
        bail!("the store must not be opened")
    }
}

fn no_model() -> Result<FakeProvider> {
    panic!("a read command must never load the embedding model")
}

// =============================================================================
// AC 1: the model loads at most once, and not at all when nothing is pending
// =============================================================================

#[test]
fn wake_log_score_loads_the_model_at_most_once_and_never_when_nothing_is_pending() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let loads = Cell::new(0usize);
    let load = || {
        loads.set(loads.get() + 1);
        Ok(FakeProvider::ok())
    };

    let counts = score(&db, AGENT, load, &mut Vec::new()).unwrap();
    assert_eq!(counts, ScoreCounts::default());
    assert_eq!(loads.get(), 0, "an empty log must not load the model");

    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    insert(
        &db,
        &fx("s1", 1, 11, "kn-harbor", "Harbor Map", "boats at anchor"),
    );
    insert(
        &db,
        &fx("s1", 2, 12, "kn-tide", "Tide Table", "water rising slowly"),
    );
    let load = || {
        loads.set(loads.get() + 1);
        Ok(FakeProvider::ok())
    };
    let counts = score(&db, AGENT, load, &mut Vec::new()).unwrap();
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 3,
            skipped: 0
        }
    );
    assert_eq!(loads.get(), 1, "three pending rows, one model load");

    let load = || {
        loads.set(loads.get() + 1);
        Ok(FakeProvider::ok())
    };
    assert_eq!(
        score(&db, AGENT, load, &mut Vec::new()).unwrap(),
        ScoreCounts::default()
    );
    assert_eq!(loads.get(), 1, "everything scored: no second load");
}

#[test]
fn wake_log_score_model_load_failure_leaves_every_row_pending_and_prints_nothing() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    insert(
        &db,
        &fx("s1", 1, 11, "kn-harbor", "Harbor Map", "boats at anchor"),
    );

    let mut diag = Vec::new();
    let result = run(
        WakeLogCommands::Score,
        Some(AGENT.to_string()),
        false,
        || Ok(db),
        || -> Result<FakeProvider> { bail!("model files missing") },
        &mut diag,
    );
    let err = result.expect_err("a load failure is an error, so no stdout is printed");
    assert!(format!("{err:#}").contains("Failed to load the embedding model"));
    assert!(diag.is_empty());

    // `run` consumed the store; reopen the same scenario to check the rows.
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    score(
        &db,
        AGENT,
        || -> Result<FakeProvider> { bail!("model files missing") },
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(stored(&db, "s1", 0)["scored"], false);
    assert_eq!(db.wake_log_pending(AGENT).unwrap().len(), 1);
}

// =============================================================================
// AC 2: score's stdout is counts only
// =============================================================================

#[test]
fn wake_log_score_stdout_is_one_json_object_with_no_float_or_guess_text() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let rows = [
        fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
        fx("s1", 1, 11, "kn-harbor", "Harbor Map", "boats at anchor"),
        fx(
            "s2",
            0,
            20,
            "kn-lantern",
            "Lantern Notes",
            "a lamp near the water",
        ),
        fx(
            "s2",
            1,
            21,
            "kn-harbor",
            "Harbor Map",
            "unembeddable marmalade",
        ),
    ];
    for r in &rows {
        insert(&db, r);
    }
    let mut diag = Vec::new();
    let counts = score(
        &db,
        AGENT,
        || {
            Ok(FakeProvider {
                fail_on: Some("unembeddable marmalade".to_string()),
            })
        },
        &mut diag,
    )
    .unwrap();
    let stdout = score_stdout(counts);
    let diag = String::from_utf8(diag).unwrap();

    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let keys: std::collections::BTreeSet<&str> =
        v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["rows", "skipped", "status"].into_iter().collect());
    assert_eq!(v["status"], "scored");
    assert_eq!(v["rows"], 3);
    assert_eq!(v["skipped"], 1);

    let float = regex::Regex::new(r"\d\.\d").unwrap();
    for (name, text) in [("stdout", &stdout), ("stderr", &diag)] {
        assert!(!float.is_match(text), "{name} carries a float: {text}");
        for r in &rows {
            assert!(!text.contains(&r.guess), "{name} carries a guess");
            assert!(!text.contains(&r.title), "{name} carries a title");
        }
    }
    assert!(diag.contains("skipped"), "the skip is diagnosed: {diag}");
}

// =============================================================================
// AC 3: a row that fails stays pending, counts as skipped, scores next run
// =============================================================================

#[test]
fn wake_log_a_row_that_fails_to_score_stays_pending_and_scores_next_run() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    insert(
        &db,
        &fx(
            "s1",
            1,
            11,
            "kn-harbor",
            "Harbor Map",
            "a guess the fake refuses",
        ),
    );
    insert(
        &db,
        &fx("s1", 2, 12, "kn-tide", "Tide Table", "water rising slowly"),
    );

    let mut diag = Vec::new();
    let counts = score(
        &db,
        AGENT,
        || {
            Ok(FakeProvider {
                fail_on: Some("a guess the fake refuses".to_string()),
            })
        },
        &mut diag,
    )
    .unwrap();
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 2,
            skipped: 1
        }
    );
    assert_eq!(stored(&db, "s1", 1)["scored"], false);
    assert!(is_none(&stored(&db, "s1", 1), "sim_title"));
    assert_eq!(
        stored(&db, "s1", 2)["scored"],
        true,
        "later rows still score"
    );
    let diag = String::from_utf8(diag).unwrap();
    assert!(diag.contains("session s1 step 1"), "{diag}");

    // A failing phrase embed skips the row too, not just a failing guess.
    insert(
        &db,
        &fx("s2", 0, 20, "kn-lantern", "Lantern Notes", "the lamp again")
            .source("authored", &["a phrase the fake refuses"]),
    );
    let counts = score(
        &db,
        AGENT,
        || {
            Ok(FakeProvider {
                fail_on: Some("a phrase the fake refuses".to_string()),
            })
        },
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 1
        },
        "s1/1 now scores"
    );
    assert_eq!(stored(&db, "s1", 1)["scored"], true);
    assert_eq!(stored(&db, "s2", 0)["scored"], false);

    assert_eq!(
        run_score(&db, AGENT),
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    assert_eq!(stored(&db, "s2", 0)["scored"], true);
}

// =============================================================================
// AC 4: rows written before this change score
// =============================================================================

fn wake_session_at(
    db: &SurrealDatabase,
    session_id: &str,
    step: u32,
) -> crate::wake_token::WakeSession {
    let cascade = crate::store::WakeCascade {
        core: vec![
            entry("kn-lantern", "Lantern Notes", None, "public"),
            entry("kn-harbor", "Harbor Map", None, "public"),
        ],
        ..Default::default()
    };
    let mut session =
        crate::wake_token::WakeSession::new(&cascade, AGENT.to_string(), Some(4), None);
    session.session_id = session_id.to_string();
    session.step = step;
    db.create_wake_session(&session).unwrap();
    session
}

#[test]
fn wake_log_score_scores_rows_written_before_this_change() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Through the respond path's own writer.
    let row = crate::wake_guess::WakeGuessRow {
        agent: AGENT.to_string(),
        wake: Some(4),
        session_id: "old-session".to_string(),
        bloom_id: "kn-lantern".to_string(),
        chunk_index: 0,
        chunk_total: 1,
        position: 0,
        bloom_position: 1,
        bloom_total: 2,
        title_shown: "Lantern Notes".to_string(),
        guess: "a lamp on the pier".to_string(),
        model_id: Some("model-a".to_string()),
        phrase_source: "authored".to_string(),
        phrases: vec!["lantern by the quay".to_string()],
        match_kind: "none".to_string(),
        match_index: None,
        bucket: "revealed".to_string(),
        content_hash: crate::wake_guess::content_hash("chunk text"),
    };
    let mut session = wake_session_at(&db, "old-session", 0);
    session.step = 1;
    db.record_wake_guess(&row, &session, 0).unwrap();

    // As the PR-1 binary left it: a random id and none of the split fields.
    db.query_json_for_test(
        "CREATE wake_guess SET agent = 'agent-a', wake = 3, session_id = 'older-session',
            bloom_id = 'kn-harbor', chunk_index = 0, chunk_total = 1, position = 0,
            bloom_position = 1, bloom_total = 1, title_shown = 'Harbor Map',
            guess = 'boats at anchor', model_id = NONE, phrase_source = 'derived',
            phrases = ['harbor map'], match_kind = 'none', match_index = NONE,
            bucket = 'revealed', content_hash = 'abc' RETURN NONE",
    )
    .unwrap();

    assert_eq!(
        run_score(&db, AGENT),
        ScoreCounts {
            rows: 2,
            skipped: 0
        }
    );

    let a = stored(&db, "old-session", 0);
    assert_eq!(a["scored"], true);
    assert_eq!(a["embedding_model"], FAKE_MODEL);
    assert_eq!(a["prior_n"], 0);
    assert_eq!(a["prior_n_same"], 0);

    let b = stored(&db, "older-session", 0);
    assert_eq!(b["scored"], true);
    assert!(
        is_none(&b, "sim_phrase"),
        "derived phrases give no sim_phrase"
    );
    assert!(b["sim_title"].as_f64().is_some());
    assert!(is_none(&b, "prior_n_same"), "unknown model: no split");
}

// =============================================================================
// AC 5: the terminal-only rule
// =============================================================================

fn read_commands(out: Option<std::path::PathBuf>) -> Vec<WakeLogCommands> {
    vec![
        WakeLogCommands::Report {
            wake: None,
            out: out.clone(),
        },
        WakeLogCommands::Bloom {
            id: "lantern".to_string(),
            limit: 20,
            out: out.clone(),
        },
        WakeLogCommands::Wake { wake: 1, out },
    ]
}

#[test]
fn wake_log_read_commands_refuse_when_not_a_terminal_and_read_nothing() {
    for command in read_commands(None) {
        let opened = Cell::new(0usize);
        let err = run(
            command,
            Some(AGENT.to_string()),
            false,
            no_read(&opened),
            no_model,
            &mut Vec::new(),
        )
        .expect_err("a non-terminal stdout without --out is refused");
        let msg = err.to_string();
        assert!(msg.contains("terminal-only"), "{msg}");
        assert!(msg.contains("--out FILE"), "{msg}");
        assert_eq!(opened.get(), 0, "the guard runs before any read");
    }
}

#[test]
fn wake_log_read_commands_write_json_or_text_with_out() {
    let dir = tempfile::tempdir().unwrap();
    let seed = |db: &SurrealDatabase| {
        insert(
            db,
            &fx(
                "s1",
                0,
                10,
                "kn-lantern",
                "Lantern Notes",
                "a lamp on the pier",
            ),
        );
        insert(
            db,
            &fx(
                "s2",
                0,
                20,
                "kn-lantern",
                "Lantern Notes",
                "a lamp near the water",
            )
            .wake(Some(2)),
        );
        run_score(db, AGENT);
    };

    for (ext, is_json) in [("json", true), ("txt", false)] {
        for (i, command) in read_commands(Some(dir.path().join(format!("x.{ext}"))))
            .into_iter()
            .enumerate()
        {
            let path = dir.path().join(format!("out-{i}.{ext}"));
            let command = match command {
                WakeLogCommands::Report { wake, .. } => WakeLogCommands::Report {
                    wake,
                    out: Some(path.clone()),
                },
                WakeLogCommands::Bloom { id, limit, .. } => WakeLogCommands::Bloom {
                    id,
                    limit,
                    out: Some(path.clone()),
                },
                WakeLogCommands::Wake { wake, .. } => WakeLogCommands::Wake {
                    wake,
                    out: Some(path.clone()),
                },
                WakeLogCommands::Score => unreachable!(),
            };
            let db = SurrealDatabase::open_in_memory().unwrap();
            seed(&db);
            let stdout = run(
                command,
                Some(AGENT.to_string()),
                false,
                || Ok(db),
                no_model,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                stdout,
                path.display().to_string(),
                "only the path is printed"
            );
            let body = std::fs::read_to_string(&path).unwrap();
            if is_json {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(v["goodhart"], GOODHART_TEXT);
                assert!(!body.contains("\"embedding\""));
                if i == 1 {
                    assert_eq!(v["bloom_id"], "kn-lantern");
                    assert_eq!(v["rows"].as_array().unwrap().len(), 2);
                }
            } else {
                assert!(body.starts_with(GOODHART_TEXT), "{body}");
            }
        }
    }

    // On a terminal the same commands print the text layout.
    let db = SurrealDatabase::open_in_memory().unwrap();
    seed(&db);
    let stdout = run(
        WakeLogCommands::Wake { wake: 1, out: None },
        Some(AGENT.to_string()),
        true,
        || Ok(db),
        no_model,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(stdout.starts_with(GOODHART_TEXT));
    assert!(stdout.contains("a lamp on the pier"));
}

#[test]
fn wake_log_refuses_without_a_calling_agent() {
    for command in read_commands(None)
        .into_iter()
        .chain([WakeLogCommands::Score])
    {
        let opened = Cell::new(0usize);
        let err = run(
            command,
            None,
            true,
            no_read(&opened),
            no_model,
            &mut Vec::new(),
        )
        .expect_err("no agent, no read");
        assert!(err.to_string().contains("MX_CURRENT_AGENT"), "{err}");
        assert_eq!(opened.get(), 0);
    }
    assert!(require_agent(Some(String::new())).is_err());
    assert_eq!(require_agent(Some("x".into())).unwrap(), "x");
}

// =============================================================================
// AC 6: Axis B
// =============================================================================

#[test]
fn wake_log_axis_b_is_null_with_no_history() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    // One ritual: other entries in the SAME session must not form a baseline.
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    insert(
        &db,
        &fx("s1", 1, 11, "kn-harbor", "Harbor Map", "boats at anchor"),
    );
    insert(
        &db,
        &fx("s1", 2, 12, "kn-tide", "Tide Table", "water rising slowly"),
    );
    run_score(&db, AGENT);

    for pos in 0..3 {
        let v = stored(&db, "s1", pos);
        assert_eq!(v["scored"], true);
        assert!(is_none(&v, "sim_prior"), "{v}");
        assert!(is_none(&v, "sim_prior_null"), "{v}");
        assert_eq!(v["prior_n"], 0);
        assert!(is_none(&v, "sim_prior_same"));
        assert!(is_none(&v, "sim_prior_null_same"));
        assert_eq!(v["prior_n_same"], 0);
        assert!(is_none(&v, "sim_prior_cross"));
        assert_eq!(v["prior_n_cross"], 0);
    }
}

#[test]
fn wake_log_prior_set_uses_same_title_and_model_from_other_earlier_sessions() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let title = "Lantern Notes";

    // Six qualifying priors: only the five most recent count.
    let priors = [
        "a lamp left burning",
        "the lamp on the pier",
        "light over the quay",
        "lantern in the fog",
        "a beacon for the boats",
        "glow by the harbor wall",
    ];
    for (i, g) in priors.iter().enumerate() {
        insert(
            &db,
            &fx(&format!("p{i}"), 0, 100 + i as u32, "kn-lantern", title, g),
        );
    }
    // Same bloom, a different title: another stimulus.
    insert(
        &db,
        &fx(
            "x-title",
            0,
            150,
            "kn-lantern",
            "Lantern Notes, revised",
            "a renamed lamp",
        ),
    );
    // Same title, same session as the scored row, earlier.
    insert(
        &db,
        &fx("target", 0, 160, "kn-lantern", title, "a same session lamp"),
    );
    // Same title, scored under another embedding model.
    insert(
        &db,
        &fx("x-model", 0, 170, "kn-lantern", title, "another model lamp").prescored("other/model"),
    );
    // Same title, logged after the scored row but scored before it.
    insert(
        &db,
        &fx(
            "x-later",
            0,
            900,
            "kn-lantern",
            title,
            "a lamp from the future",
        )
        .prescored(FAKE_MODEL),
    );
    // The scored row.
    let guess = "the lamp by the quay";
    insert(&db, &fx("target", 1, 200, "kn-lantern", title, guess));

    run_score(&db, AGENT);

    let v = stored(&db, "target", 1);
    assert_eq!(v["prior_n"], 5);
    let expected = cos(&fake_vec(guess), &centroid_of(&priors[1..]));
    assert_eq!(v["sim_prior"].as_f64().unwrap(), expected);
}

#[test]
fn wake_log_baseline_uses_other_blooms_with_the_same_embedding_model() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Eleven other blooms with earlier history: the ten most recent count.
    let others: Vec<(String, String)> = (0..11)
        .map(|i| {
            (
                format!("kn-other{i}"),
                format!("guess number {i} about rope"),
            )
        })
        .collect();
    for (i, (bloom, g)) in others.iter().enumerate() {
        insert(
            &db,
            &fx(
                &format!("o{i}"),
                0,
                100 + i as u32,
                bloom,
                &format!("Other {i}"),
                g,
            ),
        );
    }
    // A second row for the newest other bloom, under the same title: its
    // centroid is built from both.
    insert(
        &db,
        &fx(
            "o-extra",
            0,
            120,
            "kn-other10",
            "Other 10",
            "rope coiled on deck",
        ),
    );
    // The same bloom under an older title: the key title is the newest row's.
    insert(
        &db,
        &fx(
            "o-oldtitle",
            0,
            90,
            "kn-other10",
            "Other 10 old",
            "an old rope title",
        ),
    );
    // Excluded: another embedding model, and the scored row's own session.
    insert(
        &db,
        &fx("x-model", 0, 130, "kn-model", "Model Bloom", "anchor chain").prescored("other/model"),
    );
    insert(
        &db,
        &fx(
            "target",
            0,
            140,
            "kn-session",
            "Session Bloom",
            "a same session net",
        ),
    );
    // The scored row's own bloom history does not feed the baseline.
    insert(
        &db,
        &fx(
            "own",
            0,
            141,
            "kn-lantern",
            "Lantern Notes",
            "the lamp at dusk",
        ),
    );

    let guess = "a lamp and some rope";
    insert(
        &db,
        &fx("target", 1, 200, "kn-lantern", "Lantern Notes", guess),
    );
    run_score(&db, AGENT);

    let g = fake_vec(guess);
    let mut sims: Vec<f64> = (1..10)
        .map(|i| cos(&g, &centroid_of(&[others[i].1.as_str()])))
        .collect();
    sims.push(cos(
        &g,
        &centroid_of(&["rope coiled on deck", others[10].1.as_str()]),
    ));
    let expected = sims.iter().sum::<f64>() / sims.len() as f64;

    let v = stored(&db, "target", 1);
    assert_eq!(v["prior_n"], 1, "the own-bloom row is the prior set");
    assert_eq!(v["sim_prior_null"].as_f64().unwrap(), expected);
}

#[test]
fn wake_log_axis_b_splits_by_substrate_and_leaves_unknown_models_unsplit() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let title = "Lantern Notes";
    insert(
        &db,
        &fx("p-same", 0, 100, "kn-lantern", title, "a lamp left burning"),
    );
    insert(
        &db,
        &fx(
            "p-cross",
            0,
            101,
            "kn-lantern",
            title,
            "light over the quay",
        )
        .model(Some("model-b")),
    );
    insert(
        &db,
        &fx("p-none", 0, 102, "kn-lantern", title, "lantern in the fog").model(None),
    );
    insert(
        &db,
        &fx(
            "b-same",
            0,
            103,
            "kn-harbor",
            "Harbor Map",
            "boats at anchor",
        ),
    );
    insert(
        &db,
        &fx("b-cross", 0, 104, "kn-tide", "Tide Table", "water rising").model(Some("model-b")),
    );

    let guess = "the lamp by the quay";
    insert(&db, &fx("target", 0, 200, "kn-lantern", title, guess));
    insert(
        &db,
        &fx("target-none", 0, 201, "kn-lantern", title, guess).model(None),
    );
    run_score(&db, AGENT);

    let g = fake_vec(guess);
    let v = stored(&db, "target", 0);
    assert_eq!(v["prior_n"], 3);
    assert_eq!(v["prior_n_same"], 1);
    assert_eq!(v["prior_n_cross"], 1, "a NONE model_id is not cross");
    assert_eq!(
        v["sim_prior_same"].as_f64().unwrap(),
        cos(&g, &centroid_of(&["a lamp left burning"]))
    );
    assert_eq!(
        v["sim_prior_cross"].as_f64().unwrap(),
        cos(&g, &centroid_of(&["light over the quay"]))
    );
    assert_eq!(
        v["sim_prior_null_same"].as_f64().unwrap(),
        cos(&g, &centroid_of(&["boats at anchor"]))
    );
    assert_eq!(
        v["sim_prior_null_cross"].as_f64().unwrap(),
        cos(&g, &centroid_of(&["water rising"]))
    );

    let none = stored(&db, "target-none", 0);
    assert_eq!(none["prior_n"], 4, "the unsplit set still counts");
    for key in [
        "sim_prior_same",
        "sim_prior_null_same",
        "prior_n_same",
        "sim_prior_cross",
        "sim_prior_null_cross",
        "prior_n_cross",
    ] {
        assert!(is_none(&none, key), "{key} must be NONE: {none}");
    }
}

// =============================================================================
// Axis A and the title control
// =============================================================================

#[test]
fn wake_log_sim_title_strips_the_part_suffix_and_the_prior_key_does_not() {
    assert_eq!(
        title_for_similarity("Harbor Map (Part 2/3)", 1, 3),
        "Harbor Map"
    );
    assert_eq!(
        title_for_similarity("Harbor Map (Part 2/3)", 0, 3),
        "Harbor Map (Part 2/3)"
    );
    assert_eq!(
        title_for_similarity("Harbor Map (Part 1/2)", 0, 1),
        "Harbor Map (Part 1/2)"
    );
    assert_eq!(title_for_similarity("Harbor Map", 0, 1), "Harbor Map");

    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "p",
            0,
            100,
            "kn-harbor",
            "Harbor Map (Part 2/2)",
            "the second half",
        )
        .chunk(1, 2),
    );
    let guess = "a map of the harbor";
    insert(
        &db,
        &fx("t", 0, 200, "kn-harbor", "Harbor Map (Part 1/2)", guess).chunk(0, 2),
    );
    run_score(&db, AGENT);

    let v = stored(&db, "t", 0);
    assert_eq!(
        v["sim_title"].as_f64().unwrap(),
        cos(&fake_vec(guess), &fake_vec("Harbor Map"))
    );
    assert_eq!(v["prior_n"], 0, "another part is another stimulus");
}

#[test]
fn wake_log_axis_a_uses_authored_phrases_and_same_model_entry_vectors() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut lantern = entry("kn-lantern", "Lantern Notes", None, "public");
    lantern.embedding = Some(fake_vec("a lamp kept burning by the water"));
    lantern.embedding_model = Some(FAKE_MODEL.to_string());
    db.upsert_knowledge(&lantern).unwrap();
    db.insert_embedding_chunk(
        "kn-lantern",
        0,
        "chunk",
        0,
        1,
        &fake_vec("lamp lit near water"),
        FAKE_MODEL,
    )
    .unwrap();
    db.insert_embedding_chunk(
        "kn-lantern",
        1,
        "chunk",
        1,
        1,
        &fake_vec("a lamp on the pier"),
        "other/model",
    )
    .unwrap();
    db.insert_embedding_chunk("kn-lantern", 2, "chunk", 2, 1, &[1.0, 0.0], FAKE_MODEL)
        .unwrap();

    let mut secret = entry("kn-secret", "Secret Notes", Some(OTHER_AGENT), "private");
    secret.embedding = Some(fake_vec("a lamp on the pier"));
    secret.embedding_model = Some(FAKE_MODEL.to_string());
    db.upsert_knowledge(&secret).unwrap();

    let guess = "a lamp on the pier";
    insert(
        &db,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", guess)
            .source("authored", &["burning lamp", "a pier at night"]),
    );
    insert(&db, &fx("s1", 1, 11, "kn-secret", "Secret Notes", guess));
    insert(
        &db,
        &fx("s1", 2, 12, "kn-gone", "Gone Notes", guess).source("auto", &["gone"]),
    );
    assert_eq!(
        run_score(&db, AGENT),
        ScoreCounts {
            rows: 3,
            skipped: 0
        }
    );

    let g = fake_vec(guess);
    let v = stored(&db, "s1", 0);
    let phrase = cos(&g, &fake_vec("burning lamp")).max(cos(&g, &fake_vec("a pier at night")));
    assert_eq!(v["sim_phrase"].as_f64().unwrap(), phrase);
    // Computed server side in f64, so compared in f64. The other-model chunk is
    // the guess's own vector: had it leaked in, this would be 1.
    let content = cos64(&g, &fake_vec("a lamp kept burning by the water"))
        .max(cos64(&g, &fake_vec("lamp lit near water")));
    assert!(content < 0.999, "{content}");
    let got = v["sim_content"].as_f64().unwrap();
    assert!((got - content).abs() < 1e-12, "{got} vs {content}");

    let secret = stored(&db, "s1", 1);
    assert_eq!(secret["scored"], true);
    assert!(
        is_none(&secret, "sim_content"),
        "unreadable entry: {secret}"
    );

    let gone = stored(&db, "s1", 2);
    assert_eq!(
        gone["scored"], true,
        "a deleted entry does not stay pending"
    );
    assert!(is_none(&gone, "sim_content"));
    assert!(
        is_none(&gone, "sim_phrase"),
        "auto phrases give no sim_phrase"
    );
}

// =============================================================================
// AC 7: another agent's rows are never read
// =============================================================================

#[test]
fn wake_log_write_score_touches_only_this_agents_pending_row() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-lantern",
            "Lantern Notes",
            "a lamp on the pier",
        ),
    );
    let key = serde_json::json!(["s1", 0]);
    let none = PriorStats {
        sim_prior: None,
        sim_prior_null: None,
        prior_n: 0,
    };
    let fields = ScoredFields {
        embedding: fake_vec("x"),
        embedding_model: FAKE_MODEL.into(),
        sim_phrase: None,
        sim_content: None,
        sim_title: Some(0.5),
        all: none,
        same: None,
        cross: None,
    };
    assert!(!db.wake_log_write_score(OTHER_AGENT, &key, &fields).unwrap());
    assert_eq!(stored(&db, "s1", 0)["scored"], false);
    assert!(db.wake_log_write_score(AGENT, &key, &fields).unwrap());
    let first = stored(&db, "s1", 0);
    let again = ScoredFields {
        sim_title: Some(0.25),
        ..fields
    };
    assert!(!db.wake_log_write_score(AGENT, &key, &again).unwrap());
    assert_eq!(stored(&db, "s1", 0), first);
}

#[test]
fn wake_log_rows_of_another_agent_are_never_read() {
    let mine = [
        fx(
            "a1",
            0,
            100,
            "kn-lantern",
            "Lantern Notes",
            "a lamp left burning",
        ),
        fx("a1", 1, 101, "kn-harbor", "Harbor Map", "boats at anchor"),
        fx(
            "a2",
            0,
            200,
            "kn-lantern",
            "Lantern Notes",
            "the lamp by the quay",
        )
        .wake(Some(2)),
        fx("a2", 1, 201, "kn-harbor", "Harbor Map", "ships in harbor").wake(Some(2)),
    ];
    // B's rows would change A's prior set, baseline and pending count.
    let theirs = [
        fx(
            "b1",
            0,
            150,
            "kn-lantern",
            "Lantern Notes",
            "a totally different lamp",
        )
        .agent(OTHER_AGENT)
        .prescored(FAKE_MODEL)
        .wake(Some(9)),
        fx("b1", 1, 151, "kn-tide", "Tide Table", "rope and salt")
            .agent(OTHER_AGENT)
            .prescored(FAKE_MODEL)
            .wake(Some(9)),
        fx(
            "b2",
            0,
            160,
            "kn-lantern",
            "Lantern Notes",
            "an unscored lamp",
        )
        .agent(OTHER_AGENT),
    ];

    let alone = SurrealDatabase::open_in_memory().unwrap();
    let crowded = SurrealDatabase::open_in_memory().unwrap();
    for r in &mine {
        insert(&alone, r);
        insert(&crowded, r);
    }
    for r in &theirs {
        insert(&crowded, r);
    }

    assert_eq!(alone.wake_log_pending(AGENT).unwrap().len(), 4);
    assert_eq!(crowded.wake_log_pending(AGENT).unwrap().len(), 4);
    assert_eq!(run_score(&alone, AGENT), run_score(&crowded, AGENT));

    for r in &mine {
        assert_eq!(
            stored(&alone, &r.session, r.position),
            stored(&crowded, &r.session, r.position),
            "{}/{}",
            r.session,
            r.position
        );
    }
    assert_eq!(
        stored(&crowded, "b2", 0)["scored"],
        false,
        "B's row untouched"
    );

    assert_eq!(alone.wake_log_latest_scored_wake(AGENT).unwrap(), Some(2));
    assert_eq!(crowded.wake_log_latest_scored_wake(AGENT).unwrap(), Some(2));
    let a = crowded.wake_log_rows_for_wake(AGENT, 1).unwrap();
    assert_eq!(a.len(), 2);
    let lantern = crowded
        .wake_log_rows_for_bloom(AGENT, "kn-lantern", 50)
        .unwrap();
    assert_eq!(lantern.len(), 2);
    assert!(lantern.iter().all(|r| !r.guess.contains("different")));
}

// =============================================================================
// AC 8: no axis value reaches the ritual
// =============================================================================

const FORBIDDEN_KEYS: [&str; 5] = ["sim_", "embedding", "prior_n", "scored_at", "goodhart"];

/// Every non-integer number anywhere in a JSON value.
fn floats_in(v: &serde_json::Value, out: &mut Vec<f64>) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => out.push(n.as_f64().unwrap()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| floats_in(x, out)),
        serde_json::Value::Object(o) => o.values().for_each(|x| floats_in(x, out)),
        _ => {}
    }
}

fn assert_no_axis_value(output: &str, stored_values: &[f64]) {
    for key in FORBIDDEN_KEYS {
        assert!(
            !output.contains(key),
            "ritual output carries {key}: {output}"
        );
    }
    let parsed: serde_json::Value = serde_json::from_str(output).expect("ritual output is JSON");
    let mut floats = Vec::new();
    floats_in(&parsed, &mut floats);
    for f in floats {
        assert!(
            !stored_values.contains(&f),
            "ritual output carries a stored score: {output}"
        );
    }
}

/// Every stored similarity value.
fn stored_floats(db: &SurrealDatabase) -> Vec<f64> {
    all_stored(db)
        .iter()
        .flat_map(|row| {
            row.as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| k.starts_with("sim_"))
                .filter_map(|(_, v)| v.as_f64())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn wake_log_no_axis_value_in_begin_or_respond_output_after_scoring() {
    use crate::wake_ritual::{RitualMeta, begin_ritual, respond_ritual};

    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = AgentContext::for_agent(AGENT);
    let mut first = entry("kn-first", "Lantern Notes", None, "public");
    first.wake_phrases = vec!["lantern by the quay".to_string()];
    let mut second = entry("kn-second", "Harbor Map", None, "public");
    second.wake_phrases = vec!["boats at anchor".to_string()];
    db.upsert_knowledge(&first).unwrap();
    db.upsert_knowledge(&second).unwrap();
    let cascade = crate::store::WakeCascade {
        core: vec![first, second],
        ..Default::default()
    };
    let meta = || RitualMeta {
        agent: AGENT.to_string(),
        wake: None,
        model_id: Some("model-a".to_string()),
    };
    let token_of = |out: &str| {
        let v: serde_json::Value = serde_json::from_str(out).unwrap();
        v["session"].as_str().unwrap().to_string()
    };

    // A whole earlier ritual, scored, so the next one has history.
    let begin = begin_ritual(&db, &cascade, meta(), false).unwrap();
    let r1 = respond_ritual(
        &db,
        &ctx,
        "kn-first",
        "a lamp on the pier",
        &token_of(&begin),
    )
    .unwrap();
    respond_ritual(&db, &ctx, "kn-second", "ships at rest", &token_of(&r1)).unwrap();
    run_score(&db, AGENT);

    let begin = begin_ritual(&db, &cascade, meta(), false).unwrap();
    let token0 = token_of(&begin);
    let r1 = respond_ritual(&db, &ctx, "kn-first", "the lamp by the quay", &token0).unwrap();
    assert_eq!(run_score(&db, AGENT).rows, 1);
    let floats = stored_floats(&db);
    assert!(!floats.is_empty());
    assert_eq!(
        stored(&db, token0.split('.').next().unwrap(), 0)["prior_n"],
        1,
        "the scored step has history, so it carries axis values"
    );
    assert_no_axis_value(&begin, &floats);
    assert_no_axis_value(&r1, &floats);

    // The lost-response replay of the scored step.
    let replay = respond_ritual(&db, &ctx, "kn-first", "a retyped guess", &token0).unwrap();
    let v: serde_json::Value = serde_json::from_str(&replay).unwrap();
    assert_eq!(v["replayed"], true);
    assert_eq!(v["guess"], "the lamp by the quay");
    assert_no_axis_value(&replay, &floats);

    let r2 = respond_ritual(&db, &ctx, "kn-second", "boats at anchor", &token_of(&r1)).unwrap();
    run_score(&db, AGENT);
    let floats = stored_floats(&db);
    assert_no_axis_value(&r2, &floats);

    // The half-written replay: a scored row whose session never advanced.
    let begin = begin_ritual(&db, &cascade, meta(), false).unwrap();
    let token = token_of(&begin);
    let session_id = token.split('.').next().unwrap().to_string();
    insert(
        &db,
        &Fx {
            session: session_id.clone(),
            position: 0,
            bloom: "kn-first".to_string(),
            title: "Lantern Notes".to_string(),
            guess: "a lamp logged before a crash".to_string(),
            sec: 0,
            ..fx("", 0, 0, "", "", "")
        }
        .wake(None),
    );
    db.query_json_for_test(&format!(
        "UPDATE wake_guess SET ts = time::now() WHERE session_id = {} RETURN NONE",
        lit(&session_id)
    ))
    .unwrap();
    assert_eq!(run_score(&db, AGENT).rows, 1);
    let floats = stored_floats(&db);
    let replay = respond_ritual(&db, &ctx, "kn-first", "anything", &token).unwrap();
    let v: serde_json::Value = serde_json::from_str(&replay).unwrap();
    assert_eq!(v["replayed"], true);
    assert_eq!(v["guess"], "a lamp logged before a crash");
    assert_no_axis_value(&begin, &floats);
    assert_no_axis_value(&replay, &floats);

    let logged = db.get_wake_guess(&session_id, 0).unwrap().unwrap();
    assert_eq!(logged.guess, "a lamp logged before a crash");
}

// =============================================================================
// AC 9: one retroactive run stores what per-ritual runs would have
// =============================================================================

fn three_rituals() -> Vec<Vec<Fx>> {
    vec![
        vec![
            fx(
                "r1",
                0,
                100,
                "kn-lantern",
                "Lantern Notes",
                "a lamp left burning",
            ),
            fx("r1", 1, 101, "kn-harbor", "Harbor Map", "boats at anchor").model(Some("model-b")),
            fx(
                "r1",
                2,
                102,
                "kn-tide",
                "Tide Table (Part 1/2)",
                "water rising",
            )
            .chunk(0, 2),
        ],
        vec![
            fx(
                "r2",
                0,
                200,
                "kn-lantern",
                "Lantern Notes",
                "light over the quay",
            ),
            fx(
                "r2",
                1,
                201,
                "kn-harbor",
                "Harbor Map",
                "ships in the harbor",
            )
            .model(None),
            fx(
                "r2",
                2,
                202,
                "kn-tide",
                "Tide Table (Part 1/2)",
                "the tide comes in",
            )
            .chunk(0, 2),
        ],
        vec![
            fx(
                "r3",
                0,
                300,
                "kn-lantern",
                "Lantern Notes",
                "the lamp by the quay",
            ),
            fx(
                "r3",
                1,
                301,
                "kn-harbor",
                "Harbor Map",
                "boats in the harbor",
            ),
            fx(
                "r3",
                2,
                302,
                "kn-tide",
                "Tide Table (Part 1/2)",
                "water rising again",
            )
            .chunk(0, 2),
        ],
    ]
}

#[test]
fn wake_log_scoring_three_rituals_at_once_matches_scoring_after_each() {
    let at_once = SurrealDatabase::open_in_memory().unwrap();
    for ritual in three_rituals() {
        for r in &ritual {
            insert(&at_once, r);
        }
    }
    assert_eq!(run_score(&at_once, AGENT).rows, 9);

    let each = SurrealDatabase::open_in_memory().unwrap();
    for ritual in three_rituals() {
        for r in &ritual {
            insert(&each, r);
        }
        assert_eq!(run_score(&each, AGENT).rows, 3);
    }

    let a = all_stored(&at_once);
    assert_eq!(a, all_stored(&each));
    assert!(
        a.iter().any(|r| r["sim_prior"].as_f64().is_some()),
        "the comparison covers real Axis B values"
    );
    assert!(
        a.iter()
            .any(|r| r["sim_prior_null_cross"].as_f64().is_some())
    );
}

// =============================================================================
// AC 10: the Goodhart text
// =============================================================================

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn wake_log_goodhart_text_is_in_report_help_and_tops_every_report() {
    use clap::CommandFactory;

    let mut cli = crate::cli::Cli::command();
    let report = cli
        .find_subcommand_mut("memory")
        .and_then(|m| m.find_subcommand_mut("wake-log"))
        .and_then(|w| w.find_subcommand_mut("report"))
        .expect("mx memory wake-log report exists");
    let help = report.render_long_help().to_string();
    assert!(squash(&help).contains(&squash(GOODHART_TEXT)), "{help}");

    let rows = vec![LogRow {
        wake: Some(3),
        ts: ts_at(5),
        session_id: "s".into(),
        position: 0,
        bloom_id: "kn-lantern".into(),
        chunk_index: 0,
        chunk_total: 1,
        bloom_position: 1,
        bloom_total: 1,
        title_shown: "Lantern Notes".into(),
        guess: "a lamp".into(),
        model_id: Some("model-a".into()),
        phrase_source: "authored".into(),
        bucket: "unhinted".into(),
        leaked: false,
        scored: true,
        sim_phrase: Some(0.5),
        sim_content: Some(0.25),
        sim_title: Some(0.125),
        sim_prior: Some(0.75),
        sim_prior_null: Some(0.5),
        prior_n: Some(1),
        sim_prior_same: Some(0.75),
        sim_prior_null_same: Some(0.5),
        sim_prior_cross: None,
        sim_prior_null_cross: None,
    }];
    let r = build_report(3, &rows);
    let text = report_text(&r);
    assert!(text.starts_with(&format!("{GOODHART_TEXT}\n\n")));
    let json = serde_json::to_string_pretty(&r).unwrap();
    assert!(json.starts_with("{\n  \"goodhart\": "), "{json}");

    let listing = RowListing {
        goodhart: GOODHART_TEXT,
        bloom_id: Some("kn-lantern".into()),
        wake: None,
        rows,
    };
    assert!(listing_text(&listing).starts_with(GOODHART_TEXT));
    assert!(
        serde_json::to_string_pretty(&listing)
            .unwrap()
            .starts_with("{\n  \"goodhart\": ")
    );
}

#[test]
fn wake_log_report_layout_counts_buckets_and_takes_medians() {
    let row =
        |bucket: &str, source: &str, model: Option<&str>, scored: bool, p: Option<f64>| LogRow {
            wake: Some(3),
            ts: ts_at(1),
            session_id: "s".into(),
            position: 0,
            bloom_id: format!("kn-{bucket}-{source}"),
            chunk_index: 0,
            chunk_total: 1,
            bloom_position: 1,
            bloom_total: 4,
            title_shown: "T".into(),
            guess: "g".into(),
            model_id: model.map(str::to_string),
            phrase_source: source.into(),
            bucket: bucket.into(),
            leaked: false,
            scored,
            sim_phrase: p,
            sim_content: None,
            sim_title: None,
            sim_prior: None,
            sim_prior_null: None,
            prior_n: None,
            sim_prior_same: p,
            sim_prior_null_same: p.map(|x| x - 0.25),
            sim_prior_cross: None,
            sim_prior_null_cross: None,
        };
    let rows = vec![
        row("unhinted", "authored", Some("model-a"), true, Some(0.5)),
        row("revealed", "derived", Some("model-a"), true, Some(0.75)),
        row("revealed", "auto", None, true, Some(1.0)),
        row("revealed", "auto", None, false, None),
    ];
    let r = build_report(3, &rows);
    let text = report_text(&r);
    let lines: Vec<&str> = text.lines().collect();
    let body = &lines[2..];
    assert_eq!(
        body[0],
        "Wake 3   model model-a, unknown   4 chunks, 3 entries   scored 3/4"
    );
    assert_eq!(
        body[2],
        "Buckets            authored  derived  auto  unphrased  leaked"
    );
    assert_eq!(
        body[3],
        "  unhinted                1        0     0          0       0"
    );
    assert_eq!(
        body[4],
        "  revealed                0        1     2          0       0"
    );
    assert_eq!(body[7], "  sim_phrase    median 0.75   n 3");
    assert_eq!(body[8], "  sim_content   median -   n 0");
    assert_eq!(
        body[11],
        "  same model    gap median 0.25   sim_prior 0.75   null 0.50   n 3"
    );
    assert_eq!(
        body[12],
        "  cross model   gap median -   sim_prior -   null -   n 0"
    );
    assert_eq!(median(vec![1.0, 4.0, 2.0, 3.0]), Some(2.5));
}

/// An invented, scored row for the report and listing tests.
fn report_row(
    chunk_index: i64,
    source: &str,
    bucket: &str,
    sim_phrase: Option<f64>,
    sim_content: Option<f64>,
) -> LogRow {
    LogRow {
        wake: Some(4),
        ts: ts_at(chunk_index as u32),
        session_id: "s".into(),
        position: chunk_index,
        bloom_id: "kn-orchard".into(),
        chunk_index,
        chunk_total: 4,
        bloom_position: 1,
        bloom_total: 1,
        title_shown: format!("Orchard Ledger (Part {}/4)", chunk_index + 1),
        guess: "rows of pear trees".into(),
        model_id: Some("model-a".into()),
        phrase_source: source.into(),
        bucket: bucket.into(),
        leaked: leaked(chunk_index, source),
        scored: true,
        sim_phrase,
        sim_content,
        sim_title: None,
        sim_prior: None,
        sim_prior_null: None,
        prior_n: None,
        sim_prior_same: None,
        sim_prior_null_same: None,
        sim_prior_cross: None,
        sim_prior_null_cross: None,
    }
}

#[test]
fn wake_log_report_counts_chunk_two_authored_rows_as_leaked() {
    let rows = vec![
        report_row(0, "authored", "unhinted", None, None),
        report_row(1, "authored", "unhinted", None, None),
        report_row(2, "authored", "revealed", None, None),
        report_row(3, "derived", "unhinted", None, None),
    ];
    let r = build_report(4, &rows);
    let text = report_text(&r);
    let lines: Vec<&str> = text.lines().collect();
    let body = &lines[2..];
    assert_eq!(
        body[2],
        "Buckets            authored  derived  auto  unphrased  leaked"
    );
    assert_eq!(
        body[3],
        "  unhinted                1        1     0          0       1"
    );
    assert_eq!(
        body[4],
        "  revealed                0        0     0          0       1"
    );

    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["buckets"]["unhinted"]["authored"], 1);
    assert_eq!(json["buckets"]["unhinted"]["leaked"], 1);
    assert_eq!(json["buckets"]["revealed"]["authored"], 0);
    assert_eq!(json["buckets"]["revealed"]["leaked"], 1);
}

#[test]
fn wake_log_report_counts_phraseless_rows_under_unphrased() {
    let rows = vec![report_row(1, "unphrased", "revealed", None, None)];
    let r = build_report(4, &rows);
    let text = report_text(&r);
    let lines: Vec<&str> = text.lines().collect();
    let body = &lines[2..];
    assert_eq!(
        body[2],
        "Buckets            authored  derived  auto  unphrased  leaked"
    );
    assert_eq!(
        body[3],
        "  unhinted                0        0     0          0       0"
    );
    assert_eq!(
        body[4],
        "  revealed                0        0     0          1       0"
    );

    let json = serde_json::to_value(&r).unwrap();
    let revealed = &json["buckets"]["revealed"];
    assert_eq!(revealed["unphrased"], 1);
    assert_eq!(revealed["derived"], 0);
    assert_eq!(revealed["auto"], 0);
    assert_eq!(revealed["leaked"], 0);
}

#[test]
fn wake_log_report_sim_phrase_excludes_leaked_rows() {
    let rows = vec![
        report_row(0, "authored", "revealed", Some(0.25), Some(0.5)),
        report_row(1, "authored", "unhinted", Some(1.0), Some(0.75)),
        report_row(2, "authored", "unhinted", Some(1.0), Some(0.75)),
    ];
    let r = build_report(4, &rows);
    let text = report_text(&r);
    assert!(
        text.contains("  sim_phrase    median 0.25   n 1\n"),
        "{text}"
    );
    assert!(
        text.contains("  sim_content   median 0.75   n 3\n"),
        "sim_content keeps every row: {text}"
    );
}

#[test]
fn wake_log_listing_marks_leaked_rows() {
    // Read back through the store, so log_row is what sets the flag.
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx(
            "s1",
            0,
            10,
            "kn-orchard",
            "Orchard Ledger (Part 1/3)",
            "pear rows",
        )
        .chunk(0, 3),
    );
    insert(
        &db,
        &fx(
            "s1",
            1,
            11,
            "kn-orchard",
            "Orchard Ledger (Part 2/3)",
            "pear rows",
        )
        .chunk(1, 3)
        .bucket("unhinted"),
    );
    insert(
        &db,
        &fx(
            "s1",
            2,
            12,
            "kn-orchard",
            "Orchard Ledger (Part 3/3)",
            "cider press",
        )
        .chunk(2, 3)
        .source("derived", &["The press runs in October."]),
    );
    let rows = db.wake_log_rows_for_wake(AGENT, 1).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.leaked).collect::<Vec<_>>(),
        vec![false, true, false]
    );

    let listing = RowListing {
        goodhart: GOODHART_TEXT,
        bloom_id: None,
        wake: Some(1),
        rows,
    };
    let text = listing_text(&listing);
    let headers: Vec<&str> = text.lines().filter(|l| l.contains("   bucket ")).collect();
    assert_eq!(headers.len(), 3, "{text}");
    assert!(headers[0].ends_with("bucket revealed"), "{}", headers[0]);
    assert!(
        headers[1].ends_with("bucket unhinted   leaked"),
        "{}",
        headers[1]
    );
    assert!(headers[2].ends_with("bucket revealed"), "{}", headers[2]);

    let json = serde_json::to_value(&listing).unwrap();
    assert_eq!(json["rows"][0]["leaked"], false);
    assert_eq!(json["rows"][1]["leaked"], true);
    assert_eq!(json["rows"][2]["leaked"], false);
}

// =============================================================================
// The phrase-embedding cache
// =============================================================================

/// Counts embed calls per text. Its vectors are `fake_vec` mapped onto awkward
/// f32 values, so a cache round trip that changed a single bit would show.
#[derive(Clone)]
struct CountingProvider {
    model: &'static str,
    calls: Arc<Mutex<HashMap<String, usize>>>,
}

impl CountingProvider {
    fn new(model: &'static str) -> Self {
        Self {
            model,
            calls: Arc::default(),
        }
    }
    fn calls(&self, text: &str) -> usize {
        self.calls.lock().unwrap().get(text).copied().unwrap_or(0)
    }
    fn total(&self) -> usize {
        self.calls.lock().unwrap().values().sum()
    }
    fn texts(&self) -> Vec<String> {
        self.calls.lock().unwrap().keys().cloned().collect()
    }
}

fn odd_vec(text: &str) -> Vec<f32> {
    fake_vec(text)
        .iter()
        .enumerate()
        .map(|(i, x)| x * 0.123_456_79 + (i as f32 + 1.0) / 7_000.0)
        .collect()
}

impl EmbeddingProvider for CountingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(text.to_string())
            .or_default() += 1;
        Ok(odd_vec(text))
    }
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
    fn dimensions(&self) -> usize {
        DIM
    }
    fn model_id(&self) -> &str {
        self.model
    }
}

fn score_counting(db: &SurrealDatabase, provider: &CountingProvider) -> (ScoreCounts, String) {
    let mut diag = Vec::new();
    let p = provider.clone();
    let counts = score(db, AGENT, move || Ok(p), &mut diag).unwrap();
    (counts, String::from_utf8(diag).unwrap())
}

fn best_phrase_sim(guess: &str, phrases: &[&str]) -> f64 {
    let g = odd_vec(guess);
    phrases
        .iter()
        .map(|p| cos(&g, &odd_vec(&normalize_phrase(p))))
        .reduce(f64::max)
        .unwrap()
}

/// Every cache record: its field names, the whole record printed, its id.
fn cache_rows(db: &SurrealDatabase) -> Vec<serde_json::Value> {
    db.query_json_for_test(
        "SELECT object::keys($this) AS fields, <string>$this AS dump, <string>id AS id,
            embedding_model, embedding
        FROM wake_phrase_embedding",
    )
    .unwrap()
}

const RITUAL: [(&str, &str, [&str; 3]); 3] = [
    (
        "kn-lantern",
        "Lantern Notes",
        ["lantern by the quay", "a lamp left lit", "glow over harbor"],
    ),
    (
        "kn-tide",
        "Tide Table",
        [
            "water climbing stairs",
            "moon pulls the sea",
            "salt on stones",
        ],
    ),
    (
        "kn-rope",
        "Rope Knots",
        [
            "bowline tied twice",
            "frayed hemp coil",
            "hitch around post",
        ],
    ),
];

fn insert_ritual(db: &SurrealDatabase, session: &str, base: u32, guesses: [&str; 3]) {
    for (i, ((bloom, title, phrases), guess)) in RITUAL.iter().zip(guesses).enumerate() {
        insert(
            db,
            &fx(session, i as i64, base + i as u32, bloom, title, guess)
                .source("authored", phrases),
        );
    }
}

#[test]
fn wake_log_phrase_cache_second_ritual_embeds_only_guess_and_title() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let first = ["a candle on a dock", "the sea rising", "sailor knots"];
    insert_ritual(&db, "s1", 10, first);
    let r1 = CountingProvider::new(FAKE_MODEL);
    let (counts, diag) = score_counting(&db, &r1);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 3,
            skipped: 0
        }
    );
    assert_eq!(diag, "");
    assert_eq!(r1.total(), 15, "3 guesses + 3 titles + 9 phrases");
    for (_, _, phrases) in RITUAL {
        for phrase in phrases {
            assert_eq!(r1.calls(phrase), 1, "{phrase}");
        }
    }
    assert_eq!(cache_rows(&db).len(), 9);

    let second = ["a lamp by the water", "high tide again", "a knot in a rope"];
    insert_ritual(&db, "s2", 100, second);
    let r2 = CountingProvider::new(FAKE_MODEL);
    let (counts, diag) = score_counting(&db, &r2);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 3,
            skipped: 0
        }
    );
    assert_eq!(diag, "");
    assert_eq!(
        r2.total(),
        6,
        "only guess + title per row: {:?}",
        r2.texts()
    );
    for ((_, title, phrases), guess) in RITUAL.iter().zip(second) {
        assert_eq!(r2.calls(guess), 1);
        assert_eq!(r2.calls(title), 1);
        for phrase in phrases {
            assert_eq!(r2.calls(phrase), 0, "{phrase} came from the cache");
        }
    }
    for (i, ((_, _, phrases), guess)) in RITUAL.iter().zip(second).enumerate() {
        assert_eq!(
            stored(&db, "s2", i as i64)["sim_phrase"].as_f64().unwrap(),
            best_phrase_sim(guess, phrases)
        );
    }
    assert_eq!(cache_rows(&db).len(), 9);
}

#[test]
fn wake_log_phrase_cache_keys_on_normalized_text_and_embedding_model() {
    assert_eq!(
        normalize_phrase("  lantern \t by\n\nthe   quay "),
        "lantern by the quay"
    );
    assert_ne!(
        phrase_cache_key(FAKE_MODEL, "Lantern by the quay"),
        phrase_cache_key(FAKE_MODEL, "lantern by the quay"),
        "case is kept"
    );

    let db = SurrealDatabase::open_in_memory().unwrap();
    let variant = "  lantern \t by\n\nthe   quay ";
    insert(
        &db,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", "a candle")
            .source("authored", &[variant, "lantern by the quay"]),
    );
    insert(
        &db,
        &fx("s2", 0, 20, "kn-lantern", "Lantern Notes", "a lamp")
            .source("authored", &["lantern  by the\tquay"]),
    );
    let m1 = CountingProvider::new(FAKE_MODEL);
    let (counts, _) = score_counting(&db, &m1);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 2,
            skipped: 0
        }
    );
    assert_eq!(m1.calls("lantern by the quay"), 1, "{:?}", m1.texts());
    assert_eq!(
        m1.calls(variant),
        0,
        "the normalized form is what gets embedded"
    );
    assert_eq!(m1.calls("lantern  by the\tquay"), 0);
    assert_eq!(cache_rows(&db).len(), 1, "every variant shares one entry");

    insert(
        &db,
        &fx("s3", 0, 30, "kn-lantern", "Lantern Notes", "a torch")
            .source("authored", &["lantern by the quay"]),
    );
    let m2 = CountingProvider::new("test/other-embed");
    let (counts, _) = score_counting(&db, &m2);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    assert_eq!(m2.calls("lantern by the quay"), 1, "another model misses");
    assert_eq!(cache_rows(&db).len(), 2);
}

#[test]
fn wake_log_phrase_cache_ignores_a_vector_of_the_wrong_dimension() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let phrase = "lantern by the quay";
    let guess = "a lamp on the pier";
    let key = phrase_cache_key(FAKE_MODEL, phrase);
    db.wake_phrase_embeddings_store(FAKE_MODEL, &[(key.clone(), vec![1.0, 0.0])])
        .unwrap();
    insert(
        &db,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", guess).source("authored", &[phrase]),
    );
    let p = CountingProvider::new(FAKE_MODEL);
    let (counts, _) = score_counting(&db, &p);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    assert_eq!(p.calls(phrase), 1, "a short cached vector is a miss");
    assert_eq!(
        stored(&db, "s1", 0)["sim_phrase"].as_f64().unwrap(),
        best_phrase_sim(guess, &[phrase])
    );
    assert_eq!(
        db.wake_phrase_embeddings(FAKE_MODEL, std::slice::from_ref(&key))
            .unwrap()[&key]
            .len(),
        DIM
    );
}

#[test]
fn wake_log_phrase_cache_hit_gives_the_same_sim_phrase_as_a_fresh_embed() {
    let phrases = ["glow  over harbor", "a lamp left lit"];
    let guess = "a lamp glowing over the harbor";

    let fresh = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &fresh,
        &fx("s2", 0, 20, "kn-lantern", "Lantern Notes", guess).source("authored", &phrases),
    );
    let p = CountingProvider::new(FAKE_MODEL);
    score_counting(&fresh, &p);
    assert_eq!(p.calls("glow over harbor"), 1);
    let from_fresh = stored(&fresh, "s2", 0)["sim_phrase"].as_f64().unwrap();

    let cached = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &cached,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", "something else")
            .source("authored", &phrases),
    );
    score_counting(&cached, &CountingProvider::new(FAKE_MODEL));
    insert(
        &cached,
        &fx("s2", 0, 20, "kn-lantern", "Lantern Notes", guess).source("authored", &phrases),
    );
    let p = CountingProvider::new(FAKE_MODEL);
    score_counting(&cached, &p);
    assert_eq!(p.calls("glow over harbor") + p.calls("a lamp left lit"), 0);
    let from_cache = stored(&cached, "s2", 0)["sim_phrase"].as_f64().unwrap();

    assert_eq!(from_cache.to_bits(), from_fresh.to_bits());
    assert_eq!(from_cache, best_phrase_sim(guess, &phrases));

    let keys: Vec<String> = phrases
        .iter()
        .map(|p| phrase_cache_key(FAKE_MODEL, &normalize_phrase(p)))
        .collect();
    let read = cached.wake_phrase_embeddings(FAKE_MODEL, &keys).unwrap();
    for (key, phrase) in keys.iter().zip(phrases) {
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&read[key]), bits(&odd_vec(&normalize_phrase(phrase))));
    }
}

#[test]
fn wake_log_phrase_cache_stores_no_phrase_text() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let phrases = ["quillfeather beacon", "marrowlight  sounding"];
    insert(
        &db,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", "a lamp").source("authored", &phrases),
    );
    run_score(&db, AGENT);

    let rows = cache_rows(&db);
    assert_eq!(rows.len(), 2);
    let mut ids: Vec<String> = Vec::new();
    for row in &rows {
        let mut fields: Vec<&str> = row["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        fields.sort();
        assert_eq!(fields, ["created_at", "embedding", "embedding_model", "id"]);
        assert_eq!(row["embedding_model"], FAKE_MODEL);
        assert_eq!(row["embedding"].as_array().unwrap().len(), DIM);
        let dump = row["dump"].as_str().unwrap();
        for word in ["quillfeather", "beacon", "marrowlight", "sounding"] {
            assert!(!dump.contains(word), "{dump}");
        }
        ids.push(row["id"].as_str().unwrap().to_string());
    }
    for phrase in phrases {
        let key = phrase_cache_key(FAKE_MODEL, &normalize_phrase(phrase));
        assert_eq!(key.len(), 64);
        assert!(ids.iter().any(|id| id.contains(&key)), "{key} in {ids:?}");
    }
}

#[test]
fn wake_log_phrase_cache_errors_never_skip_the_row() {
    let phrases = ["lantern by the quay", "a lamp left lit"];
    let guess = "a lamp on the pier";
    let seed = |db: &SurrealDatabase| {
        insert(
            db,
            &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", guess).source("authored", &phrases),
        );
    };
    let check = |db: &SurrealDatabase, diag: &str, stage: &str| {
        let row = stored(db, "s1", 0);
        assert_eq!(row["scored"], true);
        assert_eq!(
            row["sim_phrase"].as_f64().unwrap(),
            best_phrase_sim(guess, &phrases)
        );
        assert!(
            diag.contains(&format!("({stage} failed, ignored)")),
            "{diag}"
        );
        for text in phrases.iter().chain([&guess, &"Lantern Notes"]) {
            assert!(!diag.contains(text), "{diag}");
        }
        assert!(!diag.contains("0."), "no value in diag: {diag}");
    };

    // A cache that cannot be written: the row scores, the vectors are dropped.
    let db = SurrealDatabase::open_in_memory().unwrap();
    db.test_exec(
        "DEFINE FIELD OVERWRITE embedding_model ON wake_phrase_embedding TYPE string \
         ASSERT false",
    )
    .unwrap();
    seed(&db);
    let p = CountingProvider::new(FAKE_MODEL);
    let (counts, diag) = score_counting(&db, &p);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    check(&db, &diag, "phrase cache write");
    assert!(cache_rows(&db).is_empty());

    // A cache that cannot be read: every phrase is a miss and is embedded.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let keys: Vec<(String, Vec<f32>)> = phrases
        .iter()
        .map(|p| (phrase_cache_key(FAKE_MODEL, p), odd_vec(p)))
        .collect();
    db.wake_phrase_embeddings_store(FAKE_MODEL, &keys).unwrap();
    // A record link where a vector belongs cannot be read back as JSON.
    db.test_exec(
        "DEFINE FIELD OVERWRITE embedding ON wake_phrase_embedding TYPE any;
        UPDATE wake_phrase_embedding SET embedding = id",
    )
    .unwrap();
    assert!(
        db.wake_phrase_embeddings(
            FAKE_MODEL,
            &keys.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
        )
        .is_err(),
        "precondition: the read fails"
    );
    seed(&db);
    let p = CountingProvider::new(FAKE_MODEL);
    let (counts, diag) = score_counting(&db, &p);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    for phrase in phrases {
        assert_eq!(p.calls(phrase), 1, "a failed read is all-miss");
    }
    check(&db, &diag, "phrase cache read");
}

#[test]
fn wake_log_blank_phrases_are_never_embedded_or_cached() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(
        &db,
        &fx("s1", 0, 10, "kn-lantern", "Lantern Notes", "a candle").source("authored", &["", "  "]),
    );
    let p = CountingProvider::new(FAKE_MODEL);
    let (counts, diag) = score_counting(&db, &p);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    assert_eq!(diag, "");
    let row = stored(&db, "s1", 0);
    assert_eq!(row["scored"], true, "scored, not skipped");
    assert!(
        is_none(&row, "sim_phrase"),
        "no non-blank phrase, no sim_phrase"
    );
    assert_eq!(p.total(), 2, "guess + title only: {:?}", p.texts());
    assert_eq!(p.calls(""), 0);
    assert!(cache_rows(&db).is_empty());

    let real = "real phrase";
    let guess = "a lamp";
    insert(
        &db,
        &fx("s2", 0, 20, "kn-lantern", "Lantern Notes", guess).source("authored", &["", real]),
    );
    let p = CountingProvider::new(FAKE_MODEL);
    let (counts, _) = score_counting(&db, &p);
    assert_eq!(
        counts,
        ScoreCounts {
            rows: 1,
            skipped: 0
        }
    );
    assert_eq!(p.calls(""), 0);
    assert_eq!(p.calls(real), 1);
    assert_eq!(
        p.total(),
        3,
        "guess + title + the real phrase: {:?}",
        p.texts()
    );
    assert_eq!(cache_rows(&db).len(), 1);
    assert_eq!(
        stored(&db, "s2", 0)["sim_phrase"].as_f64().unwrap(),
        best_phrase_sim(guess, &[real])
    );
}

// =============================================================================
// --out must name a regular file
// =============================================================================

#[test]
fn wake_log_out_must_name_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let refused = |path: &std::path::Path, why: &str| {
        let err = output_target("report", false, Some(path.to_path_buf()))
            .expect_err("not a regular file");
        let msg = format!("{err:#}");
        assert!(msg.contains(why), "{msg}");
        assert!(msg.contains(&path.display().to_string()), "{msg}");
    };
    let opened =
        |path: &std::path::Path| match output_target("report", false, Some(path.to_path_buf()))
            .unwrap()
        {
            OutputTarget::File(file, p) => {
                assert_eq!(p, path);
                file.metadata().unwrap()
            }
            OutputTarget::Terminal => panic!("{} gave Terminal", path.display()),
        };
    refused(dir.path(), "--out must name a regular file");

    let missing = dir.path().join("new.json");
    assert!(opened(&missing).is_file());
    assert_eq!(
        std::fs::read(&missing).unwrap(),
        b"",
        "created, not written"
    );
    let file = dir.path().join("old.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(opened(&file).is_file());
    assert_eq!(std::fs::read(&file).unwrap(), b"x", "opened, not truncated");

    #[cfg(unix)]
    {
        refused(
            std::path::Path::new("/dev/null"),
            "--out must name a regular file",
        );
        let to_dir = dir.path().join("to-dir");
        std::os::unix::fs::symlink(dir.path(), &to_dir).unwrap();
        refused(&to_dir, "--out must name a regular file");
        let to_file = dir.path().join("to-file.txt");
        std::os::unix::fs::symlink(&file, &to_file).unwrap();
        assert!(opened(&to_file).is_file());
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
    }
}

// =============================================================================
// The text layout escapes control characters
// =============================================================================

#[test]
fn terminal_text_layout_does_not_pass_raw_escape_sequences() {
    let title = "Lantern \u{1b}]52;c;Y2xpcA==\u{7} Notes";
    let guess = "a lamp \u{1b}[2J\u{1b}[H cleared";
    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(&db, &fx("s1", 0, 10, "kn-lantern", title, guess));
    insert(
        &db,
        &fx(
            "s1",
            1,
            11,
            "kn-tide",
            "Tide Table",
            "first line\nsecond line",
        ),
    );
    insert(
        &db,
        &fx(
            "s1",
            2,
            12,
            "kn-rope",
            "Rope Knots",
            "left \u{202E}thgir\u{2066} end",
        ),
    );
    run_score(&db, AGENT);
    let text = run(
        WakeLogCommands::Wake { wake: 1, out: None },
        Some(AGENT.to_string()),
        true,
        || Ok(db),
        no_model,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        !text.contains('\u{1b}'),
        "raw ESC reaches the terminal: {text:?}"
    );
    assert!(!text.contains('\u{7}'), "{text:?}");
    assert!(
        !text.contains('\u{202E}') && !text.contains('\u{2066}'),
        "{text:?}"
    );
    assert!(
        text.lines()
            .any(|l| l == r"  guess  left \u{202e}thgir\u{2066} end"),
        "{text}"
    );
    for c in [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ] {
        assert_eq!(
            shown(&format!("a{c}b")),
            format!("a{}b", c.escape_default()),
            "U+{:04X}",
            c as u32
        );
    }
    assert_eq!(
        shown("café naïve"),
        "café naïve",
        "other non-ASCII is not escaped"
    );
    assert!(
        text.lines()
            .any(|l| l == r"  guess  first line\nsecond line"),
        "a newline in a guess shows as \\n on one line: {text}"
    );
    assert!(
        text.contains(r"guess  a lamp \u{1b}[2J\u{1b}[H cleared"),
        "{text}"
    );

    let db = SurrealDatabase::open_in_memory().unwrap();
    insert(&db, &fx("s1", 0, 10, "kn-lantern", title, guess));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.json");
    run(
        WakeLogCommands::Wake {
            wake: 1,
            out: Some(path.clone()),
        },
        Some(AGENT.to_string()),
        false,
        || Ok(db),
        no_model,
        &mut Vec::new(),
    )
    .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["rows"][0]["guess"], guess, "JSON keeps the stored text");
    assert_eq!(v["rows"][0]["title_shown"], title);
}

// =============================================================================
// Per-embed timing (not run by default; run in release by hand)
// =============================================================================

fn median_ms(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

#[test]
#[ignore = "loads the real embedding model; run by hand in release with --ignored --nocapture"]
fn wake_log_embed_timing() {
    use std::time::Instant;

    let provider = crate::embeddings::TractProvider::new().expect("load the embedding model");
    let guess = "I think this one is about a lantern left burning on the pier at night";
    let phrase = "lantern on the pier";
    for _ in 0..2 {
        provider.embed(guess).unwrap();
    }

    let time = |text: &str| -> Vec<f64> {
        (0..20)
            .map(|_| {
                let start = Instant::now();
                provider.embed(text).unwrap();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect()
    };
    let guess_ms = median_ms(time(guess));
    let phrase_ms = median_ms(time(phrase));
    println!("wake_log_embed_timing: guess-length median {guess_ms:.1} ms per embed (n=20)");
    println!("wake_log_embed_timing: phrase-length median {phrase_ms:.1} ms per embed (n=20)");
}
