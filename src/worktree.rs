//! Git worktree lifecycle management for Claude Code agent dispatches.
//!
//! When Claude Code dispatches an agent with `isolation: "worktree"`, it creates
//! a locked git worktree under `{repo}/.claude/worktrees/agent-{id}/`. If the
//! agent crashes or the session dies, these worktrees are never cleaned up --
//! the lock file prevents `git worktree prune` from touching them.
//!
//! `mx worktree sweep` detects stale worktrees (dead PID), verifies merge
//! status via `gh`, and cleans them up.

use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single worktree discovered by `git worktree list --porcelain`.
#[derive(Debug)]
struct Worktree {
    /// Absolute path to the worktree working directory.
    path: PathBuf,
    /// The worktree name (e.g. `agent-a12bcb68e8df9996a`).
    name: String,
    /// The branch checked out in this worktree (e.g. `refs/heads/feat/foo`).
    branch: Option<String>,
    /// Whether the worktree is locked (has a lock file).
    locked: bool,
    /// Raw lock reason from the lock file, if any.
    lock_reason: Option<String>,
}

/// What happened to a single worktree during sweep.
#[derive(Debug)]
enum SweepAction {
    /// Cleaned up successfully.
    Swept {
        name: String,
        branch: String,
        reason: String,
    },
    /// Skipped (ambiguous, dirty, etc).
    Skipped {
        name: String,
        branch: String,
        reason: String,
    },
    /// Agent is still alive.
    Alive {
        name: String,
        branch: String,
        pid: u32,
    },
}

/// Aggregate result of sweeping one repo.
#[derive(Debug)]
struct RepoSweepResult {
    repo_path: PathBuf,
    actions: Vec<SweepAction>,
}

impl RepoSweepResult {
    fn swept_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, SweepAction::Swept { .. }))
            .count()
    }

    fn skipped_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, SweepAction::Skipped { .. }))
            .count()
    }

    fn alive_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, SweepAction::Alive { .. }))
            .count()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse `git worktree list --porcelain` output into structured data.
fn parse_worktree_list(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut current_locked = false;
    let mut current_lock_reason: Option<String> = None;

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            // Flush previous
            if let Some(p) = current_path.take() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                worktrees.push(Worktree {
                    path: p,
                    name,
                    branch: current_branch.take(),
                    locked: current_locked,
                    lock_reason: current_lock_reason.take(),
                });
                current_locked = false;
            }
            current_path = Some(PathBuf::from(path));
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current_branch = Some(branch.to_string());
        } else if line == "locked" {
            current_locked = true;
        } else if let Some(reason) = line.strip_prefix("locked ") {
            current_locked = true;
            current_lock_reason = Some(reason.to_string());
        }
    }

    // Flush last
    if let Some(p) = current_path.take() {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        worktrees.push(Worktree {
            path: p,
            name,
            branch: current_branch.take(),
            locked: current_locked,
            lock_reason: current_lock_reason.take(),
        });
    }

    worktrees
}

/// Parse PID from a lock reason string.
/// Expected format: `claude agent agent-{id} (pid NNNN)`
fn parse_pid_from_lock(reason: &str) -> Option<u32> {
    let re = Regex::new(r"\(pid (\d+)\)").ok()?;
    let caps = re.captures(reason)?;
    caps.get(1)?.as_str().parse().ok()
}

/// Check if a PID is alive by probing /proc/{pid}.
fn pid_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{}", pid)).exists()
}

/// Extract the short branch name from a full ref.
/// `refs/heads/feat/foo` -> `feat/foo`
fn short_branch(refname: &str) -> &str {
    refname
        .strip_prefix("refs/heads/")
        .unwrap_or(refname)
}

/// Parse owner/repo from a git remote URL.
/// Handles both SSH (`git@github.com:owner/repo.git`) and HTTPS
/// (`https://github.com/owner/repo.git`) formats.
fn parse_remote_owner_repo(url: &str) -> Option<String> {
    let url = url.trim();

    // SSH format: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let slug = rest.strip_suffix(".git").unwrap_or(rest);
        return Some(slug.to_string());
    }

    // HTTPS format: https://github.com/owner/repo.git
    if url.contains("github.com/") {
        let idx = url.find("github.com/")? + "github.com/".len();
        let slug = &url[idx..];
        let slug = slug.strip_suffix(".git").unwrap_or(slug);
        return Some(slug.to_string());
    }

    None
}

/// Get the owner/repo slug for the repo at `repo_path`.
fn get_remote_slug(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .context("Failed to run git remote get-url origin")?;

    if !output.status.success() {
        bail!(
            "git remote get-url origin failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_remote_owner_repo(&url)
        .ok_or_else(|| anyhow::anyhow!("Could not parse owner/repo from remote URL: {}", url))
}

// ---------------------------------------------------------------------------
// Merge / staleness checks
// ---------------------------------------------------------------------------

/// Check if a branch has a merged PR via `gh`.
/// Returns Some(pr_number) if merged, None otherwise.
fn check_merged_pr(branch: &str, remote_slug: &str) -> Option<u32> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "merged",
            "--head",
            branch,
            "--repo",
            remote_slug,
            "--limit",
            "1",
            "--json",
            "number",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).ok()?;

    let arr = json.as_array()?;
    if arr.is_empty() {
        return None;
    }
    arr[0]["number"].as_u64().map(|n| n as u32)
}

/// Check if a branch still exists on the remote.
fn branch_exists_on_remote(branch: &str, repo_path: &Path) -> Option<bool> {
    let output = Command::new("git")
        .args(["ls-remote", "--heads", "origin", branch])
        .current_dir(repo_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(!stdout.trim().is_empty())
}

/// Check if a worktree directory has uncommitted changes.
fn worktree_is_dirty(worktree_path: &Path) -> bool {
    if !worktree_path.exists() {
        return false;
    }

    let output = Command::new("git")
        .args(["-C", &worktree_path.to_string_lossy(), "status", "--porcelain"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            !String::from_utf8_lossy(&o.stdout).trim().is_empty()
        }
        _ => false, // Can't tell; assume clean
    }
}

// ---------------------------------------------------------------------------
// Cleanup actions
// ---------------------------------------------------------------------------

/// Clean up a single worktree: unlock, remove, delete branches.
fn clean_worktree(
    repo_path: &Path,
    wt: &Worktree,
    dry_run: bool,
) -> Result<()> {
    let branch = wt
        .branch
        .as_deref()
        .map(short_branch)
        .unwrap_or("(detached)");

    if dry_run {
        return Ok(());
    }

    // 1. Unlock
    let _ = Command::new("git")
        .args(["worktree", "unlock", &wt.name])
        .current_dir(repo_path)
        .output();

    // 2. Remove worktree (--force in case directory is already gone)
    let _ = Command::new("git")
        .args([
            "worktree",
            "remove",
            &wt.path.to_string_lossy(),
            "--force",
        ])
        .current_dir(repo_path)
        .output();

    // 3. Delete the feature branch
    if branch != "(detached)" {
        let _ = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(repo_path)
            .output();
    }

    // 4. Delete the snapshot branch (worktree-agent-{id})
    let snapshot_branch = format!("worktree-{}", wt.name);
    let _ = Command::new("git")
        .args(["branch", "-D", &snapshot_branch])
        .current_dir(repo_path)
        .output();

    Ok(())
}

// ---------------------------------------------------------------------------
// Per-repo sweep
// ---------------------------------------------------------------------------

/// Run `git worktree prune` to clean unlocked worktrees with missing dirs.
fn prune_worktrees(repo_path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_path)
        .output()
        .context("Failed to run git worktree prune")?;

    if !output.status.success() {
        eprintln!(
            "Warning: git worktree prune failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Sweep a single repo. Returns a result describing what happened.
fn sweep_repo(repo_path: &Path, dry_run: bool, force: bool) -> Result<RepoSweepResult> {
    // Step 0: prune first
    if !dry_run {
        prune_worktrees(repo_path)?;
    }

    // Step 1: list worktrees
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .context("Failed to run git worktree list")?;

    if !output.status.success() {
        bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let porcelain = String::from_utf8_lossy(&output.stdout).to_string();
    let worktrees = parse_worktree_list(&porcelain);

    // Get the main worktree path (first entry is always the main one)
    let main_path = worktrees
        .first()
        .map(|w| w.path.clone())
        .unwrap_or_else(|| repo_path.to_path_buf());

    // Get remote slug for gh calls (best-effort)
    let remote_slug = get_remote_slug(repo_path).ok();

    let mut actions = Vec::new();

    // Step 2: process each non-main worktree
    for wt in worktrees.iter().skip(1) {
        let branch = wt
            .branch
            .as_deref()
            .map(short_branch)
            .unwrap_or("(detached)");

        // 2a. Parse PID from lock reason
        let pid = wt
            .lock_reason
            .as_deref()
            .and_then(parse_pid_from_lock);

        // If we can't parse a PID and there's no lock, skip (not ours)
        if pid.is_none() && !wt.locked {
            // Not a Claude worktree, skip silently
            continue;
        }

        // If PID is absent from the lock reason but worktree is locked,
        // it might be a manually locked worktree. Skip unless forced.
        let pid = match pid {
            Some(p) => p,
            None => {
                if force {
                    0 // Treat as dead PID to force cleanup
                } else {
                    actions.push(SweepAction::Skipped {
                        name: wt.name.clone(),
                        branch: branch.to_string(),
                        reason: "locked without parseable PID".to_string(),
                    });
                    continue;
                }
            }
        };

        // 2b. If PID is alive, skip
        if pid > 0 && pid_is_alive(pid) {
            actions.push(SweepAction::Alive {
                name: wt.name.clone(),
                branch: branch.to_string(),
                pid,
            });
            continue;
        }

        // 2c. PID is dead. Check for uncommitted changes.
        if worktree_is_dirty(&wt.path) && !force {
            actions.push(SweepAction::Skipped {
                name: wt.name.clone(),
                branch: branch.to_string(),
                reason: "uncommitted changes (use --force to override)".to_string(),
            });
            continue;
        }

        // 2d. Check merge status
        let sweep_reason = if let Some(ref slug) = remote_slug {
            if let Some(pr_num) = check_merged_pr(branch, slug) {
                format!("PR #{} merged", pr_num)
            } else {
                // No merged PR -- check if branch is gone from remote
                match branch_exists_on_remote(branch, repo_path) {
                    Some(false) => "branch deleted from remote".to_string(),
                    Some(true) => {
                        // Branch still on remote with no merged PR
                        if force {
                            "forced (branch on remote, no merged PR)".to_string()
                        } else {
                            actions.push(SweepAction::Skipped {
                                name: wt.name.clone(),
                                branch: branch.to_string(),
                                reason: "branch still on remote with no merged PR (use --force to override)".to_string(),
                            });
                            continue;
                        }
                    }
                    None => {
                        // Network failure -- fall back to PID-only staleness
                        "stale (PID dead, remote check failed)".to_string()
                    }
                }
            }
        } else {
            // No remote slug available -- fall back to PID-only staleness
            "stale (PID dead, no remote configured)".to_string()
        };

        // 2e. Clean!
        if let Err(e) = clean_worktree(&main_path, wt, dry_run) {
            eprintln!("Warning: cleanup of {} failed: {}", wt.name, e);
            actions.push(SweepAction::Skipped {
                name: wt.name.clone(),
                branch: branch.to_string(),
                reason: format!("cleanup failed: {}", e),
            });
            continue;
        }

        actions.push(SweepAction::Swept {
            name: wt.name.clone(),
            branch: branch.to_string(),
            reason: sweep_reason,
        });
    }

    Ok(RepoSweepResult {
        repo_path: repo_path.to_path_buf(),
        actions,
    })
}

// ---------------------------------------------------------------------------
// Repo discovery (for --all)
// ---------------------------------------------------------------------------

/// Find all git repos under ~/recipes/ up to depth 3.
fn discover_repos() -> Vec<PathBuf> {
    let recipes_dir = dirs::home_dir()
        .expect("Could not determine home directory")
        .join("recipes");

    if !recipes_dir.is_dir() {
        return Vec::new();
    }

    let mut repos = Vec::new();
    walk_for_repos(&recipes_dir, 0, 3, &mut repos);
    repos.sort();
    repos
}

fn walk_for_repos(dir: &Path, depth: usize, max_depth: usize, repos: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }

    let git_dir = dir.join(".git");
    if git_dir.exists() {
        repos.push(dir.to_path_buf());
        return; // Don't descend into sub-repos
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_for_repos(&path, depth + 1, max_depth, repos);
        }
    }
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn print_repo_result(result: &RepoSweepResult, dry_run: bool) {
    println!("mx worktree sweep -- {}", result.repo_path.display());

    if result.actions.is_empty() {
        println!("  No worktrees found.");
        return;
    }

    for action in &result.actions {
        match action {
            SweepAction::Swept {
                name,
                branch,
                reason,
            } => {
                let label = if dry_run { "WOULD SWEEP" } else { "SWEPT" };
                println!(
                    "  [{:<12}] {:<26} {:<34} {}",
                    label, name, branch, reason
                );
            }
            SweepAction::Skipped {
                name,
                branch,
                reason,
            } => {
                let label = if dry_run { "WOULD SKIP" } else { "SKIPPED" };
                println!(
                    "  [{:<12}] {:<26} {:<34} {}",
                    label, name, branch, reason
                );
            }
            SweepAction::Alive {
                name,
                branch,
                pid,
            } => {
                println!(
                    "  [{:<12}] {:<26} {:<34} pid {} alive",
                    "ALIVE", name, branch, pid
                );
            }
        }
    }

    let swept = result.swept_count();
    let skipped = result.skipped_count();
    let alive = result.alive_count();
    println!("  {} swept, {} skipped, {} alive", swept, skipped, alive);
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run `mx worktree sweep`.
///
/// Returns an exit code: 0 = success, 1 = error, 2 = ambiguous worktrees skipped.
pub fn run_sweep(all: bool, dry_run: bool, force: bool) -> Result<i32> {
    let repos = if all {
        discover_repos()
    } else {
        // Current repo: find the git root
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to determine git root (are you in a git repo?)")?;

        if !output.status.success() {
            bail!(
                "Not in a git repository: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        vec![PathBuf::from(root)]
    };

    if repos.is_empty() {
        if all {
            println!("No repos found under ~/recipes/.");
        }
        return Ok(0);
    }

    let mut any_skipped = false;
    let mut first = true;

    for repo in &repos {
        if !first {
            println!();
        }
        first = false;

        match sweep_repo(repo, dry_run, force) {
            Ok(result) => {
                if result.skipped_count() > 0 {
                    any_skipped = true;
                }
                print_repo_result(&result, dry_run);
            }
            Err(e) => {
                eprintln!("Error sweeping {}: {}", repo.display(), e);
            }
        }
    }

    if any_skipped {
        Ok(2)
    } else {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pid_from_lock_standard() {
        assert_eq!(
            parse_pid_from_lock("claude agent agent-a12bcb68e8df9996a (pid 1574839)"),
            Some(1574839)
        );
    }

    #[test]
    fn test_parse_pid_from_lock_different_id() {
        assert_eq!(
            parse_pid_from_lock("claude agent agent-abc123 (pid 42)"),
            Some(42)
        );
    }

    #[test]
    fn test_parse_pid_from_lock_no_pid() {
        assert_eq!(parse_pid_from_lock("manually locked"), None);
    }

    #[test]
    fn test_parse_pid_from_lock_empty() {
        assert_eq!(parse_pid_from_lock(""), None);
    }

    #[test]
    fn test_parse_remote_owner_repo_ssh() {
        assert_eq!(
            parse_remote_owner_repo("git@github.com:coryzibell/mx.git"),
            Some("coryzibell/mx".to_string())
        );
    }

    #[test]
    fn test_parse_remote_owner_repo_ssh_no_git_suffix() {
        assert_eq!(
            parse_remote_owner_repo("git@github.com:coryzibell/mx"),
            Some("coryzibell/mx".to_string())
        );
    }

    #[test]
    fn test_parse_remote_owner_repo_https() {
        assert_eq!(
            parse_remote_owner_repo("https://github.com/coryzibell/mx.git"),
            Some("coryzibell/mx".to_string())
        );
    }

    #[test]
    fn test_parse_remote_owner_repo_https_no_git_suffix() {
        assert_eq!(
            parse_remote_owner_repo("https://github.com/coryzibell/mx"),
            Some("coryzibell/mx".to_string())
        );
    }

    #[test]
    fn test_parse_remote_owner_repo_not_github() {
        assert_eq!(
            parse_remote_owner_repo("git@gitlab.com:owner/repo.git"),
            None
        );
    }

    #[test]
    fn test_short_branch_strips_prefix() {
        assert_eq!(short_branch("refs/heads/feat/foo"), "feat/foo");
    }

    #[test]
    fn test_short_branch_no_prefix() {
        assert_eq!(short_branch("main"), "main");
    }

    #[test]
    fn test_short_branch_nested() {
        assert_eq!(
            short_branch("refs/heads/chore/close-docs-audit"),
            "chore/close-docs-audit"
        );
    }

    #[test]
    fn test_parse_worktree_list_empty() {
        assert!(parse_worktree_list("").is_empty());
    }

    #[test]
    fn test_parse_worktree_list_main_only() {
        let input = "\
worktree /home/charlie/recipes/coryzibell/mx
HEAD abc123
branch refs/heads/main
";
        let wts = parse_worktree_list(input);
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].name, "mx");
        assert_eq!(
            wts[0].branch.as_deref(),
            Some("refs/heads/main")
        );
        assert!(!wts[0].locked);
    }

    #[test]
    fn test_parse_worktree_list_with_locked() {
        let input = "\
worktree /home/charlie/recipes/coryzibell/mx
HEAD abc123
branch refs/heads/main

worktree /home/charlie/recipes/coryzibell/mx/.claude/worktrees/agent-abc
HEAD def456
branch refs/heads/feat/foo
locked claude agent agent-abc (pid 12345)
";
        let wts = parse_worktree_list(input);
        assert_eq!(wts.len(), 2);

        assert_eq!(wts[1].name, "agent-abc");
        assert!(wts[1].locked);
        assert_eq!(
            wts[1].lock_reason.as_deref(),
            Some("claude agent agent-abc (pid 12345)")
        );
    }

    #[test]
    fn test_parse_worktree_list_locked_no_reason() {
        let input = "\
worktree /tmp/repo
HEAD abc
branch refs/heads/main

worktree /tmp/repo/.claude/worktrees/agent-x
HEAD def
branch refs/heads/test
locked
";
        let wts = parse_worktree_list(input);
        assert_eq!(wts.len(), 2);
        assert!(wts[1].locked);
        assert!(wts[1].lock_reason.is_none());
    }

    #[test]
    fn test_pid_check_dead() {
        // PID 1 is init and always alive, PID 999999999 almost certainly isn't
        assert!(!pid_is_alive(999_999_999));
    }

    #[test]
    fn test_pid_check_alive() {
        // PID 1 should exist on any Linux system
        assert!(pid_is_alive(1));
    }
}
