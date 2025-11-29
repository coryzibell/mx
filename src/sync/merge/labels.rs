//! Label merge logic - union semantics
//!
//! Merges labels from local and remote, preserving additions from both sides
//! and removing deletions from both sides.
//!
//! Formula: `merged = base ∪ (local - base) ∪ (remote - base) - (base - local) - (base - remote)`
//! Simplified: Keep all additions from both, remove deletions from both.

use std::collections::HashSet;

/// Merge labels using union semantics
///
/// - Preserves additions from both local and remote
/// - Respects deletions from both local and remote
/// - Base is the common ancestor (last_synced state)
pub fn merge_labels(local: &[String], remote: &[String], base: &[String]) -> Vec<String> {
    let local_set: HashSet<_> = local.iter().collect();
    let remote_set: HashSet<_> = remote.iter().collect();
    let base_set: HashSet<_> = base.iter().collect();

    // Additions: labels added by local or remote (not in base)
    let local_additions: HashSet<_> = local_set.difference(&base_set).copied().collect();
    let remote_additions: HashSet<_> = remote_set.difference(&base_set).copied().collect();

    // Deletions: labels removed by local or remote (in base but not in local/remote)
    let local_deletions: HashSet<_> = base_set.difference(&local_set).copied().collect();
    let remote_deletions: HashSet<_> = base_set.difference(&remote_set).copied().collect();

    // Start with base, add all additions, remove all deletions
    let mut result: HashSet<_> = base_set;
    result.extend(local_additions);
    result.extend(remote_additions);
    for deletion in local_deletions.union(&remote_deletions) {
        result.remove(deletion);
    }

    // Return sorted for consistency
    let mut labels: Vec<_> = result.into_iter().cloned().collect();
    labels.sort();
    labels
}

/// Check if labels have diverged (both local and remote changed from base)
pub fn labels_diverged(local: &[String], remote: &[String], base: &[String]) -> bool {
    let local_set: HashSet<_> = local.iter().collect();
    let remote_set: HashSet<_> = remote.iter().collect();
    let base_set: HashSet<_> = base.iter().collect();

    let local_changed = local_set != base_set;
    let remote_changed = remote_set != base_set;

    local_changed && remote_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn test_no_changes() {
        let base = labels(&["bug", "enhancement"]);
        let result = merge_labels(&base, &base, &base);
        assert_eq!(result, labels(&["bug", "enhancement"]));
    }

    #[test]
    fn test_local_addition() {
        let base = labels(&["bug"]);
        let local = labels(&["bug", "new-label"]);
        let remote = labels(&["bug"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug", "new-label"]));
    }

    #[test]
    fn test_remote_addition() {
        let base = labels(&["bug"]);
        let local = labels(&["bug"]);
        let remote = labels(&["bug", "remote-label"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug", "remote-label"]));
    }

    #[test]
    fn test_both_add_different() {
        let base = labels(&["bug"]);
        let local = labels(&["bug", "local-label"]);
        let remote = labels(&["bug", "remote-label"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug", "local-label", "remote-label"]));
    }

    #[test]
    fn test_both_add_same() {
        let base = labels(&["bug"]);
        let local = labels(&["bug", "new-label"]);
        let remote = labels(&["bug", "new-label"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug", "new-label"]));
    }

    #[test]
    fn test_local_deletion() {
        let base = labels(&["bug", "to-remove"]);
        let local = labels(&["bug"]);
        let remote = labels(&["bug", "to-remove"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug"]));
    }

    #[test]
    fn test_remote_deletion() {
        let base = labels(&["bug", "to-remove"]);
        let local = labels(&["bug", "to-remove"]);
        let remote = labels(&["bug"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug"]));
    }

    #[test]
    fn test_both_delete_same() {
        let base = labels(&["bug", "to-remove"]);
        let local = labels(&["bug"]);
        let remote = labels(&["bug"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["bug"]));
    }

    #[test]
    fn test_add_and_delete_different() {
        let base = labels(&["bug", "to-remove"]);
        let local = labels(&["bug", "local-add"]);
        let remote = labels(&["bug", "to-remove", "remote-add"]);

        let result = merge_labels(&local, &remote, &base);
        // local added local-add, local deleted to-remove
        // remote added remote-add, remote kept to-remove
        // Result: base + local-add + remote-add - to-remove
        assert_eq!(result, labels(&["bug", "local-add", "remote-add"]));
    }

    #[test]
    fn test_empty_base() {
        let base = labels(&[]);
        let local = labels(&["a"]);
        let remote = labels(&["b"]);

        let result = merge_labels(&local, &remote, &base);
        assert_eq!(result, labels(&["a", "b"]));
    }

    #[test]
    fn test_labels_diverged() {
        let base = labels(&["bug"]);
        let local = labels(&["bug", "local"]);
        let remote = labels(&["bug", "remote"]);

        assert!(labels_diverged(&local, &remote, &base));
    }

    #[test]
    fn test_labels_not_diverged_local_only() {
        let base = labels(&["bug"]);
        let local = labels(&["bug", "local"]);

        assert!(!labels_diverged(&local, &base, &base));
    }

    #[test]
    fn test_labels_not_diverged_remote_only() {
        let base = labels(&["bug"]);
        let remote = labels(&["bug", "remote"]);

        assert!(!labels_diverged(&base, &remote, &base));
    }
}
