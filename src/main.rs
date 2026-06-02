mod config;
mod db;
mod migrate;
mod photos;
mod schedule;
pub mod settings;
mod storybook;
mod tasks;

use anyhow::Result;
use axum::{routing::{get, post}, Router};
use axum::routing::get_service;
use clap::Parser;
use dotenvy::{EnvLoader, EnvMap};
use std::fs;
use std::path::Path;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

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

    /// Port to bind the server to (default: 3000)
    /// Overrides the PORT environment variable
    #[arg(short = 'p', long)]
    port: Option<u16>,

    /// Automatically run pending migrations on startup (default: true)
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    automigrate: bool,
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
    // Set up dual-drain logging: console + rolling file
    fs::create_dir_all("logs")?;
    let file_appender = tracing_appender::rolling::daily("logs", "chores.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stdout)
                .with_ansi(true),
        )
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false),
        )
        .with(tracing_subscriber::filter::LevelFilter::from_level(Level::INFO))
        .init();

    // Load .env file (just read, don't modify environment)
    let dotenv = EnvLoader::new()
        .load()
        .unwrap_or_default();

    // Parse CLI arguments
    let args = Args::parse();

    // Get timezone: CLI flag > env var > .env > UTC
    let tz_str = get_config("TZ", args.tz, &dotenv, "UTC");
    config::init_timezone(&tz_str);
    info!("Using timezone: {}", config::get_timezone());

    // Get touch mode: CLI flag > env var > .env > false
    let touch_enabled = if args.touch {
        true
    } else {
        let touch_str = get_config("TOUCH", None, &dotenv, "false");
        touch_str.eq_ignore_ascii_case("true") || touch_str == "1"
    };
    config::init_touch_mode(touch_enabled);
    if touch_enabled {
        info!("Touch mode: enabled");
    }

    // Get database URL: env var > .env > default
    let database_url = get_config("DATABASE_URL", None, &dotenv, "chores.db");
    let database_url = format!("sqlite:{}?mode=rwc", database_url);

    // Initialize database connection
    let pool = db::init_db(&database_url).await?;
    info!("Database initialized at: {}", database_url);

    // Run migrations if automigrate is enabled
    if args.automigrate {
        let migrations_path = migrate::default_migrations_path();
        let count = migrate::run_up(&pool, &migrations_path, None).await?;
        if count > 0 {
            info!("Applied {} migration(s)", count);
        }
    }

    // Create photos and thumbnails folders, sync photos
    fs::create_dir_all("photos")?;
    fs::create_dir_all("thumbnails")?;
    let photos_path = Path::new("photos");
    photos::sync_photos(&pool, photos_path).await?;

    fs::create_dir_all("static")?;
    let static_dir = ServeDir::new("static");

    // build our application with a single route
    let app = Router::new()
        .route("/", get(tasks::homepage))
        .route("/idle", get(photos::idle_page))
        .route("/photos", get(photos::photos_index))
        .route("/photos/list", get(photos::photos_list))
        .route("/photos/upload", get(photos::upload_page).post(photos::upload_photo))
        .route("/photos/upload/check", get(photos::upload_check))
        .route("/photos/{*path}", get(photos::serve_photo))
        .route("/photo/{id}", get(photos::photo_show))
        .route("/photo/{id}/edit", get(photos::photo_edit))
        .route("/photo/{id}/config", post(photos::save_photo_config))
        .route("/photo/{id}/toggle-active", post(photos::toggle_active))
        .route("/photo/crop-controls", get(photos::crop_controls))
        .route("/photo/background-controls", get(photos::background_controls))
        .route("/thumbnails/{*path}", get(photos::serve_thumbnail))
        .route("/daily", get(tasks::daily_today))
        .route("/daily/{year}/{month}/{day}", get(tasks::daily_page))
        .route("/calendar", get(tasks::calendar_today))
        .route("/calendar/{year}/{month}", get(tasks::calendar_page))
        .route("/settings", get(settings::settings_page).post(settings::save_settings))
        .nest("/storybook", storybook::router())
        .nest("/tasks", tasks::router())
        .with_state(pool)
        .nest_service("/static", get_service(static_dir))
        .layer(TraceLayer::new_for_http());

    // Get port: CLI flag > env var > .env > 3000
    let port: u16 = args.port.unwrap_or_else(|| {
        get_config("PORT", None, &dotenv, "3000")
            .parse()
            .unwrap_or(3000)
    });

    // run our app with hyper, listening globally on the configured port
    let bind_addr = format!("0.0.0.0:{}", port);
    info!("Listening on http://{}", bind_addr);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
