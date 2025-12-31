mod config;
mod db;
mod schedule;
mod storybook;
mod task;
mod tasks;

use axum::{routing::get, Router};
use std::fs;
use anyhow::Result;
use axum::routing::get_service;
use tower_http::services::ServeDir;
use dotenvy::{EnvLoader, EnvMap};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "chores")]
#[command(about = "A task management application")]
struct Args {
    /// Timezone identifier (e.g., "America/New_York", "Europe/London")
    /// Overrides the TZ environment variable
    #[arg(long)]
    tz: Option<String>,

    /// Enable touch mode (use larger buttons instead of links)
    /// Overrides the TOUCH environment variable
    #[arg(short = 't', long)]
    touch: bool,
}

/// Load a config value from sources in priority order:
/// 1. CLI argument (if provided)
/// 2. Process environment variable
/// 3. .env file
/// 4. Default value
fn get_config(
    key: &str,
    cli_value: Option<String>,
    dotenv: &EnvMap,
    default: &str,
) -> String {
    cli_value
        .or_else(|| std::env::var(key).ok())
        .or_else(|| dotenv.get(key).cloned())
        .unwrap_or_else(|| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file (just read, don't modify environment)
    let dotenv = EnvLoader::new()
        .load()
        .unwrap_or_default();

    // Parse CLI arguments
    let args = Args::parse();

    // Get timezone: CLI flag > env var > .env > UTC
    let tz_str = get_config("TZ", args.tz, &dotenv, "UTC");
    config::init_timezone(&tz_str);
    println!("Using timezone: {}", config::get_timezone());

    // Get touch mode: CLI flag > env var > .env > false
    let touch_enabled = if args.touch {
        true
    } else {
        let touch_str = get_config("TOUCH", None, &dotenv, "false");
        touch_str.eq_ignore_ascii_case("true") || touch_str == "1"
    };
    config::init_touch_mode(touch_enabled);
    if touch_enabled {
        println!("Touch mode: enabled");
    }

    // Get database URL: env var > .env > default
    let database_url = get_config("DATABASE_URL", None, &dotenv, "sqlite:chores.db?mode=rwc");

    // Initialize database
    let pool = db::init_db(&database_url).await?;
    println!("Database initialized at: {}", database_url);

    fs::create_dir_all("static")?;
    let static_dir = ServeDir::new("static");

    // build our application with a single route
    let app = Router::new()
        .route("/", get(tasks::homepage))
        .nest("/storybook", storybook::router())
        .nest("/tasks", tasks::router())
        .with_state(pool)
        .nest_service("/static", get_service(static_dir));

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
