//! Shrink binary for removing photo BLOBs when files exist on disk.
//!
//! Usage: cargo run --bin shrink
//!        cargo run --bin shrink -- --db sqlite:other.db
//!
//! Deletes all photo_blobs entries for photos that are not missing (exist on disk),
//! then VACUUMs the database to reclaim space.

mod config;
mod db;
mod migrate;
mod schedule;
mod settings;
mod tasks;

use anyhow::Result;
use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(name = "shrink")]
#[command(about = "Remove photo BLOBs for files that exist on disk")]
struct Args {
    /// Database URL (overrides DATABASE_URL from .env)
    #[arg(long)]
    db: Option<String>,
}

fn get_file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.len())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let dotenv: HashMap<String, String> = dotenvy::dotenv_iter()
        .ok()
        .map(|iter| iter.filter_map(|item| item.ok()).collect())
        .unwrap_or_default();

    let database_url = args
        .db
        .or_else(|| dotenv.get("DATABASE_URL").cloned())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:chores.db?mode=rwc".to_string());

    let db_path = database_url
        .strip_prefix("sqlite:")
        .and_then(|s| s.split('?').next())
        .unwrap_or("chores.db");

    let size_before = get_file_size(Path::new(db_path));

    println!("Connecting to database: {}", database_url);
    let pool = db::init_db(&database_url).await?;

    let migrations_path = migrate::default_migrations_path();
    let count = migrate::run_up(&pool, &migrations_path, None).await?;
    if count > 0 {
        println!("Applied {} migration(s)", count);
    }

    let blob_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM photo_blobs WHERE photo_id IN (SELECT id FROM photos WHERE missing = 0)",
    )
    .fetch_one(&pool)
    .await?;

    println!(
        "Found {} blobs for photos that exist on disk",
        blob_count.0
    );

    if blob_count.0 == 0 {
        println!("Nothing to shrink.");
        return Ok(());
    }

    let result = sqlx::query(
        "DELETE FROM photo_blobs WHERE photo_id IN (SELECT id FROM photos WHERE missing = 0)",
    )
    .execute(&pool)
    .await?;

    println!("Deleted {} blob(s)", result.rows_affected());

    println!("Running VACUUM to reclaim disk space...");
    sqlx::query("VACUUM").execute(&pool).await?;

    drop(pool);

    let size_after = get_file_size(Path::new(db_path));

    println!("\nShrink complete!");
    if let (Some(before), Some(after)) = (size_before, size_after) {
        println!("  Before: {}", format_size(before));
        println!("  After:  {}", format_size(after));
        if before > after {
            println!("  Saved:  {}", format_size(before - after));
        }
    }

    Ok(())
}
