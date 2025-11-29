//! GitHub operations - cleanup, comments, etc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sync::github::app_auth::get_installation_token;
use crate::sync::github::auth::get_github_token;
use crate::sync::github::graphql::{DiscussionCommentCreated, GraphQLClient};
use crate::sync::github::rest::{RestClient, UpdateIssueRequest};

/// Parse comma-separated numbers
fn parse_numbers(input: &str) -> Result<Vec<u64>> {
    input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u64>()
                .with_context(|| format!("Invalid number: {}", s))
        })
        .collect()
}

/// Clean up GitHub issues and discussions
pub fn cleanup(
    repo: &str,
    issues: Option<String>,
    discussions: Option<String>,
    dry_run: bool,
) -> Result<()> {
    // Parse repo
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repository format. Expected owner/repo");
    }
    let (owner, repo_name) = (parts[0], parts[1]);

    // Parse issue and discussion numbers
    let issue_numbers = issues
        .as_ref()
        .map(|s| parse_numbers(s))
        .transpose()?
        .unwrap_or_default();

    let discussion_numbers = discussions
        .as_ref()
        .map(|s| parse_numbers(s))
        .transpose()?
        .unwrap_or_default();

    if issue_numbers.is_empty() && discussion_numbers.is_empty() {
        println!("Nothing to clean up. Specify --issues or --discussions.");
        return Ok(());
    }

    println!("Cleaning up {}/{}", owner, repo_name);
    if dry_run {
        println!("[DRY RUN MODE]");
    }
    println!();

    // Get GitHub token
    let token = get_github_token()?;

    // Initialize clients
    let rest_client = RestClient::new(token.clone())?;
    let graphql_client = GraphQLClient::new(&token)?;

    // Track results
    let mut issues_closed = 0;
    let mut discussions_deleted = 0;
    let mut errors = 0;

    // Close issues
    if !issue_numbers.is_empty() {
        println!("Issues:");
        for number in issue_numbers {
            match close_issue(&rest_client, owner, repo_name, number, dry_run) {
                Ok(_) => {
                    println!("  #{} → closed with duplicate label", number);
                    issues_closed += 1;
                }
                Err(e) => {
                    println!("  #{} → error: {}", number, e);
                    errors += 1;
                }
            }
        }
        println!();
    }

    // Delete discussions
    if !discussion_numbers.is_empty() {
        println!("Discussions:");
        for number in discussion_numbers {
            match delete_discussion(&graphql_client, owner, repo_name, number, dry_run) {
                Ok(_) => {
                    println!("  D#{} → deleted", number);
                    discussions_deleted += 1;
                }
                Err(e) => {
                    println!("  D#{} → error: {}", number, e);
                    errors += 1;
                }
            }
        }
        println!();
    }

    // Summary
    let mut summary_parts = Vec::new();
    if issues_closed > 0 {
        summary_parts.push(format!("{} issues closed", issues_closed));
    }
    if discussions_deleted > 0 {
        summary_parts.push(format!("{} discussions deleted", discussions_deleted));
    }
    if errors > 0 {
        summary_parts.push(format!("{} errors", errors));
    }

    println!("Summary: {}", summary_parts.join(", "));

    Ok(())
}

/// Close a single issue with "duplicate" label
fn close_issue(
    client: &RestClient,
    owner: &str,
    repo: &str,
    number: u64,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }

    let req = UpdateIssueRequest {
        title: None,
        body: None,
        labels: Some(vec!["duplicate".to_string()]),
        assignees: None,
        state: Some("closed".to_string()),
    };

    client.update_issue(owner, repo, number, &req)?;
    Ok(())
}

/// Delete a single discussion
fn delete_discussion(
    client: &GraphQLClient,
    owner: &str,
    repo: &str,
    number: u64,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }

    // Get discussion ID
    let id = client.get_discussion_id(owner, repo, number)?;

    // Delete discussion
    client.delete_discussion(&id)?;

    Ok(())
}

/// Format comment body with optional identity signature
fn format_comment_body(message: &str, identity: Option<&str>) -> String {
    if let Some(id) = identity {
        format!(
            "**[{}]**\n\n{}\n\n---\n*Posted by dotmatrix-ai • Identity: {}*",
            id, message, id
        )
    } else {
        message.to_string()
    }
}

/// Post a comment to a GitHub issue
pub fn post_issue_comment(
    repo: &str,
    number: u64,
    message: &str,
    identity: Option<&str>,
) -> Result<String> {
    // Parse repo
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repository format. Expected owner/repo");
    }
    let (owner, repo_name) = (parts[0], parts[1]);

    // Get GitHub App token
    let token = get_installation_token().context("Failed to get GitHub App installation token")?;

    // Create REST client
    let client = RestClient::new(token)?;

    // Format comment body with optional identity
    let body = format_comment_body(message, identity);

    // Post comment
    let comment = create_issue_comment(&client, owner, repo_name, number, &body)?;

    Ok(comment.html_url)
}

/// Post a comment to a GitHub discussion
pub fn post_discussion_comment(
    repo: &str,
    number: u64,
    message: &str,
    identity: Option<&str>,
) -> Result<String> {
    // Parse repo
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repository format. Expected owner/repo");
    }
    let (owner, repo_name) = (parts[0], parts[1]);

    // Get GitHub App token
    let token = get_installation_token().context("Failed to get GitHub App installation token")?;

    // Create GraphQL client
    let graphql_client = GraphQLClient::new(&token)?;

    // Format comment body with optional identity
    let body = format_comment_body(message, identity);

    // Get discussion ID from number
    let discussion_id = graphql_client
        .get_discussion_id(owner, repo_name, number)
        .with_context(|| format!("Failed to get discussion ID for D#{}", number))?;

    // Post comment
    let comment = add_discussion_comment(&graphql_client, &discussion_id, &body)?;

    Ok(comment.url)
}

// ============================================================================
// REST API helpers
// ============================================================================

/// Create a comment on an issue
fn create_issue_comment(
    client: &RestClient,
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<IssueComment> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/issues/{}/comments",
        owner, repo, number
    );

    let req = CreateCommentRequest {
        body: body.to_string(),
    };

    client
        .post_json(&url, &req)
        .with_context(|| format!("Failed to create comment on issue #{}", number))
}

#[derive(Debug, Serialize)]
struct CreateCommentRequest {
    body: String,
}

#[derive(Debug, Deserialize)]
struct IssueComment {
    html_url: String,
}

// ============================================================================
// GraphQL helpers
// ============================================================================

/// Add a comment to a discussion
fn add_discussion_comment(
    client: &GraphQLClient,
    discussion_id: &str,
    body: &str,
) -> Result<DiscussionCommentCreated> {
    client.add_discussion_comment(discussion_id, body)
}
