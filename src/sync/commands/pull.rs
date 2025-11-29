//! Pull command - download issues/discussions from GitHub to YAML

use anyhow::Result;
use std::path::PathBuf;

use crate::sync::default_sync_dir;
use crate::sync::github::auth::get_github_token;
use crate::sync::github::graphql::GraphQLClient;
use crate::sync::github::rest::RestClient;
use crate::sync::yaml::schema::{yaml_filename, Comment, LastSynced, Metadata, SyncYaml};
use crate::sync::yaml::store::YamlStore;

/// Run the pull command
pub fn run(repo: &str, output: Option<String>, dry_run: bool) -> Result<()> {
    // Parse owner/repo
    let (owner, repo_name) = parse_repo(repo)?;

    // Determine output directory
    let output_dir = output
        .map(PathBuf::from)
        .unwrap_or_else(|| default_sync_dir(repo));

    if dry_run {
        println!("[DRY RUN] Pulling from {}/{}", owner, repo_name);
    } else {
        println!("Pulling from {}/{}", owner, repo_name);
    }
    println!("Output: {}", output_dir.display());
    println!();

    // Get GitHub token and create clients
    let token = get_github_token()?;
    let rest_client = RestClient::new(token.clone())?;
    let graphql_client = GraphQLClient::new(&token)?;

    // Create store
    let store = YamlStore::new(output_dir.clone());
    if !dry_run {
        store.ensure_dir()?;
    }

    // Track stats
    let mut issues_created = 0;
    let mut issues_updated = 0;
    let mut issues_unchanged = 0;
    let mut discussions_created = 0;
    let mut discussions_updated = 0;
    let mut discussions_unchanged = 0;

    // Fetch and process issues
    println!("Fetching issues...");
    let issues = rest_client.list_issues(&owner, &repo_name, "open")?;
    println!("Found {} open issues", issues.len());
    println!();

    println!("Issues:");
    for issue in &issues {
        let comments = rest_client.list_issue_comments(&owner, &repo_name, issue.number)?;
        let existing = store.find_by_issue_number(issue.number)?;
        let filename = yaml_filename(issue.number, &issue.title);

        if let Some((path, mut yaml)) = existing {
            let remote_body = issue.body.as_deref().unwrap_or("");
            let remote_labels = issue.label_names();
            let remote_assignees = issue.assignee_logins();

            let base = yaml.last_synced();
            let local_changed = base.is_some_and(|b| {
                yaml.title() != b.title
                    || yaml.body() != b.body
                    || yaml.labels() != b.labels.as_slice()
            });

            if local_changed {
                println!(
                    "  #{} {} → local changes, skipping",
                    issue.number, issue.title
                );
                issues_unchanged += 1;
            } else {
                yaml.metadata.title = Some(issue.title.clone());
                yaml.body_markdown = remote_body.to_string();
                yaml.metadata.labels = remote_labels.clone();
                yaml.metadata.assignees = remote_assignees.clone();
                yaml.metadata.state = Some(issue.state.clone());
                yaml.metadata.github_updated_at = Some(issue.updated_at.clone());
                yaml.comments = comments
                    .iter()
                    .map(|c| Comment {
                        id: c.id.to_string(),
                        author: c.user.login.clone(),
                        created_at: c.created_at.clone(),
                        body: c.body.clone().unwrap_or_default(),
                    })
                    .collect();

                yaml.metadata.last_synced = Some(LastSynced::new(
                    &issue.title,
                    remote_body,
                    remote_labels,
                    &issue.updated_at,
                    Some(remote_assignees),
                ));

                if dry_run {
                    println!(
                        "  #{} {} ({} comments) → would update",
                        issue.number,
                        issue.title,
                        comments.len()
                    );
                } else {
                    store.write(&path, &yaml)?;
                    println!(
                        "  #{} {} ({} comments) → updated",
                        issue.number,
                        issue.title,
                        comments.len()
                    );
                }
                issues_updated += 1;
            }
        } else {
            let remote_body = issue.body.as_deref().unwrap_or("");
            let remote_labels = issue.label_names();
            let remote_assignees = issue.assignee_logins();

            let yaml = SyncYaml {
                metadata: Metadata {
                    title: Some(issue.title.clone()),
                    r#type: Some("issue".to_string()),
                    labels: remote_labels.clone(),
                    assignees: remote_assignees.clone(),
                    state: Some(issue.state.clone()),
                    github_issue_number: Some(issue.number),
                    github_updated_at: Some(issue.updated_at.clone()),
                    last_synced: Some(LastSynced::new(
                        &issue.title,
                        remote_body,
                        remote_labels,
                        &issue.updated_at,
                        Some(remote_assignees),
                    )),
                    ..Default::default()
                },
                body_markdown: remote_body.to_string(),
                comments: comments
                    .iter()
                    .map(|c| Comment {
                        id: c.id.to_string(),
                        author: c.user.login.clone(),
                        created_at: c.created_at.clone(),
                        body: c.body.clone().unwrap_or_default(),
                    })
                    .collect(),
                ..Default::default()
            };

            if dry_run {
                println!(
                    "  #{} {} ({} comments) → would create {}",
                    issue.number,
                    issue.title,
                    comments.len(),
                    filename
                );
            } else {
                store.write_new(&filename, &yaml)?;
                println!(
                    "  #{} {} ({} comments) → created {}",
                    issue.number,
                    issue.title,
                    comments.len(),
                    filename
                );
            }
            issues_created += 1;
        }
    }

    // Fetch and process discussions
    println!();
    println!("Fetching discussions...");
    let discussions = graphql_client.list_discussions(&owner, &repo_name)?;
    println!("Found {} discussions", discussions.len());
    println!();

    println!("Discussions:");
    if discussions.is_empty() {
        println!("  (none)");
    }

    for discussion in &discussions {
        let existing = store.find_by_discussion_id(&discussion.id)?;
        let filename = format!(
            "d{}-{}.yaml",
            discussion.number,
            crate::sync::yaml::schema::slugify(&discussion.title, 50)
        );

        if let Some((path, mut yaml)) = existing {
            let remote_body = discussion.body.as_deref().unwrap_or("");
            let remote_labels = discussion.label_names();

            let base = yaml.last_synced();
            let local_changed = base.is_some_and(|b| {
                yaml.title() != b.title
                    || yaml.body() != b.body
                    || yaml.labels() != b.labels.as_slice()
            });

            if local_changed {
                println!(
                    "  D#{} {} → local changes, skipping",
                    discussion.number, discussion.title
                );
                discussions_unchanged += 1;
            } else {
                yaml.metadata.title = Some(discussion.title.clone());
                yaml.body_markdown = remote_body.to_string();
                yaml.metadata.labels = remote_labels.clone();
                yaml.metadata.category = Some(discussion.category.slug.clone());
                yaml.metadata.github_updated_at = Some(discussion.updated_at.clone());
                yaml.comments = discussion
                    .comments
                    .nodes
                    .iter()
                    .map(|c| Comment {
                        id: c.id.clone(),
                        author: c
                            .author
                            .as_ref()
                            .map(|a| a.login.clone())
                            .unwrap_or_default(),
                        created_at: c.created_at.clone(),
                        body: c.body.clone().unwrap_or_default(),
                    })
                    .collect();

                yaml.metadata.last_synced = Some(LastSynced::new(
                    &discussion.title,
                    remote_body,
                    remote_labels,
                    &discussion.updated_at,
                    None,
                ));

                if dry_run {
                    println!(
                        "  D#{} {} ({} comments) → would update",
                        discussion.number,
                        discussion.title,
                        discussion.comments.nodes.len()
                    );
                } else {
                    store.write(&path, &yaml)?;
                    println!(
                        "  D#{} {} ({} comments) → updated",
                        discussion.number,
                        discussion.title,
                        discussion.comments.nodes.len()
                    );
                }
                discussions_updated += 1;
            }
        } else {
            let remote_body = discussion.body.as_deref().unwrap_or("");
            let remote_labels = discussion.label_names();

            let yaml = SyncYaml {
                metadata: Metadata {
                    title: Some(discussion.title.clone()),
                    r#type: Some("idea".to_string()),
                    labels: remote_labels.clone(),
                    category: Some(discussion.category.slug.clone()),
                    github_discussion_id: Some(discussion.id.clone()),
                    github_discussion_number: Some(discussion.number),
                    github_updated_at: Some(discussion.updated_at.clone()),
                    last_synced: Some(LastSynced::new(
                        &discussion.title,
                        remote_body,
                        remote_labels,
                        &discussion.updated_at,
                        None,
                    )),
                    ..Default::default()
                },
                body_markdown: remote_body.to_string(),
                comments: discussion
                    .comments
                    .nodes
                    .iter()
                    .map(|c| Comment {
                        id: c.id.clone(),
                        author: c
                            .author
                            .as_ref()
                            .map(|a| a.login.clone())
                            .unwrap_or_default(),
                        created_at: c.created_at.clone(),
                        body: c.body.clone().unwrap_or_default(),
                    })
                    .collect(),
                ..Default::default()
            };

            if dry_run {
                println!(
                    "  D#{} {} ({} comments) → would create {}",
                    discussion.number,
                    discussion.title,
                    discussion.comments.nodes.len(),
                    filename
                );
            } else {
                store.write_new(&filename, &yaml)?;
                println!(
                    "  D#{} {} ({} comments) → created {}",
                    discussion.number,
                    discussion.title,
                    discussion.comments.nodes.len(),
                    filename
                );
            }
            discussions_created += 1;
        }
    }

    println!();
    println!("Summary:");
    println!(
        "  Issues: {} created, {} updated, {} unchanged",
        issues_created, issues_updated, issues_unchanged
    );
    println!(
        "  Discussions: {} created, {} updated, {} unchanged",
        discussions_created, discussions_updated, discussions_unchanged
    );

    Ok(())
}

fn parse_repo(repo: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Repository must be in format 'owner/repo', got: {}", repo);
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}
