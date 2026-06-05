//! Unpack binary for restoring photo files from database BLOBs.
//!
//! Usage: cargo run --bin unpack
//!        cargo run --bin unpack -- --db sqlite:other.db
//!        cargo run --bin unpack -- --force
//!
//! Extracts all photo BLOBs from photo_blobs table and writes them to the photos folder.

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
#[command(name = "unpack")]
#[command(about = "Unpack photo BLOBs from the database to disk")]
struct Args {
    /// Database URL (overrides DATABASE_URL from .env)
    #[arg(long)]
    db: Option<String>,

    /// Photos directory path (default: photos)
    #[arg(long, default_value = "photos")]
    photos_dir: PathBuf,

    /// Overwrite existing files on disk
    #[arg(long, default_value = "false")]
    force: bool,
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

    fs::create_dir_all(&args.photos_dir)?;

    let blobs: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT pb.photo_id, p.path, pb.data FROM photo_blobs pb JOIN photos p ON p.id = pb.photo_id",
    )
    .fetch_all(&pool)
    .await?;

    println!("Found {} photo blobs to unpack", blobs.len());

    let mut unpacked = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for (photo_id, path, data) in blobs {
        let file_path = args.photos_dir.join(&path);

        if file_path.exists() && !args.force {
            skipped += 1;
            continue;
        }

        match fs::write(&file_path, &data) {
            Ok(_) => {
                match sqlx::query("UPDATE photos SET missing = 0 WHERE id = ?")
                    .bind(photo_id)
                    .execute(&pool)
                    .await
                {
                    Ok(_) => {
                        unpacked += 1;
                        if unpacked % 100 == 0 {
                            println!("  Unpacked {} photos...", unpacked);
                        }
                    }
                    Err(e) => {
                        println!("  Error updating missing flag for {}: {}", path, e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                println!("  Error writing {}: {}", path, e);
                errors += 1;
            }
        }
    }

    println!("\nUnpack complete!");
    println!("  Unpacked: {}", unpacked);
    println!("  Skipped (already exist): {}", skipped);
    println!("  Errors: {}", errors);

    Ok(())
}
