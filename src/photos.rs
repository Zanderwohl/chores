use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use hypertext::{prelude::*, Raw};
use image::imageops::FilterType;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::{error, info};

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

pub async fn get_photos_paginated(pool: &DbPool, page: i64, per_page: i64) -> Result<Vec<Photo>> {
    let offset = (page - 1) * per_page;
    let rows: Vec<(i64, String, i32, i32, Option<String>, String)> = sqlx::query_as(
        "SELECT id, path, missing, active, caption, config FROM photos ORDER BY path LIMIT ? OFFSET ?"
    )
        .bind(per_page)
        .bind(offset)
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

pub async fn get_photo_count(pool: &DbPool) -> Result<i64> {
    let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await?;
    Ok(result.0)
}

pub async fn set_photo_active(pool: &DbPool, id: i64, active: bool) -> Result<()> {
    sqlx::query("UPDATE photos SET active = ? WHERE id = ?")
        .bind(active as i32)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
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
// Thumbnail Generation
// ============================================================================

fn ensure_thumbnail_sync(photo_path: &str) -> Result<PathBuf> {
    let thumb_filename = format!("{}.png", photo_path);
    let thumb_path = PathBuf::from("thumbnails").join(&thumb_filename);
    
    if thumb_path.exists() {
        return Ok(thumb_path);
    }

    let source_path = PathBuf::from("photos").join(photo_path);
    let img = image::open(&source_path)?;
    
    let thumbnail = img.resize(256, 256, FilterType::Lanczos3);
    
    if let Some(parent) = thumb_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    thumbnail.save(&thumb_path)?;
    
    Ok(thumb_path)
}

pub async fn serve_thumbnail(Path(path): Path<String>) -> impl IntoResponse {
    if !is_safe_path(&path) {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
    }

    let path_clone = path.clone();
    let thumb_result = tokio::task::spawn_blocking(move || {
        ensure_thumbnail_sync(&path_clone)
    }).await;

    let thumb_path = match thumb_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            error!(path = %path, error = %e, "Failed to generate thumbnail");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap();
        }
        Err(e) => {
            error!(path = %path, error = %e, "Thumbnail task panicked");
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap();
        }
    };

    match fs::read(&thumb_path).await {
        Ok(contents) => {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
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

// ============================================================================
// Photos Management Pages
// ============================================================================

#[derive(Deserialize)]
pub struct PhotosListQuery {
    page: Option<i64>,
    per_page: Option<i64>,
}

pub async fn photos_index(State(_pool): State<DbPool>) -> Html<String> {
    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Photos - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="/static/htmx.min.js" {}
            }
            body {
                div .photos-page {
                    div .photos-page-header {
                        a href="/" { "← Back" }
                    }
                    h1 { "Photos" }
                    (Raw::dangerously_create(r##"<div id="photo-list" hx-get="/photos/list?page=1" hx-trigger="load" hx-swap="innerHTML"><p>Loading...</p></div>"##))
                }
            }
        }
    };

    Html(html.render().into_inner())
}

pub async fn photos_list(
    State(pool): State<DbPool>,
    Query(query): Query<PhotosListQuery>,
) -> Html<String> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).max(1).min(100);

    let total_count = get_photo_count(&pool).await.unwrap_or(0);
    let total_pages = (total_count as f64 / per_page as f64).ceil() as i64;
    let total_pages = total_pages.max(1);

    let photos = get_photos_paginated(&pool, page, per_page).await.unwrap_or_default();

    if photos.is_empty() && total_count == 0 {
        return Html(maud! {
            div .empty-list {
                p { "No photos yet. Add images to the photos folder." }
            }
        }.render().into_inner());
    }

    let mut items_html = String::new();
    for photo in &photos {
        items_html.push_str(&render_photo_list_item(photo));
    }

    let pagination_html = render_photo_pagination(page, total_pages, per_page, total_count);

    Html(maud! {
        ul .photo-list {
            (Raw::dangerously_create(&items_html))
        }
        (Raw::dangerously_create(&pagination_html))
    }.render().into_inner())
}

fn render_photo_list_item(photo: &Photo) -> String {
    let filename = photo.path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let thumb_url = format!("/thumbnails/{}", photo.path.display());
    let edit_url = format!("/photo/{}/edit", photo.id);
    let toggle_url = format!("/photo/{}/toggle-active", photo.id);
    let checked = if photo.active { "checked" } else { "" };
    let caption_display = photo.caption.as_deref().unwrap_or("");

    format!(
        r##"<li class="photo-list-item" id="photo-row-{}">
            <img class="photo-thumbnail" src="{}" alt="{}">
            <input type="checkbox" {} hx-post="{}" hx-target="#photo-row-{}" hx-swap="outerHTML">
            <span class="photo-filename">{}</span>
            <span class="photo-caption">{}</span>
            <a class="btn" href="{}">Edit</a>
        </li>"##,
        photo.id, thumb_url, filename, checked, toggle_url, photo.id, filename, caption_display, edit_url
    )
}

fn render_photo_pagination(current_page: i64, total_pages: i64, per_page: i64, total_count: i64) -> String {
    if total_pages <= 1 {
        return String::new();
    }

    let start = ((current_page - 1) * per_page + 1).min(total_count);
    let end = (current_page * per_page).min(total_count);

    let mut page_links = String::new();
    for p in 1..=total_pages {
        if p == current_page {
            page_links.push_str(&format!(
                r#"<span class="pagination-page pagination-current">{}</span>"#,
                p
            ));
        } else {
            page_links.push_str(&format!(
                r##"<button class="btn pagination-page" hx-get="/photos/list?page={}&amp;per_page={}" hx-target="#photo-list" hx-swap="innerHTML">{}</button>"##,
                p, per_page, p
            ));
        }
    }

    let first_btn = if current_page > 1 {
        format!(
            r##"<button class="btn pagination-btn" hx-get="/photos/list?page=1&amp;per_page={}" hx-target="#photo-list" hx-swap="innerHTML">«</button>"##,
            per_page
        )
    } else {
        r#"<button class="btn pagination-btn" disabled>«</button>"#.to_string()
    };

    let prev_btn = if current_page > 1 {
        format!(
            r##"<button class="btn pagination-btn" hx-get="/photos/list?page={}&amp;per_page={}" hx-target="#photo-list" hx-swap="innerHTML">‹</button>"##,
            current_page - 1, per_page
        )
    } else {
        r#"<button class="btn pagination-btn" disabled>‹</button>"#.to_string()
    };

    let next_btn = if current_page < total_pages {
        format!(
            r##"<button class="btn pagination-btn" hx-get="/photos/list?page={}&amp;per_page={}" hx-target="#photo-list" hx-swap="innerHTML">›</button>"##,
            current_page + 1, per_page
        )
    } else {
        r#"<button class="btn pagination-btn" disabled>›</button>"#.to_string()
    };

    let last_btn = if current_page < total_pages {
        format!(
            r##"<button class="btn pagination-btn" hx-get="/photos/list?page={}&amp;per_page={}" hx-target="#photo-list" hx-swap="innerHTML">»</button>"##,
            total_pages, per_page
        )
    } else {
        r#"<button class="btn pagination-btn" disabled>»</button>"#.to_string()
    };

    format!(
        r#"<div class="pagination">
            <div class="pagination-info">Showing {}-{} of {}</div>
            <div class="pagination-controls">
                {}
                {}
                {}
                {}
                {}
            </div>
        </div>"#,
        start, end, total_count, first_btn, prev_btn, page_links, next_btn, last_btn
    )
}

pub async fn toggle_active(
    State(pool): State<DbPool>,
    Path(id): Path<i64>,
) -> Html<String> {
    let photo = match get_photo(&pool, id).await {
        Ok(Some(p)) => p,
        _ => {
            return Html("<li class=\"photo-list-item\">Photo not found</li>".to_string());
        }
    };

    let new_active = !photo.active;
    if let Err(e) = set_photo_active(&pool, id, new_active).await {
        error!(id = id, error = %e, "Failed to toggle photo active state");
    }

    let updated_photo = Photo {
        active: new_active,
        ..photo
    };

    Html(render_photo_list_item(&updated_photo))
}

pub async fn photo_edit(
    State(pool): State<DbPool>,
    Path(id): Path<i64>,
) -> Html<String> {
    let photo = match get_photo(&pool, id).await {
        Ok(Some(p)) => p,
        _ => {
            return Html(maud! {
                !DOCTYPE
                html {
                    head {
                        meta charset="utf-8";
                        title { "Photo Not Found - Chores" }
                        link rel="stylesheet" href="/static/system.css";
                        link rel="stylesheet" href="/static/app.css";
                    }
                    body {
                        div .photo-edit-page {
                            h1 { "Photo Not Found" }
                            a href="/photos" { "← Back to Photos" }
                        }
                    }
                }
            }.render().into_inner());
        }
    };

    let photo_url = format!("/photos/{}", photo.path.display());
    let filename = photo.path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (filename) " - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
            }
            body {
                div .photo-edit-page {
                    div .photo-edit-header {
                        a .btn href="/photos" { "← Back to Photos" }
                    }
                    h1 { (filename) }
                    div .photo-edit-content {
                        img .photo-edit-image src=(photo_url) alt=(filename);
                    }
                }
            }
        }
    };

    Html(html.render().into_inner())
}
