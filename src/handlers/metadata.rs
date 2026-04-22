use anyhow::{Result, bail};

use crate::cli::*;
use crate::index::IndexConfig;
use crate::store;
use crate::types;

use super::{AgentFrontmatter, parse_frontmatter};

pub(crate) fn handle_agents(cmd: AgentsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        AgentsCommands::List { json } => {
            let agents = db.list_agents()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&agents)?);
            } else if agents.is_empty() {
                println!("No agents registered");
            } else {
                println!("Registered agents:\n");
                for agent in agents {
                    println!(
                        "  {} - {}",
                        agent.id,
                        agent.description.as_deref().unwrap_or("")
                    );
                    if let Some(domain) = &agent.domain {
                        println!("    Domain: {}", domain);
                    }
                }
            }
        }

        AgentsCommands::Add {
            id,
            description,
            domain,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let agent = types::Agent {
                id: id.clone(),
                description: Some(description.clone()),
                domain: Some(domain.clone()),
                created_at: Some(now.clone()),
                updated_at: Some(now),
            };

            db.upsert_agent(&agent)?;
            println!("Added agent: {}", id);
            println!("  Description: {}", description);
            println!("  Domain: {}", domain);
        }

        AgentsCommands::Show { id } => match db.get_agent(&id)? {
            Some(agent) => {
                println!("Agent: {}", agent.id);
                if let Some(desc) = &agent.description {
                    println!("Description: {}", desc);
                }
                if let Some(domain) = &agent.domain {
                    println!("Domain: {}", domain);
                }
                if let Some(created) = &agent.created_at {
                    println!("Created: {}", created);
                }
                if let Some(updated) = &agent.updated_at {
                    println!("Updated: {}", updated);
                }
            }
            None => {
                bail!("Agent '{}' not found", id);
            }
        },

        AgentsCommands::Seed { path } => {
            use anyhow::Context;
            use std::fs;
            use std::path::PathBuf;

            // Determine agents directory
            let agents_dir = if let Some(p) = path {
                PathBuf::from(p)
            } else {
                // Default: $MX_HOME/agents/
                crate::paths::agents_dir()
            };

            if !agents_dir.exists() {
                bail!("Agents directory does not exist: {:?}", agents_dir);
            }

            // Scan for .md files
            let entries = fs::read_dir(&agents_dir)
                .with_context(|| format!("Failed to read directory: {:?}", agents_dir))?;

            let mut seeded = Vec::new();
            let now = chrono::Utc::now().to_rfc3339();

            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                // Skip if not a markdown file
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                // Skip files starting with _
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && name.starts_with('_')
                {
                    continue;
                }

                // Read file
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read file: {:?}", path))?;

                // Parse frontmatter
                if let Some((frontmatter, _body)) = parse_frontmatter(&content)
                    && let Ok(agent_data) = serde_yaml::from_str::<AgentFrontmatter>(&frontmatter)
                {
                    let agent = types::Agent {
                        id: agent_data.name.clone(),
                        description: Some(agent_data.description.clone()),
                        domain: agent_data.domain,
                        created_at: Some(now.clone()),
                        updated_at: Some(now.clone()),
                    };

                    db.upsert_agent(&agent)?;
                    seeded.push(agent_data.name);
                }
            }

            if seeded.is_empty() {
                println!("No agents seeded from {:?}", agents_dir);
            } else {
                println!("Seeded {} agents:", seeded.len());
                for name in &seeded {
                    println!("  {}", name);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_projects(cmd: ProjectsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        ProjectsCommands::List { json } => {
            let projects = db.list_projects(false)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else if projects.is_empty() {
                println!("No projects registered");
            } else {
                println!("Registered projects:\n");
                for project in projects {
                    println!("  {} - {}", project.id, project.name);
                    if let Some(path) = &project.path {
                        println!("    Path: {}", path);
                    }
                    if let Some(url) = &project.repo_url {
                        println!("    Repo: {}", url);
                    }
                    if let Some(desc) = &project.description {
                        println!("    Description: {}", desc);
                    }
                    println!();
                }
            }
        }

        ProjectsCommands::Add {
            id,
            name,
            path,
            repo_url,
            description,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let project = types::Project {
                id: id.clone(),
                name: name.clone(),
                path,
                repo_url,
                description,
                active: true,
                created_at: now.clone(),
                updated_at: now,
            };

            db.upsert_project(&project)?;
            println!("Added project: {}", id);
            println!("  Name: {}", name);
        }
    }

    Ok(())
}

pub(crate) fn handle_applicability(cmd: ApplicabilityCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        ApplicabilityCommands::List => {
            let types = db.list_applicability_types()?;
            if types.is_empty() {
                println!("No applicability types registered");
            } else {
                println!("Registered applicability types:\n");
                for atype in types {
                    println!("  {} - {}", atype.id, atype.description);
                    if let Some(scope) = &atype.scope {
                        println!("    Scope: {}", scope);
                    }
                    println!();
                }
            }
        }

        ApplicabilityCommands::Add {
            id,
            description,
            scope,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let atype = types::ApplicabilityType {
                id: id.clone(),
                description: description.clone(),
                scope,
                created_at: now,
            };

            db.upsert_applicability_type(&atype)?;
            println!("Added applicability type: {}", id);
            println!("  Description: {}", description);
        }
    }

    Ok(())
}

pub(crate) fn handle_sessions(cmd: SessionsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        SessionsCommands::List { project, json } => {
            let sessions = db.list_sessions(project.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else if sessions.is_empty() {
                println!("No sessions found");
            } else {
                println!("Sessions:\n");
                for session in sessions {
                    println!("  ID: {}", session.id);
                    println!("    Type: {}", session.session_type_id);
                    if let Some(proj) = &session.project_id {
                        println!("    Project: {}", proj);
                    }
                    println!("    Started: {}", session.started_at);
                    if let Some(ended) = &session.ended_at {
                        println!("    Ended: {}", ended);
                    }
                    println!();
                }
            }
        }

        SessionsCommands::Create {
            session_type,
            project,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let id = format!("sess-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
            let session = types::Session {
                id: id.clone(),
                session_type_id: session_type,
                project_id: project,
                started_at: now,
                ended_at: None,
                metadata: None,
            };

            db.upsert_session(&session)?;
            println!("Created session: {}", id);
        }

        SessionsCommands::Close { id } => {
            if let Some(mut session) = db.get_session(&id)? {
                session.ended_at = Some(chrono::Utc::now().to_rfc3339());
                db.upsert_session(&session)?;
                println!("Closed session: {}", id);
            } else {
                bail!("Session '{}' not found", id);
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_categories(cmd: CategoriesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        CategoriesCommands::List { json } => {
            let categories = db.list_categories()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&categories)?);
            } else if categories.is_empty() {
                println!("No categories registered");
            } else {
                println!("Registered categories:\n");
                for category in categories {
                    println!("  {} - {}", category.id, category.description);
                }
            }
        }
        CategoriesCommands::Add { id, description } => {
            // Check if category already exists
            if db.get_category(&id)?.is_some() {
                bail!("Category '{}' already exists", id);
            }

            let now = chrono::Utc::now().to_rfc3339();
            let category = types::Category {
                id: id.clone(),
                description: description.clone(),
                created_at: now,
            };

            db.upsert_category(&category)?;
            println!("Added category: {}", id);
            println!("  Description: {}", description);
        }
        CategoriesCommands::Remove { id } => {
            // Check if category exists
            if db.get_category(&id)?.is_none() {
                bail!("Category '{}' not found", id);
            }

            // delete_category will check if entries use it and error if so
            match db.delete_category(&id) {
                Ok(true) => {
                    println!("Deleted category: {}", id);
                }
                Ok(false) => {
                    bail!("Category '{}' not found", id);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_tags(cmd: TagsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        TagsCommands::List { category, json } => {
            // Validate category if provided
            if let Some(ref cat) = category
                && db.get_category(cat)?.is_none()
            {
                let categories = db.list_categories()?;
                let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                bail!(
                    "Unknown category '{}'. Valid categories: {}",
                    cat,
                    valid_ids.join(", ")
                );
            }

            let tags = db.list_all_tags(category.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tags)?);
            } else if tags.is_empty() {
                if let Some(cat) = &category {
                    println!("No tags found in category '{}'", cat);
                } else {
                    println!("No tags found");
                }
            } else {
                if let Some(cat) = &category {
                    println!("Tags in category '{}':\n", cat);
                } else {
                    println!("All tags:\n");
                }
                for tag in tags {
                    println!("  {}", tag);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_source_types(cmd: SourceTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        SourceTypesCommands::List { json } => {
            let types = db.list_source_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No source types registered");
            } else {
                println!("Registered source types:\n");
                for stype in types {
                    println!("  {} - {}", stype.id, stype.description);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_entry_types(cmd: EntryTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        EntryTypesCommands::List { json } => {
            let types = db.list_entry_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No entry types registered");
            } else {
                println!("Registered entry types:\n");
                for etype in types {
                    println!("  {} - {}", etype.id, etype.description);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_session_types(cmd: SessionTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        SessionTypesCommands::List { json } => {
            let types = db.list_session_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No session types registered");
            } else {
                println!("Registered session types:\n");
                for stype in types {
                    println!("  {} - {}", stype.id, stype.description);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_relationship_types(
    cmd: RelationshipTypesCommands,
    config: &IndexConfig,
) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        RelationshipTypesCommands::List { json } => {
            let types = db.list_relationship_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No relationship types registered");
            } else {
                println!("Registered relationship types:\n");
                for rtype in types {
                    let directional = if rtype.directional {
                        "(directional)"
                    } else {
                        "(bidirectional)"
                    };
                    println!("  {} - {} {}", rtype.id, rtype.description, directional);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_relationships(cmd: RelationshipsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        RelationshipsCommands::List { id, json } => {
            let relationships = db.list_relationships_for_entry(&id)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&relationships)?);
            } else if relationships.is_empty() {
                println!("No relationships found for '{}'", id);
            } else {
                println!("Relationships for '{}':\n", id);
                for rel in relationships {
                    let direction = if rel.from_entry_id == id {
                        format!("-> {} ({})", rel.to_entry_id, rel.relationship_type)
                    } else {
                        format!("<- {} ({})", rel.from_entry_id, rel.relationship_type)
                    };
                    println!("  {} {}", rel.id, direction);
                }
            }
        }

        RelationshipsCommands::Add { from, to, r#type } => {
            let id = db.add_relationship(&from, &to, &r#type)?;
            println!("Added relationship: {}", id);
            println!("  From: {}", from);
            println!("  To: {}", to);
            println!("  Type: {}", r#type);
        }

        RelationshipsCommands::Delete { id } => {
            if db.delete_relationship(&id)? {
                println!("Deleted relationship: {}", id);
            } else {
                bail!("Relationship '{}' not found", id);
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_content_types(cmd: ContentTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        ContentTypesCommands::List { json } => {
            let types = db.list_content_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No content types registered");
            } else {
                println!("Registered content types:\n");
                for ctype in types {
                    println!("  {} - {}", ctype.id, ctype.description);
                    if let Some(exts) = &ctype.file_extensions {
                        println!("    Extensions: {}", exts);
                    }
                }
            }
        }
    }

    Ok(())
}
