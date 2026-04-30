use anyhow::{Context, Result};
use std::fs;

use super::images::{count_images_in_jsonl, extract_images_from_jsonl};
use super::transcript::migrate_clean_transcripts;

use super::MANIFEST_WRITE_VERSION;
use super::archive::{collect_archives, get_codex_dir};

/// Migrate all archives below the current write version up to it.
///
/// v1 archives are upgraded in two phases:
///   1. Image extraction (v1 → v2) pulls images out of the JSONLs into a
///      per-archive `images/` directory and records counts. This is the
///      only phase that touches bytes on disk.
///   2. A metadata-only bump (v2 → v5) folds in the v3/v4/v5 fields
///      (`has_clean_transcript`, `user_name`, `assistant_name`,
///      `tool_output_count`, `mcp_log_count`, `history_lines`,
///      `source_breakdown`). Both phases are applied to the same manifest
///      write, so v1 archives end up at v5 in a single pass.
///
/// v2/v3/v4 archives skip the image-extraction phase and receive only the
/// metadata-only bump up to v5. All the new fields are `Option`, so older
/// archives keep deserializing and the bump just rewrites the manifest with
/// the higher version number plus `None` defaults for anything missing. The
/// new sidecars (`mcp/`, `tool-output/`, `history/`) do not exist on disk
/// for these archives — that's fine; absent sidecars are represented by
/// `None` counts.
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

    // Split archives into two buckets:
    //   - Pre-v2 archives need image extraction *and* a metadata bump.
    //   - v2..v(MANIFEST_WRITE_VERSION-1) archives just need the metadata bump.
    let mut to_extract_images = Vec::new();
    let mut metadata_only_bumps = Vec::new();
    for archive in archives {
        if archive.manifest.version < 2 {
            to_extract_images.push(archive);
        } else if archive.manifest.version < MANIFEST_WRITE_VERSION {
            metadata_only_bumps.push(archive);
        }
    }

    if to_extract_images.is_empty() && metadata_only_bumps.is_empty() {
        println!(
            "All archives are already at schema v{}! Nothing to migrate.",
            MANIFEST_WRITE_VERSION
        );
        return Ok(());
    }

    println!(
        "Found {} archive(s) needing image extraction and {} archive(s) needing metadata bump",
        to_extract_images.len(),
        metadata_only_bumps.len()
    );

    if dry_run {
        println!("\n[DRY RUN MODE - No changes will be made]\n");
    }

    let mut total_migrated = 0;
    let mut total_images = 0;
    let mut total_bytes_saved = 0u64;

    for archive in to_extract_images {
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

            // Update manifest to current write version. The image fields
            // are the v1→v2 step; the version bump folds in v3/v4/v5
            // (which are all-Option, so no field defaults change here).
            let mut manifest = archive.manifest.clone();
            manifest.version = MANIFEST_WRITE_VERSION;
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

    // Metadata-only bump for v2/v3/v4 archives → MANIFEST_WRITE_VERSION (v5).
    // No bytes-on-disk change; just rewrite the manifest with the higher
    // version number. The new v5 fields are Option-defaulted so they stay
    // None for archives that don't have the new sidecars yet.
    let mut metadata_bumped = 0;
    for archive in metadata_only_bumps {
        let archive_dir = codex_dir.join(&archive.dir_name);
        if verbose {
            println!(
                "Metadata bump: {} (v{} -> v{})",
                archive.short_id, archive.manifest.version, MANIFEST_WRITE_VERSION
            );
        }
        if !dry_run {
            let mut manifest = archive.manifest.clone();
            manifest.version = MANIFEST_WRITE_VERSION;
            let manifest_json = serde_json::to_string_pretty(&manifest)?;
            fs::write(archive_dir.join("manifest.json"), manifest_json)?;
        }
        metadata_bumped += 1;
    }

    println!("\n--- Migration Summary ---");
    println!("Archives migrated: {}", total_migrated);
    println!("Archives metadata-bumped: {}", metadata_bumped);
    println!("Total images extracted: {}", total_images);

    if !dry_run {
        println!("Total space saved: {} KB", total_bytes_saved / 1024);
        println!("\n✓ Migration complete! Original files backed up as *.bak");
    } else {
        println!("\nRun without --dry-run to perform migration");
    }

    Ok(())
}
