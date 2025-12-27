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
use dotenvy::EnvLoader;

#[tokio::main]
async fn main() -> Result<()> {
    let _env = EnvLoader::new().load()?;

    // Initialize database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:chores.db?mode=rwc".to_string());
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
