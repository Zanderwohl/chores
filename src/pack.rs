//! Pack binary for storing photo files as BLOBs in the database.
//!
//! Usage: cargo run --bin pack
//!        cargo run --bin pack -- --db sqlite:other.db
//!
//! Reads all non-missing photos from disk and stores their contents in photo_blobs table.

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
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pack")]
#[command(about = "Pack photo files into the database as BLOBs")]
struct Args {
    /// Database URL (overrides DATABASE_URL from .env)
    #[arg(long)]
    db: Option<String>,

    /// Photos directory path (default: photos)
    #[arg(long, default_value = "photos")]
    photos_dir: PathBuf,
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

    println!("Connecting to database: {}", database_url);
    let pool = db::init_db(&database_url).await?;

    let migrations_path = migrate::default_migrations_path();
    let count = migrate::run_up(&pool, &migrations_path, None).await?;
    if count > 0 {
        println!("Applied {} migration(s)", count);
    }

    let photos: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, path FROM photos WHERE missing = 0")
            .fetch_all(&pool)
            .await?;

    println!("Found {} photos to pack", photos.len());

    let mut packed = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for (id, path) in photos {
        let file_path = args.photos_dir.join(&path);

        if !file_path.exists() {
            println!("  Skipping {} (file not found)", path);
            skipped += 1;
            continue;
        }

        match fs::read(&file_path) {
            Ok(data) => {
                match sqlx::query(
                    "INSERT OR REPLACE INTO photo_blobs (photo_id, data) VALUES (?, ?)",
                )
                .bind(id)
                .bind(&data)
                .execute(&pool)
                .await
                {
                    Ok(_) => {
                        packed += 1;
                        if packed % 100 == 0 {
                            println!("  Packed {} photos...", packed);
                        }
                    }
                    Err(e) => {
                        println!("  Error storing {}: {}", path, e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                println!("  Error reading {}: {}", path, e);
                errors += 1;
            }
        }
    }

    println!("\nPack complete!");
    println!("  Packed: {}", packed);
    println!("  Skipped: {}", skipped);
    println!("  Errors: {}", errors);

    Ok(())
}
