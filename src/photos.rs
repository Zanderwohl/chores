use anyhow::Result;
use axum::{
    body::Body,
    extract::{Form, Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use hypertext::{prelude::*, Raw};
use image::imageops::FilterType;
use image::DynamicImage;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::{error, info};

use crate::db::DbPool;
use crate::settings;

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn js_string_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('<', "\\x3c")
}

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
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhotoConfig {
    pub crop: PhotoCrop,
    pub background: PhotoBackground,
    #[serde(default)]
    pub caption_location: CaptionLocation,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CaptionLocation {
    #[default]
    Left,
    Center,
    Right,
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

    // Get all tags for these photos in one query
    let photo_ids: Vec<i64> = rows.iter().map(|(id, ..)| *id).collect();
    let tags_map = get_tags_for_photos(pool, &photo_ids).await.unwrap_or_default();

    let mut photos = Vec::new();
    for (id, path, missing, active, caption, config_str) in rows {
        let config: PhotoConfig = serde_json::from_str(&config_str).unwrap_or_default();
        let tags = tags_map.get(&id).cloned().unwrap_or_default();
        photos.push(Photo {
            id,
            path: PathBuf::from(path),
            missing: missing != 0,
            active: active != 0,
            caption,
            config,
            tags,
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

    match row {
        Some((id, path, missing, active, caption, config_str)) => {
            let config: PhotoConfig = serde_json::from_str(&config_str).unwrap_or_default();
            let tags = get_photo_tags(pool, id).await.unwrap_or_default();
            Ok(Some(Photo {
                id,
                path: PathBuf::from(path),
                missing: missing != 0,
                active: active != 0,
                caption,
                config,
                tags,
            }))
        }
        None => Ok(None),
    }
}

pub async fn get_photos_paginated(pool: &DbPool, page: i64, per_page: i64) -> Result<Vec<Photo>> {
    let offset = (page - 1) * per_page;
    let rows: Vec<(i64, String, i32, i32, Option<String>, String)> = sqlx::query_as(
        "SELECT id, path, missing, active, caption, config FROM photos ORDER BY id LIMIT ? OFFSET ?"
    )
        .bind(per_page)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    // Get all tags for these photos in one query
    let photo_ids: Vec<i64> = rows.iter().map(|(id, ..)| *id).collect();
    let tags_map = get_tags_for_photos(pool, &photo_ids).await.unwrap_or_default();

    let mut photos = Vec::new();
    for (id, path, missing, active, caption, config_str) in rows {
        let config: PhotoConfig = serde_json::from_str(&config_str).unwrap_or_default();
        let tags = tags_map.get(&id).cloned().unwrap_or_default();
        photos.push(Photo {
            id,
            path: PathBuf::from(path),
            missing: missing != 0,
            active: active != 0,
            caption,
            config,
            tags,
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

pub async fn get_adjacent_photo_ids(pool: &DbPool, id: i64) -> Result<(Option<i64>, Option<i64>)> {
    let prev: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM photos WHERE id < ? ORDER BY id DESC LIMIT 1"
    )
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let next: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM photos WHERE id > ? ORDER BY id ASC LIMIT 1"
    )
        .bind(id)
        .fetch_optional(pool)
        .await?;

    Ok((prev.map(|(pid,)| pid), next.map(|(nid,)| nid)))
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
// Tag Functions
// ============================================================================

pub async fn get_photo_tags(pool: &DbPool, photo_id: i64) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT t.name FROM tags t 
         INNER JOIN photo_tags pt ON t.id = pt.tag_id 
         WHERE pt.photo_id = ? 
         ORDER BY t.name"
    )
        .bind(photo_id)
        .fetch_all(pool)
        .await?;
    
    Ok(rows.into_iter().map(|(name,)| name).collect())
}

async fn get_tags_for_photos(pool: &DbPool, photo_ids: &[i64]) -> Result<std::collections::HashMap<i64, Vec<String>>> {
    use std::collections::HashMap;
    
    if photo_ids.is_empty() {
        return Ok(HashMap::new());
    }
    
    let placeholders = photo_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT pt.photo_id, t.name FROM tags t 
         INNER JOIN photo_tags pt ON t.id = pt.tag_id 
         WHERE pt.photo_id IN ({}) 
         ORDER BY pt.photo_id, t.name",
        placeholders
    );
    
    let mut query_builder = sqlx::query_as::<_, (i64, String)>(&query);
    for id in photo_ids {
        query_builder = query_builder.bind(id);
    }
    let rows: Vec<(i64, String)> = query_builder.fetch_all(pool).await?;
    
    let mut map: HashMap<i64, Vec<String>> = HashMap::new();
    for (photo_id, tag_name) in rows {
        map.entry(photo_id).or_default().push(tag_name);
    }
    
    Ok(map)
}

pub async fn save_photo_tags(pool: &DbPool, photo_id: i64, tags: &[String]) -> Result<()> {
    let mut tx = pool.begin().await?;
    
    // Delete existing photo-tag relations
    sqlx::query("DELETE FROM photo_tags WHERE photo_id = ?")
        .bind(photo_id)
        .execute(&mut *tx)
        .await?;
    
    if tags.is_empty() {
        tx.commit().await?;
        return Ok(());
    }
    
    // Normalize tags: trim, lowercase, deduplicate
    let normalized_tags: Vec<String> = tags
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    
    if normalized_tags.is_empty() {
        tx.commit().await?;
        return Ok(());
    }
    
    // Insert new tags that don't exist (using INSERT OR IGNORE for efficiency)
    for tag in &normalized_tags {
        sqlx::query("INSERT OR IGNORE INTO tags (name) VALUES (?)")
            .bind(tag)
            .execute(&mut *tx)
            .await?;
    }
    
    // Get tag IDs for all normalized tags
    let placeholders = normalized_tags.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT id, name FROM tags WHERE name IN ({})",
        placeholders
    );
    
    let mut query_builder = sqlx::query_as::<_, (i64, String)>(&query);
    for tag in &normalized_tags {
        query_builder = query_builder.bind(tag);
    }
    let tag_rows: Vec<(i64, String)> = query_builder.fetch_all(&mut *tx).await?;
    
    // Insert photo-tag relations
    for (tag_id, _) in tag_rows {
        sqlx::query("INSERT INTO photo_tags (photo_id, tag_id) VALUES (?, ?)")
            .bind(photo_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }
    
    tx.commit().await?;
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

fn read_exif_orientation(path: &std::path::Path) -> Option<u32> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf_reader = std::io::BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut buf_reader).ok()?;
    let orientation = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    orientation.value.get_uint(0)
}

fn apply_exif_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn ensure_thumbnail_sync(photo_path: &str) -> Result<PathBuf> {
    let thumb_filename = format!("{}.png", photo_path);
    let thumb_path = PathBuf::from("thumbnails").join(&thumb_filename);
    
    if thumb_path.exists() {
        return Ok(thumb_path);
    }

    let source_path = PathBuf::from("photos").join(photo_path);
    let orientation = read_exif_orientation(&source_path).unwrap_or(1);
    let img = image::open(&source_path)?;
    let img = apply_exif_orientation(img, orientation);
    
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

const BATCH_SIZE: usize = 10;

#[derive(Serialize)]
pub struct SlideshowPhoto {
    id: i64,
    url: String,
    caption: Option<String>,
    config: PhotoConfig,
}

/// Pick up to `count` random photos matching the current time-of-day tag set.
/// `exclude` drops a specific photo id (used to avoid an immediate repeat at a
/// batch seam when a tag set has very few photos).
async fn pick_slideshow_photos(
    pool: &DbPool,
    filter_tags: &[String],
    count: usize,
    exclude: Option<i64>,
) -> Vec<SlideshowPhoto> {
    let mut photos = get_all_photos(pool).await.unwrap_or_default();

    // Filter by tags if configured
    if !filter_tags.is_empty() {
        photos.retain(|p| {
            p.tags.iter().any(|tag| filter_tags.contains(&tag.to_lowercase()))
        });
    }

    if let Some(exclude_id) = exclude {
        photos.retain(|p| p.id != exclude_id);
    }

    let mut rng = rand::rng();
    photos.shuffle(&mut rng);

    photos
        .into_iter()
        .take(count)
        .map(|p| SlideshowPhoto {
            id: p.id,
            url: format!("/photos/{}", p.path.display()),
            caption: p.caption,
            config: p.config,
        })
        .collect()
}

#[derive(Deserialize)]
pub struct IdleQuery {
    #[serde(rename = "return")]
    return_to: Option<String>,
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
}

pub async fn idle_page(
    State(pool): State<DbPool>,
    Query(query): Query<IdleQuery>,
    headers: HeaderMap,
) -> Html<String> {
    let user_settings = settings::read_settings(&headers);
    let filter_tags = user_settings.parsed_tags();

    // Send only a small first batch; the client refills from /idle/photos as it
    // approaches the end, which also lets the set drift as the time of day changes.
    let slideshow_photos = pick_slideshow_photos(&pool, &filter_tags, BATCH_SIZE, None).await;

    let photos_json = serde_json::to_string(&slideshow_photos).unwrap_or_else(|_| "[]".to_string());
    
    // Build return URL based on query params
    let return_url = match query.return_to.as_deref() {
        Some("daily") => {
            if let (Some(y), Some(m), Some(d)) = (query.year, query.month, query.day) {
                format!("/daily/{}/{}/{}", y, m, d)
            } else {
                "/daily".to_string()
            }
        }
        Some("calendar") => {
            if let (Some(y), Some(m)) = (query.year, query.month) {
                format!("/calendar/{}/{}", y, m)
            } else {
                "/calendar".to_string()
            }
        }
        _ => "/".to_string(),
    };

    // Display time in milliseconds (or null for default)
    let display_time_js = match user_settings.display_time {
        Some(seconds) => format!("{}", seconds as u32 * 1000),
        None => "null".to_string(),
    };

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
                    (Raw::dangerously_create(&format!(
                        "window.SLIDESHOW_PHOTOS = {};\nwindow.RETURN_URL = \"{}\";\nwindow.SLIDESHOW_DISPLAY_TIME = {};\nwindow.SLIDESHOW_FILTER_TAGS = {};",
                        photos_json, return_url, display_time_js,
                        serde_json::to_string(&filter_tags).unwrap_or_else(|_| "[]".to_string())
                    )))
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

#[derive(Deserialize)]
pub struct IdlePhotosQuery {
    count: Option<usize>,
    exclude: Option<i64>,
}

/// JSON refill endpoint for the slideshow. Re-evaluates the time-of-day tag set
/// on every call (via `parsed_tags`), so a long-running slideshow drifts into
/// evening/night photos as real time passes.
pub async fn idle_photos(
    State(pool): State<DbPool>,
    Query(q): Query<IdlePhotosQuery>,
    headers: HeaderMap,
) -> Json<Vec<SlideshowPhoto>> {
    let user_settings = settings::read_settings(&headers);
    let filter_tags = user_settings.parsed_tags();
    let count = q.count.unwrap_or(BATCH_SIZE).min(50);
    Json(pick_slideshow_photos(&pool, &filter_tags, count, q.exclude).await)
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
                script src="/static/auto-sleep.js" {}
            }
            body {
                div .photos-page {
                    div .photos-page-header {
                        a href="/" { "← Back" }
                    }
                    div .page-header {
                        h1 { "Photos" }
                        div .page-header-buttons {
                            a .btn href="/photos/upload" { "Upload" }
                        }
                    }
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
    let caption_display = html_escape(photo.caption.as_deref().unwrap_or(""));

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

pub async fn photo_show(
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
                        div .photo-show-page {
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
                script src="/static/auto-sleep.js" {}
            }
            body {
                div .photo-show-page {
                    div .photo-show-header {
                        a .btn href="/photos" { "← Back to Photos" }
                    }
                    h1 { (filename) }
                    div .photo-show-content {
                        img .photo-show-image src=(photo_url) alt=(filename);
                    }
                }
            }
        }
    };

    Html(html.render().into_inner())
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

    let config_json = serde_json::to_string(&photo.config).unwrap_or_else(|_| "{}".to_string());
    
    // Extract current crop values
    let (crop_type, crop_dx, crop_dy, crop_z) = match &photo.config.crop {
        PhotoCrop::Letterbox => ("letterbox", 0.0, 0.0, 1.0),
        PhotoCrop::Expand { dx, dy } => ("expand", *dx, *dy, 1.0),
        PhotoCrop::Zoom { z, dx, dy } => ("zoom", *dx, *dy, *z),
    };
    
    // Extract current background values
    let (bg_type, bg_r, bg_g, bg_b, bg_blur_r) = match &photo.config.background {
        PhotoBackground::Black => ("black", 0.0, 0.0, 0.0, 0.0),
        PhotoBackground::Color { r, g, b } => ("color", *r, *g, *b, 0.0),
        PhotoBackground::Gaussian { r } => ("gaussian", 0.0, 0.0, 0.0, *r),
    };
    
    // Extract current caption location
    let caption_location = match &photo.config.caption_location {
        CaptionLocation::Left => "left",
        CaptionLocation::Center => "center",
        CaptionLocation::Right => "right",
    };

    let save_url = format!("/photo/{}/config", id);
    
    // Get adjacent photo IDs for navigation
    let (prev_id, next_id) = get_adjacent_photo_ids(&pool, id).await.unwrap_or((None, None));
    
    let caption_raw = photo.caption.as_deref().unwrap_or("");
    let caption_html = html_escape(caption_raw);
    let caption_js = js_string_escape(caption_raw);
    
    let tags_str = photo.tags.join(", ");
    let tags_html = html_escape(&tags_str);
    
    // Determine which sliders should be disabled
    let zoom_disabled = if crop_type != "zoom" { "disabled" } else { "" };
    let expand_disabled = if crop_type == "letterbox" { "disabled" } else { "" };
    let color_disabled = if bg_type != "color" { "disabled" } else { "" };
    let gaussian_disabled = if bg_type != "gaussian" { "disabled" } else { "" };

    let html = format!(
        r##"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{filename} - Edit - Chores</title>
    <link rel="stylesheet" href="/static/system.css">
    <link rel="stylesheet" href="/static/app.css">
    <script>
        window.PHOTO_URL = "{photo_url}";
        window.PHOTO_CONFIG = {config_json};
        window.PHOTO_CAPTION = "{caption_js}";
    </script>
    <script src="/static/auto-sleep.js"></script>
</head>
<body>
    <button class="gutter-nav gutter-nav-left btn btn-default" id="nav-prev" onclick="navigateTo('prev')" {prev_disabled}>◀</button>
    <button class="gutter-nav gutter-nav-right btn btn-default" id="nav-next" onclick="navigateTo('next')" {next_disabled}>▶</button>
    <div class="photo-edit-page">
        <div class="photo-edit-header">
            <a class="btn" href="/photos">← Back to Photos</a>
        </div>
        <h1>{filename}</h1>
        
        <div class="photo-preview-container">
            <canvas id="preview-canvas" class="photo-preview-canvas" width="960" height="540"></canvas>
        </div>
        
        <form id="photo-config-form" method="post" action="{save_url}">
            <input type="hidden" id="redirect_to" name="redirect_to" value="">
            <div class="photo-controls">
                <div class="control-section">
                    <h3>Caption &amp; Tags</h3>
                    <div class="input-group">
                        <label for="caption">Caption:</label>
                        <input type="text" id="caption" name="caption" value="{caption_html}" class="caption-input" oninput="updatePreview()">
                    </div>
                    <div class="caption-location-group">
                        <span class="caption-location-label">Position:</span>
                        <div class="radio-group-horizontal">
                            <input type="radio" id="caption_left" name="caption_location" value="left" {caption_left_checked} onchange="updatePreview()">
                            <label for="caption_left">Left</label>
                            <input type="radio" id="caption_center" name="caption_location" value="center" {caption_center_checked} onchange="updatePreview()">
                            <label for="caption_center">Center</label>
                            <input type="radio" id="caption_right" name="caption_location" value="right" {caption_right_checked} onchange="updatePreview()">
                            <label for="caption_right">Right</label>
                        </div>
                    </div>
                    <div class="input-group" style="margin-top: 12px;">
                        <label for="tags">Tags:</label>
                        <input type="text" id="tags" name="tags" value="{tags_html}" class="caption-input" placeholder="tag1, tag2, tag3">
                    </div>
                </div>
                
                <div class="control-section">
                    <h3>Crop</h3>
                    <div class="radio-group-vertical">
                        <div class="field-row">
                            <input type="radio" id="crop_letterbox" name="crop_type" value="letterbox" {letterbox_checked} onchange="updateCropControls(); updatePreview()">
                            <label for="crop_letterbox">Letterbox</label>
                        </div>
                        <div class="field-row">
                            <input type="radio" id="crop_expand" name="crop_type" value="expand" {expand_checked} onchange="updateCropControls(); updatePreview()">
                            <label for="crop_expand">Expand</label>
                        </div>
                        <div class="field-row">
                            <input type="radio" id="crop_zoom" name="crop_type" value="zoom" {zoom_checked} onchange="updateCropControls(); updatePreview()">
                            <label for="crop_zoom">Zoom</label>
                        </div>
                    </div>
                    <div class="slider-params">
                        <div class="slider-group" id="crop_z_group">
                            <label for="crop_z">Zoom:</label>
                            <input type="range" id="crop_z" name="crop_z" min="1" max="3" step="0.01" value="{crop_z}" {zoom_disabled} oninput="updateCropControls(); updatePreview()">
                            <span id="crop_z_val">{crop_z:.2}</span>
                        </div>
                        <div class="slider-group" id="crop_dx_group">
                            <label for="crop_dx">X Offset:</label>
                            <input type="range" id="crop_dx" name="crop_dx" min="-1" max="1" step="0.01" value="{crop_dx}" {expand_disabled} oninput="updatePreview()">
                            <span id="crop_dx_val">{crop_dx:.2}</span>
                        </div>
                        <div class="slider-group" id="crop_dy_group">
                            <label for="crop_dy">Y Offset:</label>
                            <input type="range" id="crop_dy" name="crop_dy" min="-1" max="1" step="0.01" value="{crop_dy}" {expand_disabled} oninput="updatePreview()">
                            <span id="crop_dy_val">{crop_dy:.2}</span>
                        </div>
                    </div>
                </div>
                
                <div class="control-section">
                    <h3>Background</h3>
                    <div class="radio-group-vertical">
                        <div class="field-row">
                            <input type="radio" id="bg_black" name="bg_type" value="black" {black_checked} onchange="updateBgControls(); updatePreview()">
                            <label for="bg_black">Black</label>
                        </div>
                        <div class="field-row">
                            <input type="radio" id="bg_color" name="bg_type" value="color" {color_checked} onchange="updateBgControls(); updatePreview()">
                            <label for="bg_color">Color</label>
                        </div>
                        <div class="field-row">
                            <input type="radio" id="bg_gaussian" name="bg_type" value="gaussian" {gaussian_checked} onchange="updateBgControls(); updatePreview()">
                            <label for="bg_gaussian">Gaussian Blur</label>
                        </div>
                    </div>
                    <div class="slider-params">
                        <div class="slider-group" id="bg_r_group">
                            <label for="bg_r">Red:</label>
                            <input type="range" id="bg_r" name="bg_r" min="0" max="255" step="1" value="{bg_r}" {color_disabled} oninput="updatePreview()">
                            <span id="bg_r_val">{bg_r:.0}</span>
                        </div>
                        <div class="slider-group" id="bg_g_group">
                            <label for="bg_g">Green:</label>
                            <input type="range" id="bg_g" name="bg_g" min="0" max="255" step="1" value="{bg_g}" {color_disabled} oninput="updatePreview()">
                            <span id="bg_g_val">{bg_g:.0}</span>
                        </div>
                        <div class="slider-group" id="bg_b_group">
                            <label for="bg_b">Blue:</label>
                            <input type="range" id="bg_b" name="bg_b" min="0" max="255" step="1" value="{bg_b}" {color_disabled} oninput="updatePreview()">
                            <span id="bg_b_val">{bg_b:.0}</span>
                        </div>
                        <div class="slider-group" id="bg_blur_r_group">
                            <label for="bg_blur_r">Blur:</label>
                            <input type="range" id="bg_blur_r" name="bg_blur_r" min="0" max="50" step="1" value="{bg_blur_r}" {gaussian_disabled} oninput="updatePreview()">
                            <span id="bg_blur_r_val">{bg_blur_r:.0}</span>
                        </div>
                    </div>
                </div>
                
                <div class="control-actions">
                    <button type="submit" class="btn btn-default">Save</button>
                    <a class="btn" href="/photo/{id}">Discard</a>
                </div>
            </div>
        </form>
    </div>
    <script src="/static/photo-editor.js"></script>
    <script>
        window.PREV_PHOTO_ID = {prev_id_js};
        window.NEXT_PHOTO_ID = {next_id_js};
        function navigateTo(dir) {{
            var targetId = (dir === 'prev') ? window.PREV_PHOTO_ID : window.NEXT_PHOTO_ID;
            if (targetId === null) return;
            document.getElementById('redirect_to').value = '/photo/' + targetId + '/edit';
            document.getElementById('photo-config-form').submit();
        }}
    </script>
</body>
</html>"##,
        filename = filename,
        photo_url = photo_url,
        config_json = config_json,
        caption_html = caption_html,
        caption_js = caption_js,
        save_url = save_url,
        id = id,
        tags_html = tags_html,
        prev_disabled = if prev_id.is_none() { "disabled" } else { "" },
        next_disabled = if next_id.is_none() { "disabled" } else { "" },
        prev_id_js = prev_id.map_or("null".to_string(), |id| id.to_string()),
        next_id_js = next_id.map_or("null".to_string(), |id| id.to_string()),
        letterbox_checked = if crop_type == "letterbox" { "checked" } else { "" },
        expand_checked = if crop_type == "expand" { "checked" } else { "" },
        zoom_checked = if crop_type == "zoom" { "checked" } else { "" },
        black_checked = if bg_type == "black" { "checked" } else { "" },
        color_checked = if bg_type == "color" { "checked" } else { "" },
        gaussian_checked = if bg_type == "gaussian" { "checked" } else { "" },
        caption_left_checked = if caption_location == "left" { "checked" } else { "" },
        caption_center_checked = if caption_location == "center" { "checked" } else { "" },
        caption_right_checked = if caption_location == "right" { "checked" } else { "" },
        crop_z = crop_z,
        crop_dx = crop_dx,
        crop_dy = crop_dy,
        bg_r = bg_r,
        bg_g = bg_g,
        bg_b = bg_b,
        bg_blur_r = bg_blur_r,
        zoom_disabled = zoom_disabled,
        expand_disabled = expand_disabled,
        color_disabled = color_disabled,
        gaussian_disabled = gaussian_disabled,
    );

    Html(html)
}

// ============================================================================
// Photo Config Control Endpoints (htmx)
// ============================================================================

#[derive(Deserialize)]
pub struct CropControlsQuery {
    #[serde(rename = "type")]
    crop_type: Option<String>,
    dx: Option<f32>,
    dy: Option<f32>,
    z: Option<f32>,
}

pub async fn crop_controls(Query(params): Query<CropControlsQuery>) -> Html<String> {
    let crop_type = params.crop_type.as_deref().unwrap_or("letterbox");
    let dx = params.dx.unwrap_or(0.0);
    let dy = params.dy.unwrap_or(0.0);
    let z = params.z.unwrap_or(1.0);

    let html = match crop_type {
        "expand" => format!(
            r##"<div class="slider-group">
    <label for="crop_dx">X Offset:</label>
    <input type="range" id="crop_dx" name="crop_dx" min="-1" max="1" step="0.01" value="{dx}" oninput="updatePreview()">
    <span id="crop_dx_val">{dx:.2}</span>
</div>
<div class="slider-group">
    <label for="crop_dy">Y Offset:</label>
    <input type="range" id="crop_dy" name="crop_dy" min="-1" max="1" step="0.01" value="{dy}" oninput="updatePreview()">
    <span id="crop_dy_val">{dy:.2}</span>
</div>
<input type="hidden" name="crop_z" value="1">"##,
            dx = dx, dy = dy
        ),
        "zoom" => format!(
            r##"<div class="slider-group">
    <label for="crop_z">Zoom:</label>
    <input type="range" id="crop_z" name="crop_z" min="0.5" max="3" step="0.01" value="{z}" oninput="updatePreview()">
    <span id="crop_z_val">{z:.2}</span>
</div>
<div class="slider-group">
    <label for="crop_dx">X Offset:</label>
    <input type="range" id="crop_dx" name="crop_dx" min="-1" max="1" step="0.01" value="{dx}" oninput="updatePreview()">
    <span id="crop_dx_val">{dx:.2}</span>
</div>
<div class="slider-group">
    <label for="crop_dy">Y Offset:</label>
    <input type="range" id="crop_dy" name="crop_dy" min="-1" max="1" step="0.01" value="{dy}" oninput="updatePreview()">
    <span id="crop_dy_val">{dy:.2}</span>
</div>"##,
            z = z, dx = dx, dy = dy
        ),
        _ => {
            // Letterbox - no parameters, but include hidden inputs for form submission
            r##"<p class="control-hint">Image will be scaled to fit, centered.</p>
<input type="hidden" name="crop_dx" value="0">
<input type="hidden" name="crop_dy" value="0">
<input type="hidden" name="crop_z" value="1">"##.to_string()
        }
    };

    Html(html)
}

#[derive(Deserialize)]
pub struct BackgroundControlsQuery {
    #[serde(rename = "type")]
    bg_type: Option<String>,
    r: Option<f32>,
    g: Option<f32>,
    b: Option<f32>,
    blur_r: Option<f32>,
}

pub async fn background_controls(Query(params): Query<BackgroundControlsQuery>) -> Html<String> {
    let bg_type = params.bg_type.as_deref().unwrap_or("black");
    let r = params.r.unwrap_or(0.0);
    let g = params.g.unwrap_or(0.0);
    let b = params.b.unwrap_or(0.0);
    let blur_r = params.blur_r.unwrap_or(10.0);

    let html = match bg_type {
        "color" => format!(
            r##"<div class="slider-group">
    <label for="bg_r">Red:</label>
    <input type="range" id="bg_r" name="bg_r" min="0" max="255" step="1" value="{r}" oninput="updatePreview()">
    <span id="bg_r_val">{r:.0}</span>
</div>
<div class="slider-group">
    <label for="bg_g">Green:</label>
    <input type="range" id="bg_g" name="bg_g" min="0" max="255" step="1" value="{g}" oninput="updatePreview()">
    <span id="bg_g_val">{g:.0}</span>
</div>
<div class="slider-group">
    <label for="bg_b">Blue:</label>
    <input type="range" id="bg_b" name="bg_b" min="0" max="255" step="1" value="{b}" oninput="updatePreview()">
    <span id="bg_b_val">{b:.0}</span>
</div>
<input type="hidden" name="bg_blur_r" value="0">"##,
            r = r, g = g, b = b
        ),
        "gaussian" => format!(
            r##"<div class="slider-group">
    <label for="bg_blur_r">Blur:</label>
    <input type="range" id="bg_blur_r" name="bg_blur_r" min="0" max="50" step="1" value="{blur_r}" oninput="updatePreview()">
    <span id="bg_blur_r_val">{blur_r:.0}</span>
</div>
<input type="hidden" name="bg_r" value="0">
<input type="hidden" name="bg_g" value="0">
<input type="hidden" name="bg_b" value="0">"##,
            blur_r = blur_r
        ),
        _ => {
            // Black - no parameters
            r##"<p class="control-hint">Solid black background.</p>
<input type="hidden" name="bg_r" value="0">
<input type="hidden" name="bg_g" value="0">
<input type="hidden" name="bg_b" value="0">
<input type="hidden" name="bg_blur_r" value="0">"##.to_string()
        }
    };

    Html(html)
}

// ============================================================================
// Photo Config Save
// ============================================================================

#[derive(Deserialize)]
pub struct PhotoConfigForm {
    #[serde(default)]
    caption: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    caption_location: String,
    crop_type: String,
    #[serde(default)]
    crop_dx: f32,
    #[serde(default)]
    crop_dy: f32,
    #[serde(default = "default_zoom")]
    crop_z: f32,
    bg_type: String,
    #[serde(default)]
    bg_r: f32,
    #[serde(default)]
    bg_g: f32,
    #[serde(default)]
    bg_b: f32,
    #[serde(default)]
    bg_blur_r: f32,
    #[serde(default)]
    redirect_to: String,
}

fn default_zoom() -> f32 {
    1.0
}

pub async fn save_photo_config(
    State(pool): State<DbPool>,
    Path(id): Path<i64>,
    Form(form): Form<PhotoConfigForm>,
) -> impl IntoResponse {
    let crop = match form.crop_type.as_str() {
        "expand" => PhotoCrop::Expand { dx: form.crop_dx, dy: form.crop_dy },
        "zoom" => PhotoCrop::Zoom { z: form.crop_z, dx: form.crop_dx, dy: form.crop_dy },
        _ => PhotoCrop::Letterbox,
    };

    let background = match form.bg_type.as_str() {
        "color" => PhotoBackground::Color { r: form.bg_r, g: form.bg_g, b: form.bg_b },
        "gaussian" => PhotoBackground::Gaussian { r: form.bg_blur_r },
        _ => PhotoBackground::Black,
    };

    let caption_location = match form.caption_location.as_str() {
        "center" => CaptionLocation::Center,
        "right" => CaptionLocation::Right,
        _ => CaptionLocation::Left,
    };

    let config = PhotoConfig { crop, background, caption_location };
    let caption = if form.caption.trim().is_empty() {
        None
    } else {
        Some(form.caption.trim().to_string())
    };

    // Parse tags from comma-separated string
    let tags: Vec<String> = form.tags
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if let Err(e) = update_photo(&pool, id, &config, caption.as_deref()).await {
        error!(id = id, error = %e, "Failed to save photo");
    }

    if let Err(e) = save_photo_tags(&pool, id, &tags).await {
        error!(id = id, error = %e, "Failed to save photo tags");
    }

    let target = if form.redirect_to.trim().is_empty() {
        format!("/photo/{}/edit", id)
    } else {
        form.redirect_to.trim().to_string()
    };

    Redirect::to(&target)
}

pub async fn update_photo(pool: &DbPool, id: i64, config: &PhotoConfig, caption: Option<&str>) -> Result<()> {
    let config_json = serde_json::to_string(config)?;

    sqlx::query("UPDATE photos SET config = ?, caption = ? WHERE id = ?")
        .bind(&config_json)
        .bind(caption)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

// ============================================================================
// Photo Upload
// ============================================================================

pub async fn upload_page(State(_pool): State<DbPool>) -> Html<String> {
    let accept_types = "image/jpeg,image/png,image/gif,image/webp";
    
    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Upload Photo - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="/static/auto-sleep.js" {}
            }
            body {
                div .photos-page {
                    div .photos-page-header {
                        a href="/photos" { "← Back to Photos" }
                    }
                    h1 { "Upload Photo" }
                    
                    div #upload-dropzone .upload-dropzone {
                        input #file-input type="file" accept=(accept_types) style="display:none";
                        p { "Drag and drop an image here" }
                        p { "or" }
                        button #select-btn .btn type="button" { "Select File" }
                    }
                    
                    div #upload-preview .upload-preview style="display:none" {
                        img #preview-image src="" alt="Preview";
                        p #filename-display {}
                        div #upload-warning .upload-warning {}
                        div .upload-actions {
                            button #clear-btn .btn type="button" { "Clear" }
                            button #upload-btn ."btn btn-default" type="button" { "Upload" }
                        }
                    }
                    
                    div #upload-status {}
                }
                script src="/static/upload.js" {}
            }
        }
    };

    Html(html.render().into_inner())
}

#[derive(Deserialize)]
pub struct UploadCheckQuery {
    filename: String,
}

pub async fn upload_check(
    State(pool): State<DbPool>,
    Query(query): Query<UploadCheckQuery>,
) -> Html<String> {
    let filename = query.filename.trim();
    
    if filename.is_empty() {
        return Html(String::new());
    }
    
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM photos WHERE path = ?"
    )
        .bind(filename)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None);
    
    if exists.is_some() {
        let stem = std::path::Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        
        let new_name = if ext.is_empty() {
            format!("{}-1", stem)
        } else {
            format!("{}-1.{}", stem, ext)
        };
        
        Html(format!(
            r#"<p>A photo with this name already exists. It will be saved as <strong>{}</strong>.</p>"#,
            new_name
        ))
    } else {
        Html(String::new())
    }
}

fn find_unique_filename(base_filename: &str, existing_paths: &std::collections::HashSet<String>) -> String {
    if !existing_paths.contains(base_filename) {
        return base_filename.to_string();
    }
    
    let path = std::path::Path::new(base_filename);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(base_filename);
    let ext = path.extension().and_then(|s| s.to_str());
    
    let mut n = 1;
    loop {
        let new_name = match ext {
            Some(e) => format!("{}-{}.{}", stem, n, e),
            None => format!("{}-{}", stem, n),
        };
        if !existing_paths.contains(&new_name) {
            return new_name;
        }
        n += 1;
    }
}

pub async fn upload_photo(
    State(pool): State<DbPool>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut file_data: Option<(String, Vec<u8>)> = None;
    
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            let original_filename = field.file_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "upload.jpg".to_string());
            
            let ext = std::path::Path::new(&original_filename)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            
            if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header(header::CONTENT_TYPE, "text/html")
                    .body(Body::from(r#"<div class="upload-error">Invalid file type. Allowed: jpg, jpeg, png, gif, webp</div>"#))
                    .unwrap();
            }
            
            match field.bytes().await {
                Ok(bytes) => {
                    file_data = Some((original_filename, bytes.to_vec()));
                }
                Err(e) => {
                    error!(error = %e, "Failed to read upload data");
                    return Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .header(header::CONTENT_TYPE, "text/html")
                        .body(Body::from(r#"<div class="upload-error">Failed to read file data</div>"#))
                        .unwrap();
                }
            }
        }
    }
    
    let (original_filename, bytes) = match file_data {
        Some(data) => data,
        None => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(r#"<div class="upload-error">No file uploaded</div>"#))
                .unwrap();
        }
    };
    
    let existing: Vec<(String,)> = sqlx::query_as("SELECT path FROM photos")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    
    let existing_db: std::collections::HashSet<String> = existing.into_iter().map(|(p,)| p).collect();
    
    let mut existing_paths = existing_db.clone();
    if let Ok(mut entries) = fs::read_dir("photos").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                existing_paths.insert(name.to_string());
            }
        }
    }
    
    let final_filename = find_unique_filename(&original_filename, &existing_paths);
    let file_path = PathBuf::from("photos").join(&final_filename);
    
    if let Err(e) = fs::write(&file_path, &bytes).await {
        error!(error = %e, path = %file_path.display(), "Failed to write photo file");
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(r#"<div class="upload-error">Failed to save file</div>"#))
            .unwrap();
    }
    
    let default_config = serde_json::to_string(&PhotoConfig::default()).unwrap_or_else(|_| "{}".to_string());
    let insert_result = sqlx::query(
        "INSERT INTO photos (path, missing, active, config) VALUES (?, 0, 1, ?)"
    )
        .bind(&final_filename)
        .bind(&default_config)
        .execute(&pool)
        .await;
    
    let photo_id = match insert_result {
        Ok(result) => result.last_insert_rowid(),
        Err(e) => {
            error!(error = %e, filename = %final_filename, "Failed to insert photo into database");
            let _ = fs::remove_file(&file_path).await;
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(r#"<div class="upload-error">Failed to save to database</div>"#))
                .unwrap();
        }
    };
    
    let filename_clone = final_filename.clone();
    let _ = tokio::task::spawn_blocking(move || {
        ensure_thumbnail_sync(&filename_clone)
    }).await;
    
    info!(filename = %final_filename, id = photo_id, "Photo uploaded successfully");
    
    Response::builder()
        .status(StatusCode::OK)
        .header("HX-Redirect", format!("/photo/{}/edit", photo_id))
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(format!(
            r#"<div class="upload-success">Photo uploaded successfully as {}</div>"#,
            final_filename
        )))
        .unwrap()
}
