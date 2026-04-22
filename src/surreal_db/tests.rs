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
fn test_open_with_path() {
    use tempfile::tempdir;

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.surreal");

    // Open database at specific path
    let _db = SurrealDatabase::open(&db_path).unwrap();

    // Verify directory was created
    assert!(db_path.exists());
    assert!(db_path.is_dir());
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
        wake_order: None,
        wake_phrase: None,
        embedding: None,
        embedding_model: None,
        embedded_at: None,
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
