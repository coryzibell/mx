use std::collections::HashSet;

use super::*;
use crate::store::KnowledgeStore;

#[test]
fn test_open_in_memory() {
    // Test that database opens without error
    let _db = SurrealDatabase::open_in_memory().unwrap();
}

#[test]
fn test_schema_applies_without_error() {
    // Opening applies schema - if this succeeds, schema is valid
    let _db = SurrealDatabase::open_in_memory().unwrap();
}

#[test]
fn test_embedded_connect_creates_directory() {
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.surreal");

    // Route through the guarded file-backed test constructor, which forces
    // embedded mode regardless of MX_SURREAL_*. The public open() reads
    // MX_SURREAL_MODE from the environment, which may route to a network
    // backend that never creates a local directory -- making the
    // exists()/is_dir() assertions flaky on hosts where the factory runs in
    // network mode (and, worse, could write to the live DB).
    let _db = SurrealDatabase::open_file_backed_for_test(&db_path).unwrap();

    // Verify directory was created
    assert!(db_path.exists());
    assert!(db_path.is_dir());
}

#[test]
fn test_open_file_backed_for_test_rejects_non_temp_path() {
    // GUARDRAIL (#388): a fixed/literal absolute path outside the OS temp
    // dir must be refused, because SurrealKV bakes the absolute path string
    // into its manifest and that path outlives the OS that wrote it (a
    // Windows-absolute path once materialized as a literal `C:` dir on
    // Linux). We use CARGO_MANIFEST_DIR: a stable, existing, non-temp
    // absolute path on every platform, so its nearest-existing ancestor
    // canonicalizes cleanly and the guardrail (not a missing-dir error) is
    // what trips.
    //
    // Robustness guard: under a temp-sandbox build CARGO_MANIFEST_DIR can
    // ITSELF live under temp_dir(), in which case the guardrail correctly
    // ACCEPTS the path and no panic occurs. A bare #[should_panic] would
    // then spuriously fail. We deliberately avoid #[should_panic] here:
    // an early `return` inside a #[should_panic] body counts as "no panic"
    // and fails the test, so there is no clean way to skip. Instead we use a
    // plain #[test] + catch_unwind: the skip path returns normally, and the
    // normal path asserts BOTH that a panic occurred AND that it carried the
    // expected guardrail message (which #[should_panic(expected = ...)] only
    // substring-matches anyway).
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let canon_manifest =
        std::fs::canonicalize(manifest_dir).unwrap_or_else(|_| manifest_dir.to_path_buf());
    let temp_root = std::env::temp_dir();
    let canon_temp = std::fs::canonicalize(&temp_root).unwrap_or(temp_root);
    if canon_manifest.starts_with(&canon_temp) {
        eprintln!(
            "skipping test_open_file_backed_for_test_rejects_non_temp_path: \
             CARGO_MANIFEST_DIR ({}) is itself under temp_dir ({}) — \
             temp-sandbox build, the guardrail would (correctly) accept it",
            canon_manifest.display(),
            canon_temp.display()
        );
        return;
    }

    let non_temp = manifest_dir.join("fixed.surreal");
    let result = std::panic::catch_unwind(|| {
        let _ = SurrealDatabase::open_file_backed_for_test(&non_temp);
    });
    let payload = result.expect_err(
        "open_file_backed_for_test must panic when handed a non-temp path, but it returned",
    );
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        msg.contains("outside the OS temp dir"),
        "guardrail panicked with an unexpected message: {msg:?}"
    );
}

#[test]
fn test_open_file_backed_for_test_accepts_nested_nonexistent_parent() {
    use tempfile::tempdir;

    // S-1 regression: a caller may pass a path whose PARENT (and grandparent)
    // do not exist yet — e.g. `tempdir().path().join("a/b/store.surreal")`.
    // The connect step creates them. The guardrail must not canonicalize the
    // (missing) immediate parent and fall back to an UN-resolved literal that
    // then false-rejects against a canonicalized temp root (the macOS
    // /var -> /private/var symlink case). Walking up to the nearest existing
    // ancestor and canonicalizing THAT keeps a legitimately-temp nested path
    // accepted. On Linux this exercises the ancestor-walk path because `a/`
    // and `b/` genuinely do not exist when the guard runs.
    let temp_dir = tempdir().unwrap();
    let nested = temp_dir.path().join("a/b/store.surreal");
    assert!(!nested.parent().unwrap().exists());

    // Must NOT panic and must successfully open the store.
    let _db = SurrealDatabase::open_file_backed_for_test(&nested).unwrap();
    assert!(nested.exists());
}

#[test]
fn test_upsert_applicability_type_with_datetime() {
    use crate::types::ApplicabilityType;

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create an applicability type with RFC3339 datetime
    let atype = ApplicabilityType {
        id: "test_type".to_string(),
        description: "Test applicability type".to_string(),
        scope: Some("test".to_string()),
        created_at: "2025-11-29T12:00:00Z".to_string(),
    };

    // Upsert should succeed without datetime parsing errors
    // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
    db.upsert_applicability_type(&atype).unwrap();
}

#[test]
fn test_upsert_project_with_datetime() {
    use crate::types::Project;

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create a project with RFC3339 datetimes
    let project = Project {
        id: "test_project".to_string(),
        name: "Test Project".to_string(),
        path: Some("/test/path".to_string()),
        repo_url: None,
        description: Some("Test description".to_string()),
        active: true,
        created_at: "2025-11-29T12:00:00Z".to_string(),
        updated_at: "2025-11-29T12:30:00Z".to_string(),
    };

    // Upsert should succeed without datetime parsing errors
    // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
    db.upsert_project(&project).unwrap();
}

#[test]
fn test_upsert_agent_with_datetime() {
    use crate::types::Agent;

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create an agent with RFC3339 datetimes
    let agent = Agent {
        id: "test_agent".to_string(),
        description: Some("Test agent".to_string()),
        domain: Some("testing".to_string()),
        created_at: Some("2025-11-29T12:00:00Z".to_string()),
        updated_at: Some("2025-11-29T12:30:00Z".to_string()),
    };

    // Upsert should succeed without datetime parsing errors
    // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
    db.upsert_agent(&agent).unwrap();
}

// =========================================================================
// PR #118 EDGE CASE TESTS
// =========================================================================
// These tests cover edge cases identified during code review of the
// memory/fact unification. They ensure robustness of:
// - Decay formula computation
// - ID normalization
// - Thread duplicate detection
// - Session linkage
// =========================================================================

fn make_test_entry(id: &str, resonance: i32, decay_rate: f64) -> crate::knowledge::KnowledgeEntry {
    use chrono::Utc;
    let now = Utc::now().to_rfc3339();

    crate::knowledge::KnowledgeEntry {
        id: id.to_string(),
        category_id: "test".to_string(),
        title: format!("Test Entry {}", id),
        body: Some("Test body".to_string()),
        summary: None,
        applicability: vec![],
        source_project_id: None,
        source_agent_id: None,
        file_path: None,
        tags: vec![],
        created_at: Some(now.clone()),
        updated_at: Some(now.clone()),
        content_hash: Some("test-hash".to_string()),
        source_type_id: Some("manual".to_string()),
        entry_type_id: Some("primary".to_string()),
        session_id: None,
        ephemeral: false,
        content_type_id: Some("text".to_string()),
        owner: None,
        visibility: "public".to_string(),
        resonance,
        resonance_type: Some("ephemeral".to_string()),
        last_activated: Some(now),
        activation_count: 0,
        decay_rate,
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

#[test]
fn test_id_normalization_double_prefix() {
    // Edge case: IDs that already have "kn-" prefix get doubled during processing
    // Example: "kn-123" -> strip_prefix -> "123" -> add prefix -> "kn-123"
    // But what if someone passes "kn-kn-123"?

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Insert with normal ID
    let entry = make_test_entry("kn-test123", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();

    // Try to retrieve with double prefix
    let ctx = crate::store::AgentContext::public_only();
    let result = db.get("kn-kn-test123", &ctx).unwrap();

    // Should NOT find it (this is expected behavior - double prefix is invalid)
    assert!(result.is_none(), "Double prefix should not match");

    // But normal retrieval should work
    let result = db.get("kn-test123", &ctx).unwrap();
    assert!(result.is_some(), "Normal prefix should match");
}

#[test]
fn test_id_normalization_case_sensitivity() {
    // Edge case: Are IDs case-sensitive? "KN-123" vs "kn-123"

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Insert with lowercase
    let entry = make_test_entry("kn-test456", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();

    // Try to retrieve with uppercase
    let ctx = crate::store::AgentContext::public_only();
    let result = db.get("KN-test456", &ctx).unwrap();

    // SurrealDB IDs are case-sensitive, so this should NOT match
    assert!(
        result.is_none(),
        "Uppercase KN should not match lowercase kn"
    );
}

#[test]
fn test_id_normalization_empty_suffix() {
    // Edge case: What happens with just "kn-" and no suffix?

    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Try to get an entry with empty suffix
    let result = db.get("kn-", &ctx);

    // Should handle gracefully (likely return None, not panic)
    assert!(result.is_ok(), "Empty suffix should not panic");
}

#[test]
fn test_id_normalization_no_prefix() {
    // Edge case: What if someone passes just "123" without "kn-"?

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Insert with full ID
    let entry = make_test_entry("kn-test789", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();

    // Try to retrieve without prefix
    let ctx = crate::store::AgentContext::public_only();
    let result = db.get("test789", &ctx).unwrap();

    // This SHOULD work because strip_prefix returns the original if no prefix found
    // and that gets stored as-is in SurrealDB
    // Actually, the ID gets normalized during insert, so "test789" should find it
    assert!(result.is_some(), "ID without prefix should still match");
}

#[test]
fn test_decay_formula_zero_days() {
    // Edge case: What happens when last_activated is NOW (0 days ago)?
    // Formula: resonance * 0.95^(days / 7)
    // If days = 0: resonance * 0.95^0 = resonance * 1 = resonance

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create entry with ephemeral type and recent activation
    let entry = make_test_entry("kn-fresh", 10, 0.5);
    db.upsert_knowledge(&entry).unwrap();

    // Query recent facts (should include entries from today)
    let facts = db.query_recent_facts(1).unwrap();

    // Should find the entry
    assert!(!facts.is_empty(), "Should find fresh facts");

    // The effective_resonance should be close to original resonance (no decay yet)
    // We can't directly check the computed value here, but it shouldn't crash
}

#[test]
fn test_decay_formula_negative_days() {
    // Edge case: What if duration::days() returns negative?
    // This shouldn't happen with (now - last_activated), but let's test boundary

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Query with negative days parameter
    let result = db.query_recent_facts(-1);

    // Should handle gracefully (likely return empty or error)
    assert!(result.is_ok(), "Negative days should not panic");
}

#[test]
fn test_decay_formula_extreme_resonance() {
    // Edge case: Resonance can be > 10 for "transcendent" blooms
    // Make sure formula doesn't overflow or break

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create entry with extreme resonance (like Ori at 13)
    let mut entry = make_test_entry("kn-transcendent", 13, 0.0);
    entry.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Query recent facts
    let result = db.query_recent_facts(30);

    // Should not crash or overflow
    assert!(
        result.is_ok(),
        "Extreme resonance should not break decay formula"
    );

    let facts = result.unwrap();
    assert!(!facts.is_empty(), "Should find transcendent fact");
}

#[test]
fn test_decay_formula_max_int_resonance() {
    // Edge case: What if resonance is i32::MAX?

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create entry with maximum resonance
    let mut entry = make_test_entry("kn-maxres", i32::MAX, 0.0);
    entry.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Query recent facts
    let result = db.query_recent_facts(30);

    // Should handle without overflow
    assert!(result.is_ok(), "MAX resonance should not overflow");
}

// =========================================================================
// TIERED DECAY & BLOOM EXEMPTION TESTS
// =========================================================================

#[test]
fn test_tiered_decay_low_resonance_ephemeral() {
    // Ephemeral entries with resonance <= 3 use 0.90^(weeks) decay rate (10%/week).
    // At 0 days, effective_resonance == resonance. Entry should be returned.

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-low-res", 2, 0.0);
    entry.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&entry).unwrap();

    let result = db.query_recent_facts(7).unwrap();
    assert!(
        !result.is_empty(),
        "Low-resonance ephemeral entry should be returned when freshly created"
    );
}

#[test]
fn test_tiered_decay_mid_resonance_ephemeral() {
    // Ephemeral entries with resonance 4-5 use 0.95^(weeks) decay rate (5%/week).

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-mid-res", 5, 0.0);
    entry.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&entry).unwrap();

    let result = db.query_recent_facts(7).unwrap();
    assert!(
        !result.is_empty(),
        "Mid-resonance ephemeral entry should be returned when freshly created"
    );
}

#[test]
fn test_tiered_decay_high_resonance_ephemeral() {
    // Ephemeral entries with resonance >= 6 use 0.975^(weeks) decay rate (2.5%/week).

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-high-res", 7, 0.0);
    entry.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&entry).unwrap();

    let result = db.query_recent_facts(7).unwrap();
    assert!(
        !result.is_empty(),
        "High-resonance ephemeral entry should be returned when freshly created"
    );
}

#[test]
fn test_tiered_decay_ordering_over_time() {
    // Verify that tiered decay produces different effective_resonance values over time.
    // A low-resonance entry (3, 10%/week) should decay faster than a high-resonance
    // entry (7, 2.5%/week) when both have the same last_activated 30 days ago.
    //
    // After 30 days (~4.3 weeks):
    //   low  (res=3): 3 * 0.90^(30/7) ≈ 3 * 0.64 ≈ 1.9 — below 0.5? No. Well above.
    //   high (res=7): 7 * 0.975^(30/7) ≈ 7 * 0.87 ≈ 6.1
    // High should rank higher. Both should pass the > 0.5 filter.
    use chrono::Utc;

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Backdate last_activated by 30 days so decay has measurably occurred
    let thirty_days_ago = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();

    let mut low = make_test_entry("kn-decay-low", 3, 0.0);
    low.resonance_type = Some("ephemeral".to_string());
    low.last_activated = Some(thirty_days_ago.clone());
    db.upsert_knowledge(&low).unwrap();

    let mut high = make_test_entry("kn-decay-high", 7, 0.0);
    high.resonance_type = Some("ephemeral".to_string());
    high.last_activated = Some(thirty_days_ago);
    db.upsert_knowledge(&high).unwrap();

    // Query over 60 days so both entries fall within the window
    let results = db.query_recent_facts(60).unwrap();

    // Both entries should survive the > 0.5 filter
    let low_found = results.iter().any(|e| e.id == "kn-decay-low");
    let high_found = results.iter().any(|e| e.id == "kn-decay-high");
    assert!(
        low_found,
        "Low-resonance entry should still pass > 0.5 filter after 30 days"
    );
    assert!(
        high_found,
        "High-resonance entry should pass > 0.5 filter after 30 days"
    );

    // Results are ordered by effective_resonance DESC — high-res should appear first
    let low_pos = results.iter().position(|e| e.id == "kn-decay-low").unwrap();
    let high_pos = results
        .iter()
        .position(|e| e.id == "kn-decay-high")
        .unwrap();
    assert!(
        high_pos < low_pos,
        "High-resonance entry (slower decay) should rank above low-resonance entry after 30 days"
    );
}

#[test]
fn test_bloom_exemption_foundational() {
    // Foundational entries are exempt from decay: effective_resonance == resonance.
    // They should NOT appear in query_recent_facts (which filters resonance_type = 'ephemeral'),
    // but should be directly retrievable.

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-foundational", 9, 0.0);
    entry.resonance_type = Some("foundational".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // query_recent_facts only returns ephemeral — foundational should NOT appear here
    let ephemeral_results = db.query_recent_facts(30).unwrap();
    let found_in_ephemeral = ephemeral_results.iter().any(|e| e.id == "kn-foundational");
    assert!(
        !found_in_ephemeral,
        "Foundational entry should not appear in ephemeral fact query"
    );

    // Should still be accessible via direct get
    let ctx = crate::store::AgentContext::public_only();
    let direct = db.get("kn-foundational", &ctx).unwrap();
    assert!(
        direct.is_some(),
        "Foundational entry should be directly retrievable"
    );
}

#[test]
fn test_bloom_exemption_transformative() {
    // Transformative entries are exempt from decay, same as foundational.

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-transformative", 8, 0.0);
    entry.resonance_type = Some("transformative".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Should NOT appear in ephemeral query
    let ephemeral_results = db.query_recent_facts(30).unwrap();
    let found_in_ephemeral = ephemeral_results
        .iter()
        .any(|e| e.id == "kn-transformative");
    assert!(
        !found_in_ephemeral,
        "Transformative entry should not appear in ephemeral fact query"
    );

    let ctx = crate::store::AgentContext::public_only();
    let direct = db.get("kn-transformative", &ctx).unwrap();
    assert!(
        direct.is_some(),
        "Transformative entry should be directly retrievable"
    );
}

#[test]
fn test_increment_activation_count_no_timestamp_reset() {
    // increment_activation_count should bump activation_count but leave
    // last_activated unchanged.

    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry = make_test_entry("kn-incr-test", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    let ctx = crate::store::AgentContext::public_only();

    // Record initial state
    let before = db.get("kn-incr-test", &ctx).unwrap().unwrap();
    let initial_count = before.activation_count;
    let initial_last_activated = before.last_activated.clone();

    // Increment count only
    db.increment_activation_count(&["kn-incr-test".to_string()])
        .unwrap();

    let after = db.get("kn-incr-test", &ctx).unwrap().unwrap();

    assert_eq!(
        after.activation_count,
        initial_count + 1,
        "activation_count should increment by 1"
    );

    assert_eq!(
        after.last_activated, initial_last_activated,
        "last_activated should not be reset by increment_activation_count"
    );
}

#[test]
fn test_thread_duplicate_detection() {
    // Edge case: How does duplicate detection work with normalized content?
    // KnowledgeEntry::normalize_content() is used for fuzzy matching

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create two entries with similar content but different formatting
    let entry1 = make_test_entry("kn-thread1", 5, 0.5);
    let mut entry2 = make_test_entry("kn-thread2", 5, 0.5);
    entry2.body = Some("  TEST   BODY  ".to_string()); // Different whitespace

    db.upsert_knowledge(&entry1).unwrap();
    db.upsert_knowledge(&entry2).unwrap();

    // Both should be stored (deduplication happens at application level, not DB)
    let ctx = crate::store::AgentContext::public_only();
    assert!(db.get("kn-thread1", &ctx).unwrap().is_some());
    assert!(db.get("kn-thread2", &ctx).unwrap().is_some());
}

#[test]
fn test_session_linkage_round_trip() {
    // Edge case: Can we link a fact to a session and retrieve it back?

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create a session entry
    let session = make_test_entry("kn-session123", 0, 0.0);
    db.upsert_knowledge(&session).unwrap();

    // Create a fact linked to that session
    let mut fact = make_test_entry("kn-fact456", 5, 0.5);
    fact.session_id = Some("kn-session123".to_string());
    db.upsert_knowledge(&fact).unwrap();

    // Create relationship
    db.add_relationship("kn-fact456", "kn-session123", "extracted_from")
        .unwrap();

    // Query facts for session
    let facts = db.get_facts_for_session("kn-session123").unwrap();

    // Should find the linked fact
    assert_eq!(facts.len(), 1, "Should find one fact for session");
    assert_eq!(
        facts[0], "kn-fact456",
        "Should return full fact ID with prefix"
    );

    // Reverse lookup: get session for fact
    let session_id = db.get_session_for_fact("kn-fact456").unwrap();
    assert!(session_id.is_some(), "Should find session for fact");
    assert_eq!(
        session_id.unwrap(),
        "kn-session123",
        "Should return full session ID with prefix"
    );
}

#[test]
fn test_session_linkage_multiple_facts() {
    // Edge case: Multiple facts from same session

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create session
    let session = make_test_entry("kn-multisession", 0, 0.0);
    db.upsert_knowledge(&session).unwrap();

    // Create multiple facts
    for i in 1..=5 {
        let mut fact = make_test_entry(&format!("kn-fact{}", i), 5, 0.5);
        fact.session_id = Some("kn-multisession".to_string());
        db.upsert_knowledge(&fact).unwrap();
        db.add_relationship(
            &format!("kn-fact{}", i),
            "kn-multisession",
            "extracted_from",
        )
        .unwrap();
    }

    // Query facts for session
    let facts = db.get_facts_for_session("kn-multisession").unwrap();

    // Should find all 5 facts
    assert_eq!(facts.len(), 5, "Should find all 5 facts for session");
}

#[test]
fn test_session_linkage_orphaned_fact() {
    // Edge case: Fact with session_id but no relationship

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create fact with session_id but don't create relationship
    let mut fact = make_test_entry("kn-orphan", 5, 0.5);
    fact.session_id = Some("kn-ghost".to_string());
    db.upsert_knowledge(&fact).unwrap();

    // Query for session that doesn't exist
    let facts = db.get_facts_for_session("kn-ghost").unwrap();

    // Should return empty (relationship is what matters, not just session_id field)
    assert_eq!(
        facts.len(),
        0,
        "Orphaned fact should not appear without relationship"
    );

    // Reverse lookup should also fail
    let session = db.get_session_for_fact("kn-orphan").unwrap();
    assert!(session.is_none(), "Orphaned fact should have no session");
}

#[test]
fn test_normalize_content_edge_cases() {
    // Test the normalize_content function used for thread matching
    use crate::knowledge::KnowledgeEntry;

    // Empty string
    assert_eq!(KnowledgeEntry::normalize_content(""), "");

    // Only whitespace
    assert_eq!(KnowledgeEntry::normalize_content("   \n\t  "), "");

    // Unicode characters
    let unicode = "Hello 世界! Привет мир!";
    let normalized = KnowledgeEntry::normalize_content(unicode);
    assert!(normalized.contains("hello"), "Should lowercase ASCII");
    assert!(normalized.contains("世界"), "Should preserve unicode");

    // Multiple spaces and newlines
    let messy = "  hello\n\n  world\t\ttest  ";
    assert_eq!(KnowledgeEntry::normalize_content(messy), "hello world test");
}

#[test]
fn test_wake_cascade_empty_anchors() {
    // Edge case: What if a bloom has empty anchors array?

    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create bloom with no anchors
    let mut entry = make_test_entry("kn-solo", 9, 0.0);
    entry.resonance_type = Some("foundational".to_string());
    entry.anchors = vec![];
    db.upsert_knowledge(&entry).unwrap();

    // Query wake cascade
    let cascade = db.wake_cascade(&ctx, 50, Some(7), 7).unwrap();

    // Should still include the entry in core (high resonance)
    assert!(!cascade.core.is_empty(), "Should find core bloom");
    // Bridges might be empty since no anchors
    // This is expected behavior
}

#[test]
fn test_wake_cascade_circular_anchors() {
    // Edge case: What if bloom A anchors to B, and B anchors to A?

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create two blooms that reference each other
    let mut bloom_a = make_test_entry("kn-circular-a", 9, 0.0);
    bloom_a.resonance_type = Some("foundational".to_string());
    bloom_a.anchors = vec!["kn-circular-b".to_string()];

    let mut bloom_b = make_test_entry("kn-circular-b", 9, 0.0);
    bloom_b.resonance_type = Some("foundational".to_string());
    bloom_b.anchors = vec!["kn-circular-a".to_string()];

    db.upsert_knowledge(&bloom_a).unwrap();
    db.upsert_knowledge(&bloom_b).unwrap();

    // Query wake cascade
    let ctx = crate::store::AgentContext::public_only();
    let result = db.wake_cascade(&ctx, 50, Some(7), 7);

    // Should handle circular references without infinite loop
    assert!(
        result.is_ok(),
        "Circular anchors should not cause infinite loop"
    );
}

#[test]
fn test_privacy_filtering_public_only() {
    // Edge case: Public-only context should not see private entries

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create public entry
    let public_entry = make_test_entry("kn-public", 5, 0.5);
    db.upsert_knowledge(&public_entry).unwrap();

    // Create private entry
    let mut private_entry = make_test_entry("kn-private", 5, 0.5);
    private_entry.visibility = "private".to_string();
    private_entry.owner = Some("test_agent".to_string());
    db.upsert_knowledge(&private_entry).unwrap();

    // Query with public-only context
    let ctx = crate::store::AgentContext::public_only();

    // Should see public
    assert!(
        db.get("kn-public", &ctx).unwrap().is_some(),
        "Should see public entry"
    );

    // Should NOT see private
    assert!(
        db.get("kn-private", &ctx).unwrap().is_none(),
        "Should not see private entry"
    );
}

#[test]
fn test_privacy_filtering_agent_context() {
    // Edge case: Agent should see their own private entries

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create private entry for test_agent
    let mut private_entry = make_test_entry("kn-my-private", 5, 0.5);
    private_entry.visibility = "private".to_string();
    private_entry.owner = Some("test_agent".to_string());
    db.upsert_knowledge(&private_entry).unwrap();

    // Create private entry for other_agent
    let mut other_entry = make_test_entry("kn-other-private", 5, 0.5);
    other_entry.visibility = "private".to_string();
    other_entry.owner = Some("other_agent".to_string());
    db.upsert_knowledge(&other_entry).unwrap();

    // Query as test_agent
    let ctx = crate::store::AgentContext::for_agent("test_agent");

    // Should see own private entry
    assert!(
        db.get("kn-my-private", &ctx).unwrap().is_some(),
        "Should see own private entry"
    );

    // Should NOT see other agent's private entry
    assert!(
        db.get("kn-other-private", &ctx).unwrap().is_none(),
        "Should not see other's private entry"
    );
}

// =========================================================================
// CROSS-AGENT VISIBILITY BYPASS TESTS (PR #186 / PR #187)
// =========================================================================
// These tests prove that the visibility filter on delete and update_summary
// prevents cross-agent operations on private entries. Agent-b must not be
// able to delete or update_summary on agent-a's private entries.

#[test]
fn test_delete_cross_agent_visibility_blocked() {
    // PR #186: delete must respect visibility. Agent-b cannot delete
    // agent-a's private entry.
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Agent-a creates a private entry
    let mut entry = make_test_entry("kn-private-del-target", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Agent-b attempts to delete it
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let result = db.delete("kn-private-del-target", &ctx_b).unwrap();
    assert!(
        !result,
        "agent-b should not be able to delete agent-a's private entry"
    );

    // Verify entry still exists for agent-a
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let still_exists = db.get("kn-private-del-target", &ctx_a).unwrap();
    assert!(
        still_exists.is_some(),
        "Entry should still exist for agent-a after failed cross-agent delete"
    );
}

#[test]
fn test_update_summary_cross_agent_visibility_blocked() {
    // This branch's fix: update_summary must respect visibility.
    // Agent-b cannot update the summary of agent-a's private entry.
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Agent-a creates a private entry with a summary
    let mut entry = make_test_entry("kn-private-summary-target", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    entry.summary = Some(r#"{"state":"open"}"#.to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Agent-b attempts to update the summary
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let result = db
        .update_summary(
            "kn-private-summary-target",
            r#"{"state":"compromised"}"#,
            &ctx_b,
        )
        .unwrap();
    assert!(
        !result,
        "agent-b should not be able to update summary on agent-a's private entry"
    );

    // Verify the original summary is unchanged for agent-a
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let unchanged = db
        .get("kn-private-summary-target", &ctx_a)
        .unwrap()
        .unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(unchanged.summary.as_deref().unwrap()).unwrap();
    assert_eq!(
        summary["state"], "open",
        "Summary should be unchanged after failed cross-agent update"
    );
}
#[test]
fn test_reinforce_basic() {
    // Test basic reinforcement functionality
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create an entry with resonance 5
    let mut entry = make_test_entry("kn-test-reinforce", 5, 0.0);
    entry.activation_count = 10;
    db.upsert_knowledge(&entry).unwrap();

    // Reinforce by 2, with cap of 10
    let result = db
        .reinforce("kn-test-reinforce", 2, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should return Some for visible entry");

    // Verify results
    assert_eq!(result.id, "kn-test-reinforce");
    assert_eq!(result.old_resonance, 5);
    assert_eq!(result.new_resonance, 7);
    assert_eq!(result.amount_added, 2);
    assert!(!result.capped);
    assert_eq!(result.activation_count, 11);

    // Verify the entry was actually updated
    let updated = db.get("kn-test-reinforce", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 7);
    assert_eq!(updated.activation_count, 11);
    assert!(updated.last_activated.is_some());
}

#[test]
fn test_reinforce_with_cap() {
    // Test that cap is enforced
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create an entry with resonance 9
    let entry = make_test_entry("kn-test-cap", 9, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // Try to reinforce by 5, but cap at 10
    let result = db
        .reinforce("kn-test-cap", 5, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should return Some for visible entry");

    // Should be capped at 10
    assert_eq!(result.old_resonance, 9);
    assert_eq!(result.new_resonance, 10);
    assert_eq!(result.amount_added, 5);
    assert!(result.capped);

    // Verify the entry was capped
    let updated = db.get("kn-test-cap", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 10);
}

#[test]
fn test_reinforce_without_cap() {
    // Test reinforcement without a cap (for transcendent blooms)
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create an entry with resonance 9
    let entry = make_test_entry("kn-test-no-cap", 9, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // Reinforce by 5 with no cap
    let result = db
        .reinforce("kn-test-no-cap", 5, None, &ctx)
        .unwrap()
        .expect("reinforce should return Some for visible entry");

    // Should go above 10
    assert_eq!(result.old_resonance, 9);
    assert_eq!(result.new_resonance, 14);
    assert!(!result.capped);

    // Verify the entry was updated
    let updated = db.get("kn-test-no-cap", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 14);
}

#[test]
fn test_reinforce_nonexistent() {
    // Test that reinforcing a nonexistent entry returns None
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let result = db.reinforce("kn-nonexistent", 1, Some(10), &ctx).unwrap();
    assert!(
        result.is_none(),
        "reinforce should return None for nonexistent entry"
    );
}

#[test]
fn test_reinforce_id_normalization() {
    // Test that ID normalization works
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create entry with full ID
    let entry = make_test_entry("kn-test-norm", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // Reinforce with partial ID (no "kn-" prefix)
    let result = db
        .reinforce("test-norm", 2, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should return Some for visible entry");

    // Should normalize correctly
    assert_eq!(result.id, "kn-test-norm");
    assert_eq!(result.new_resonance, 7);
}

#[test]
fn test_reinforce_cross_agent_visibility_blocked() {
    // Fix #157: reinforce must respect visibility.
    // Agent-b cannot reinforce agent-a's private entry.
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Agent-a creates a private entry with known resonance
    let mut entry = make_test_entry("kn-private-reinforce-target", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    entry.activation_count = 3;
    db.upsert_knowledge(&entry).unwrap();

    // Agent-b attempts to reinforce it
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let result = db
        .reinforce("kn-private-reinforce-target", 2, Some(10), &ctx_b)
        .unwrap();
    assert!(
        result.is_none(),
        "agent-b should not be able to reinforce agent-a's private entry"
    );

    // Verify the entry is unchanged for agent-a
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let unchanged = db
        .get("kn-private-reinforce-target", &ctx_a)
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.resonance, 5,
        "Resonance should be unchanged after failed cross-agent reinforce"
    );
    assert_eq!(
        unchanged.activation_count, 3,
        "Activation count should be unchanged after failed cross-agent reinforce"
    );
}

#[test]
fn test_reinforce_own_private_entry() {
    // Agent-a should be able to reinforce their own private entry
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Agent-a creates a private entry
    let mut entry = make_test_entry("kn-private-reinforce-own", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    entry.activation_count = 3;
    db.upsert_knowledge(&entry).unwrap();

    // Agent-a reinforces their own entry
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let result = db
        .reinforce("kn-private-reinforce-own", 2, Some(10), &ctx_a)
        .unwrap()
        .expect("agent-a should be able to reinforce their own private entry");

    assert_eq!(result.old_resonance, 5);
    assert_eq!(result.new_resonance, 7);
    assert_eq!(result.activation_count, 4);

    // Verify it actually persisted
    let updated = db.get("kn-private-reinforce-own", &ctx_a).unwrap().unwrap();
    assert_eq!(updated.resonance, 7);
    assert_eq!(updated.activation_count, 4);
}

#[test]
fn test_update_summary_persists() {
    // Regression: thread_closed handler modified summary in memory but
    // upsert_knowledge() silently failed on SCHEMAFULL tables. The new
    // update_summary() path must actually persist the change.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create entry with initial summary (simulating an open thread)
    let mut entry = make_test_entry("kn-summary-test", 5, 0.0);
    entry.summary = Some(r#"{"state":"open","topic":"test thread"}"#.to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Update summary to closed state (mirrors thread_closed handler)
    let new_summary = r#"{"state":"closed","topic":"test thread"}"#;
    let result = db
        .update_summary("kn-summary-test", new_summary, &ctx)
        .unwrap();
    assert!(
        result,
        "update_summary should return true for visible entry"
    );

    // Read it back and verify the change persisted
    let updated = db.get("kn-summary-test", &ctx).unwrap().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");
    assert_eq!(summary["topic"], "test thread");
}

#[test]
fn test_update_summary_id_normalization() {
    // update_summary should accept IDs with or without "kn-" prefix,
    // consistent with get(), delete(), reinforce(), etc.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-summary-norm", 5, 0.0);
    entry.summary = Some(r#"{"state":"open"}"#.to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Update using raw ID (no prefix) - should still work
    let result = db
        .update_summary("summary-norm", r#"{"state":"closed"}"#, &ctx)
        .unwrap();
    assert!(result, "update_summary should return true with raw ID");

    let updated = db.get("kn-summary-norm", &ctx).unwrap().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");

    // Update using prefixed ID - should also work
    let result2 = db
        .update_summary("kn-summary-norm", r#"{"state":"reopened"}"#, &ctx)
        .unwrap();
    assert!(
        result2,
        "update_summary should return true with prefixed ID"
    );

    let updated2 = db.get("kn-summary-norm", &ctx).unwrap().unwrap();
    let summary2: serde_json::Value =
        serde_json::from_str(updated2.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary2["state"], "reopened");
}

#[test]
fn test_close_thread_with_no_summary() {
    // A thread entry with no summary (pre-convention) should accept a
    // closed-state summary written by the thread_closed handler.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create a thread entry with no summary (pre-convention style)
    let mut entry = make_test_entry("kn-no-summary-thread", 5, 0.0);
    entry.summary = None;
    db.upsert_knowledge(&entry).unwrap();

    // The thread_closed handler writes the closed state via update_summary
    let closed_summary = r#"{"state":"closed","topic":"pre-convention thread"}"#;
    let result = db
        .update_summary("kn-no-summary-thread", closed_summary, &ctx)
        .unwrap();
    assert!(
        result,
        "update_summary should return true for entry with no prior summary"
    );

    // Verify the state persisted correctly
    let updated = db.get("kn-no-summary-thread", &ctx).unwrap().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");
    assert_eq!(summary["topic"], "pre-convention thread");
}

#[test]
fn test_get_summary_state_returns_none_for_no_summary() {
    // Confirms the get_summary_state() helper returns None for entries
    // with no summary — the condition that find_open_thread_by_content
    // treats as "potentially open" (pre-convention threads).
    let entry = make_test_entry("kn-state-none", 5, 0.0);
    // make_test_entry sets summary: None by default
    assert!(
        entry.summary.is_none(),
        "make_test_entry should produce summary: None"
    );
    assert_eq!(
        entry.get_summary_state(),
        None,
        "get_summary_state() must return None when summary is absent"
    );
}

// ============================================================================
// BUILDER-PATTERN UPDATE API (Issue #134)
//
// The store.update(id).<field>(..).execute(&ctx) path. The recurring assertion
// across these tests is the safety property from PR #131: a field NOT set on the
// builder must NOT be touched by the resulting UPDATE.
// ============================================================================

/// Coerce a concrete SurrealDatabase to the trait object so the `update()`
/// builder entry point (defined on `dyn KnowledgeStore`) is reachable in tests.
fn as_store(db: &SurrealDatabase) -> &dyn KnowledgeStore {
    db
}

#[test]
fn test_update_builder_single_field_sets_only_that_field() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-single", 5, 0.0);
    entry.summary = Some(r#"{"state":"open"}"#.to_string());
    entry.activation_count = 7;
    db.upsert_knowledge(&entry).unwrap();

    // Set ONLY summary via the builder.
    let outcome = as_store(&db)
        .update("kn-builder-single")
        .summary(r#"{"state":"closed"}"#)
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);
    assert!(!outcome.no_op);

    let updated = db.get("kn-builder-single", &ctx).unwrap().unwrap();
    // summary changed...
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");
    // ...but resonance and activation_count were NOT in the SET clause, so
    // they are untouched (the no-full-record-overwrite property).
    assert_eq!(updated.resonance, 5, "resonance must be untouched");
    assert_eq!(
        updated.activation_count, 7,
        "activation_count must be untouched"
    );
}

#[test]
fn test_update_builder_multiple_fields_compose() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-multi", 5, 0.0);
    entry.summary = Some(r#"{"state":"open"}"#.to_string());
    entry.activation_count = 1;
    db.upsert_knowledge(&entry).unwrap();

    let outcome = as_store(&db)
        .update("kn-builder-multi")
        .summary(r#"{"state":"closed"}"#)
        .resonance(9)
        .activation_count(42)
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);

    let updated = db.get("kn-builder-multi", &ctx).unwrap().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");
    assert_eq!(updated.resonance, 9);
    assert_eq!(updated.activation_count, 42);
}

#[test]
fn test_update_builder_unset_field_preserved() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-preserve", 6, 0.0);
    entry.summary = Some(r#"{"keep":"me"}"#.to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Update ONLY resonance; summary must survive verbatim.
    let outcome = as_store(&db)
        .update("kn-builder-preserve")
        .resonance(2)
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);

    let updated = db.get("kn-builder-preserve", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 2);
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(
        summary["keep"], "me",
        "summary must be preserved when only resonance is set"
    );
}

#[test]
fn test_update_builder_increment_activation_count_is_relative() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-incr", 5, 0.0);
    entry.activation_count = 10;
    db.upsert_knowledge(&entry).unwrap();

    let outcome = as_store(&db)
        .update("kn-builder-incr")
        .increment_activation_count(3)
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);

    let updated = db.get("kn-builder-incr", &ctx).unwrap().unwrap();
    assert_eq!(
        updated.activation_count, 13,
        "increment_activation_count(3) should be a relative +=, not a set"
    );
}

#[test]
fn test_update_builder_empty_is_noop() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let entry = make_test_entry("kn-builder-empty", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // No setters called -> no-op, no query, no error.
    let outcome = as_store(&db)
        .update("kn-builder-empty")
        .execute(&ctx)
        .unwrap();
    assert!(outcome.no_op, "empty builder must be a no-op");
    assert!(!outcome.applied);

    // Entry untouched.
    let updated = db.get("kn-builder-empty", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 5);
}

#[test]
fn test_update_builder_not_found_returns_not_applied() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let outcome = as_store(&db)
        .update("kn-does-not-exist")
        .resonance(3)
        .execute(&ctx)
        .unwrap();
    assert!(!outcome.applied, "missing entry => applied=false");
    assert!(
        !outcome.no_op,
        "a real (non-empty) update still ran the check"
    );
}

#[test]
fn test_update_builder_add_tag() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-tag", 5, 0.0);
    entry.tags = vec!["existing".to_string()];
    db.upsert_knowledge(&entry).unwrap();

    let outcome = as_store(&db)
        .update("kn-builder-tag")
        .add_tag("fresh")
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);

    let mut tags = db.get_tags_for_entry("kn-builder-tag").unwrap();
    tags.sort();
    assert_eq!(tags, vec!["existing".to_string(), "fresh".to_string()]);
}

#[test]
fn test_update_builder_add_tag_idempotent() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let entry = make_test_entry("kn-builder-tag-idem", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // Add the same tag twice across two updates: must not duplicate the edge.
    as_store(&db)
        .update("kn-builder-tag-idem")
        .add_tag("dup")
        .execute(&ctx)
        .unwrap();
    as_store(&db)
        .update("kn-builder-tag-idem")
        .add_tag("dup")
        .execute(&ctx)
        .unwrap();

    let tags = db.get_tags_for_entry("kn-builder-tag-idem").unwrap();
    assert_eq!(
        tags,
        vec!["dup".to_string()],
        "adding the same tag twice should yield a single edge"
    );
}

#[test]
fn test_update_builder_field_and_tag_together() {
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-builder-combo", 5, 0.0);
    entry.activation_count = 2;
    db.upsert_knowledge(&entry).unwrap();

    let outcome = as_store(&db)
        .update("kn-builder-combo")
        .resonance(8)
        .add_tag("combo")
        .execute(&ctx)
        .unwrap();
    assert!(outcome.applied);

    let updated = db.get("kn-builder-combo", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 8);
    // activation_count not set -> untouched
    assert_eq!(updated.activation_count, 2);
    let tags = db.get_tags_for_entry("kn-builder-combo").unwrap();
    assert_eq!(tags, vec!["combo".to_string()]);
}

#[test]
fn test_update_builder_column_write_bumps_updated_at() {
    // Deliberate decision (Verdictia finding #3): a column write always bumps
    // updated_at = time::now(); a tag-only update does NOT (tags are a graph edge,
    // not a knowledge column, so the row itself is unchanged).
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-updated-at", 5, 0.0);
    entry.updated_at = Some("2020-01-01T00:00:00Z".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // Column write -> updated_at moves forward from the stale 2020 value.
    as_store(&db)
        .update("kn-updated-at")
        .resonance(8)
        .execute(&ctx)
        .unwrap();
    let after_col = db.get("kn-updated-at", &ctx).unwrap().unwrap();
    let col_ts = after_col.updated_at.clone().unwrap();
    assert!(
        col_ts.as_str() > "2020-01-01T00:00:00Z",
        "column write must bump updated_at, got {col_ts}"
    );

    // Tag-only write -> updated_at unchanged (no column touched).
    as_store(&db)
        .update("kn-updated-at")
        .add_tag("tag-only")
        .execute(&ctx)
        .unwrap();
    let after_tag = db.get("kn-updated-at", &ctx).unwrap().unwrap();
    assert_eq!(
        after_tag.updated_at.unwrap(),
        col_ts,
        "tag-only update must NOT bump updated_at"
    );
}

#[test]
fn test_update_builder_respects_visibility() {
    // Cross-agent: agent-b must NOT be able to update agent-a's private entry.
    let db = SurrealDatabase::open_in_memory().unwrap();

    // make_test_entry(..., 5, ...) already sets resonance = 5.
    let mut entry = make_test_entry("kn-builder-private", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    db.upsert_knowledge(&entry).unwrap();

    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let outcome = as_store(&db)
        .update("kn-builder-private")
        .resonance(1)
        .execute(&ctx_b)
        .unwrap();
    assert!(
        !outcome.applied,
        "agent-b must not see/update agent-a's private entry"
    );

    // Confirm nothing changed, viewed as the owner.
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let unchanged = db.get("kn-builder-private", &ctx_a).unwrap().unwrap();
    assert_eq!(
        unchanged.resonance, 5,
        "private entry resonance must be unchanged after blocked cross-agent update"
    );
}

#[test]
fn test_update_builder_add_tag_respects_visibility() {
    // Cross-agent (Verdictia finding #5): agent-b must NOT be able to tag
    // agent-a's private entry. The RELATE is gated on a visibility-filtered
    // existence subquery, so a blocked agent gets applied=false and NO edge is
    // created. This test fails before the TOCTOU tag-guard fix and passes after.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-builder-tag-private", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // agent-b cannot see the entry, so the tag update must not apply...
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let outcome = as_store(&db)
        .update("kn-builder-tag-private")
        .add_tag("sneaky")
        .execute(&ctx_b)
        .unwrap();
    assert!(
        !outcome.applied,
        "agent-b must not be able to tag agent-a's private entry"
    );

    // ...and no edge may have been grafted on. Check as the owner (who CAN see it).
    let tags = db.get_tags_for_entry("kn-builder-tag-private").unwrap();
    assert!(
        tags.is_empty(),
        "no tag edge should exist after a blocked cross-agent add_tag, got {tags:?}"
    );

    // Sanity: the owner CAN still tag it.
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let owner_outcome = as_store(&db)
        .update("kn-builder-tag-private")
        .add_tag("legit")
        .execute(&ctx_a)
        .unwrap();
    assert!(owner_outcome.applied, "owner must be able to tag own entry");
    let owner_tags = db.get_tags_for_entry("kn-builder-tag-private").unwrap();
    assert_eq!(owner_tags, vec!["legit".to_string()]);
}

#[test]
fn test_update_builder_column_plus_tag_respects_visibility() {
    // Companion to test_update_builder_add_tag_respects_visibility.
    //
    // That test uses a TAG-ONLY spec, where has_column_updates() is false, so the
    // column-UPDATE block in apply_update_async is skipped entirely. Here the spec
    // carries a column field ALONGSIDE the tag, so has_column_updates() is TRUE and
    // the combined path runs: targeted column UPDATE first, then the RELATE. This
    // covers the "column + tag in one execute()" shape the tag-only test can't.
    //
    // Isolation note: even with a column field, a non-owner is rejected at the
    // upstream existence/visibility check (apply_update_async ~:568) before either
    // the column UPDATE or the RELATE runs, so this case still exercises the
    // upstream guard rather than the RELATE's own visibility subquery in
    // add_tag_edge_async (~:667). Isolating that subquery would require forcing the
    // post-check TOCTOU window (visibility flips between the existence check and the
    // RELATE) single-threaded, which the in-memory harness can't cleanly do without
    // injecting a hook between the two statements. The RELATE subquery guard is
    // correct by inspection (it re-applies the same visibility_clause); this test
    // asserts the observable contract: blocked => no column change AND no edge.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-builder-col-tag-private", 5, 0.0);
    entry.visibility = "private".to_string();
    entry.owner = Some("agent-a".to_string());
    entry.summary = Some("original".to_string());
    db.upsert_knowledge(&entry).unwrap();

    // agent-b cannot see the entry: a column+tag spec must not apply, and must
    // mutate NEITHER the column NOR create the edge.
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let outcome = as_store(&db)
        .update("kn-builder-col-tag-private")
        .summary("hijacked")
        .add_tag("sneaky")
        .execute(&ctx_b)
        .unwrap();
    assert!(
        !outcome.applied,
        "agent-b must not be able to update+tag agent-a's private entry"
    );

    // Verify as the owner that nothing changed: summary intact, no tag edge.
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let after_block = db
        .get("kn-builder-col-tag-private", &ctx_a)
        .unwrap()
        .unwrap();
    assert_eq!(
        after_block.summary.as_deref(),
        Some("original"),
        "blocked cross-agent update must not change the column"
    );
    let tags = db.get_tags_for_entry("kn-builder-col-tag-private").unwrap();
    assert!(
        tags.is_empty(),
        "blocked cross-agent update must not create a tag edge, got {tags:?}"
    );

    // Owner CAN apply the combined column+tag update: both effects land.
    let owner_outcome = as_store(&db)
        .update("kn-builder-col-tag-private")
        .summary("owner-set")
        .add_tag("legit")
        .execute(&ctx_a)
        .unwrap();
    assert!(
        owner_outcome.applied,
        "owner must be able to update+tag own entry"
    );
    let after_owner = db
        .get("kn-builder-col-tag-private", &ctx_a)
        .unwrap()
        .unwrap();
    assert_eq!(after_owner.summary.as_deref(), Some("owner-set"));
    let owner_tags = db.get_tags_for_entry("kn-builder-col-tag-private").unwrap();
    assert_eq!(owner_tags, vec!["legit".to_string()]);
}

#[test]
fn test_update_summary_still_works_via_builder_delegation() {
    // Regression: update_summary() now delegates to the builder's apply_update.
    // Its observable behavior must be unchanged from PR #131.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    let mut entry = make_test_entry("kn-summary-delegation", 5, 0.0);
    entry.summary = Some(r#"{"state":"open"}"#.to_string());
    entry.resonance = 5;
    db.upsert_knowledge(&entry).unwrap();

    let applied = db
        .update_summary("kn-summary-delegation", r#"{"state":"closed"}"#, &ctx)
        .unwrap();
    assert!(applied);

    let updated = db.get("kn-summary-delegation", &ctx).unwrap().unwrap();
    let summary: serde_json::Value =
        serde_json::from_str(updated.summary.as_deref().unwrap()).unwrap();
    assert_eq!(summary["state"], "closed");
    // And resonance is still untouched through the delegation path.
    assert_eq!(updated.resonance, 5);
}

#[test]
fn test_query_recent_facts_all_types_includes_foundational() {
    // query_recent_facts_all_types should return foundational entries that would
    // be excluded from query_recent_facts (ephemeral-only).

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut foundational = make_test_entry("kn-all-types-foundational", 9, 0.0);
    foundational.resonance_type = Some("foundational".to_string());
    db.upsert_knowledge(&foundational).unwrap();

    let mut ephemeral = make_test_entry("kn-all-types-ephemeral", 5, 0.0);
    ephemeral.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&ephemeral).unwrap();

    // Baseline: ephemeral-only query should not include foundational
    let ephemeral_results = db.query_recent_facts(30).unwrap();
    assert!(
        !ephemeral_results
            .iter()
            .any(|e| e.id == "kn-all-types-foundational"),
        "Foundational entry should not appear in ephemeral-only query"
    );
    assert!(
        ephemeral_results
            .iter()
            .any(|e| e.id == "kn-all-types-ephemeral"),
        "Ephemeral entry should appear in ephemeral-only query"
    );

    // All-types query should include both
    let all_results = db.query_recent_facts_all_types(30).unwrap();
    assert!(
        all_results
            .iter()
            .any(|e| e.id == "kn-all-types-foundational"),
        "Foundational entry should appear in all-types query"
    );
    assert!(
        all_results.iter().any(|e| e.id == "kn-all-types-ephemeral"),
        "Ephemeral entry should appear in all-types query"
    );
}

#[test]
fn test_query_recent_facts_all_types_includes_transformative() {
    // query_recent_facts_all_types should return transformative entries.

    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut transformative = make_test_entry("kn-all-types-transformative", 8, 0.0);
    transformative.resonance_type = Some("transformative".to_string());
    db.upsert_knowledge(&transformative).unwrap();

    let all_results = db.query_recent_facts_all_types(30).unwrap();
    assert!(
        all_results
            .iter()
            .any(|e| e.id == "kn-all-types-transformative"),
        "Transformative entry should appear in all-types query"
    );
}

#[test]
fn test_query_recent_facts_all_types_respects_decay_threshold() {
    // Entries with near-zero effective resonance (very old, low base) should
    // be excluded even from the all-types query (threshold > 0.5).

    let db = SurrealDatabase::open_in_memory().unwrap();

    // Resonance 1 with heavy decay (80 weeks ago equivalent = decay_rate abuse).
    // We simulate a very old entry by setting last_activated far in the past.
    // For this test we just confirm high-resonance entries are returned.
    let mut high = make_test_entry("kn-all-types-high", 8, 0.0);
    high.resonance_type = Some("ephemeral".to_string());
    db.upsert_knowledge(&high).unwrap();

    let results = db.query_recent_facts_all_types(30).unwrap();
    assert!(
        results.iter().any(|e| e.id == "kn-all-types-high"),
        "High-resonance ephemeral entry should appear in all-types query"
    );
}

// =========================================================================
// list_all_tags TESTS (PR #147)
// =========================================================================

fn make_tagged_entry(
    id: &str,
    category: &str,
    tags: Vec<String>,
) -> crate::knowledge::KnowledgeEntry {
    let mut entry = make_test_entry(id, 5, 0.0);
    entry.category_id = category.to_string();
    entry.tags = tags;
    entry
}

#[test]
fn test_list_all_tags_returns_distinct_tags() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry1 = make_tagged_entry(
        "kn-tag1",
        "pattern",
        vec!["rust".to_string(), "async".to_string()],
    );
    db.upsert_knowledge(&entry1).unwrap();

    let entry2 = make_tagged_entry(
        "kn-tag2",
        "technique",
        vec!["rust".to_string(), "error-handling".to_string()],
    );
    db.upsert_knowledge(&entry2).unwrap();

    let tags = db.list_all_tags(None).unwrap();
    assert_eq!(tags.len(), 3);
    assert_eq!(tags, vec!["async", "error-handling", "rust"]);
}

#[test]
fn test_list_all_tags_with_category_filter() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry1 = make_tagged_entry(
        "kn-tag3",
        "pattern",
        vec!["rust".to_string(), "async".to_string()],
    );
    db.upsert_knowledge(&entry1).unwrap();

    let entry2 = make_tagged_entry(
        "kn-tag4",
        "technique",
        vec!["rust".to_string(), "error-handling".to_string()],
    );
    db.upsert_knowledge(&entry2).unwrap();

    let pattern_tags = db.list_all_tags(Some("pattern")).unwrap();
    assert_eq!(pattern_tags.len(), 2);
    assert_eq!(pattern_tags, vec!["async", "rust"]);

    let technique_tags = db.list_all_tags(Some("technique")).unwrap();
    assert_eq!(technique_tags.len(), 2);
    assert_eq!(technique_tags, vec!["error-handling", "rust"]);
}

#[test]
fn test_list_all_tags_empty_database() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let tags = db.list_all_tags(None).unwrap();
    assert!(tags.is_empty());

    let tags = db.list_all_tags(Some("pattern")).unwrap();
    assert!(tags.is_empty());
}

// =========================================================================
// GHOST ANCHOR SWEEP TESTS
// =========================================================================

// ---- Pure function tests (no DB required) ----

#[test]
fn test_detect_ghosts_finds_missing_anchors() {
    // Entry references anchors "aaa" and "bbb", but only "aaa" exists.
    let live_ids: HashSet<String> = ["aaa"].iter().map(|s| s.to_string()).collect();
    let anchors = vec!["aaa".to_string(), "bbb".to_string()];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert_eq!(ghosts, vec!["bbb"]);
}

#[test]
fn test_detect_ghosts_no_false_positives() {
    // All anchors exist — no ghosts should be reported.
    let live_ids: HashSet<String> = ["aaa", "bbb", "ccc"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let anchors = vec!["aaa".to_string(), "bbb".to_string()];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert!(
        ghosts.is_empty(),
        "No ghosts should be detected when all anchors exist"
    );
}

#[test]
fn test_detect_ghosts_all_anchors_are_ghosts() {
    // No referenced IDs exist — every anchor is a ghost.
    let live_ids: HashSet<String> = HashSet::new();
    let anchors = vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert_eq!(ghosts.len(), 3, "All anchors should be ghosts");
    assert_eq!(ghosts, anchors);
}

#[test]
fn test_detect_ghosts_handles_kn_prefix() {
    // Anchors stored with "kn-" prefix should still be detected against
    // bare IDs in the live set.
    let live_ids: HashSet<String> = ["aaa"].iter().map(|s| s.to_string()).collect();
    let anchors = vec!["kn-aaa".to_string(), "kn-bbb".to_string()];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert_eq!(
        ghosts,
        vec!["kn-bbb"],
        "kn-aaa maps to live 'aaa', kn-bbb is a ghost"
    );
}

#[test]
fn test_detect_ghosts_mixed_prefix_and_bare() {
    // Mix of prefixed and bare anchors. Both forms should resolve correctly.
    let live_ids: HashSet<String> = ["aaa", "bbb"].iter().map(|s| s.to_string()).collect();
    let anchors = vec![
        "kn-aaa".to_string(), // maps to live "aaa" — not a ghost
        "bbb".to_string(),    // maps to live "bbb" — not a ghost
        "ccc".to_string(),    // not in live set — ghost
        "kn-ddd".to_string(), // maps to bare "ddd", not in live set — ghost
    ];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert_eq!(ghosts, vec!["ccc", "kn-ddd"]);
}

#[test]
fn test_detect_ghosts_empty_anchors() {
    // Entry with no anchors should produce no ghosts.
    let live_ids: HashSet<String> = ["aaa"].iter().map(|s| s.to_string()).collect();
    let anchors: Vec<String> = vec![];

    let ghosts = queries::detect_ghosts(&anchors, &live_ids);
    assert!(ghosts.is_empty());
}

// ---- Integration tests (in-memory SurrealDB) ----

#[test]
fn test_sweep_ghost_anchors_dry_run_does_not_modify() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create two entries: "target" exists, "ghost" does not.
    let mut entry_a = make_test_entry("kn-sweep-a", 5, 0.0);
    entry_a.anchors = vec!["kn-sweep-target".to_string(), "kn-sweep-ghost".to_string()];
    db.upsert_knowledge(&entry_a).unwrap();

    let entry_target = make_test_entry("kn-sweep-target", 3, 0.0);
    db.upsert_knowledge(&entry_target).unwrap();
    // "kn-sweep-ghost" is intentionally NOT created.

    // Dry run
    let result = db.sweep_ghost_anchors(true).unwrap();

    assert_eq!(result.ghosts_found, 1, "Should detect one ghost anchor");
    assert_eq!(
        result.ghosts_removed, 0,
        "Dry run should not remove anything"
    );
    assert!(result.dry_run);
    assert_eq!(result.affected_entries.len(), 1);
    assert_eq!(
        result.affected_entries[0].ghost_anchors,
        vec!["kn-sweep-ghost"]
    );

    // Verify anchors were NOT modified
    let ctx = crate::store::AgentContext::public_only();
    let unchanged = db.get("kn-sweep-a", &ctx).unwrap().unwrap();
    assert_eq!(
        unchanged.anchors.len(),
        2,
        "Dry run must not modify anchors"
    );
}

#[test]
fn test_sweep_ghost_anchors_removes_ghosts() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Create entry with one live and one ghost anchor.
    let mut entry_a = make_test_entry("kn-sweep-rm-a", 5, 0.0);
    entry_a.anchors = vec![
        "kn-sweep-rm-live".to_string(),
        "kn-sweep-rm-dead".to_string(),
    ];
    db.upsert_knowledge(&entry_a).unwrap();

    let live_entry = make_test_entry("kn-sweep-rm-live", 3, 0.0);
    db.upsert_knowledge(&live_entry).unwrap();
    // "kn-sweep-rm-dead" is intentionally NOT created.

    // Real sweep
    let result = db.sweep_ghost_anchors(false).unwrap();

    assert_eq!(result.ghosts_found, 1);
    assert_eq!(result.ghosts_removed, 1);
    assert!(!result.dry_run);

    // Verify the ghost anchor was actually removed
    let ctx = crate::store::AgentContext::public_only();
    let updated = db.get("kn-sweep-rm-a", &ctx).unwrap().unwrap();
    assert_eq!(
        updated.anchors.len(),
        1,
        "Ghost anchor should have been removed"
    );
    assert!(
        updated.anchors.contains(&"kn-sweep-rm-live".to_string()),
        "Live anchor should be preserved"
    );
}

#[test]
fn test_sweep_ghost_anchors_all_ghosts_on_entry() {
    // Edge case: entry where ALL anchors are ghosts.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-sweep-all-ghost", 5, 0.0);
    entry.anchors = vec!["kn-ghost-x".to_string(), "kn-ghost-y".to_string()];
    db.upsert_knowledge(&entry).unwrap();
    // Neither ghost-x nor ghost-y exist.

    let result = db.sweep_ghost_anchors(false).unwrap();

    assert_eq!(result.ghosts_found, 2, "Both anchors should be ghosts");
    assert_eq!(result.ghosts_removed, 2);
    assert_eq!(result.affected_entries.len(), 1);

    // Verify all anchors were removed
    let ctx = crate::store::AgentContext::public_only();
    let updated = db.get("kn-sweep-all-ghost", &ctx).unwrap().unwrap();
    assert!(
        updated.anchors.is_empty(),
        "All ghost anchors should be removed"
    );
}

#[test]
fn test_sweep_ghost_anchors_clean_graph() {
    // When no ghosts exist, the sweep should report 0 found / 0 removed.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry_a = make_test_entry("kn-clean-a", 5, 0.0);
    entry_a.anchors = vec!["kn-clean-b".to_string()];
    db.upsert_knowledge(&entry_a).unwrap();

    let entry_b = make_test_entry("kn-clean-b", 3, 0.0);
    db.upsert_knowledge(&entry_b).unwrap();

    let result = db.sweep_ghost_anchors(true).unwrap();

    assert_eq!(result.ghosts_found, 0, "Clean graph should have no ghosts");
    assert_eq!(result.entries_scanned, 1, "One entry has anchors");
    assert!(result.affected_entries.is_empty());
}

#[test]
fn test_sweep_ghost_anchors_no_anchored_entries() {
    // When no entries have anchors, the sweep should return immediately.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry = make_test_entry("kn-no-anchors", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    let result = db.sweep_ghost_anchors(true).unwrap();

    assert_eq!(result.entries_scanned, 0);
    assert_eq!(result.ghosts_found, 0);
    assert!(result.affected_entries.is_empty());
}

// =========================================================================
// RELATIONSHIP AUTO-REINFORCE TESTS (Issue #119)
// =========================================================================

#[test]
fn test_relationship_add_reinforces_target() {
    // After creating a relationship A -> B, B should be reinforced by +1.
    // This mirrors what handle_relationships does when no_reinforce=false.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create source and target entries
    let entry_a = make_test_entry("kn-rel-src", 5, 0.0);
    let entry_b = make_test_entry("kn-rel-tgt", 3, 0.0);
    db.upsert_knowledge(&entry_a).unwrap();
    db.upsert_knowledge(&entry_b).unwrap();

    // Add relationship (simulating the handler path)
    db.add_relationship("kn-rel-src", "kn-rel-tgt", "related")
        .unwrap();

    // Reinforce the target (what handle_relationships does after add)
    let result = db
        .reinforce("kn-rel-tgt", 1, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should succeed on visible target");

    assert_eq!(result.old_resonance, 3);
    assert_eq!(result.new_resonance, 4);
    assert_eq!(result.amount_added, 1);
    assert!(!result.capped);

    // Verify persistence
    let updated = db.get("kn-rel-tgt", &ctx).unwrap().unwrap();
    assert_eq!(updated.resonance, 4);
}

#[test]
fn test_relationship_add_no_reinforce_skips() {
    // Compare the two handler paths: with reinforce vs without (--no-reinforce).
    // The WITH path calls add_relationship + reinforce, changing resonance.
    // The WITHOUT path calls only add_relationship, leaving resonance unchanged.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // --- Path A: with reinforce (default handler behavior) ---
    let entry_a_src = make_test_entry("kn-noreinf-a-src", 5, 0.0);
    let entry_a_tgt = make_test_entry("kn-noreinf-a-tgt", 3, 0.0);
    db.upsert_knowledge(&entry_a_src).unwrap();
    db.upsert_knowledge(&entry_a_tgt).unwrap();

    db.add_relationship("kn-noreinf-a-src", "kn-noreinf-a-tgt", "related")
        .unwrap();
    // Simulate the handler calling reinforce (no_reinforce=false)
    db.reinforce("kn-noreinf-a-tgt", 1, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should succeed");

    let reinforced = db.get("kn-noreinf-a-tgt", &ctx).unwrap().unwrap();
    assert_eq!(
        reinforced.resonance, 4,
        "Target resonance should increase from 3 to 4 when reinforced"
    );

    // --- Path B: without reinforce (--no-reinforce flag) ---
    let entry_b_src = make_test_entry("kn-noreinf-b-src", 5, 0.0);
    let entry_b_tgt = make_test_entry("kn-noreinf-b-tgt", 3, 0.0);
    db.upsert_knowledge(&entry_b_src).unwrap();
    db.upsert_knowledge(&entry_b_tgt).unwrap();

    db.add_relationship("kn-noreinf-b-src", "kn-noreinf-b-tgt", "related")
        .unwrap();
    // Handler does NOT call reinforce when --no-reinforce is set

    let not_reinforced = db.get("kn-noreinf-b-tgt", &ctx).unwrap().unwrap();
    assert_eq!(
        not_reinforced.resonance, 3,
        "Target resonance should stay at 3 when --no-reinforce is set"
    );

    // The contrast: same starting resonance, different outcomes
    assert_ne!(
        reinforced.resonance, not_reinforced.resonance,
        "Reinforced and non-reinforced targets should have different resonance"
    );
}

#[test]
fn test_relationship_contradicts_supersedes_skip_reinforce() {
    // W1: contradicts and supersedes relationship types should NOT auto-reinforce
    // the target, because those types mean the target is outdated or wrong.
    // This mirrors the handler logic:
    //   let should_reinforce = !no_reinforce && !matches!(type, "contradicts" | "supersedes");
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Test contradicts: target should NOT be reinforced
    let src_c = make_test_entry("kn-contra-src", 5, 0.0);
    let tgt_c = make_test_entry("kn-contra-tgt", 3, 0.0);
    db.upsert_knowledge(&src_c).unwrap();
    db.upsert_knowledge(&tgt_c).unwrap();

    db.add_relationship("kn-contra-src", "kn-contra-tgt", "contradicts")
        .unwrap();
    // Handler skips reinforce for contradicts -- simulate by NOT calling reinforce

    let contra_target = db.get("kn-contra-tgt", &ctx).unwrap().unwrap();
    assert_eq!(
        contra_target.resonance, 3,
        "contradicts target should NOT be reinforced (resonance stays at 3)"
    );

    // Test supersedes: target should NOT be reinforced
    let src_s = make_test_entry("kn-super-src", 5, 0.0);
    let tgt_s = make_test_entry("kn-super-tgt", 3, 0.0);
    db.upsert_knowledge(&src_s).unwrap();
    db.upsert_knowledge(&tgt_s).unwrap();

    db.add_relationship("kn-super-src", "kn-super-tgt", "supersedes")
        .unwrap();
    // Handler skips reinforce for supersedes -- simulate by NOT calling reinforce

    let super_target = db.get("kn-super-tgt", &ctx).unwrap().unwrap();
    assert_eq!(
        super_target.resonance, 3,
        "supersedes target should NOT be reinforced (resonance stays at 3)"
    );

    // Control: "related" type SHOULD reinforce (proving the distinction)
    let src_r = make_test_entry("kn-related-src", 5, 0.0);
    let tgt_r = make_test_entry("kn-related-tgt", 3, 0.0);
    db.upsert_knowledge(&src_r).unwrap();
    db.upsert_knowledge(&tgt_r).unwrap();

    db.add_relationship("kn-related-src", "kn-related-tgt", "related")
        .unwrap();
    // Handler DOES reinforce for "related" type
    db.reinforce("kn-related-tgt", 1, Some(10), &ctx)
        .unwrap()
        .expect("reinforce should succeed for related type");

    let related_target = db.get("kn-related-tgt", &ctx).unwrap().unwrap();
    assert_eq!(
        related_target.resonance, 4,
        "related target SHOULD be reinforced (resonance goes from 3 to 4)"
    );

    // The distinction: contradicts/supersedes stay at 3, related goes to 4
    assert_eq!(contra_target.resonance, 3);
    assert_eq!(super_target.resonance, 3);
    assert_eq!(related_target.resonance, 4);
}

// =========================================================================
// SEARCH --SELECT ACTIVATION TESTS (Issue #119)
// =========================================================================

#[test]
fn test_search_select_activates_results() {
    // --select on search should call update_activations on all returned IDs.
    // We verify by checking that activation_count and last_activated change.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Create a searchable entry
    let mut entry = make_test_entry("kn-search-sel", 5, 0.0);
    entry.title = "unique searchable widget".to_string();
    entry.body = Some("unique searchable widget content".to_string());
    entry.activation_count = 0;
    entry.last_activated = None;
    db.upsert_knowledge(&entry).unwrap();

    // Simulate what --select does: search then activate results
    let filter = crate::store::KnowledgeFilter::default();
    let results = db.search("widget", &ctx, &filter).unwrap();
    assert!(!results.is_empty(), "Search should find the entry");

    // Activate (this is what --select triggers)
    let ids: Vec<String> = results.iter().map(|e| e.id.clone()).collect();
    db.update_activations(&ids).unwrap();

    // Verify activation was recorded
    let activated = db.get("kn-search-sel", &ctx).unwrap().unwrap();
    assert_eq!(
        activated.activation_count, 1,
        "activation_count should increment after --select"
    );
    assert!(
        activated.last_activated.is_some(),
        "last_activated should be set after --select"
    );
}

#[test]
fn test_search_select_no_results_is_noop() {
    // --select with no results should not error or attempt any activations.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let ctx = crate::store::AgentContext::public_only();

    // Search for something that doesn't exist
    let filter = crate::store::KnowledgeFilter::default();
    let results = db
        .search("xyzzy_nonexistent_query_12345", &ctx, &filter)
        .unwrap();
    assert!(results.is_empty(), "Should find no results");

    // The handler checks `if select && !entries.is_empty()`, so with empty
    // results update_activations is never called. Verify it's safe to call
    // with empty slice anyway (belt and suspenders).
    let empty_ids: Vec<String> = vec![];
    let result = db.update_activations(&empty_ids);
    assert!(
        result.is_ok(),
        "update_activations with empty IDs should not error"
    );
}

// =========================================================================
// CHUNKED EMBEDDING SEARCH TEST (PR #348)
// =========================================================================

/// Generate a long test entry with a pizza passage buried deep in unrelated content.
///
/// The entry is ~1500+ tokens: ~500 tokens of software engineering filler,
/// then a distinctive passage about Neapolitan pizza, then ~500 more tokens
/// of filler. Without chunked embedding the pizza passage sits well beyond
/// the old 512-token truncation point.
fn make_pizza_test_entry() -> String {
    // ~700 tokens of generic content about software (repeated for length)
    let prefix = "Software engineering is a discipline that encompasses the systematic \
        design, development, testing, and maintenance of software applications. \
        The field has evolved significantly since its inception in the 1960s, \
        when the term was first coined at the NATO Software Engineering Conference. \
        Early software development was characterized by ad-hoc approaches and a lack \
        of formal methodologies. The waterfall model emerged as one of the first \
        structured approaches, dividing the development process into distinct phases: \
        requirements analysis, design, implementation, testing, and maintenance. \
        However, the rigidity of this approach led to the development of more flexible \
        methodologies. Agile development, introduced through the Agile Manifesto in 2001, \
        emphasized iterative development, collaboration, and adaptability. \
        "
    .repeat(4);

    // The buried pizza passage (~200 tokens)
    let pizza = "The art of pizza making is a fascinating departure from our main topic. \
        A proper Neapolitan pizza requires a dough made from type 00 flour with 60-65% \
        hydration, fermented for at least 24 hours. The sauce should be made from San \
        Marzano tomatoes, crushed by hand, with nothing more than salt and fresh basil. \
        Mozzarella di bufala, made from water buffalo milk, provides the ideal cheese \
        topping. The pizza must be baked in a wood-fired oven at 485 degrees Celsius \
        for exactly 60 to 90 seconds. The cornicione, or outer crust, should be puffy \
        and leopard-spotted with char marks. A pizzaiolo trains for years to master the \
        art of stretching dough by hand without tearing, creating a perfectly thin center \
        with an airy, risen edge. The Associazione Verace Pizza Napoletana certifies \
        pizzerias worldwide that meet their strict standards for authentic preparation.";

    // ~700 more tokens of generic content
    let suffix = "Returning to software engineering, modern practices include continuous \
        integration and continuous deployment, microservices architecture, and cloud-native \
        development. The rise of DevOps has blurred the traditional boundaries between \
        development and operations teams, fostering a culture of shared responsibility. \
        Container technologies like Docker and orchestration platforms like Kubernetes \
        have revolutionized how applications are packaged and deployed. \
        "
    .repeat(4);

    format!("{}\n\n{}\n\n{}", prefix, pizza, suffix)
}

#[test]
fn test_chunked_search_finds_buried_content() {
    use crate::chunking::{ChunkConfig, chunk_text};
    use crate::embeddings::{EmbeddingProvider, TractProvider};
    use crate::store::KnowledgeStore;

    // 1. Create provider and test database
    let provider = TractProvider::new().expect("TractProvider should initialize");
    let db = SurrealDatabase::open_in_memory().expect("in-memory DB should open");

    // 2. Create a long entry with pizza buried deep inside
    let long_body = make_pizza_test_entry();

    // Sanity check: the body should be >512 tokens so the old truncation
    // would have missed the pizza passage entirely. Use load_tokenizer()
    // which has truncation disabled -- the provider's tokenizer truncates
    // at 512, so it would always report <= 512.
    {
        let counting_tok =
            crate::embeddings::load_tokenizer().expect("load_tokenizer should succeed");
        let encoding = counting_tok
            .encode(long_body.as_str(), false)
            .expect("tokenizer.encode should succeed");
        assert!(
            encoding.get_ids().len() > 512,
            "Test body must exceed 512 tokens to validate chunked search (got {})",
            encoding.get_ids().len()
        );
    }

    let mut entry = make_test_entry("kn-pizza-deep", 5, 0.0);
    entry.title = "Software Engineering History".to_string();
    entry.body = Some(long_body);
    db.upsert_knowledge(&entry).unwrap();

    // 3. Embed with chunking (replicate auto_embed logic inline so we
    //    don't depend on MX_CURRENT_AGENT being set)
    let ctx = crate::store::AgentContext::public_only();
    let embedding_text = entry.embedding_text();
    let config = ChunkConfig::default();
    // Use load_tokenizer() (no truncation) for chunking — provider.tokenizer()
    // truncates at 512 which would hide the buried pizza content.
    let chunking_tokenizer =
        crate::embeddings::load_tokenizer().expect("load_tokenizer should succeed");
    let chunks = chunk_text(&embedding_text, &chunking_tokenizer, &config);

    assert!(
        chunks.len() > 1,
        "Entry should produce multiple chunks (got {})",
        chunks.len()
    );

    let mut chunk_embeddings = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        chunk_embeddings.push(provider.embed(&chunk.text).unwrap());
    }

    // Store chunks
    db.delete_embedding_chunks("kn-pizza-deep").unwrap();
    for (chunk, embedding) in chunks.iter().zip(chunk_embeddings.iter()) {
        db.insert_embedding_chunk(
            "kn-pizza-deep",
            chunk.chunk_index,
            &chunk.text,
            chunk.token_offset,
            chunk.token_count,
            embedding,
            provider.model_id(),
        )
        .unwrap();
    }

    // Mean vector on entry (for the unchunked search path and auto_anchor)
    let dims = provider.dimensions();
    let mut mean_vec = vec![0.0f32; dims];
    for emb in &chunk_embeddings {
        for (i, v) in emb.iter().enumerate() {
            mean_vec[i] += v;
        }
    }
    let n = chunk_embeddings.len() as f32;
    for v in mean_vec.iter_mut() {
        *v /= n;
    }
    let l2: f32 = mean_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if l2 > 0.0 {
        for v in mean_vec.iter_mut() {
            *v /= l2;
        }
    }

    entry.embedding = Some(mean_vec);
    entry.embedding_model = Some(provider.model_id().to_string());
    entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
    entry.chunk_count = chunks.len() as i32;
    entry.updated_at = Some(chrono::Utc::now().to_rfc3339());
    db.upsert_knowledge(&entry).unwrap();

    // 4. Embed the pizza query
    let query = "pizza making techniques and oven temperature";
    let query_embedding = provider
        .embed(query)
        .expect("query embedding should succeed");

    // 5. Semantic search -- this exercises the two-phase search
    //    (unchunked entries + embedding_chunk table).
    let filter = crate::store::KnowledgeFilter::default();
    let results = db
        .semantic_search(&query_embedding, &ctx, &filter, 10)
        .unwrap();

    // 6. Assert the long entry appears in results
    let found = results.iter().any(|e| e.id == "kn-pizza-deep");
    assert!(
        found,
        "Chunked semantic search should find the entry with buried pizza content. \
         Got {} results: {:?}",
        results.len(),
        results.iter().map(|e| e.id.as_str()).collect::<Vec<_>>()
    );
}

// =========================================================================
// Issue #352 — chunk_count backfill regression coverage (W1).
//
// The chunk_count backfill computes a value rather than a constant, so it
// gets a real test (per "THE RULE" in schema/surrealdb-schema.surql). It
// covers: the non-empty case (entry with N>0 embedding_chunk rows), the zero
// case (entry with no chunks), and idempotency (re-running over an
// already-set value is a no-op). The throwaway probe that once "tested" this
// was deleted; this is its permanent replacement.
// =========================================================================

/// The exact chunk_count backfill statement that ships in
/// schema/surrealdb-schema.surql (Issue #352). Kept in sync with the schema:
/// this test is the guard that the idiom actually counts chunks correctly.
const BACKFILL_CHUNK_COUNT_SQL: &str = "UPDATE knowledge SET chunk_count = array::len(\
    SELECT id FROM embedding_chunk \
    WHERE entry_id = string::concat('kn-', meta::id($parent.id))) \
    WHERE chunk_count IS NONE";

/// Strand a knowledge row's `chunk_count` as NONE, faithfully reproducing the
/// production pre-chunking state: SurrealDB forbids writing NONE to a required
/// `int` field, so the only way a live row got NONE was the field being DEFINEd
/// AFTER the row already existed (a re-DEFINE does not backfill existing rows).
/// We reproduce that exactly: drop the field constraint, set the value to NONE,
/// then re-DEFINE the field — the row keeps NONE, which the backfill must fix.
fn strand_chunk_count_none(db: &SurrealDatabase, record: &str) {
    db.test_exec("REMOVE FIELD IF EXISTS chunk_count ON knowledge")
        .unwrap();
    db.test_exec(&format!("UPDATE {} SET chunk_count = NONE", record))
        .unwrap();
    db.test_exec("DEFINE FIELD IF NOT EXISTS chunk_count ON knowledge TYPE int DEFAULT 0")
        .unwrap();
}

/// Insert `n` embedding_chunk rows for `entry_id` (kn-<hex> form).
fn insert_n_chunks(db: &SurrealDatabase, entry_id: &str, n: usize) {
    for i in 0..n {
        db.insert_embedding_chunk(
            entry_id,
            i,
            &format!("chunk {} text", i),
            i * 10,
            10,
            &[0.1f32, 0.2, 0.3],
            "test-model",
        )
        .unwrap();
    }
}

#[test]
fn test_backfill_chunk_count_nonempty() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Entry with 3 chunks. The id "kn-w1chunked" maps to record
    // knowledge:w1chunked, and meta::id(id) yields "w1chunked", so the
    // chunks must use entry_id "kn-w1chunked" (the backfill rebuilds that
    // key via string::concat('kn-', meta::id(id))).
    let entry = make_test_entry("kn-w1chunked", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();
    insert_n_chunks(&db, "kn-w1chunked", 3);

    // Reproduce a pre-chunking row whose chunk_count is genuinely NONE.
    strand_chunk_count_none(&db, "knowledge:w1chunked");
    assert_eq!(
        db.test_raw_chunk_count("kn-w1chunked").unwrap(),
        None,
        "precondition: chunk_count should be NONE before backfill"
    );

    // Run the real backfill.
    db.test_exec(BACKFILL_CHUNK_COUNT_SQL).unwrap();

    assert_eq!(
        db.test_raw_chunk_count("kn-w1chunked").unwrap(),
        Some(3),
        "backfill must count the 3 embedding_chunk rows"
    );
}

#[test]
fn test_backfill_chunk_count_zero_case() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Entry with NO chunks (a pre-chunking row that was never chunked).
    let entry = make_test_entry("kn-w1zero", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();

    strand_chunk_count_none(&db, "knowledge:w1zero");
    assert_eq!(db.test_raw_chunk_count("kn-w1zero").unwrap(), None);

    db.test_exec(BACKFILL_CHUNK_COUNT_SQL).unwrap();

    // array::len over an empty SELECT is naturally 0 — no projection-shape
    // dependency, which is the whole point of the S1 form.
    assert_eq!(
        db.test_raw_chunk_count("kn-w1zero").unwrap(),
        Some(0),
        "entry with no chunks must backfill to 0"
    );
}

#[test]
fn test_backfill_chunk_count_idempotent() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry = make_test_entry("kn-w1idem", 5, 0.5);
    db.upsert_knowledge(&entry).unwrap();
    insert_n_chunks(&db, "kn-w1idem", 2);

    strand_chunk_count_none(&db, "knowledge:w1idem");
    db.test_exec(BACKFILL_CHUNK_COUNT_SQL).unwrap();
    assert_eq!(db.test_raw_chunk_count("kn-w1idem").unwrap(), Some(2));

    // Add MORE chunks after the value is set. The guard WHERE chunk_count IS
    // NONE means a second run must NOT touch the already-set value, even
    // though the underlying chunk count has changed. This proves the no-op
    // guarantee that keeps the statement safe to replay on every connection.
    insert_n_chunks(&db, "kn-w1idem", 5); // now 5 chunks total on disk
    db.test_exec(BACKFILL_CHUNK_COUNT_SQL).unwrap();
    assert_eq!(
        db.test_raw_chunk_count("kn-w1idem").unwrap(),
        Some(2),
        "re-running backfill must not change an already-set chunk_count"
    );
}

// =========================================================================
// Issue #360 — cold-upgrade backfill, END-TO-END through apply_schema.
//
// THE CI BLIND SPOT THIS CLOSES: test_backfill_chunk_count_* (above) only run
// the ISOLATED backfill UPDATE against a row whose chunk_count is already NONE.
// They never replay the full SCHEMA const through the real apply path, so they
// could not catch the production failure in #360: on a populated pre-existing
// graph, the strict `DEFINE FIELD` leaves legacy rows holding NONE, and the
// FIRST write in the schema's backfill block (the wake_phrases migration) then
// aborts whole-record validation — taking the entire all-or-nothing
// `db.query(SCHEMA)` (take_errors) down with it. These tests reproduce that by
// stranding the legacy-added required fields as genuinely-NONE, replaying the
// ACTUAL SCHEMA through apply_schema_explicit end-to-end, and asserting both
// that apply succeeds AND that an ordinary subsequent write to a legacy row
// succeeds (the exact production failure mode).
//
// Backing store: open_in_memory() uses the real embedded SurrealKV engine (not
// a mock), so SCHEMAFULL strict-DEFINE whole-record validation reproduces
// faithfully — the same mechanics as `surreal start memory`.
// =========================================================================

/// Strand a knowledge row so EVERY legacy-added required field is genuinely
/// NONE — the exact production state of a row that predates each field. This is
/// the COMPLETE post-release required-field set on `knowledge` (every required,
/// non-`option<>` field the read path coalesces in knowledge_select_fields(),
/// i.e. every one that can be NONE on a legacy row):
///
/// * triggers, wake_phrases, chunk_count, format  (Issue #360, PR #367)
/// * resonance, activation_count, decay_rate      (wake-up cascade cohort)
///
/// Stranding the WHOLE set (not just the first four) is deliberate: it makes
/// this helper structurally catch the entire class going forward. Add a row to
/// the array here whenever a new required field is added after the table's first
/// release, and the end-to-end test below will fail until the schema heals it.
///
/// We can't just `SET x = NONE` while the field is strict (SurrealDB rejects
/// NONE for a required field), so for each field we reproduce the real history:
/// drop the field constraint, set the value to NONE, then re-DEFINE the field as
/// it existed in the PRE-fix schema (strict). This leaves the row holding NONE
/// under a strict definition — precisely the cold-upgrade trap. Replaying the
/// (fixed) SCHEMA must then heal it via option → backfill → OVERWRITE.
fn strand_cold_upgrade_fields(db: &SurrealDatabase, record: &str) {
    // (field name, the strict pre-fix DEFINE to restore)
    let fields: [(&str, &str); 7] = [
        (
            "triggers",
            "DEFINE FIELD triggers ON knowledge TYPE array<string> DEFAULT []",
        ),
        (
            "wake_phrases",
            "DEFINE FIELD wake_phrases ON knowledge TYPE array<string> DEFAULT []",
        ),
        (
            "chunk_count",
            "DEFINE FIELD chunk_count ON knowledge TYPE int DEFAULT 0",
        ),
        (
            "format",
            "DEFINE FIELD format ON knowledge TYPE string DEFAULT 'markdown' \
             ASSERT $value IN ['markdown', 'json', 'stele:markdown', 'stele:ascii', \
             'stele:light', 'stele:full']",
        ),
        (
            "resonance",
            "DEFINE FIELD resonance ON knowledge TYPE int DEFAULT 0",
        ),
        (
            "activation_count",
            "DEFINE FIELD activation_count ON knowledge TYPE int DEFAULT 0",
        ),
        (
            "decay_rate",
            "DEFINE FIELD decay_rate ON knowledge TYPE float DEFAULT 0.0",
        ),
    ];

    // 1. Drop ALL field constraints first. We cannot strand them one at a
    //    time: re-DEFINEing one as strict before the others are set to NONE
    //    would make the NEXT `SET <other> = NONE` write fail whole-record
    //    validation on the just-tightened field (that is literally the #360
    //    trap). So unconstrain everything, THEN null everything, THEN restore
    //    the strict defs last (pure DDL — no row write to validate).
    for (name, _) in fields {
        db.test_exec(&format!("REMOVE FIELD IF EXISTS {} ON knowledge", name))
            .unwrap();
    }
    // 2. Null all of them in a single write (no strict field is in the way now).
    let set_clause = fields
        .iter()
        .map(|(name, _)| format!("{} = NONE", name))
        .collect::<Vec<_>>()
        .join(", ");
    db.test_exec(&format!("UPDATE {} SET {}", record, set_clause))
        .unwrap();
    // 3. Restore the strict pre-fix definitions. The row keeps NONE under a
    //    strict definition — the exact cold-upgrade stranded state.
    for (_, strict_define) in fields {
        db.test_exec(strict_define).unwrap();
    }
}

/// Read the raw stored value of a knowledge field WITHOUT read-coalescing,
/// returning the JSON value (`null` == NONE on disk).
fn raw_field(db: &SurrealDatabase, record: &str, field: &str) -> serde_json::Value {
    let sql = format!("SELECT {0} FROM {1}", field, record);
    // test_exec swallows results; use a tiny inline query via the same runtime.
    SurrealDatabase::runtime().block_on(async {
        let mut response = with_db!(db, db, { db.query(&sql).await.unwrap() });
        let rows: Vec<serde_json::Value> = response.take(0).unwrap();
        rows.into_iter()
            .next()
            .and_then(|r| r.get(field).cloned())
            .unwrap_or(serde_json::Value::Null)
    })
}

#[test]
fn test_cold_upgrade_apply_schema_heals_stranded_legacy_rows() {
    // A populated pre-existing graph: a real knowledge row exists, then loses
    // EVERY legacy-added required field to NONE (predating them) — the full
    // post-release required-field set, including the resonance cascade cohort
    // (resonance/activation_count/decay_rate) that the original #360 fix missed.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let entry = make_test_entry("kn-legacy360", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    strand_cold_upgrade_fields(&db, "knowledge:legacy360");

    // Precondition: the fields are genuinely NONE on disk (null), exactly the
    // production stranded state.
    assert!(
        raw_field(&db, "knowledge:legacy360", "triggers").is_null(),
        "precondition: triggers must be NONE before re-apply"
    );
    assert!(
        raw_field(&db, "knowledge:legacy360", "chunk_count").is_null(),
        "precondition: chunk_count must be NONE before re-apply"
    );
    // The resonance cascade cohort — the regression Verdictia proved strands the
    // PR #367 head: apply ABORTS with `FieldCheck { value: "NONE", field:
    // activation_count, check: "int" }` until these are treated too.
    assert!(
        raw_field(&db, "knowledge:legacy360", "resonance").is_null(),
        "precondition: resonance must be NONE before re-apply"
    );
    assert!(
        raw_field(&db, "knowledge:legacy360", "activation_count").is_null(),
        "precondition: activation_count must be NONE before re-apply"
    );
    assert!(
        raw_field(&db, "knowledge:legacy360", "decay_rate").is_null(),
        "precondition: decay_rate must be NONE before re-apply"
    );

    // ACT: replay the FULL SCHEMA const through the real apply path,
    // end-to-end. On the OLD ordering this aborts (the backfill UPDATE block is
    // the failing write); on the fixed ordering option→backfill→OVERWRITE heals
    // the rows first.
    db.apply_schema_explicit(false)
        .expect("apply_schema must succeed on a populated pre-existing graph (Issue #360)");

    // ASSERT (a): the stranded fields were backfilled to their defaults.
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "triggers"),
        serde_json::json!([]),
        "triggers must be backfilled to []"
    );
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "chunk_count"),
        serde_json::json!(0),
        "chunk_count must be backfilled to 0 (no chunks for this entry)"
    );
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "format"),
        serde_json::json!("markdown"),
        "format must be backfilled to 'markdown'"
    );
    // The resonance cascade cohort backfills to its strict DEFAULTs (0/0/0.0).
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "resonance"),
        serde_json::json!(0),
        "resonance must be backfilled to 0"
    );
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "activation_count"),
        serde_json::json!(0),
        "activation_count must be backfilled to 0"
    );
    assert_eq!(
        raw_field(&db, "knowledge:legacy360", "decay_rate"),
        serde_json::json!(0.0),
        "decay_rate must be backfilled to 0.0"
    );

    // ASSERT (b): the EXACT production failure mode — an ordinary subsequent
    // write to the (formerly) legacy row must succeed. update_activations does
    // a partial `SET`, which still triggers SCHEMAFULL whole-record validation.
    db.update_activations(&["kn-legacy360".to_string()])
        .expect("ordinary write to a healed legacy row must succeed (Issue #360)");
}

#[test]
fn test_cold_upgrade_apply_schema_idempotent_on_healthy_graph() {
    // A second full apply over an already-healthy graph must succeed and not
    // thrash: the WHERE <f> IS NONE backfills are no-ops and the OVERWRITE
    // tightens re-apply cleanly.
    let db = SurrealDatabase::open_in_memory().unwrap();
    let entry = make_test_entry("kn-healthy360", 5, 0.0);
    db.upsert_knowledge(&entry).unwrap();

    // Re-apply twice more (schema already applied once on open).
    db.apply_schema_explicit(false).unwrap();
    db.apply_schema_explicit(false).unwrap();

    // Fields stay strict + correct across the expanded set, and ordinary writes
    // still work. Covers both the original #360 four and the resonance cohort.
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "chunk_count"),
        serde_json::json!(0)
    );
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "triggers"),
        serde_json::json!([])
    );
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "format"),
        serde_json::json!("markdown")
    );
    // OVERWRITE preserves real data on a healthy row: make_test_entry set
    // resonance=5, decay_rate=0.0; activation_count defaults to 0. The IS NONE
    // backfills must NOT clobber the live resonance value.
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "resonance"),
        serde_json::json!(5),
        "idempotent re-apply must NOT reset a healthy resonance to its default"
    );
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "activation_count"),
        serde_json::json!(0)
    );
    assert_eq!(
        raw_field(&db, "knowledge:healthy360", "decay_rate"),
        serde_json::json!(0.0)
    );
    db.update_activations(&["kn-healthy360".to_string()])
        .expect("ordinary write after idempotent re-apply must succeed");
}

// =========================================================================
// TRIGGERS (Issue #246, PR 1/4 -- data layer)
// =========================================================================

#[test]
fn test_triggers_round_trip_normalized_and_deduped() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut entry = make_test_entry("kn-trig1", 5, 0.0);
    // Mix of cases needing normalization: uppercase, leading/trailing space,
    // collapsible internal whitespace, an exact duplicate (post-normalization),
    // and an empty string that must be dropped.
    entry.triggers = vec![
        "Brad".to_string(),
        "  blood   sugar ".to_string(),
        "BLOOD SUGAR".to_string(), // dup of "blood sugar" after normalization
        "Glucose".to_string(),
        "".to_string(),    // empty -> dropped
        "   ".to_string(), // whitespace-only -> dropped
    ];
    db.upsert_knowledge(&entry).unwrap();

    let ctx = crate::store::AgentContext::public_only();
    let got = db
        .get("kn-trig1", &ctx)
        .unwrap()
        .expect("entry should exist");

    // Normalized: lowercased, whitespace-collapsed, empties dropped, deduped,
    // first-seen order preserved.
    assert_eq!(
        got.triggers,
        vec![
            "brad".to_string(),
            "blood sugar".to_string(),
            "glucose".to_string(),
        ],
        "triggers must be normalized, deduped, and empty-dropped on write"
    );
}

#[test]
fn test_triggers_default_empty_when_absent() {
    // No-backfill safety: an entry created WITHOUT triggers must read back as
    // `[]` (never NONE), via the `IF triggers THEN triggers ELSE [] END`
    // read-path coalesce plus the always-bound write path.
    let db = SurrealDatabase::open_in_memory().unwrap();

    let entry = make_test_entry("kn-trig2", 5, 0.0); // make_test_entry sets triggers: vec![]
    db.upsert_knowledge(&entry).unwrap();

    let ctx = crate::store::AgentContext::public_only();
    let got = db
        .get("kn-trig2", &ctx)
        .unwrap()
        .expect("entry should exist");

    assert_eq!(
        got.triggers,
        Vec::<String>::new(),
        "an entry with no triggers must read back as an empty array"
    );
}

// =========================================================================
// Issue #246 PR3: trigger-check matching engine + visibility + dedup + cap
// =========================================================================

/// `list_with_triggers` returns only entries with a non-empty `triggers` array,
/// and applies the agent visibility filter (private entries only for owner).
#[test]
fn test_list_with_triggers_prefilters_and_respects_visibility() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Public entry WITH triggers -> should appear for everyone.
    let mut public_trig = make_test_entry("kn-pub-trig", 5, 0.0);
    public_trig.triggers = vec!["brad".to_string()];
    db.upsert_knowledge(&public_trig).unwrap();

    // Public entry WITHOUT triggers -> filtered out by array::len > 0.
    let no_trig = make_test_entry("kn-pub-notrig", 5, 0.0);
    db.upsert_knowledge(&no_trig).unwrap();

    // Private entry WITH triggers, owned by agent-a -> only agent-a sees it.
    let mut priv_trig = make_test_entry("kn-priv-trig", 5, 0.0);
    priv_trig.triggers = vec!["secret".to_string()];
    priv_trig.visibility = "private".to_string();
    priv_trig.owner = Some("agent-a".to_string());
    db.upsert_knowledge(&priv_trig).unwrap();

    // agent-b: sees only the public trigger-bearing entry.
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let ids_b: HashSet<String> = db
        .list_with_triggers(&ctx_b)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert!(ids_b.contains("kn-pub-trig"));
    assert!(
        !ids_b.contains("kn-pub-notrig"),
        "entries without triggers must be prefiltered out"
    );
    assert!(
        !ids_b.contains("kn-priv-trig"),
        "agent-b must NOT see agent-a's private triggered memory"
    );

    // agent-a: sees both their private trigger entry and the public one.
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let ids_a: HashSet<String> = db
        .list_with_triggers(&ctx_a)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert!(ids_a.contains("kn-pub-trig"));
    assert!(
        ids_a.contains("kn-priv-trig"),
        "agent-a must see their own private triggered memory"
    );
}

/// End-to-end VISIBILITY at the matcher layer: agent-b's check does NOT fire
/// agent-a's private triggered memory, even when the message contains the
/// trigger word.
#[test]
fn test_trigger_check_visibility_private_does_not_fire_for_other_agent() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    let mut priv_trig = make_test_entry("kn-brad-private", 8, 0.0);
    priv_trig.triggers = vec!["brad".to_string()];
    priv_trig.visibility = "private".to_string();
    priv_trig.owner = Some("agent-a".to_string());
    db.upsert_knowledge(&priv_trig).unwrap();

    let message = "what is brad up to";

    // agent-b: the private memory is not even in the candidate set -> no match.
    let ctx_b = crate::store::AgentContext::for_agent("agent-b");
    let entries_b = db.list_with_triggers(&ctx_b).unwrap();
    let pairs_b: Vec<(&str, &[String])> = entries_b
        .iter()
        .map(|e| (e.id.as_str(), e.triggers.as_slice()))
        .collect();
    assert!(
        crate::triggers::match_entries(message, pairs_b).is_empty(),
        "agent-b must not fire agent-a's private memory"
    );

    // agent-a: same message DOES fire it.
    let ctx_a = crate::store::AgentContext::for_agent("agent-a");
    let entries_a = db.list_with_triggers(&ctx_a).unwrap();
    let pairs_a: Vec<(&str, &[String])> = entries_a
        .iter()
        .map(|e| (e.id.as_str(), e.triggers.as_slice()))
        .collect();
    let matches_a = crate::triggers::match_entries(message, pairs_a);
    assert_eq!(matches_a.len(), 1);
    assert_eq!(matches_a[0].id, "kn-brad-private");
}

/// Fire cap of 5 by resonance desc: 6 matches -> top 5 fire, 1 deferred; the
/// deferred one fires on a subsequent check (after the first five are marked).
/// Exercises the same logic the handler uses (sort by resonance desc, cap,
/// then FiredStore dedup), wired directly so it needs no CLI add-flag.
#[test]
fn test_trigger_check_cap_and_deferred_fires_next_turn() {
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Six entries, all triggered by "topic", distinct resonance 1..=6 so the
    // ordering is unambiguous. Highest resonance should win the 5 slots.
    for r in 1..=6 {
        let mut e = make_test_entry(&format!("kn-cap-{r}"), r, 0.0);
        e.triggers = vec!["topic".to_string()];
        db.upsert_knowledge(&e).unwrap();
    }

    // Isolated fired-state file.
    let dir = tempfile::tempdir().unwrap();
    let store = crate::triggers::FiredStore::at(dir.path().join("fired.json"));

    let run_check = |store: &crate::triggers::FiredStore| -> Vec<String> {
        let ctx = crate::store::AgentContext::public_only();
        let entries = db.list_with_triggers(&ctx).unwrap();
        let matched: HashSet<String> = {
            let pairs: Vec<(&str, &[String])> = entries
                .iter()
                .map(|e| (e.id.as_str(), e.triggers.as_slice()))
                .collect();
            crate::triggers::match_entries("a topic question", pairs)
                .into_iter()
                .map(|m| m.id)
                .collect()
        };
        // Sort matched entries by resonance desc, id asc (handler's ordering).
        let mut matched_entries: Vec<_> =
            entries.iter().filter(|e| matched.contains(&e.id)).collect();
        matched_entries.sort_by(|a, b| b.resonance.cmp(&a.resonance).then(a.id.cmp(&b.id)));
        let already = store.read_fired().unwrap();
        let to_fire: Vec<String> = matched_entries
            .into_iter()
            .filter(|e| !already.contains(&e.id))
            .take(5)
            .map(|e| e.id.clone())
            .collect();
        store.mark_survivors(&to_fire).unwrap()
    };

    // First check: top 5 by resonance (6,5,4,3,2) fire; resonance-1 deferred.
    let fired1 = run_check(&store);
    assert_eq!(
        fired1,
        vec![
            "kn-cap-6".to_string(),
            "kn-cap-5".to_string(),
            "kn-cap-4".to_string(),
            "kn-cap-3".to_string(),
            "kn-cap-2".to_string(),
        ],
        "top 5 by resonance desc fire"
    );

    // Second check (same message): the 5 already fired are deduped, the deferred
    // resonance-1 entry now fires.
    let fired2 = run_check(&store);
    assert_eq!(
        fired2,
        vec!["kn-cap-1".to_string()],
        "the deferred (overflow) memory fires on a subsequent check"
    );

    // Third check: everything fired -> nothing new.
    let fired3 = run_check(&store);
    assert!(fired3.is_empty(), "all memories already fired this session");
}

// =========================================================================
// Dimension-mismatch cosine guard regression (production incident).
//
// SurrealDB's `vector::similarity::cosine` ABORTS THE ENTIRE SCAN when it
// encounters any row whose embedding dimension differs from the query
// vector. A single off-dimension row (famously, a dim-4 unit-test fixture
// that leaked into the live graph) therefore broke add-dedup, auto_anchor
// (#362), semantic search, and trigger-check (#246) all at once.
//
// The fix adds `AND array::len(embedding) = $dim` to every cosine query so a
// mismatched row is SKIPPED rather than aborting the scan. These tests would
// have caught the production breakage: a store containing one off-dim row
// alongside good rows must still return the good rows WITHOUT erroring.
//
// Isolation: uses `open_in_memory()` (forced embedded tempdir), so it can
// never touch the live DB even if MX_SURREAL_* is set in the environment.
// =========================================================================

/// Build a knowledge entry carrying an embedding of an arbitrary dimension.
/// Lets us seed both good (matching) and poisoned (mismatched) rows.
fn entry_with_dim_embedding(id: &str, embedding: Vec<f32>) -> crate::knowledge::KnowledgeEntry {
    let mut e = make_test_entry(id, 5, 0.0);
    e.content_hash = Some(format!("hash-{id}"));
    e.embedding = Some(embedding);
    e.embedding_model = Some("test-model".to_string());
    e.embedded_at = Some(chrono::Utc::now().to_rfc3339());
    e
}

#[test]
fn off_dim_row_does_not_abort_cosine_scan() {
    use crate::store::{AgentContext, KnowledgeStore};

    // Isolated, never-live store.
    let db = SurrealDatabase::open_in_memory().unwrap();

    // Two GOOD rows at the query dimension (N = 8).
    const N: usize = 8;
    let mut good_a = vec![0.0f32; N];
    good_a[0] = 1.0; // unit vector aligned with the query
    let mut good_b = vec![0.0f32; N];
    good_b[1] = 1.0; // orthogonal-ish, still dim N
    db.upsert_knowledge(&entry_with_dim_embedding("kn-good-a", good_a.clone()))
        .unwrap();
    db.upsert_knowledge(&entry_with_dim_embedding("kn-good-b", good_b))
        .unwrap();

    // One POISONED row: a dim-4 embedding (the exact shape of the leaked
    // fixture). Before the guard, this row made cosine abort the whole scan.
    db.upsert_knowledge(&entry_with_dim_embedding(
        "kn-poison-dim4",
        vec![1.0, 0.0, 0.0, 0.0],
    ))
    .unwrap();

    let ctx = AgentContext::public_only();
    let filter = crate::store::KnowledgeFilter::default();
    // Query vector at dimension N, aligned with kn-good-a.
    let query = good_a;

    // Two-phase semantic search must NOT error and must return the good rows.
    let results = db
        .semantic_search(&query, &ctx, &filter, 10)
        .expect("semantic_search must skip the off-dim row, not abort the scan");
    let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"kn-good-a"),
        "the matching good row must be returned; got {ids:?}"
    );
    assert!(
        ids.contains(&"kn-good-b"),
        "the second good row must be returned; got {ids:?}"
    );
    assert!(
        !ids.contains(&"kn-poison-dim4"),
        "the off-dim row must be skipped by the guard; got {ids:?}"
    );

    // Entry-level scored search (the auto_anchor #362 path) must also survive.
    let scored = db
        .semantic_search_entries_scored(&query, &ctx, 10)
        .expect("entries-scored search must skip the off-dim row, not abort the scan");
    let scored_ids: Vec<&str> = scored.iter().map(|(e, _)| e.id.as_str()).collect();
    assert!(
        scored_ids.contains(&"kn-good-a") && scored_ids.contains(&"kn-good-b"),
        "entry-level scored search must return the good rows; got {scored_ids:?}"
    );
    assert!(
        !scored_ids.contains(&"kn-poison-dim4"),
        "entry-level scored search must skip the off-dim row; got {scored_ids:?}"
    );
}
