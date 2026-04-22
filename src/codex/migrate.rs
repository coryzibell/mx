use anyhow::{Context, Result};
use std::fs;

use super::images::{count_images_in_jsonl, extract_images_from_jsonl};
use super::transcript::migrate_clean_transcripts;

use super::archive::{collect_archives, get_codex_dir};

/// Migrate all v1 archives to v2 (extract images to files)
pub(crate) fn migrate_archives(
    dry_run: bool,
    verbose: bool,
    clean: bool,
    include_agents: bool,
) -> Result<()> {
    let codex_dir = get_codex_dir()?;

    if !codex_dir.exists() {
        println!("No archives found (codex directory doesn't exist)");
        return Ok(());
    }

    let archives = collect_archives(&codex_dir)?;

    if archives.is_empty() {
        println!("No archives found");
        return Ok(());
    }

    // --clean mode: generate conversation.md for archives that have session.jsonl but no transcript
    if clean {
        return migrate_clean_transcripts(&codex_dir, archives, dry_run, verbose, include_agents);
    }

    // Find archives that need migration (version < 2 or missing version)
    let mut to_migrate = Vec::new();
    for archive in archives {
        if archive.manifest.version < 2 {
            to_migrate.push(archive);
        }
    }

    if to_migrate.is_empty() {
        println!("All archives are already v2! Nothing to migrate.");
        return Ok(());
    }

    println!("Found {} archive(s) to migrate", to_migrate.len());

    if dry_run {
        println!("\n[DRY RUN MODE - No changes will be made]\n");
    }

    let mut total_migrated = 0;
    let mut total_images = 0;
    let mut total_bytes_saved = 0u64;

    for archive in to_migrate {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");

        if !session_file.exists() {
            eprintln!(
                "Warning: session.jsonl not found in {}, skipping",
                archive.dir_name
            );
            continue;
        }

        if verbose {
            println!("Migrating archive: {}", archive.short_id);
        }

        if !dry_run {
            // Create backup of original session.jsonl
            let backup_file = archive_dir.join("session.jsonl.bak");
            fs::copy(&session_file, &backup_file).context("Failed to create backup")?;

            // Create images directory
            let images_dir = archive_dir.join("images");
            fs::create_dir_all(&images_dir)?;

            // Extract images from session.jsonl
            let session_content = fs::read_to_string(&session_file)?;
            let (modified_session_content, mut all_images) =
                extract_images_from_jsonl(&session_content, &images_dir)?;

            // Write back modified session.jsonl
            fs::write(&session_file, modified_session_content)?;

            // Process agent files if they exist
            let agents_dir = archive_dir.join("agents");
            if agents_dir.exists() {
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();

                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        if verbose {
                            println!(
                                "  Processing agent file: {}",
                                path.file_name().unwrap().to_string_lossy()
                            );
                        }

                        // Backup agent file
                        let backup_path = path.with_extension("jsonl.bak");
                        fs::copy(&path, &backup_path)?;

                        // Extract images from agent file
                        let agent_content = fs::read_to_string(&path)?;
                        let (modified_agent_content, agent_images) =
                            extract_images_from_jsonl(&agent_content, &images_dir)?;

                        // Merge agent images (deduplicate)
                        for img in agent_images {
                            if !all_images.iter().any(|existing| existing.hash == img.hash) {
                                all_images.push(img);
                            }
                        }

                        // Write back modified agent file
                        fs::write(&path, modified_agent_content)?;
                    }
                }
            }

            // Calculate total bytes saved
            let bytes_saved: u64 = all_images.iter().map(|img| img.size_bytes).sum();
            total_bytes_saved += bytes_saved;

            // Update manifest to v2
            let mut manifest = archive.manifest.clone();
            manifest.version = 2;
            manifest.image_count = Some(all_images.len());
            manifest.images = Some(all_images.clone());

            let manifest_json = serde_json::to_string_pretty(&manifest)?;
            fs::write(archive_dir.join("manifest.json"), manifest_json)?;

            let image_count = all_images.len();
            total_images += image_count;

            if verbose || image_count > 0 {
                println!(
                    "  ✓ Migrated {}: {} images extracted, {} KB saved",
                    archive.short_id,
                    image_count,
                    bytes_saved / 1024
                );
            }
        } else {
            // Dry run - just count what would be migrated
            let session_content = fs::read_to_string(&session_file)?;
            let image_count = count_images_in_jsonl(&session_content)?;

            // Count images in agent files too
            let agents_dir = archive_dir.join("agents");
            let mut total_archive_images = image_count;

            if agents_dir.exists() {
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        let agent_content = fs::read_to_string(&path)?;
                        total_archive_images += count_images_in_jsonl(&agent_content)?;
                    }
                }
            }

            total_images += total_archive_images;

            if verbose || total_archive_images > 0 {
                println!(
                    "  Would migrate {}: {} images found",
                    archive.short_id, total_archive_images
                );
            }
        }

        total_migrated += 1;
    }

    println!("\n--- Migration Summary ---");
    println!("Archives migrated: {}", total_migrated);
    println!("Total images extracted: {}", total_images);

    if !dry_run {
        println!("Total space saved: {} KB", total_bytes_saved / 1024);
        println!("\n✓ Migration complete! Original files backed up as *.bak");
    } else {
        println!("\nRun without --dry-run to perform migration");
    }

    Ok(())
}
