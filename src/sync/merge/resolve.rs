//! Conflict resolution - auto and interactive modes
//!
//! Resolves three-way merge conflicts, with options for automatic resolution
//! or prompting the user.

use super::diff::FieldChange;
use super::labels::merge_labels;

/// Resolution strategy for conflicts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Use the local value
    Local,
    /// Use the remote value
    Remote,
    /// Skip this item entirely
    Skip,
}

/// Merged result for an issue/discussion
#[derive(Debug, Clone)]
pub struct MergedState {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

/// Auto-resolve a field change (no user interaction)
///
/// Returns the resolved value, or None if it's a conflict requiring user input.
pub fn auto_resolve_field<T: Clone>(change: &FieldChange<T>) -> Option<T> {
    match change {
        FieldChange::Unchanged => None, // No change needed
        FieldChange::LocalOnly(v) => Some(v.clone()),
        FieldChange::RemoteOnly(v) => Some(v.clone()),
        FieldChange::BothSame(v) => Some(v.clone()),
        FieldChange::Conflict { .. } => None, // Can't auto-resolve
    }
}

/// Resolve a field conflict with a given strategy
pub fn resolve_conflict<T: Clone>(change: &FieldChange<T>, strategy: Resolution) -> Option<T> {
    match change {
        FieldChange::Conflict { local, remote, .. } => match strategy {
            Resolution::Local => Some(local.clone()),
            Resolution::Remote => Some(remote.clone()),
            Resolution::Skip => None,
        },
        _ => auto_resolve_field(change),
    }
}

/// Merge all fields, auto-resolving where possible
///
/// For conflicts:
/// - Labels use union merge (auto-resolved)
/// - Title/body require explicit resolution if conflicted
///
/// Returns: (merged_state, has_unresolved_conflicts)
#[allow(clippy::too_many_arguments)]
pub fn merge_fields(
    local_title: &str,
    local_body: &str,
    local_labels: &[String],
    local_assignees: &[String],
    remote_title: &str,
    remote_body: &str,
    remote_labels: &[String],
    remote_assignees: &[String],
    base_title: &str,
    base_body: &str,
    base_labels: &[String],
    base_assignees: &[String],
    prefer_local: bool,
) -> (MergedState, bool) {
    let title_change = FieldChange::compute(
        &local_title.to_string(),
        &remote_title.to_string(),
        &base_title.to_string(),
    );

    let body_change = FieldChange::compute(
        &local_body.to_string(),
        &remote_body.to_string(),
        &base_body.to_string(),
    );

    let assignees_change = FieldChange::compute(
        &local_assignees.to_vec(),
        &remote_assignees.to_vec(),
        &base_assignees.to_vec(),
    );

    // Labels always use union merge (no conflicts)
    let merged_labels = merge_labels(local_labels, remote_labels, base_labels);

    // Resolve title
    let (merged_title, title_conflict) = resolve_string_field(
        &title_change,
        local_title,
        remote_title,
        base_title,
        prefer_local,
    );

    // Resolve body
    let (merged_body, body_conflict) = resolve_string_field(
        &body_change,
        local_body,
        remote_body,
        base_body,
        prefer_local,
    );

    // Resolve assignees (same logic as labels - use union)
    let merged_assignees = match &assignees_change {
        FieldChange::Unchanged => base_assignees.to_vec(),
        FieldChange::LocalOnly(v) | FieldChange::RemoteOnly(v) | FieldChange::BothSame(v) => {
            v.clone()
        }
        FieldChange::Conflict {
            local,
            remote,
            base,
        } => merge_labels(local, remote, base), // Reuse label merge logic
    };

    let has_conflicts = title_conflict || body_conflict;

    (
        MergedState {
            title: merged_title,
            body: merged_body,
            labels: merged_labels,
            assignees: merged_assignees,
        },
        has_conflicts,
    )
}

fn resolve_string_field(
    change: &FieldChange<String>,
    local: &str,
    remote: &str,
    base: &str,
    prefer_local: bool,
) -> (String, bool) {
    match change {
        FieldChange::Unchanged => (base.to_string(), false),
        FieldChange::LocalOnly(v) => (v.clone(), false),
        FieldChange::RemoteOnly(v) => (v.clone(), false),
        FieldChange::BothSame(v) => (v.clone(), false),
        FieldChange::Conflict { .. } => {
            // Auto-resolve based on preference
            if prefer_local {
                (local.to_string(), false)
            } else {
                (remote.to_string(), false)
            }
        }
    }
}

/// Check if we should update GitHub based on merged state
pub fn should_update(merged: &MergedState, remote_title: &str, remote_body: &str) -> bool {
    merged.title != remote_title || merged.body != remote_body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_resolve_unchanged() {
        let change: FieldChange<String> = FieldChange::Unchanged;
        assert_eq!(auto_resolve_field(&change), None);
    }

    #[test]
    fn test_auto_resolve_local_only() {
        let change = FieldChange::LocalOnly("local".to_string());
        assert_eq!(auto_resolve_field(&change), Some("local".to_string()));
    }

    #[test]
    fn test_auto_resolve_remote_only() {
        let change = FieldChange::RemoteOnly("remote".to_string());
        assert_eq!(auto_resolve_field(&change), Some("remote".to_string()));
    }

    #[test]
    fn test_auto_resolve_conflict_fails() {
        let change = FieldChange::Conflict {
            local: "local".to_string(),
            remote: "remote".to_string(),
            base: "base".to_string(),
        };
        assert_eq!(auto_resolve_field(&change), None);
    }

    #[test]
    fn test_resolve_conflict_local() {
        let change = FieldChange::Conflict {
            local: "local".to_string(),
            remote: "remote".to_string(),
            base: "base".to_string(),
        };
        assert_eq!(
            resolve_conflict(&change, Resolution::Local),
            Some("local".to_string())
        );
    }

    #[test]
    fn test_resolve_conflict_remote() {
        let change = FieldChange::Conflict {
            local: "local".to_string(),
            remote: "remote".to_string(),
            base: "base".to_string(),
        };
        assert_eq!(
            resolve_conflict(&change, Resolution::Remote),
            Some("remote".to_string())
        );
    }

    #[test]
    fn test_merge_fields_no_changes() {
        let (merged, has_conflicts) = merge_fields(
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            false,
        );

        assert!(!has_conflicts);
        assert_eq!(merged.title, "Title");
        assert_eq!(merged.body, "Body");
        assert_eq!(merged.labels, vec!["bug"]);
    }

    #[test]
    fn test_merge_fields_local_changes() {
        let (merged, has_conflicts) = merge_fields(
            "New Title",
            "New Body",
            &["bug".to_string(), "new".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            false,
        );

        assert!(!has_conflicts);
        assert_eq!(merged.title, "New Title");
        assert_eq!(merged.body, "New Body");
        assert_eq!(merged.labels, vec!["bug", "new"]);
    }

    #[test]
    fn test_merge_fields_remote_changes() {
        let (merged, has_conflicts) = merge_fields(
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            "Remote Title",
            "Remote Body",
            &["bug".to_string(), "remote".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            false,
        );

        assert!(!has_conflicts);
        assert_eq!(merged.title, "Remote Title");
        assert_eq!(merged.body, "Remote Body");
        assert_eq!(merged.labels, vec!["bug", "remote"]);
    }

    #[test]
    fn test_merge_fields_labels_auto_merge() {
        // Even with label conflicts, they auto-merge via union
        let (merged, has_conflicts) = merge_fields(
            "Title",
            "Body",
            &["bug".to_string(), "local".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string(), "remote".to_string()],
            &[],
            "Title",
            "Body",
            &["bug".to_string()],
            &[],
            false,
        );

        assert!(!has_conflicts);
        assert_eq!(merged.labels, vec!["bug", "local", "remote"]);
    }

    #[test]
    fn test_merge_fields_title_conflict_prefer_local() {
        let (merged, _has_conflicts) = merge_fields(
            "Local Title",
            "Body",
            &[],
            &[],
            "Remote Title",
            "Body",
            &[],
            &[],
            "Base Title",
            "Body",
            &[],
            &[],
            true, // prefer_local
        );

        assert_eq!(merged.title, "Local Title");
    }

    #[test]
    fn test_merge_fields_title_conflict_prefer_remote() {
        let (merged, _has_conflicts) = merge_fields(
            "Local Title",
            "Body",
            &[],
            &[],
            "Remote Title",
            "Body",
            &[],
            &[],
            "Base Title",
            "Body",
            &[],
            &[],
            false, // prefer_remote
        );

        assert_eq!(merged.title, "Remote Title");
    }
}
