//! Labels sync command - sync identity labels to repository
//!
//! Reads identity colors from ~/.matrix/artifacts/etc/identity-colors.yaml
//! and ensures the repository has matching labels.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::sync::artifacts_dir;
use crate::sync::github::auth::get_github_token;
use crate::sync::github::rest::{CreateLabelRequest, RestClient, UpdateLabelRequest};

/// Identity definition from YAML
#[derive(Debug, Deserialize)]
struct IdentityDef {
    color: String,
    rationale: String,
}

/// Identity colors file structure
#[derive(Debug, Deserialize)]
struct IdentityColors {
    identities: HashMap<String, IdentityDef>,
}

/// Run the labels sync command
pub fn run(repo: &str, dry_run: bool) -> Result<()> {
    // Parse owner/repo
    let (owner, repo_name) = parse_repo(repo)?;

    if dry_run {
        println!("[DRY RUN] Syncing labels to {}/{}", owner, repo_name);
    } else {
        println!("Syncing labels to {}/{}", owner, repo_name);
    }

    // Load identity colors
    let colors_path = artifacts_dir().join("etc").join("identity-colors.yaml");
    let identity_colors = load_identity_colors(&colors_path)?;

    println!(
        "Loaded {} identity definitions",
        identity_colors.identities.len()
    );

    // Get GitHub token and create client
    let token = get_github_token()?;
    let client = RestClient::new(token)?;

    // Fetch existing labels
    println!("Fetching existing labels...");
    let existing_labels = client.list_labels(&owner, &repo_name)?;
    let existing_map: HashMap<String, _> = existing_labels
        .iter()
        .map(|l| (l.name.clone(), l))
        .collect();

    println!("Found {} existing labels", existing_labels.len());

    // Track stats
    let mut created = 0;
    let mut updated = 0;
    let mut unchanged = 0;

    // Sync each identity label
    for (identity, def) in &identity_colors.identities {
        let label_name = format!("identity:{}", identity);
        let color = def.color.trim_start_matches('#');

        if let Some(existing) = existing_map.get(&label_name) {
            // Label exists - check if update needed
            let needs_color_update = existing.color.to_lowercase() != color.to_lowercase();
            let needs_desc_update = existing.description.as_deref() != Some(&def.rationale);

            if needs_color_update || needs_desc_update {
                if dry_run {
                    println!(
                        "  [DRY RUN] Would update: {} (color: {} -> {})",
                        label_name, existing.color, color
                    );
                } else {
                    let req = UpdateLabelRequest {
                        new_name: None,
                        color: if needs_color_update {
                            Some(color.to_string())
                        } else {
                            None
                        },
                        description: if needs_desc_update {
                            Some(def.rationale.clone())
                        } else {
                            None
                        },
                    };
                    client.update_label(&owner, &repo_name, &label_name, &req)?;
                    println!("  Updated: {}", label_name);
                }
                updated += 1;
            } else {
                unchanged += 1;
            }
        } else {
            // Label doesn't exist - create it
            if dry_run {
                println!(
                    "  [DRY RUN] Would create: {} (color: {})",
                    label_name, color
                );
            } else {
                let req = CreateLabelRequest {
                    name: label_name.clone(),
                    color: color.to_string(),
                    description: Some(def.rationale.clone()),
                };
                client.create_label(&owner, &repo_name, &req)?;
                println!("  Created: {}", label_name);
            }
            created += 1;
        }
    }

    // Summary
    println!();
    println!("Summary:");
    println!("  Created: {}", created);
    println!("  Updated: {}", updated);
    println!("  Unchanged: {}", unchanged);

    Ok(())
}

fn parse_repo(repo: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Repository must be in format 'owner/repo', got: {}", repo);
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn load_identity_colors(path: &PathBuf) -> Result<IdentityColors> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read identity colors from: {}", path.display()))?;

    serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse identity colors YAML: {}", path.display()))
}
