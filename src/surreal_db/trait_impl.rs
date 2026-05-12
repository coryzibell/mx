use anyhow::Result;

use crate::knowledge::KnowledgeEntry;
use crate::store::KnowledgeStore;
use crate::types::{
    Agent, ApplicabilityType, Category, ContentType, EntryType, Project, Relationship,
    RelationshipType, Session, SessionType, SourceType,
};

use super::SurrealDatabase;

// ============================================================================
// KNOWLEDGESTORE TRAIT IMPLEMENTATION
// ============================================================================

impl KnowledgeStore for SurrealDatabase {
    fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.upsert_knowledge_internal(entry)?;
        Ok(())
    }

    fn get(&self, id: &str, ctx: &crate::store::AgentContext) -> Result<Option<KnowledgeEntry>> {
        self.get_knowledge(id, ctx)
    }

    fn delete(&self, id: &str, ctx: &crate::store::AgentContext) -> Result<bool> {
        self.delete_knowledge(id, ctx)
    }

    fn search(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.search_knowledge(query, ctx, filter)
    }

    fn semantic_search(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.semantic_search_knowledge(query_embedding, ctx, filter, limit)
    }

    fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.list_by_category(category, ctx, filter)
    }

    fn count_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<usize> {
        self.count_by_category(category, ctx, filter)
    }

    fn list_all(&self, ctx: &crate::store::AgentContext) -> Result<Vec<KnowledgeEntry>> {
        self.list_all(ctx)
    }

    fn count(&self) -> Result<usize> {
        self.count()
    }

    fn wake_cascade(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        self.wake_cascade(ctx, limit, min_resonance, days)
    }

    fn update_activations(&self, ids: &[String]) -> Result<()> {
        self.update_activations(ids)
    }

    fn update_summary(
        &self,
        id: &str,
        summary: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<bool> {
        self.update_summary(id, summary, ctx)
    }

    fn increment_activation_count(&self, ids: &[String]) -> Result<()> {
        self.increment_activation_count(ids)
    }

    fn query_recent_facts(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        self.query_recent_facts(days)
    }

    fn query_recent_facts_all_types(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        self.query_recent_facts_all_types(days)
    }

    fn reinforce(
        &self,
        id: &str,
        amount: i32,
        cap: Option<i32>,
        ctx: &crate::store::AgentContext,
    ) -> Result<Option<crate::store::ReinforcementResult>> {
        self.reinforce(id, amount, cap, ctx)
    }

    fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        self.get_tags_for_entry(entry_id)
    }

    fn set_tags_for_entry(&self, entry_id: &str, tags: &[String]) -> Result<()> {
        self.set_tags_for_entry(entry_id, tags)
    }

    fn list_all_tags(&self, category: Option<&str>) -> Result<Vec<String>> {
        self.list_all_tags(category)
    }

    fn get_applicability_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        self.get_applicability_for_entry(entry_id)
    }

    fn set_applicability_for_entry(&self, entry_id: &str, ids: &[String]) -> Result<()> {
        self.set_applicability_for_entry(entry_id, ids)
    }

    fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
        self.list_applicability_types()
    }

    fn upsert_applicability_type(&self, atype: &ApplicabilityType) -> Result<()> {
        self.upsert_applicability_type(atype)
    }

    fn list_categories(&self) -> Result<Vec<Category>> {
        self.list_categories()
    }

    fn get_category(&self, id: &str) -> Result<Option<Category>> {
        self.get_category(id)
    }

    fn upsert_category(&self, category: &Category) -> Result<()> {
        self.upsert_category(category)
    }

    fn delete_category(&self, id: &str) -> Result<bool> {
        self.delete_category(id)
    }

    fn list_projects(&self, _active_only: bool) -> Result<Vec<Project>> {
        // SurrealDB implementation doesn't filter by active yet
        self.list_projects()
    }

    fn get_project(&self, id: &str) -> Result<Option<Project>> {
        self.get_project(id)
    }

    fn upsert_project(&self, project: &Project) -> Result<()> {
        self.upsert_project_internal(project)?;
        Ok(())
    }

    fn get_tags_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        self.get_tags_for_project(project_id)
    }

    fn set_tags_for_project(&self, project_id: &str, tags: &[String]) -> Result<()> {
        self.set_tags_for_project(project_id, tags)
    }

    fn get_applicability_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        self.get_applicability_for_project(project_id)
    }

    fn set_applicability_for_project(&self, project_id: &str, ids: &[String]) -> Result<()> {
        self.set_applicability_for_project(project_id, ids)
    }

    fn list_agents(&self) -> Result<Vec<Agent>> {
        self.list_agents()
    }

    fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        self.get_agent(id)
    }

    fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        self.upsert_agent(agent)
    }

    fn list_relationships_for_entry(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        self.list_relationships(entry_id)
    }

    fn add_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<String> {
        self.add_relationship(from, to, rel_type)?;
        // Return a synthetic ID since SurrealDB edge records don't have simple IDs
        Ok(format!("rel-{}-{}", from, to))
    }

    fn delete_relationship(&self, id: &str) -> Result<bool> {
        self.delete_relationship_by_id(id)
    }

    fn get_facts_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        self.get_facts_for_session(session_id)
    }

    fn get_session_for_fact(&self, fact_id: &str) -> Result<Option<String>> {
        self.get_session_for_fact(fact_id)
    }

    fn list_tables(&self) -> Result<Vec<String>> {
        self.list_tables()
    }

    fn list_sessions(&self, project_id: Option<&str>) -> Result<Vec<Session>> {
        self.list_sessions(project_id)
    }

    fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.get_session(id)
    }

    fn upsert_session(&self, session: &Session) -> Result<()> {
        self.upsert_session(session)
    }

    fn list_source_types(&self) -> Result<Vec<SourceType>> {
        self.list_source_types()
    }

    fn list_entry_types(&self) -> Result<Vec<EntryType>> {
        self.list_entry_types()
    }

    fn list_content_types(&self) -> Result<Vec<ContentType>> {
        self.list_content_types()
    }

    fn list_session_types(&self) -> Result<Vec<SessionType>> {
        self.list_session_types()
    }

    fn list_relationship_types(&self) -> Result<Vec<RelationshipType>> {
        self.list_relationship_types()
    }

    fn edit_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
        nth: Option<usize>,
    ) -> Result<crate::store::EditResult> {
        self.edit_content(id, ctx, old_text, new_text, replace_all, nth)
    }

    fn append_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        content: &str,
    ) -> Result<()> {
        self.append_content(id, ctx, content)
    }

    fn prepend_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        content: &str,
    ) -> Result<()> {
        self.prepend_content(id, ctx, content)
    }

    fn backup_content(
        &self,
        entry: &KnowledgeEntry,
        operation: &str,
        agent: Option<&str>,
    ) -> Result<String> {
        self.backup_content_internal(entry, operation, agent)
    }

    fn list_backups(&self, entry_id: &str) -> Result<Vec<crate::types::MemoryBackup>> {
        self.list_backups_internal(entry_id)
    }

    fn latest_backup(&self, entry_id: &str) -> Result<Option<crate::types::MemoryBackup>> {
        self.latest_backup_internal(entry_id)
    }

    fn purge_backups(&self, entry_id: &str, keep: usize) -> Result<()> {
        self.purge_backups_internal(entry_id, keep)
    }

    fn create_wake_session(&self, session: &crate::wake_token::WakeSession) -> Result<String> {
        self.create_wake_session(session)
    }

    fn get_wake_session(&self, session_id: &str) -> Result<Option<crate::wake_token::WakeSession>> {
        self.get_wake_session(session_id)
    }

    fn update_wake_session(&self, session: &crate::wake_token::WakeSession) -> Result<()> {
        self.update_wake_session(session)
    }

    fn delete_wake_session(&self, session_id: &str) -> Result<()> {
        self.delete_wake_session(session_id)
    }

    fn sweep_ghost_anchors(&self, dry_run: bool) -> Result<crate::store::GhostSweepResult> {
        self.sweep_ghost_anchors(dry_run)
    }
}
