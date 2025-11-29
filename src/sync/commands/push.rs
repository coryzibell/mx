//! Push command - upload YAML changes to GitHub
//!
//! Creates new issues/discussions or updates existing ones.
//! Uses three-way merge to detect and resolve conflicts.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::sync::default_sync_dir;
use crate::sync::github::auth::get_github_token;
use crate::sync::github::graphql::GraphQLClient;
use crate::sync::github::rest::{CreateIssueRequest, RestClient, UpdateIssueRequest};
use crate::sync::merge::resolve::merge_fields;
use crate::sync::yaml::schema::{yaml_filename, ItemType, LastSynced};
use crate::sync::yaml::store::YamlStore;

/// Run the push command
pub fn run(repo: &str, input: Option<String>, dry_run: bool) -> Result<()> {
    // Parse owner/repo
    let (owner, repo_name) = parse_repo(repo)?;

    // Determine input directory
    let input_dir = input
        .map(PathBuf::from)
        .unwrap_or_else(|| default_sync_dir(repo));

    if dry_run {
        println!("[DRY RUN] Pushing to {}/{}", owner, repo_name);
    } else {
        println!("Pushing to {}/{}", owner, repo_name);
    }
    println!("Source: {}", input_dir.display());
    println!();

    // Get GitHub token and create clients
    let token = get_github_token()?;
    let rest_client = RestClient::new(token.clone())?;
    let graphql_client = GraphQLClient::new(&token)?;

    // Create store
    let store = YamlStore::new(input_dir.clone());

    // Read all YAML files
    println!("Reading local YAML files...");
    let items = store.read_all()?;
    println!("Found {} items", items.len());
    println!();

    // Track stats
    let mut issues_created = 0;
    let mut issues_updated = 0;
    let mut issues_unchanged = 0;
    let mut discussions_created = 0;
    let mut discussions_updated = 0;
    let mut discussions_unchanged = 0;

    // Cache for GraphQL lookups (avoid repeated queries)
    let mut repo_id: Option<String> = None;
    let mut categories: Option<HashMap<String, String>> = None; // slug -> id

    println!("Processing:");
    for (path, yaml) in items {
        let title = yaml.title();
        let item_type = yaml.item_type();

        match item_type {
            ItemType::Idea => {
                // Handle discussion
                if let Some(discussion_id) = yaml.github_discussion_id() {
                    // Existing discussion - update
                    let local_title = yaml.title();
                    let local_body = yaml.body();

                    // For simplicity, just update title/body if different from last_synced
                    let needs_update = yaml.last_synced().map_or(true, |base| {
                        local_title != base.title || local_body != base.body
                    });

                    if needs_update {
                        if dry_run {
                            println!("  D#{} {} → would update",
                                yaml.metadata.github_discussion_number.unwrap_or(0), title);
                        } else {
                            graphql_client.update_discussion(
                                discussion_id,
                                Some(local_title),
                                Some(local_body),
                            )?;

                            // Update local snapshot
                            let mut updated_yaml = yaml.clone();
                            updated_yaml.metadata.last_synced = Some(LastSynced::new(
                                local_title,
                                local_body,
                                yaml.labels().to_vec(),
                                &chrono::Utc::now().to_rfc3339(),
                                None,
                            ));
                            store.write(&path, &updated_yaml)?;

                            println!("  D#{} {} → updated",
                                yaml.metadata.github_discussion_number.unwrap_or(0), title);
                        }
                        discussions_updated += 1;
                    } else {
                        println!("  D#{} {} → unchanged",
                            yaml.metadata.github_discussion_number.unwrap_or(0), title);
                        discussions_unchanged += 1;
                    }
                } else {
                    // New discussion - create it
                    // Need repo ID and category
                    if repo_id.is_none() {
                        repo_id = Some(graphql_client.get_repository_id(&owner, &repo_name)?);
                    }
                    if categories.is_none() {
                        let cats = graphql_client.list_discussion_categories(&owner, &repo_name)?;
                        categories = Some(cats.into_iter().map(|c| (c.slug, c.id)).collect());
                    }

                    let category_slug = yaml.category().unwrap_or("ideas");
                    let category_id = categories.as_ref().unwrap().get(category_slug);

                    if category_id.is_none() {
                        println!("  {} → skipped (category '{}' not found)", title, category_slug);
                        discussions_unchanged += 1;
                        continue;
                    }

                    let body = yaml.body();

                    if dry_run {
                        println!("  {} → would create discussion in '{}'", title, category_slug);
                    } else {
                        let discussion = graphql_client.create_discussion(
                            repo_id.as_ref().unwrap(),
                            category_id.unwrap(),
                            title,
                            body,
                        )?;

                        // Update local YAML with discussion ID and snapshot
                        let mut updated_yaml = yaml.clone();
                        updated_yaml.metadata.github_discussion_id = Some(discussion.id.clone());
                        updated_yaml.metadata.github_discussion_number = Some(discussion.number);
                        updated_yaml.metadata.github_updated_at = Some(discussion.updated_at.clone());
                        updated_yaml.metadata.last_synced = Some(LastSynced::new(
                            title,
                            body,
                            yaml.labels().to_vec(),
                            &discussion.updated_at,
                            None,
                        ));

                        // Rename file to include discussion number
                        let new_filename = format!(
                            "d{}-{}.yaml",
                            discussion.number,
                            crate::sync::yaml::schema::slugify(title, 50)
                        );
                        let new_path = path.parent().unwrap().join(&new_filename);
                        store.write(&new_path, &updated_yaml)?;

                        if path != new_path {
                            std::fs::remove_file(&path).ok();
                        }

                        println!("  {} → created D#{} ({})", title, discussion.number, new_filename);
                    }
                    discussions_created += 1;
                }
            }
            ItemType::Issue => {
                // Handle issue
                if let Some(number) = yaml.github_issue_number() {
                    // Existing issue - check for updates needed
                    let remote_issue = rest_client.get_issue(&owner, &repo_name, number)?;

                    let local_title = yaml.title();
                    let local_body = yaml.body();
                    let local_labels = yaml.labels();
                    let local_assignees = yaml.assignees();

                    let remote_title = &remote_issue.title;
                    let remote_body = remote_issue.body.as_deref().unwrap_or("");
                    let remote_labels = remote_issue.label_names();
                    let remote_assignees = remote_issue.assignee_logins();

                    let base = yaml.last_synced();
                    let (base_title, base_body, base_labels, base_assignees) =
                        if let Some(b) = base {
                            (
                                b.title.as_str(),
                                b.body.as_str(),
                                b.labels.as_slice(),
                                b.assignees.as_deref().unwrap_or(&[]),
                            )
                        } else {
                            (
                                remote_title.as_str(),
                                remote_body,
                                remote_labels.as_slice(),
                                remote_assignees.as_slice(),
                            )
                        };

                    let (merged, _has_conflicts) = merge_fields(
                        local_title,
                        local_body,
                        local_labels,
                        local_assignees,
                        remote_title,
                        remote_body,
                        &remote_labels,
                        &remote_assignees,
                        base_title,
                        base_body,
                        base_labels,
                        base_assignees,
                        true,
                    );

                    let needs_update = merged.title != *remote_title
                        || merged.body != remote_body
                        || merged.labels != remote_labels
                        || merged.assignees != remote_assignees;

                    if needs_update {
                        if dry_run {
                            println!("  #{} {} → would update", number, title);
                        } else {
                            let req = UpdateIssueRequest {
                                title: Some(merged.title.clone()),
                                body: Some(merged.body.clone()),
                                labels: Some(merged.labels.clone()),
                                assignees: Some(merged.assignees.clone()),
                                state: None,
                            };
                            rest_client.update_issue(&owner, &repo_name, number, &req)?;

                            let mut updated_yaml = yaml.clone();
                            updated_yaml.metadata.last_synced = Some(LastSynced::new(
                                &merged.title,
                                &merged.body,
                                merged.labels.clone(),
                                &chrono::Utc::now().to_rfc3339(),
                                Some(merged.assignees.clone()),
                            ));
                            store.write(&path, &updated_yaml)?;

                            println!("  #{} {} → updated", number, title);
                        }
                        issues_updated += 1;
                    } else {
                        println!("  #{} {} → unchanged", number, title);
                        issues_unchanged += 1;
                    }
                } else {
                    // New issue
                    let body = yaml.body();
                    let labels = yaml.labels().to_vec();
                    let assignees = yaml.assignees().to_vec();

                    if dry_run {
                        println!("  {} → would create new issue", title);
                    } else {
                        let req = CreateIssueRequest {
                            title: title.to_string(),
                            body: body.to_string(),
                            labels,
                            assignees: assignees.clone(),
                        };
                        let created_issue = rest_client.create_issue(&owner, &repo_name, &req)?;

                        let mut updated_yaml = yaml.clone();
                        updated_yaml.metadata.github_issue_number = Some(created_issue.number);
                        updated_yaml.metadata.github_updated_at =
                            Some(created_issue.updated_at.clone());
                        updated_yaml.metadata.last_synced = Some(LastSynced::new(
                            title,
                            body,
                            yaml.labels().to_vec(),
                            &created_issue.updated_at,
                            Some(assignees),
                        ));

                        let new_filename = yaml_filename(created_issue.number, title);
                        let new_path = path.parent().unwrap().join(&new_filename);
                        store.write(&new_path, &updated_yaml)?;

                        if path != new_path {
                            std::fs::remove_file(&path).ok();
                        }

                        println!(
                            "  {} → created #{} ({})",
                            title, created_issue.number, new_filename
                        );
                    }
                    issues_created += 1;
                }
            }
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
