use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use chrono::Timelike;
use hypertext::{prelude::*, Raw};
use serde::{Deserialize, Serialize};

use crate::db::{self, DbPool};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub display_time: Option<u16>,
    pub sleep_time: Option<u16>,
    #[serde(default)]
    pub day_tags: String,
    #[serde(default)]
    pub evening_tags: String,
    #[serde(default)]
    pub night_tags: String,
    #[serde(default)]
    pub touch_mode: bool,
}

fn parse_tag_str(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

impl Settings {
    /// Return the active filter tags based on the current local time:
    /// day (6:00–20:00), evening (20:00–0:00), night (0:00–6:00).
    pub fn parsed_tags(&self) -> Vec<String> {
        let hour = chrono::Local::now().hour();
        match hour {
            6..20 => parse_tag_str(&self.day_tags),
            20..=23 => parse_tag_str(&self.evening_tags),
            _ => parse_tag_str(&self.night_tags),
        }
    }
}

pub fn is_touch_mode(headers: &HeaderMap) -> bool {
    read_settings(headers).touch_mode
}

pub fn read_settings(headers: &HeaderMap) -> Settings {
    let cookie_header = match headers.get(header::COOKIE) {
        Some(h) => h.to_str().unwrap_or(""),
        None => return Settings::default(),
    };

    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("settings=") {
            if let Ok(decoded) = urlencoding::decode(value) {
                if let Ok(settings) = serde_json::from_str::<Settings>(&decoded) {
                    return settings;
                }
            }
        }
    }

    Settings::default()
}

fn set_cookie_header(settings: &Settings) -> String {
    let json = serde_json::to_string(settings).unwrap_or_else(|_| "{}".to_string());
    let encoded = urlencoding::encode(&json);
    format!("settings={}; Path=/; SameSite=Lax", encoded)
}

#[derive(Deserialize)]
pub struct SettingsForm {
    display_time: Option<String>,
    sleep_time: Option<String>,
    day_tags: Option<String>,
    evening_tags: Option<String>,
    night_tags: Option<String>,
    touch_mode: Option<String>,
}

pub async fn settings_page(State(pool): State<DbPool>, headers: HeaderMap) -> Html<String> {
    let settings = read_settings(&headers);
    let people = db::get_all_people(&pool).await.unwrap_or_default();
    render_settings_page(&settings, &people, None)
}

pub async fn save_settings(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Form(form): Form<SettingsForm>,
) -> Response {
    let display_time_str = form.display_time.unwrap_or_default();
    let sleep_time_str = form.sleep_time.unwrap_or_default();
    let day_tags = form.day_tags.unwrap_or_default();
    let evening_tags = form.evening_tags.unwrap_or_default();
    let night_tags = form.night_tags.unwrap_or_default();
    let touch_mode = form.touch_mode.is_some();

    let current_settings = read_settings(&headers);
    let people = db::get_all_people(&pool).await.unwrap_or_default();

    let display_time: Option<u16> = if display_time_str.trim().is_empty() {
        None
    } else {
        match display_time_str.trim().parse::<u16>() {
            Ok(n) if (1..=600).contains(&n) => Some(n),
            _ => {
                let error_settings = Settings {
                    display_time: current_settings.display_time,
                    sleep_time: current_settings.sleep_time,
                    day_tags,
                    evening_tags,
                    night_tags,
                    touch_mode,
                };
                return render_settings_page(
                    &error_settings,
                    &people,
                    Some("Display time must be a number between 1 and 600, or blank for default."),
                )
                .into_response();
            }
        }
    };

    let sleep_time: Option<u16> = if sleep_time_str.trim().is_empty() {
        None
    } else {
        match sleep_time_str.trim().parse::<u16>() {
            Ok(n) if n >= 1 => Some(n),
            _ => {
                let error_settings = Settings {
                    display_time,
                    sleep_time: current_settings.sleep_time,
                    day_tags,
                    evening_tags,
                    night_tags,
                    touch_mode,
                };
                return render_settings_page(
                    &error_settings,
                    &people,
                    Some("Sleep time must be a positive number of minutes, or blank for indefinite."),
                )
                .into_response();
            }
        }
    };

    let new_settings = Settings {
        display_time,
        sleep_time,
        day_tags,
        evening_tags,
        night_tags,
        touch_mode,
    };

    let cookie = set_cookie_header(&new_settings);

    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/settings")
        .header(header::SET_COOKIE, cookie)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

fn render_settings_page(settings: &Settings, people: &[db::Person], error: Option<&str>) -> Html<String> {
    let display_time_value = settings
        .display_time
        .map(|n| n.to_string())
        .unwrap_or_default();

    let sleep_time_value = settings
        .sleep_time
        .map(|n| n.to_string())
        .unwrap_or_default();

    let people_list_html: String = people
        .iter()
        .map(|p| {
            format!(
                r##"<li class="person-item"><span>{}</span><form method="post" action="/settings/people/{}/delete" style="display:inline"><button class="btn person-delete" type="submit">×</button></form></li>"##,
                p.initials, p.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Settings - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
                script src="/static/auto-sleep.js" {}
            }
            body {
                div .settings-page {
                    div .settings-page-header {
                        @if settings.touch_mode {
                            button .btn onclick="window.location.href='/'" { "← Back" }
                        } @else {
                            a href="/" { "← Back" }
                        }
                    }
                    h1 { "Settings" }

                    @if let Some(err) = error {
                        div .error-message {
                            (err)
                        }
                    }

                    form method="post" action="/settings" {
                        fieldset {
                            legend { "Slideshow" }

                            div .form-group {
                                label for="display_time" { "Display time (seconds)" }
                                input
                                    type="text"
                                    id="display_time"
                                    name="display_time"
                                    value=(display_time_value)
                                    placeholder="8 (default)";
                                p .form-help { "Leave blank for default (8 seconds). Must be 1-600." }
                            }

                            div .form-group {
                                label for="sleep_time" { "Sleep time (minutes)" }
                                input
                                    type="text"
                                    id="sleep_time"
                                    name="sleep_time"
                                    value=(sleep_time_value)
                                    placeholder="indefinite";
                                p .form-help { "Total slideshow duration before returning home. Leave blank for indefinite." }
                            }

                            div .form-group {
                                label for="day_tags" { "Day tags (6 am – 8 pm)" }
                                input
                                    type="text"
                                    id="day_tags"
                                    name="day_tags"
                                    value=(settings.day_tags)
                                    placeholder="tag1, tag2, tag3";
                            }

                            div .form-group {
                                label for="evening_tags" { "Evening tags (8 pm – 12 am)" }
                                input
                                    type="text"
                                    id="evening_tags"
                                    name="evening_tags"
                                    value=(settings.evening_tags)
                                    placeholder="tag1, tag2, tag3";
                            }

                            div .form-group {
                                label for="night_tags" { "Night tags (12 am – 6 am)" }
                                input
                                    type="text"
                                    id="night_tags"
                                    name="night_tags"
                                    value=(settings.night_tags)
                                    placeholder="tag1, tag2, tag3";
                                p .form-help { "Comma-separated lists. Leave blank to show all photos for that time period." }
                            }
                        }

                        fieldset {
                            legend { "Interface" }

                            div .form-group .form-group-checkbox {
                                input
                                    type="checkbox"
                                    id="touch_mode"
                                    name="touch_mode"
                                    checked[settings.touch_mode];
                                label for="touch_mode" { "Touch mode" }
                                p .form-help { "Use larger buttons instead of links for touchscreen devices." }
                            }
                        }

                        div .form-actions {
                            button .btn type="submit" { "Save" }
                        }
                    }

                    hr style="margin: 32px 0 24px;";

                    fieldset {
                        legend { "People" }

                        @if people.is_empty() {
                            p .form-help { "No people added yet." }
                        } @else {
                            ul .people-list {
                                (Raw::dangerously_create(&people_list_html))
                            }
                        }

                        form .people-add-form method="post" action="/settings/people" {
                            input
                                type="text"
                                name="initials"
                                placeholder="Initials"
                                required;
                            button .btn type="submit" { "Add" }
                        }
                    }
                }
            }
        }
    };

    Html(html.render().into_inner())
}

// ============================================================================
// People Management
// ============================================================================

#[derive(Deserialize)]
pub struct AddPersonForm {
    initials: String,
}

pub async fn add_person(
    State(pool): State<DbPool>,
    Form(form): Form<AddPersonForm>,
) -> Response {
    let initials = form.initials.trim().to_string();
    if !initials.is_empty() {
        let _ = db::add_person(&pool, &initials).await;
    }
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/settings")
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

pub async fn delete_person(
    State(pool): State<DbPool>,
    Path(id): Path<i64>,
) -> Response {
    let _ = db::delete_person(&pool, id).await;
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/settings")
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}
