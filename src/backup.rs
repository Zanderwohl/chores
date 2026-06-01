//! Backup binary for copying the chores database to a backup file.
//!
//! Usage: cargo run --bin backup
//!        cargo run --bin backup -- --target my_backup.db
//!        cargo run --bin backup -- --db sqlite:other.db --target backup.db
//!
//! Creates a backup of all database entries to a new file.

mod config;
mod db;
mod migrate;
mod schedule;
mod tasks;

use anyhow::Result;
use chrono::Datelike;
use clap::Parser;
use db::{DbSchedule, DbTask};
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
    let dotenv = EnvLoader::new().load().unwrap_or_default();

    // Get source database URL
    let source_url = args
        .db
        .or_else(|| dotenv.get("DATABASE_URL").cloned())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:chores.db?mode=rwc".to_string());

    // Generate target filename
    let now = chrono::Utc::now();
    let default_target = format!(
        "backup_{}_{:02}_{:02}.db",
        now.year(),
        now.month(),
        now.day()
    );
    let target_file = args.target.unwrap_or(default_target);
    let target_url = format!("sqlite:{}?mode=rwc", target_file);

    println!("Source database: {}", source_url);
    println!("Target backup: {}", target_file);

    // Connect to source database
    println!("Connecting to source database...");
    let source_pool = db::init_db(&source_url).await?;

    // Create and connect to target database
    println!("Creating target database...");
    let target_pool = db::init_db(&target_url).await?;

    // Run migrations on target database to create tables
    let migrations_path = migrate::default_migrations_path();
    migrate::run_up(&target_pool, &migrations_path, None).await?;

    // Copy schedules
    println!("Copying schedules...");
    let schedules: Vec<DbSchedule> = sqlx::query_as("SELECT * FROM schedules")
        .fetch_all(&source_pool)
        .await?;

    for s in &schedules {
        sqlx::query(
            r#"INSERT INTO schedules (
                id, kind,
                ndays_days, ndays_time,
                nweeks_weeks, nweeks_sunday, nweeks_monday, nweeks_tuesday,
                nweeks_wednesday, nweeks_thursday, nweeks_friday, nweeks_saturday, nweeks_time,
                monthwise_days, monthwise_time,
                weeks_of_month_weeks, weeks_of_month_sunday, weeks_of_month_monday,
                weeks_of_month_tuesday, weeks_of_month_wednesday, weeks_of_month_thursday,
                weeks_of_month_friday, weeks_of_month_saturday, weeks_of_month_time,
                certain_months_months, certain_months_days, certain_months_time,
                once_datetime
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(s.id)
        .bind(&s.kind)
        .bind(s.ndays_days)
        .bind(&s.ndays_time)
        .bind(s.nweeks_weeks)
        .bind(s.nweeks_sunday)
        .bind(s.nweeks_monday)
        .bind(s.nweeks_tuesday)
        .bind(s.nweeks_wednesday)
        .bind(s.nweeks_thursday)
        .bind(s.nweeks_friday)
        .bind(s.nweeks_saturday)
        .bind(&s.nweeks_time)
        .bind(&s.monthwise_days)
        .bind(&s.monthwise_time)
        .bind(&s.weeks_of_month_weeks)
        .bind(s.weeks_of_month_sunday)
        .bind(s.weeks_of_month_monday)
        .bind(s.weeks_of_month_tuesday)
        .bind(s.weeks_of_month_wednesday)
        .bind(s.weeks_of_month_thursday)
        .bind(s.weeks_of_month_friday)
        .bind(s.weeks_of_month_saturday)
        .bind(&s.weeks_of_month_time)
        .bind(&s.certain_months_months)
        .bind(&s.certain_months_days)
        .bind(&s.certain_months_time)
        .bind(&s.once_datetime)
        .execute(&target_pool)
        .await?;
    }
    println!("  Copied {} schedules", schedules.len());

    // Copy tasks
    println!("Copying tasks...");
    let tasks: Vec<DbTask> = sqlx::query_as("SELECT * FROM tasks")
        .fetch_all(&source_pool)
        .await?;

    for t in &tasks {
        sqlx::query(
            "INSERT INTO tasks (id, name, details, schedule_id, alerting_time, completeable, created_at, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id)
        .bind(&t.name)
        .bind(&t.details)
        .bind(t.schedule_id)
        .bind(t.alerting_time)
        .bind(t.completeable)
        .bind(&t.created_at)
        .bind(&t.deleted_at)
        .execute(&target_pool)
        .await?;
    }
    println!("  Copied {} tasks", tasks.len());

    // Copy completions
    println!("Copying completions...");
    let completions: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, task_id, completed_at FROM completions")
            .fetch_all(&source_pool)
            .await?;

    for completion in &completions {
        sqlx::query("INSERT INTO completions (id, task_id, completed_at) VALUES (?, ?, ?)")
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

