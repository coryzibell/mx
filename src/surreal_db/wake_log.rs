//! Reads and writes for `mx memory wake-log`.
//!
//! Every `wake_guess` statement here binds `$agent` and filters
//! `agent = $agent` in its own WHERE clause, so no row of another agent is ever
//! read. Only the statements that build a centroid project `embedding`; the
//! read commands never do. The phrase-embedding cache holds no agent's text,
//! only vectors keyed by a hash, so it is shared.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::{SurrealConnection, SurrealDatabase};
use crate::wake_guess::PRIOR_SET_SIZE;
use crate::wake_log::{HistoryScope, LogRow, PendingRow, ScoredFields, Substrate};

/// The columns the read commands show. `embedding` is deliberately absent.
const LOG_ROW_FIELDS: &str = "wake, <string>ts AS ts_text, ts, session_id, position, bloom_id,
    chunk_index, chunk_total, bloom_position, bloom_total, title_shown, guess, model_id,
    phrase_source, bucket, scored_at != NONE AS scored,
    sim_phrase, sim_content, sim_title, sim_prior, sim_prior_null, prior_n,
    sim_prior_same, sim_prior_null_same,
    sim_prior_cross, sim_prior_null_cross";

/// The shared filter of every Axis B read: this agent's scored rows under the
/// same embedding model, from another session, logged strictly earlier.
const ELIGIBLE: &str = "agent = $agent AND scored_at != NONE AND embedding_model = $em
    AND session_id != $session_id AND ts < <datetime>$ts";

fn substrate_clause(substrate: Substrate<'_>) -> &'static str {
    match substrate {
        Substrate::All => "",
        Substrate::Same(_) => "AND model_id = $model",
        Substrate::Cross(_) => "AND model_id != $model AND model_id != NONE",
    }
}

fn substrate_model(substrate: Substrate<'_>) -> Option<String> {
    match substrate {
        Substrate::All => None,
        Substrate::Same(m) | Substrate::Cross(m) => Some(m.to_string()),
    }
}

fn vector_of(value: &Value) -> Option<Vec<f32>> {
    value.as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_f64())
            .map(|f| f as f32)
            .collect()
    })
}

fn text(obj: &Value, key: &str) -> String {
    obj[key].as_str().unwrap_or_default().to_string()
}

fn log_row(obj: &Value) -> LogRow {
    LogRow {
        wake: obj["wake"].as_i64(),
        ts: text(obj, "ts_text"),
        session_id: text(obj, "session_id"),
        position: obj["position"].as_i64().unwrap_or(0),
        bloom_id: text(obj, "bloom_id"),
        chunk_index: obj["chunk_index"].as_i64().unwrap_or(0),
        chunk_total: obj["chunk_total"].as_i64().unwrap_or(1),
        bloom_position: obj["bloom_position"].as_i64().unwrap_or(0),
        bloom_total: obj["bloom_total"].as_i64().unwrap_or(0),
        title_shown: text(obj, "title_shown"),
        guess: text(obj, "guess"),
        model_id: obj["model_id"].as_str().map(str::to_string),
        phrase_source: text(obj, "phrase_source"),
        bucket: text(obj, "bucket"),
        scored: obj["scored"].as_bool().unwrap_or(false),
        sim_phrase: obj["sim_phrase"].as_f64(),
        sim_content: obj["sim_content"].as_f64(),
        sim_title: obj["sim_title"].as_f64(),
        sim_prior: obj["sim_prior"].as_f64(),
        sim_prior_null: obj["sim_prior_null"].as_f64(),
        prior_n: obj["prior_n"].as_i64(),
        sim_prior_same: obj["sim_prior_same"].as_f64(),
        sim_prior_null_same: obj["sim_prior_null_same"].as_f64(),
        sim_prior_cross: obj["sim_prior_cross"].as_f64(),
        sim_prior_null_cross: obj["sim_prior_null_cross"].as_f64(),
    }
}

impl SurrealDatabase {
    /// Every pending row of `agent`, in scoring order: ascending
    /// `(ts, session_id, position)`. A row an older binary wrote has no
    /// `scored_at` at all, which reads as NONE and so counts as pending.
    pub fn wake_log_pending(&self, agent: &str) -> Result<Vec<PendingRow>> {
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT meta::id(id) AS key, <string>ts AS ts_text, ts, session_id,
                        position, bloom_id, title_shown, guess, model_id, phrase_source,
                        phrases, chunk_index, chunk_total
                    FROM wake_guess
                    WHERE agent = $agent AND scored_at = NONE
                    ORDER BY ts, session_id, position",
                )
                .bind(("agent", agent.to_string()))
                .await
                .context("Failed to read pending wake guess rows")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows
                .iter()
                .map(|obj| PendingRow {
                    key: obj["key"].clone(),
                    ts: text(obj, "ts_text"),
                    session_id: text(obj, "session_id"),
                    position: obj["position"].as_i64().unwrap_or(0),
                    bloom_id: text(obj, "bloom_id"),
                    title_shown: text(obj, "title_shown"),
                    guess: text(obj, "guess"),
                    model_id: obj["model_id"].as_str().map(str::to_string),
                    phrase_source: text(obj, "phrase_source"),
                    phrases: obj["phrases"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    chunk_index: obj["chunk_index"].as_i64().unwrap_or(0),
                    chunk_total: obj["chunk_total"].as_i64().unwrap_or(1),
                })
                .collect())
        })
    }

    /// For each `(bloom_id, title_shown)` key, the embeddings of its most
    /// recent `PRIOR_SET_SIZE` eligible rows under `substrate`. All keys go in
    /// one round trip, one statement per key.
    pub fn wake_log_prior_vectors(
        &self,
        scope: &HistoryScope<'_>,
        keys: &[(String, String)],
        substrate: Substrate<'_>,
    ) -> Result<Vec<Vec<Vec<f32>>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let split = substrate_clause(substrate);
        let sql: String = (0..keys.len())
            .map(|i| {
                format!(
                    "SELECT embedding, ts, session_id, position FROM wake_guess
                    WHERE {ELIGIBLE} AND bloom_id = $bloom_{i} AND title_shown = $title_{i} {split}
                    ORDER BY ts DESC, session_id DESC, position DESC LIMIT $prior_k;"
                )
            })
            .collect();

        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                let mut query = db
                    .query(sql.as_str())
                    .bind(("agent", scope.agent.to_string()))
                    .bind(("em", scope.embedding_model.to_string()))
                    .bind(("session_id", scope.session_id.to_string()))
                    .bind(("ts", scope.ts.to_string()))
                    .bind(("prior_k", PRIOR_SET_SIZE as i64));
                if let Some(model) = substrate_model(substrate) {
                    query = query.bind(("model", model));
                }
                for (i, (bloom, title)) in keys.iter().enumerate() {
                    query = query
                        .bind((format!("bloom_{i}"), bloom.clone()))
                        .bind((format!("title_{i}"), title.clone()));
                }
                query
                    .await
                    .context("Failed to read prior wake guess vectors")
            })?;
            let mut sets = Vec::with_capacity(keys.len());
            for i in 0..keys.len() {
                let rows: Vec<Value> = response.take(i)?;
                sets.push(
                    rows.iter()
                        .filter_map(|r| vector_of(&r["embedding"]))
                        .collect(),
                );
            }
            Ok(sets)
        })
    }

    /// Eligible rows of other blooms under `substrate`, newest first, as
    /// `(bloom_id, title_shown)`. Projects no embedding.
    pub fn wake_log_baseline_rows(
        &self,
        scope: &HistoryScope<'_>,
        bloom_id: &str,
        substrate: Substrate<'_>,
    ) -> Result<Vec<(String, String)>> {
        let sql = format!(
            "SELECT bloom_id, title_shown, ts, session_id, position FROM wake_guess
            WHERE {ELIGIBLE} AND bloom_id != $bloom_id {}
            ORDER BY ts DESC, session_id DESC, position DESC",
            substrate_clause(substrate)
        );
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                let mut query = db
                    .query(sql.as_str())
                    .bind(("agent", scope.agent.to_string()))
                    .bind(("em", scope.embedding_model.to_string()))
                    .bind(("session_id", scope.session_id.to_string()))
                    .bind(("ts", scope.ts.to_string()))
                    .bind(("bloom_id", bloom_id.to_string()));
                if let Some(model) = substrate_model(substrate) {
                    query = query.bind(("model", model));
                }
                query
                    .await
                    .context("Failed to read baseline wake guess rows")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows
                .iter()
                .map(|r| (text(r, "bloom_id"), text(r, "title_shown")))
                .collect())
        })
    }

    /// Cosine similarity of `guess` to every stored vector of one entry under
    /// `embedding_model`: the mean-pooled `knowledge.embedding` and each
    /// `embedding_chunk` row. Computed server side; no vector is projected.
    /// The caller checks the entry is readable first, because
    /// `embedding_chunk` carries no visibility of its own.
    pub fn wake_log_content_similarities(
        &self,
        record_key: &str,
        chunk_entry_ids: &[String],
        guess: &[f32],
        embedding_model: &str,
    ) -> Result<Vec<f64>> {
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT vector::similarity::cosine(embedding, $v) AS s
                        FROM type::thing('knowledge', $key)
                        WHERE embedding != NONE AND embedding_model = $em
                            AND array::len(embedding) = $dim;
                    SELECT vector::similarity::cosine(embedding, $v) AS s
                        FROM embedding_chunk
                        WHERE entry_id IN $entry_ids AND embedding_model = $em
                            AND array::len(embedding) = $dim;",
                )
                .bind(("key", record_key.to_string()))
                .bind(("entry_ids", chunk_entry_ids.to_vec()))
                .bind(("v", guess.to_vec()))
                .bind(("em", embedding_model.to_string()))
                .bind(("dim", guess.len() as i64))
                .await
                .context("Failed to compare a guess with its entry's vectors")
            })?;
            let entry: Vec<Value> = response.take(0)?;
            let chunks: Vec<Value> = response.take(1)?;
            Ok(entry
                .iter()
                .chain(chunks.iter())
                .filter_map(|r| r["s"].as_f64())
                .collect())
        })
    }

    /// Write one row's scores in a single conditional UPDATE: the row is either
    /// fully scored or untouched. Returns false when it matched no record (a
    /// concurrent `score` got there first).
    pub fn wake_log_write_score(
        &self,
        agent: &str,
        key: &Value,
        fields: &ScoredFields,
    ) -> Result<bool> {
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(
                    "UPDATE type::thing('wake_guess', $key) SET
                        embedding = $embedding,
                        embedding_model = $em,
                        sim_phrase = $sim_phrase,
                        sim_content = $sim_content,
                        sim_title = $sim_title,
                        sim_prior = $sim_prior,
                        sim_prior_null = $sim_prior_null,
                        prior_n = $prior_n,
                        sim_prior_same = $sim_prior_same,
                        sim_prior_null_same = $sim_prior_null_same,
                        prior_n_same = $prior_n_same,
                        sim_prior_cross = $sim_prior_cross,
                        sim_prior_null_cross = $sim_prior_null_cross,
                        prior_n_cross = $prior_n_cross,
                        scored_at = time::now()
                    WHERE agent = $agent AND scored_at = NONE
                    RETURN VALUE meta::id(id)",
                )
                .bind(("key", key.clone()))
                .bind(("agent", agent.to_string()))
                .bind(("embedding", fields.embedding.clone()))
                .bind(("em", fields.embedding_model.clone()))
                .bind(("sim_phrase", fields.sim_phrase))
                .bind(("sim_content", fields.sim_content))
                .bind(("sim_title", fields.sim_title))
                .bind(("sim_prior", fields.all.sim_prior))
                .bind(("sim_prior_null", fields.all.sim_prior_null))
                .bind(("prior_n", fields.all.prior_n))
                .bind(("sim_prior_same", fields.same.and_then(|s| s.sim_prior)))
                .bind((
                    "sim_prior_null_same",
                    fields.same.and_then(|s| s.sim_prior_null),
                ))
                .bind(("prior_n_same", fields.same.map(|s| s.prior_n)))
                .bind(("sim_prior_cross", fields.cross.and_then(|s| s.sim_prior)))
                .bind((
                    "sim_prior_null_cross",
                    fields.cross.and_then(|s| s.sim_prior_null),
                ))
                .bind(("prior_n_cross", fields.cross.map(|s| s.prior_n)))
                .await
                .context("Failed to write wake guess scores")
            })?;
            let errors = response.take_errors();
            if !errors.is_empty() {
                return Err(anyhow!(
                    "SurrealDB error writing wake guess scores: {} statement(s) failed",
                    errors.len()
                ));
            }
            let updated: Vec<Value> = response.take(0)?;
            Ok(!updated.is_empty())
        })
    }

    /// The cached phrase vectors among `keys` under `embedding_model`, by key,
    /// in one statement. Absent keys are misses.
    pub fn wake_phrase_embeddings(
        &self,
        embedding_model: &str,
        keys: &[String],
    ) -> Result<HashMap<String, Vec<f32>>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT meta::id(id) AS key, embedding
                    FROM array::map($keys, |$k| type::thing('wake_phrase_embedding', $k))
                    WHERE embedding_model = $em",
                )
                .bind(("keys", keys.to_vec()))
                .bind(("em", embedding_model.to_string()))
                .await
                .context("Failed to read cached phrase embeddings")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows
                .iter()
                .filter_map(|r| Some((r["key"].as_str()?.to_string(), vector_of(&r["embedding"])?)))
                .collect())
        })
    }

    /// Store phrase vectors under their cache keys, one UPSERT each, in one
    /// round trip.
    pub fn wake_phrase_embeddings_store(
        &self,
        embedding_model: &str,
        entries: &[(String, Vec<f32>)],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let sql: String = (0..entries.len())
            .map(|i| {
                format!(
                    "UPSERT type::thing('wake_phrase_embedding', $key_{i})
                    SET embedding_model = $em, embedding = $v_{i} RETURN NONE;"
                )
            })
            .collect();
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                let mut query = db
                    .query(sql.as_str())
                    .bind(("em", embedding_model.to_string()));
                for (i, (key, vector)) in entries.iter().enumerate() {
                    query = query
                        .bind((format!("key_{i}"), key.clone()))
                        .bind((format!("v_{i}"), vector.clone()));
                }
                query.await.context("Failed to store phrase embeddings")
            })?;
            let errors = response.take_errors();
            if !errors.is_empty() {
                return Err(anyhow!(
                    "SurrealDB error storing phrase embeddings: {} statement(s) failed",
                    errors.len()
                ));
            }
            Ok(())
        })
    }

    /// The highest wake number with at least one scored row for `agent`.
    pub fn wake_log_latest_scored_wake(&self, agent: &str) -> Result<Option<i64>> {
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT wake FROM wake_guess
                    WHERE agent = $agent AND wake != NONE AND scored_at != NONE
                    ORDER BY wake DESC LIMIT 1",
                )
                .bind(("agent", agent.to_string()))
                .await
                .context("Failed to read the latest scored wake")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows.first().and_then(|r| r["wake"].as_i64()))
        })
    }

    /// Every row `agent` logged under wake `wake`, in sequence order.
    pub fn wake_log_rows_for_wake(&self, agent: &str, wake: i64) -> Result<Vec<LogRow>> {
        let sql = format!(
            "SELECT {LOG_ROW_FIELDS} FROM wake_guess
            WHERE agent = $agent AND wake = $wake
            ORDER BY ts, position"
        );
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(sql.as_str())
                    .bind(("agent", agent.to_string()))
                    .bind(("wake", wake))
                    .await
                    .context("Failed to read wake guess rows for a wake")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows.iter().map(log_row).collect())
        })
    }

    /// The most recent `limit` rows `agent` logged for one entry, newest first.
    pub fn wake_log_rows_for_bloom(
        &self,
        agent: &str,
        bloom_id: &str,
        limit: usize,
    ) -> Result<Vec<LogRow>> {
        let sql = format!(
            "SELECT {LOG_ROW_FIELDS} FROM wake_guess
            WHERE agent = $agent AND bloom_id = $bloom_id
            ORDER BY ts DESC, position DESC
            LIMIT $limit"
        );
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(sql.as_str())
                    .bind(("agent", agent.to_string()))
                    .bind(("bloom_id", bloom_id.to_string()))
                    .bind(("limit", limit as i64))
                    .await
                    .context("Failed to read wake guess rows for an entry")
            })?;
            let rows: Vec<Value> = response.take(0)?;
            Ok(rows.iter().map(log_row).collect())
        })
    }
}
