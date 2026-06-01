use anyhow::Result;
use clap::{Parser, Subcommand};
use dotenvy::EnvLoader;
use sqlx::sqlite::SqlitePool;

#[path = "../migrate.rs"]
mod migrate;

#[derive(Parser, Debug)]
#[command(name = "migrate")]
#[command(about = "Database migration tool for chores")]
struct Args {
    /// Database file path (overrides DATABASE_URL from .env)
    #[arg(long)]
    db: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Apply pending migrations
    Up {
        /// Number of migrations to apply (default: all)
        #[arg(long, short = 'n')]
        steps: Option<usize>,
    },
    /// Revert applied migrations
    Down {
        /// Number of migrations to revert (default: 1)
        #[arg(long, short = 'n')]
        steps: Option<usize>,
    },
    /// Create a new migration
    New {
        /// Name for the migration (optional)
        #[arg(default_value = "")]
        name: String,
    },
    /// List all migrations and their status
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let dotenv = EnvLoader::new().load().unwrap_or_default();

    let database_file = args
        .db
        .or_else(|| dotenv.get("DATABASE_URL").cloned())
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "chores.db".to_string());

    let database_url = format!("sqlite:{}?mode=rwc", database_file);

    let migrations_path = migrate::default_migrations_path();
    let schema_path = migrate::default_schema_path();

    match args.command {
        Command::List => {
            let pool = SqlitePool::connect(&database_url).await?;
            let migrations = migrate::scan_migrations(&pool, &migrations_path).await?;

            if migrations.is_empty() {
                println!("No migrations found in {}", migrations_path.display());
            } else {
                println!("Migrations:");
                for m in migrations {
                    let status = if m.applied { "✓" } else { "○" };
                    println!("  {} {}", status, m.timestamp);
                }
            }
        }
        Command::Up { steps } => {
            let pool = SqlitePool::connect(&database_url).await?;
            let count = migrate::run_up(&pool, &migrations_path, steps).await?;

            if count == 0 {
                println!("No pending migrations.");
            } else {
                println!("Applied {} migration(s).", count);
                migrate::dump_schema(&pool, &schema_path).await?;
            }
        }
        Command::Down { steps } => {
            let pool = SqlitePool::connect(&database_url).await?;
            let count = migrate::run_down(&pool, &migrations_path, steps).await?;

            if count == 0 {
                println!("No migrations to revert.");
            } else {
                println!("Reverted {} migration(s).", count);
                migrate::dump_schema(&pool, &schema_path).await?;
            }
        }
        Command::New { name } => {
            let path = migrate::create_migration(&migrations_path, &name)?;
            println!("Created migration: {}", path.display());
            println!("  - {}/up.sql", path.display());
            println!("  - {}/down.sql", path.display());
        }
    }

    Ok(())
}
