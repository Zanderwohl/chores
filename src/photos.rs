use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use hypertext::{prelude::*, Raw};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

use crate::db::DbPool;

// ============================================================================
// Photo Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub id: i64,
    pub path: PathBuf,
    pub missing: bool,
    pub active: bool,
    pub caption: Option<String>,
    pub config: PhotoConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhotoConfig {
    pub crop: PhotoCrop,
    pub background: PhotoBackground,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum PhotoCrop {
    #[default]
    Letterbox,
    Expand { dx: f32, dy: f32 },
    Zoom { z: f32, dx: f32, dy: f32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum PhotoBackground {
    #[default]
    Black,
    Color { r: f32, g: f32, b: f32 },
    Gaussian { r: f32 },
}

// ============================================================================
// Database Functions
// ============================================================================

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub async fn sync_photos(pool: &DbPool, photos_path: &std::path::Path) -> Result<()> {
    if !photos_path.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(photos_path).await?;
    let mut found_paths: Vec<String> = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() && is_image_file(&path) {
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                found_paths.push(filename.to_string());
            }
        }
    }

    // Get all existing photos from DB
    let existing: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT id, path, missing FROM photos"
    )
        .fetch_all(pool)
        .await?;

    let existing_paths: std::collections::HashSet<String> = existing.iter()
        .map(|(_, p, _)| p.clone())
        .collect();

    let mut new_count = 0;
    let mut newly_missing_count = 0;
    let mut recovered_count = 0;

    // Insert new photos
    for path in &found_paths {
        if !existing_paths.contains(path) {
            let default_config = serde_json::to_string(&PhotoConfig::default())?;
            sqlx::query(
                "INSERT INTO photos (path, missing, active, config) VALUES (?, 0, 1, ?)"
            )
                .bind(path)
                .bind(&default_config)
                .execute(pool)
                .await?;
            new_count += 1;
        }
    }

    // Mark missing photos
    let found_set: std::collections::HashSet<&str> = found_paths.iter()
        .map(|s| s.as_str())
        .collect();

    for (id, path, was_missing) in &existing {
        let is_missing = !found_set.contains(path.as_str());
        if is_missing && !was_missing {
            sqlx::query("UPDATE photos SET missing = 1 WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
            newly_missing_count += 1;
        } else if !is_missing && *was_missing {
            sqlx::query("UPDATE photos SET missing = 0 WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
            recovered_count += 1;
        }
    }

    // Count totals for summary
    let total_in_folder = found_paths.len();
    let total_missing: usize = existing.iter().filter(|(_, p, was_missing)| {
        let is_missing = !found_set.contains(p.as_str());
        (is_missing && !was_missing) || (*was_missing && is_missing)
    }).count();

    if new_count > 0 || newly_missing_count > 0 || recovered_count > 0 {
        info!(
            new = new_count,
            newly_missing = newly_missing_count,
            recovered = recovered_count,
            total_available = total_in_folder,
            "Photo sync complete"
        );
    }

    Ok(())
}

pub async fn get_all_photos(pool: &DbPool) -> Result<Vec<Photo>> {
    let rows: Vec<(i64, String, i32, i32, Option<String>, String)> = sqlx::query_as(
        "SELECT id, path, missing, active, caption, config FROM photos WHERE missing = 0 AND active = 1"
    )
        .fetch_all(pool)
        .await?;

    let mut photos = Vec::new();
    for (id, path, missing, active, caption, config_str) in rows {
        let config: PhotoConfig = serde_json::from_str(&config_str).unwrap_or_default();
        photos.push(Photo {
            id,
            path: PathBuf::from(path),
            missing: missing != 0,
            active: active != 0,
            caption,
            config,
        });
    }

    Ok(photos)
}

pub async fn get_photo(pool: &DbPool, id: i64) -> Result<Option<Photo>> {
    let row: Option<(i64, String, i32, i32, Option<String>, String)> = sqlx::query_as(
        "SELECT id, path, missing, active, caption, config FROM photos WHERE id = ?"
    )
        .bind(id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|(id, path, missing, active, caption, config_str)| {
        let config: PhotoConfig = serde_json::from_str(&config_str).unwrap_or_default();
        Photo {
            id,
            path: PathBuf::from(path),
            missing: missing != 0,
            active: active != 0,
            caption,
            config,
        }
    }))
}

// ============================================================================
// Photo Serving
// ============================================================================

fn is_safe_path(path: &str) -> bool {
    // Reject empty paths
    if path.is_empty() {
        return false;
    }
    // Reject paths with null bytes
    if path.contains('\0') {
        return false;
    }
    // Reject paths starting with /
    if path.starts_with('/') {
        return false;
    }
    // Reject paths containing ..
    if path.contains("..") {
        return false;
    }
    // Reject paths with backslashes (Windows path traversal)
    if path.contains('\\') {
        return false;
    }
    true
}

fn get_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

pub async fn serve_photo(Path(path): Path<String>) -> impl IntoResponse {
    if !is_safe_path(&path) {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
    }

    let file_path = PathBuf::from("photos").join(&path);
    
    match fs::read(&file_path).await {
        Ok(contents) => {
            let content_type = get_content_type(&file_path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .body(Body::from(contents))
                .unwrap()
        }
        Err(_) => {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap()
        }
    }
}

// ============================================================================
// Idle Page
// ============================================================================

#[derive(Serialize)]
struct SlideshowPhoto {
    url: String,
    caption: Option<String>,
}

pub async fn idle_page(State(pool): State<DbPool>) -> Html<String> {
    let mut photos = get_all_photos(&pool).await.unwrap_or_default();
    
    // Shuffle photos
    let mut rng = rand::rng();
    photos.shuffle(&mut rng);

    // Convert to slideshow format
    let slideshow_photos: Vec<SlideshowPhoto> = photos
        .iter()
        .map(|p| SlideshowPhoto {
            url: format!("/photos/{}", p.path.display()),
            caption: p.caption.clone(),
        })
        .collect();

    let photos_json = serde_json::to_string(&slideshow_photos).unwrap_or_else(|_| "[]".to_string());

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Idle - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script {
                    (Raw::dangerously_create(&format!("window.SLIDESHOW_PHOTOS = {};", photos_json)))
                }
            }
            body style="padding:0;margin:0;overflow:hidden;background:#000;" {
                div #idle-content {}
                script src="/static/slideshow.js" {}
            }
        }
    };

    Html(html.render().into_inner())
}
