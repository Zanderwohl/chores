//! Clear binary for resetting the chores database.
//! 
//! Usage: cargo run --bin clear
//! 
//! Deletes all entries from all database tables.

mod config;
mod db;
mod schedule;
mod task;
mod tasks;

use anyhow::Result;
use dotenvy::EnvLoader;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    let dotenv = EnvLoader::new()
        .load()
        .unwrap_or_default();
    
    // Get database URL
    let database_url = dotenv.get("DATABASE_URL")
        .cloned()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:chores.db?mode=rwc".to_string());
    
    println!("Connecting to database: {}", database_url);
    
    // Initialize database connection
    let pool = db::init_db(&database_url).await?;
    
    // Clear all tables
    println!("Clearing completions table...");
    sqlx::query("DELETE FROM completions")
        .execute(&pool)
        .await?;
    
    println!("Clearing tasks table...");
    sqlx::query("DELETE FROM tasks")
        .execute(&pool)
        .await?;
    
    println!("Clearing schedules table...");
    sqlx::query("DELETE FROM schedules")
        .execute(&pool)
        .await?;
    
    println!("All tables cleared successfully!");
    
    Ok(())
}

