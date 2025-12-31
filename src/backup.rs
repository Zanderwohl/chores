//! Backup binary for copying the chores database to a backup file.
//! 
//! Usage: cargo run --bin backup
//!        cargo run --bin backup -- --target my_backup.db
//!        cargo run --bin backup -- --db sqlite:other.db --target backup.db
//! 
//! Creates a backup of all database entries to a new file.

mod config;
mod db;
mod schedule;
mod task;
mod tasks;

use anyhow::Result;
use chrono::Datelike;
use clap::Parser;
use dotenvy::EnvLoader;

#[derive(Parser, Debug)]
#[command(name = "backup")]
#[command(about = "Backup the chores database to a new file")]
struct Args {
    /// Source database URL (overrides DATABASE_URL from .env)
    #[arg(long)]
    db: Option<String>,
    
    /// Target backup file path (default: backup_{year}_{month}_{day}.db)
    #[arg(long)]
    target: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Load .env file
    let dotenv = EnvLoader::new()
        .load()
        .unwrap_or_default();
    
    // Get source database URL
    let source_url = args.db
        .or_else(|| dotenv.get("DATABASE_URL").cloned())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:chores.db?mode=rwc".to_string());
    
    // Generate target filename
    let now = chrono::Utc::now();
    let default_target = format!("backup_{}_{:02}_{:02}.db", now.year(), now.month(), now.day());
    let target_file = args.target.unwrap_or(default_target);
    let target_url = format!("sqlite:{}?mode=rwc", target_file);
    
    println!("Source database: {}", source_url);
    println!("Target backup: {}", target_file);
    
    // Connect to source database
    println!("Connecting to source database...");
    let source_pool = db::init_db(&source_url).await?;
    
    // Create and connect to target database (init_db creates tables)
    println!("Creating target database...");
    let target_pool = db::init_db(&target_url).await?;
    
    // Copy schedules
    println!("Copying schedules...");
    let schedules: Vec<(i64, String, Option<i32>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = 
        sqlx::query_as(
            "SELECT id, kind, n_days, n_weeks, days_of_week, due_time, monthwise_type, monthwise_days, monthwise_week_number, monthwise_weekday, certain_months_months, once_datetime FROM schedules"
        )
        .fetch_all(&source_pool)
        .await?;
    
    for schedule in &schedules {
        sqlx::query(
            "INSERT INTO schedules (id, kind, n_days, n_weeks, days_of_week, due_time, monthwise_type, monthwise_days, monthwise_week_number, monthwise_weekday, certain_months_months, once_datetime) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(schedule.0)
        .bind(&schedule.1)
        .bind(schedule.2)
        .bind(schedule.3)
        .bind(&schedule.4)
        .bind(&schedule.5)
        .bind(&schedule.6)
        .bind(&schedule.7)
        .bind(&schedule.8)
        .bind(&schedule.9)
        .bind(&schedule.10)
        .bind(&schedule.11)
        .execute(&target_pool)
        .await?;
    }
    println!("  Copied {} schedules", schedules.len());
    
    // Copy tasks
    println!("Copying tasks...");
    let tasks: Vec<(i64, String, String, i64, String, i32, Option<String>, Option<String>)> = 
        sqlx::query_as(
            "SELECT id, name, details, schedule_id, alerting_time, completeable, created_at, deleted_at FROM tasks"
        )
        .fetch_all(&source_pool)
        .await?;
    
    for task in &tasks {
        sqlx::query(
            "INSERT INTO tasks (id, name, details, schedule_id, alerting_time, completeable, created_at, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(task.0)
        .bind(&task.1)
        .bind(&task.2)
        .bind(task.3)
        .bind(&task.4)
        .bind(task.5)
        .bind(&task.6)
        .bind(&task.7)
        .execute(&target_pool)
        .await?;
    }
    println!("  Copied {} tasks", tasks.len());
    
    // Copy completions
    println!("Copying completions...");
    let completions: Vec<(i64, String, String)> = 
        sqlx::query_as(
            "SELECT id, task_id, completed_at FROM completions"
        )
        .fetch_all(&source_pool)
        .await?;
    
    for completion in &completions {
        sqlx::query(
            "INSERT INTO completions (id, task_id, completed_at) VALUES (?, ?, ?)"
        )
        .bind(completion.0)
        .bind(&completion.1)
        .bind(&completion.2)
        .execute(&target_pool)
        .await?;
    }
    println!("  Copied {} completions", completions.len());
    
    println!("\nBackup completed successfully!");
    println!("Backup saved to: {}", target_file);
    
    Ok(())
}

