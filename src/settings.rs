use axum::{
    extract::{Form, Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use chrono::{Timelike, Utc};
use hypertext::{prelude::*, Raw};
use serde::{Deserialize, Serialize};

use crate::config::get_timezone;
use crate::db::{self, DbPool};

const DISPLAY_TIME_MIN: u16 = 1;
const DISPLAY_TIME_MAX: u16 = 600;
const SLEEP_TIME_MIN: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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
    /// Return the active filter tags based on the current time in the
    /// configured timezone: day (6:00–20:00), evening (20:00–0:00), night (0:00–6:00).
    pub fn parsed_tags(&self) -> Vec<String> {
        let hour = Utc::now().with_timezone(&get_timezone()).hour();
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

// ============================================================================
// URL Parameters
// ============================================================================

/// Query parameters that set a field of `Settings`. Visiting any page with one
/// of these merges it into the settings cookie and redirects to the clean URL,
/// so a kiosk that wipes its cookies on boot can be configured by its start URL.
const URL_PARAM_KEYS: [&str; 6] = [
    "display_time",
    "sleep_time",
    "day_tags",
    "evening_tags",
    "night_tags",
    "touch_mode",
];

/// A settings value from a URL parameter. An empty value or `none` (any case)
/// clears the setting back to its default.
#[derive(Debug, PartialEq)]
enum ParamValue<'a> {
    Clear,
    Set(&'a str),
}

impl<'a> ParamValue<'a> {
    fn parse(raw: &'a str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            ParamValue::Clear
        } else {
            ParamValue::Set(trimmed)
        }
    }
}

/// Parse an integer and clamp it into `min..=max`. `None` if it isn't a number.
fn parse_clamped(s: &str, min: u16, max: u16) -> Option<u16> {
    let n: i64 = s.parse().ok()?;
    Some(n.clamp(min as i64, max as i64) as u16)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// Apply one URL parameter to `settings`. Unknown keys are ignored; values
/// that can't be parsed are logged and leave the setting unchanged.
fn apply_url_param(settings: &mut Settings, key: &str, raw: &str) {
    let value = ParamValue::parse(raw);
    let applied = match (key, value) {
        ("display_time", ParamValue::Clear) => {
            settings.display_time = None;
            true
        }
        ("display_time", ParamValue::Set(v)) => parse_clamped(v, DISPLAY_TIME_MIN, DISPLAY_TIME_MAX)
            .map(|n| settings.display_time = Some(n))
            .is_some(),
        ("sleep_time", ParamValue::Clear) => {
            settings.sleep_time = None;
            true
        }
        ("sleep_time", ParamValue::Set(v)) => parse_clamped(v, SLEEP_TIME_MIN, u16::MAX)
            .map(|n| settings.sleep_time = Some(n))
            .is_some(),
        ("day_tags" | "evening_tags" | "night_tags", value) => {
            let tags = match value {
                ParamValue::Clear => String::new(),
                ParamValue::Set(v) => v.to_string(),
            };
            match key {
                "day_tags" => settings.day_tags = tags,
                "evening_tags" => settings.evening_tags = tags,
                _ => settings.night_tags = tags,
            }
            true
        }
        ("touch_mode", ParamValue::Clear) => {
            settings.touch_mode = false;
            true
        }
        ("touch_mode", ParamValue::Set(v)) => parse_bool(v)
            .map(|b| settings.touch_mode = b)
            .is_some(),
        _ => true,
    };
    if !applied {
        tracing::warn!("Ignoring invalid settings URL parameter {}={:?}", key, raw);
    }
}

/// Middleware: if a GET request carries any settings parameters, merge them
/// over the settings cookie and redirect to the same URL without them.
pub async fn url_settings_middleware(req: Request, next: Next) -> Response {
    if req.method() != Method::GET {
        return next.run(req).await;
    }
    let Ok(Query(params)) = Query::<Vec<(String, String)>>::try_from_uri(req.uri()) else {
        return next.run(req).await;
    };
    let (ours, rest): (Vec<_>, Vec<_>) = params
        .into_iter()
        .partition(|(k, _)| URL_PARAM_KEYS.contains(&k.as_str()));
    if ours.is_empty() {
        return next.run(req).await;
    }

    let mut settings = read_settings(req.headers());
    for (key, value) in &ours {
        apply_url_param(&mut settings, key, value);
    }

    let mut location = req.uri().path().to_string();
    if !rest.is_empty() {
        let query = rest
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        location.push('?');
        location.push_str(&query);
    }

    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, set_cookie_header(&settings))
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

/// The `scheme://host` this request was made to, honouring a reverse proxy's
/// `X-Forwarded-Proto`.
fn request_origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    format!("{}://{}", scheme, host)
}

/// A URL that reproduces `settings` from scratch when visited.
fn kiosk_url(origin: &str, settings: &Settings) -> String {
    let number = |n: Option<u16>| n.map_or("none".to_string(), |n| n.to_string());
    let tags = |s: &str| {
        let tags = parse_tag_str(s);
        if tags.is_empty() {
            "none".to_string()
        } else {
            tags.iter()
                .map(|t| urlencoding::encode(t).into_owned())
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    format!(
        "{}/?display_time={}&sleep_time={}&day_tags={}&evening_tags={}&night_tags={}&touch_mode={}",
        origin,
        number(settings.display_time),
        number(settings.sleep_time),
        tags(&settings.day_tags),
        tags(&settings.evening_tags),
        tags(&settings.night_tags),
        if settings.touch_mode { 1 } else { 0 },
    )
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
    render_settings_page(&settings, &people, &request_origin(&headers), None)
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
            Ok(n) if (DISPLAY_TIME_MIN..=DISPLAY_TIME_MAX).contains(&n) => Some(n),
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
                    &request_origin(&headers),
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
            Ok(n) if n >= SLEEP_TIME_MIN => Some(n),
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
                    &request_origin(&headers),
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

fn render_settings_page(
    settings: &Settings,
    people: &[db::Person],
    origin: &str,
    error: Option<&str>,
) -> Html<String> {
    let kiosk_url = kiosk_url(origin, settings);
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
                                p .form-help { "Idle time before slideshow starts. Leave blank for indefinite." }
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
                    fieldset {
                        legend { "Kiosk URL" }
                        p .form-help { "Open this URL to apply the settings above on any device." }
                        code .kiosk-url { (kiosk_url) }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(params: &[(&str, &str)]) -> Settings {
        let mut settings = Settings::default();
        for (k, v) in params {
            apply_url_param(&mut settings, k, v);
        }
        settings
    }

    #[test]
    fn test_param_value_none_clears() {
        assert_eq!(ParamValue::parse("none"), ParamValue::Clear);
        assert_eq!(ParamValue::parse("NONE"), ParamValue::Clear);
        assert_eq!(ParamValue::parse("  "), ParamValue::Clear);
        assert_eq!(ParamValue::parse(" 5 "), ParamValue::Set("5"));
    }

    #[test]
    fn test_display_time_clamped() {
        assert_eq!(apply(&[("display_time", "15")]).display_time, Some(15));
        assert_eq!(apply(&[("display_time", "0")]).display_time, Some(1));
        assert_eq!(apply(&[("display_time", "-3")]).display_time, Some(1));
        assert_eq!(apply(&[("display_time", "9999")]).display_time, Some(600));
        assert_eq!(apply(&[("display_time", "15"), ("display_time", "none")]).display_time, None);
    }

    #[test]
    fn test_sleep_time_clamped() {
        assert_eq!(apply(&[("sleep_time", "10")]).sleep_time, Some(10));
        assert_eq!(apply(&[("sleep_time", "0")]).sleep_time, Some(1));
        assert_eq!(apply(&[("sleep_time", "100000")]).sleep_time, Some(u16::MAX));
        assert_eq!(apply(&[("sleep_time", "10"), ("sleep_time", "none")]).sleep_time, None);
    }

    #[test]
    fn test_invalid_value_leaves_setting_unchanged() {
        let s = apply(&[("display_time", "15"), ("display_time", "abc")]);
        assert_eq!(s.display_time, Some(15));
        let s = apply(&[("touch_mode", "1"), ("touch_mode", "maybe")]);
        assert!(s.touch_mode);
    }

    #[test]
    fn test_touch_mode() {
        for v in ["1", "true", "On", "yes"] {
            assert!(apply(&[("touch_mode", v)]).touch_mode, "{v}");
        }
        for v in ["0", "false", "off", "no", "none"] {
            assert!(!apply(&[("touch_mode", "1"), ("touch_mode", v)]).touch_mode, "{v}");
        }
    }

    #[test]
    fn test_tags() {
        let s = apply(&[("day_tags", "family trips,Beach"), ("night_tags", "none")]);
        assert_eq!(s.day_tags, "family trips,Beach");
        assert_eq!(s.night_tags, "");
    }

    #[test]
    fn test_kiosk_url_round_trips() {
        let settings = Settings {
            display_time: Some(15),
            sleep_time: None,
            day_tags: "Family Trips, beach".to_string(),
            evening_tags: String::new(),
            night_tags: "a&b".to_string(),
            touch_mode: true,
        };
        let url = kiosk_url("http://chores.lan:3000", &settings);
        assert_eq!(
            url,
            "http://chores.lan:3000/?display_time=15&sleep_time=none&day_tags=family%20trips,beach\
             &evening_tags=none&night_tags=a%26b&touch_mode=1"
        );

        let uri: axum::http::Uri = url.parse().unwrap();
        let Query(params) = Query::<Vec<(String, String)>>::try_from_uri(&uri).unwrap();
        let mut round_tripped = Settings::default();
        for (k, v) in &params {
            apply_url_param(&mut round_tripped, k, v);
        }
        assert_eq!(parse_tag_str(&round_tripped.day_tags), vec!["family trips", "beach"]);
        assert_eq!(round_tripped.display_time, settings.display_time);
        assert_eq!(round_tripped.sleep_time, settings.sleep_time);
        assert_eq!(round_tripped.night_tags, "a&b");
        assert!(round_tripped.touch_mode);
    }
}
