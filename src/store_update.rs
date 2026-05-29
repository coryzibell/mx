//! Builder-pattern update API for `KnowledgeStore` (Issue #134).
//!
//! Instead of adding a new field-specific method to the [`KnowledgeStore`] trait
//! every time a column needs targeted updating (`update_summary`,
//! `increment_activation_count`, ...), callers compose an update fluently:
//!
//! ```rust,ignore
//! store.update("kn-abc123")
//!     .summary("new summary")
//!     .resonance(8)
//!     .add_tag("new-tag")
//!     .execute(&ctx)?;
//! ```
//!
//! # The safety property (carried forward from PR #131)
//!
//! The builder records ONLY the fields the caller actually set, as a list of
//! [`FieldUpdate`] entries inside an [`UpdateSpec`]. The backend translates that
//! spec into a targeted `UPDATE knowledge SET <only set columns> WHERE ...`.
//! A field that was never set on the builder has no [`FieldUpdate`] and therefore
//! CANNOT appear in the generated `SET` clause. There is no path that writes a
//! full record, so a partial update can never silently overwrite untouched
//! columns. See [`UpdateSpec::set_clause_columns`] and the backend's
//! `apply_update` for where this is enforced and tested.
//!
//! Tags are graph edges (`tagged_with`), not a column on `knowledge`, so
//! `add_tag` is tracked separately in [`UpdateSpec::add_tags`] and applied as a
//! `RELATE` alongside (not inside) the column `SET`. This keeps the `SET` clause
//! column-pure: tag adds never leak into the targeted-column safety surface. The
//! `RELATE` is itself gated on a visibility-filtered existence subquery, so the
//! edge write obeys the same agent-visibility filter as the column `UPDATE`
//! (symmetric TOCTOU safety).
//!
//! # Known exception to the single update path
//!
//! `update_summary` delegates here, so the builder is the *primary* place
//! knowledge `UPDATE ... SET` is built. One backend method is a deliberate,
//! tracked exception: `reinforce` hand-writes its own `UPDATE knowledge SET ...`
//! because it is read-compute-write (it computes the new resonance and returns a
//! `ReinforcementResult`), which a simple per-id column spec can't express. It is
//! left as-is; it is not a drift bug.

use anyhow::Result;

use crate::store::{AgentContext, KnowledgeStore};

/// A single targeted column update: the SurrealQL `SET` fragment plus its bound
/// parameter. The fragment must reference the parameter by name and must NEVER
/// interpolate a caller-supplied value directly (injection safety / repo
/// convention — all values are bound).
#[derive(Debug, Clone)]
pub struct FieldUpdate {
    /// SurrealQL assignment fragment, e.g. `"summary = $set_summary"` or
    /// `"activation_count += $set_activation_delta"`. References a bound param.
    pub assignment: String,
    /// Name of the bound parameter referenced by `assignment` (no leading `$`).
    /// Empty when the assignment binds no value (e.g. `last_activated = time::now()`).
    pub param: String,
    /// The value to bind for `param` (ignored when `param` is empty).
    pub value: UpdateValue,
}

/// Strongly-typed value to bind for a targeted update. Keeps value binding
/// explicit (no string interpolation of user data into SQL).
#[derive(Debug, Clone)]
pub enum UpdateValue {
    Str(String),
    Int(i64),
    /// No value to bind (server-side expression like `time::now()`).
    None,
}

/// The accumulated, backend-neutral description of a targeted update.
///
/// Built up by [`UpdateBuilder`] and handed to the backend's `apply_update`.
/// Holds only the fields that were explicitly set — see the module-level safety
/// note.
#[derive(Debug, Clone, Default)]
pub struct UpdateSpec {
    /// Targeted column assignments, in the order the caller set them.
    pub fields: Vec<FieldUpdate>,
    /// Tag names to add as `tagged_with` edges (graph relation, not a column).
    pub add_tags: Vec<String>,
}

impl UpdateSpec {
    /// True when the caller set nothing at all — neither a column nor a tag.
    /// The backend treats this as a no-op (see [`UpdateBuilder::execute`]).
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.add_tags.is_empty()
    }

    /// True when at least one targeted column was set (i.e. an `UPDATE ... SET`
    /// must run). Tag-only specs return false here — they need a `RELATE`, not
    /// a `SET`.
    pub fn has_column_updates(&self) -> bool {
        !self.fields.is_empty()
    }

    /// Render the comma-joined `SET` clause body from ONLY the set fields, e.g.
    /// `"summary = $set_summary, resonance = $set_resonance"`.
    ///
    /// This is the single place the targeted `SET` clause is assembled. Because
    /// it iterates `self.fields` (which only ever contains explicitly-set
    /// columns), an unset field is structurally impossible to include here.
    pub fn set_clause_columns(&self) -> String {
        self.fields
            .iter()
            .map(|f| f.assignment.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Outcome of an [`UpdateBuilder::execute`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateOutcome {
    /// Whether the target entry existed and was visible to the agent. `false`
    /// means "not found or not visible" (mirrors `update_summary`'s old bool).
    pub applied: bool,
    /// Whether the spec was empty (nothing set) — a no-op. When true no query
    /// ran and existence/visibility was never checked.
    pub no_op: bool,
}

impl UpdateOutcome {
    pub(crate) fn no_op() -> Self {
        Self {
            applied: false,
            no_op: true,
        }
    }

    /// Convenience for backends: a definite applied/not-applied result for a
    /// spec that actually ran.
    pub(crate) fn ran(applied: bool) -> Self {
        Self {
            applied,
            no_op: false,
        }
    }
}

/// Fluent builder returned by [`KnowledgeStore::update`].
///
/// Each setter appends to the [`UpdateSpec`]; nothing touches the database until
/// [`execute`](Self::execute) is called.
pub struct UpdateBuilder<'a> {
    store: &'a dyn KnowledgeStore,
    /// Target entry id (with or without `kn-` prefix; normalized by backend).
    id: String,
    spec: UpdateSpec,
}

impl<'a> UpdateBuilder<'a> {
    /// Create a builder. Prefer [`KnowledgeStore::update`] over calling this
    /// directly.
    pub fn new(store: &'a dyn KnowledgeStore, id: impl Into<String>) -> Self {
        Self {
            store,
            id: id.into(),
            spec: UpdateSpec::default(),
        }
    }

    /// Set the `summary` column.
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.spec.fields.push(FieldUpdate {
            assignment: "summary = $set_summary".to_string(),
            param: "set_summary".to_string(),
            value: UpdateValue::Str(summary.into()),
        });
        self
    }

    /// Set the `resonance` column to an absolute value.
    pub fn resonance(mut self, resonance: i32) -> Self {
        self.spec.fields.push(FieldUpdate {
            assignment: "resonance = $set_resonance".to_string(),
            param: "set_resonance".to_string(),
            value: UpdateValue::Int(resonance as i64),
        });
        self
    }

    /// Set the `activation_count` column to an absolute value.
    pub fn activation_count(mut self, count: i32) -> Self {
        self.spec.fields.push(FieldUpdate {
            assignment: "activation_count = $set_activation_count".to_string(),
            param: "set_activation_count".to_string(),
            value: UpdateValue::Int(count as i64),
        });
        self
    }

    /// Increment `activation_count` by `delta` (relative `+=`).
    pub fn increment_activation_count(mut self, delta: i32) -> Self {
        self.spec.fields.push(FieldUpdate {
            assignment: "activation_count += $set_activation_delta".to_string(),
            param: "set_activation_delta".to_string(),
            value: UpdateValue::Int(delta as i64),
        });
        self
    }

    /// Set `last_activated` to the current time (`time::now()`). No value bind
    /// needed — the timestamp is server-side, never a caller-supplied string.
    pub fn touch_last_activated(mut self) -> Self {
        self.spec.fields.push(FieldUpdate {
            assignment: "last_activated = time::now()".to_string(),
            param: String::new(),
            value: UpdateValue::None,
        });
        self
    }

    /// Add a `tagged_with` edge to the named tag (graph relation, not a column).
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.spec.add_tags.push(tag.into());
        self
    }

    /// Borrow the accumulated spec (useful for tests / introspection).
    pub fn spec(&self) -> &UpdateSpec {
        &self.spec
    }

    /// Run the targeted update.
    ///
    /// - If nothing was set, this is a no-op ([`UpdateOutcome::no_op`]) — no
    ///   query runs. This is deliberately not an error: composing an update and
    ///   conditionally setting zero fields should not blow up.
    /// - Otherwise delegates to the backend's [`KnowledgeStore::apply_update`],
    ///   which builds and runs ONE targeted `UPDATE ... SET` from only the set
    ///   columns (plus any tag `RELATE`s), under the agent visibility filter.
    pub fn execute(self, ctx: &AgentContext) -> Result<UpdateOutcome> {
        if self.spec.is_empty() {
            return Ok(UpdateOutcome::no_op());
        }
        self.store.apply_update(&self.id, &self.spec, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests exercise UpdateSpec — the structure that the targeted SQL is
    // built from — directly, without a database. They prove the no-full-record
    // safety property at the level where it is actually enforced: the SET clause
    // can only ever contain columns that were explicitly set.

    fn summary_field(s: &str) -> FieldUpdate {
        FieldUpdate {
            assignment: "summary = $set_summary".to_string(),
            param: "set_summary".to_string(),
            value: UpdateValue::Str(s.to_string()),
        }
    }

    fn resonance_field(r: i64) -> FieldUpdate {
        FieldUpdate {
            assignment: "resonance = $set_resonance".to_string(),
            param: "set_resonance".to_string(),
            value: UpdateValue::Int(r),
        }
    }

    #[test]
    fn empty_spec_is_empty_and_has_no_columns() {
        let spec = UpdateSpec::default();
        assert!(spec.is_empty());
        assert!(!spec.has_column_updates());
        assert_eq!(spec.set_clause_columns(), "");
    }

    #[test]
    fn single_field_sets_only_that_column() {
        let spec = UpdateSpec {
            fields: vec![summary_field("hello")],
            add_tags: vec![],
        };
        let set = spec.set_clause_columns();
        assert_eq!(set, "summary = $set_summary");
        // The critical property: no other column leaks in.
        assert!(!set.contains("resonance"));
        assert!(!set.contains("activation_count"));
        assert!(!set.contains("last_activated"));
    }

    #[test]
    fn multiple_fields_compose_in_order() {
        let spec = UpdateSpec {
            fields: vec![summary_field("hi"), resonance_field(8)],
            add_tags: vec![],
        };
        assert_eq!(
            spec.set_clause_columns(),
            "summary = $set_summary, resonance = $set_resonance"
        );
        assert!(spec.has_column_updates());
        assert!(!spec.is_empty());
    }

    #[test]
    fn unset_fields_never_appear_in_set_clause() {
        // Only resonance is set; summary/activation_count/last_activated must be absent.
        let spec = UpdateSpec {
            fields: vec![resonance_field(3)],
            add_tags: vec![],
        };
        let set = spec.set_clause_columns();
        assert_eq!(set, "resonance = $set_resonance");
        for forbidden in ["summary", "activation_count", "last_activated"] {
            assert!(
                !set.contains(forbidden),
                "unset column `{forbidden}` leaked into SET clause: {set}"
            );
        }
    }

    #[test]
    fn tag_only_spec_has_no_column_updates_but_is_not_empty() {
        let spec = UpdateSpec {
            fields: vec![],
            add_tags: vec!["rust".to_string()],
        };
        assert!(!spec.is_empty(), "tag-only spec is not empty");
        assert!(
            !spec.has_column_updates(),
            "tag-only spec must not trigger a column SET"
        );
        assert_eq!(spec.set_clause_columns(), "");
    }

    #[test]
    fn no_value_field_binds_nothing() {
        // last_activated = time::now() carries an empty param: nothing to bind.
        let spec = UpdateSpec {
            fields: vec![FieldUpdate {
                assignment: "last_activated = time::now()".to_string(),
                param: String::new(),
                value: UpdateValue::None,
            }],
            add_tags: vec![],
        };
        assert_eq!(spec.set_clause_columns(), "last_activated = time::now()");
        assert!(spec.fields[0].param.is_empty());
        assert!(matches!(spec.fields[0].value, UpdateValue::None));
    }
}
