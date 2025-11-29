//! Diff engine - compute field changes between local/remote/base
//!
//! Implements three-way merge detection for sync conflicts.

/// Result of comparing a field across local, remote, and base
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldChange<T> {
    /// No changes - all three match
    Unchanged,

    /// Only local changed (remote matches base)
    LocalOnly(T),

    /// Only remote changed (local matches base)
    RemoteOnly(T),

    /// Both changed to the same value
    BothSame(T),

    /// Conflict - both changed to different values
    Conflict { local: T, remote: T, base: T },
}

impl<T: Clone + PartialEq> FieldChange<T> {
    /// Compute the change type for a field
    pub fn compute(local: &T, remote: &T, base: &T) -> Self {
        let local_changed = local != base;
        let remote_changed = remote != base;

        match (local_changed, remote_changed) {
            (false, false) => FieldChange::Unchanged,
            (true, false) => FieldChange::LocalOnly(local.clone()),
            (false, true) => FieldChange::RemoteOnly(remote.clone()),
            (true, true) if local == remote => FieldChange::BothSame(local.clone()),
            (true, true) => FieldChange::Conflict {
                local: local.clone(),
                remote: remote.clone(),
                base: base.clone(),
            },
        }
    }

    /// Check if this represents a conflict
    pub fn is_conflict(&self) -> bool {
        matches!(self, FieldChange::Conflict { .. })
    }

    /// Check if any change occurred
    pub fn has_changes(&self) -> bool {
        !matches!(self, FieldChange::Unchanged)
    }

    /// Get the resolved value (if not a conflict)
    pub fn resolved_value(&self) -> Option<&T> {
        match self {
            FieldChange::Unchanged => None,
            FieldChange::LocalOnly(v) => Some(v),
            FieldChange::RemoteOnly(v) => Some(v),
            FieldChange::BothSame(v) => Some(v),
            FieldChange::Conflict { .. } => None,
        }
    }
}

/// Collection of field changes for an issue/discussion
#[derive(Debug, Default)]
pub struct DiffResult {
    pub title: Option<FieldChange<String>>,
    pub body: Option<FieldChange<String>>,
    pub labels: Option<FieldChange<Vec<String>>>,
    pub assignees: Option<FieldChange<Vec<String>>>,
}

impl DiffResult {
    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        self.title.as_ref().is_some_and(|c| c.has_changes())
            || self.body.as_ref().is_some_and(|c| c.has_changes())
            || self.labels.as_ref().is_some_and(|c| c.has_changes())
            || self.assignees.as_ref().is_some_and(|c| c.has_changes())
    }

    /// Check if there are any conflicts
    pub fn has_conflicts(&self) -> bool {
        self.title.as_ref().is_some_and(|c| c.is_conflict())
            || self.body.as_ref().is_some_and(|c| c.is_conflict())
            || self.labels.as_ref().is_some_and(|c| c.is_conflict())
            || self.assignees.as_ref().is_some_and(|c| c.is_conflict())
    }

    /// Get a summary of changes for display
    pub fn summary(&self) -> Vec<String> {
        let mut changes = Vec::new();

        if let Some(ref c) = self.title {
            if c.has_changes() {
                changes.push(format!("title: {}", change_type_str(c)));
            }
        }
        if let Some(ref c) = self.body {
            if c.has_changes() {
                changes.push(format!("body: {}", change_type_str(c)));
            }
        }
        if let Some(ref c) = self.labels {
            if c.has_changes() {
                changes.push(format!("labels: {}", change_type_str(c)));
            }
        }
        if let Some(ref c) = self.assignees {
            if c.has_changes() {
                changes.push(format!("assignees: {}", change_type_str(c)));
            }
        }

        changes
    }
}

fn change_type_str<T>(change: &FieldChange<T>) -> &'static str {
    match change {
        FieldChange::Unchanged => "unchanged",
        FieldChange::LocalOnly(_) => "local only",
        FieldChange::RemoteOnly(_) => "remote only",
        FieldChange::BothSame(_) => "both same",
        FieldChange::Conflict { .. } => "CONFLICT",
    }
}

/// Compute diff between local YAML, remote GitHub, and base (last_synced) states
#[allow(clippy::too_many_arguments)]
pub fn compute_diff(
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
) -> DiffResult {
    DiffResult {
        title: Some(FieldChange::compute(
            &local_title.to_string(),
            &remote_title.to_string(),
            &base_title.to_string(),
        )),
        body: Some(FieldChange::compute(
            &local_body.to_string(),
            &remote_body.to_string(),
            &base_body.to_string(),
        )),
        labels: Some(FieldChange::compute(
            &local_labels.to_vec(),
            &remote_labels.to_vec(),
            &base_labels.to_vec(),
        )),
        assignees: Some(FieldChange::compute(
            &local_assignees.to_vec(),
            &remote_assignees.to_vec(),
            &base_assignees.to_vec(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unchanged() {
        let change: FieldChange<String> = FieldChange::compute(
            &"same".to_string(),
            &"same".to_string(),
            &"same".to_string(),
        );
        assert_eq!(change, FieldChange::Unchanged);
        assert!(!change.has_changes());
        assert!(!change.is_conflict());
    }

    #[test]
    fn test_local_only() {
        let change = FieldChange::compute(
            &"changed".to_string(),
            &"original".to_string(),
            &"original".to_string(),
        );
        assert!(matches!(change, FieldChange::LocalOnly(_)));
        assert!(change.has_changes());
        assert!(!change.is_conflict());
        assert_eq!(change.resolved_value(), Some(&"changed".to_string()));
    }

    #[test]
    fn test_remote_only() {
        let change = FieldChange::compute(
            &"original".to_string(),
            &"changed".to_string(),
            &"original".to_string(),
        );
        assert!(matches!(change, FieldChange::RemoteOnly(_)));
        assert!(change.has_changes());
        assert!(!change.is_conflict());
        assert_eq!(change.resolved_value(), Some(&"changed".to_string()));
    }

    #[test]
    fn test_both_same() {
        let change = FieldChange::compute(
            &"new value".to_string(),
            &"new value".to_string(),
            &"original".to_string(),
        );
        assert!(matches!(change, FieldChange::BothSame(_)));
        assert!(change.has_changes());
        assert!(!change.is_conflict());
        assert_eq!(change.resolved_value(), Some(&"new value".to_string()));
    }

    #[test]
    fn test_conflict() {
        let change = FieldChange::compute(
            &"local change".to_string(),
            &"remote change".to_string(),
            &"original".to_string(),
        );
        assert!(change.is_conflict());
        assert!(change.has_changes());
        assert_eq!(change.resolved_value(), None);

        if let FieldChange::Conflict {
            local,
            remote,
            base,
        } = change
        {
            assert_eq!(local, "local change");
            assert_eq!(remote, "remote change");
            assert_eq!(base, "original");
        } else {
            panic!("Expected conflict");
        }
    }

    #[test]
    fn test_labels_diff() {
        let local = vec!["bug".to_string(), "new-label".to_string()];
        let remote = vec!["bug".to_string()];
        let base = vec!["bug".to_string()];

        let change = FieldChange::compute(&local, &remote, &base);
        assert!(matches!(change, FieldChange::LocalOnly(_)));
    }

    #[test]
    fn test_compute_diff() {
        let result = compute_diff(
            "Updated Title",
            "body",
            &["bug".to_string()],
            &[],
            "Original Title",
            "body",
            &["bug".to_string()],
            &[],
            "Original Title",
            "body",
            &["bug".to_string()],
            &[],
        );

        assert!(result.has_changes());
        assert!(!result.has_conflicts());
        assert!(result.title.as_ref().unwrap().has_changes());
        assert!(!result.body.as_ref().unwrap().has_changes());
    }

    #[test]
    fn test_diff_result_summary() {
        let result = DiffResult {
            title: Some(FieldChange::LocalOnly("new title".to_string())),
            body: Some(FieldChange::Unchanged),
            labels: Some(FieldChange::Conflict {
                local: vec!["a".to_string()],
                remote: vec!["b".to_string()],
                base: vec![],
            }),
            assignees: None,
        };

        let summary = result.summary();
        assert_eq!(summary.len(), 2);
        assert!(summary[0].contains("title"));
        assert!(summary[1].contains("CONFLICT"));
    }
}
