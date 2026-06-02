use axum::{
    extract::Form,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use hypertext::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub display_time: Option<u16>,
    pub show_tags: String,
}

impl Settings {
    pub fn parsed_tags(&self) -> Vec<String> {
        self.show_tags
            .split(',')
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
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
    show_tags: Option<String>,
}

pub async fn settings_page(headers: HeaderMap) -> Html<String> {
    let settings = read_settings(&headers);
    render_settings_page(&settings, None)
}

pub async fn save_settings(
    headers: HeaderMap,
    Form(form): Form<SettingsForm>,
) -> Response {
    let display_time_str = form.display_time.unwrap_or_default();
    let show_tags = form.show_tags.unwrap_or_default();

    let display_time: Option<u16> = if display_time_str.trim().is_empty() {
        None
    } else {
        match display_time_str.trim().parse::<u16>() {
            Ok(n) if (1..=600).contains(&n) => Some(n),
            _ => {
                let settings = read_settings(&headers);
                let error_settings = Settings {
                    display_time: settings.display_time,
                    show_tags,
                };
                return render_settings_page(
                    &error_settings,
                    Some("Display time must be a number between 1 and 600, or blank for default."),
                )
                .into_response();
            }
        }
    };

    let new_settings = Settings {
        display_time,
        show_tags,
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

fn render_settings_page(settings: &Settings, error: Option<&str>) -> Html<String> {
    let display_time_value = settings
        .display_time
        .map(|n| n.to_string())
        .unwrap_or_default();

    let html = maud! {
        !DOCTYPE
        html {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Settings - Chores" }
                link rel="stylesheet" href="/static/system.css";
                link rel="stylesheet" href="/static/app.css";
            }
            body {
                div .settings-page {
                    div .settings-page-header {
                        a href="/" { "← Back" }
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
                                label for="show_tags" { "Show tags" }
                                input
                                    type="text"
                                    id="show_tags"
                                    name="show_tags"
                                    value=(settings.show_tags)
                                    placeholder="tag1, tag2, tag3";
                                p .form-help { "Comma-separated list. Leave blank to show all photos." }
                            }
                        }

                        div .form-actions {
                            button .btn type="submit" { "Save" }
                        }
                    }
                }
            }
        }
    };

    Html(html.render().into_inner())
}
