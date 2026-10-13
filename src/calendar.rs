//! Shared-calendar subscription.
//!
//! The server fetches one or more private iCal (.ics) feeds — e.g. a Google
//! Calendar's "Secret address in iCal format" — once an hour, expands recurring
//! events, and keeps the result in memory. The kiosk never sees the feed URL;
//! events reach it as non-completeable tasks rendered by `tasks.rs`.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::RwLock;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use tracing::{error, info, warn};

use crate::config::get_timezone;

/// How often the feeds are re-fetched.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// Recurring events are expanded within this window around "now".
const WINDOW_PAST_DAYS: i64 = 120;
const WINDOW_FUTURE_DAYS: i64 = 400;
/// Safety cap on occurrences per recurring event.
const MAX_OCCURRENCES: u16 = 2000;

/// The ID prefix that marks a task as coming from a calendar feed.
pub const ID_PREFIX: &str = "cal-";

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEvent {
    pub id: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
}

impl CalendarEvent {
    /// Whether any part of the event falls on `date` in the configured timezone.
    pub fn overlaps_date(&self, date: NaiveDate) -> bool {
        let tz = get_timezone();
        let Some(day_start) = local_midnight(date, tz) else { return false };
        let Some(day_end) = local_midnight(date + Duration::days(1), tz) else { return false };
        if self.end <= self.start {
            // Zero-length event: it's on the day it starts.
            return self.start >= day_start && self.start < day_end;
        }
        self.start < day_end && self.end > day_start
    }

    /// Start time to sort and display on `date`: midnight if it began earlier.
    pub fn time_on(&self, date: NaiveDate) -> NaiveTime {
        let local = self.start.with_timezone(&get_timezone());
        if self.all_day || local.date_naive() < date {
            NaiveTime::MIN
        } else {
            local.time()
        }
    }
}

/// Events per feed, in the order the URLs were configured. A feed that fails to
/// refresh keeps its last good events.
static FEEDS: RwLock<Vec<Vec<CalendarEvent>>> = RwLock::new(Vec::new());

pub fn is_calendar_id(id: &str) -> bool {
    id.starts_with(ID_PREFIX)
}

/// Every cached event, sorted by start time.
pub fn events() -> Vec<CalendarEvent> {
    let feeds = FEEDS.read().unwrap();
    let mut all: Vec<CalendarEvent> = feeds.iter().flatten().cloned().collect();
    all.sort_by_key(|e| e.start);
    all
}

pub fn get_event(id: &str) -> Option<CalendarEvent> {
    FEEDS.read().unwrap().iter().flatten().find(|e| e.id == id).cloned()
}

/// Events that touch `date`, in start order.
pub fn events_on(date: NaiveDate) -> Vec<CalendarEvent> {
    events().into_iter().filter(|e| e.overlaps_date(date)).collect()
}

/// Parse a comma-separated list of feed URLs and, if any, start refreshing them
/// in the background: once now, then every hour.
pub fn start(urls: &str) {
    let urls: Vec<String> = urls
        .split(',')
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    if urls.is_empty() {
        return;
    }
    info!("Calendar: subscribed to {} feed(s), refreshing hourly", urls.len());
    *FEEDS.write().unwrap() = vec![Vec::new(); urls.len()];

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "Calendar: could not build HTTP client");
                return;
            }
        };
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            refresh(&client, &urls).await;
        }
    });
}

async fn refresh(client: &reqwest::Client, urls: &[String]) {
    for (i, url) in urls.iter().enumerate() {
        // The URL is a secret, so logs name feeds by position only.
        let feed_no = i + 1;
        match fetch_feed(client, url, i).await {
            Ok(events) => {
                info!("Calendar feed #{}: loaded {} event(s)", feed_no, events.len());
                if let Some(slot) = FEEDS.write().unwrap().get_mut(i) {
                    *slot = events;
                }
            }
            Err(e) => warn!("Calendar feed #{}: refresh failed, keeping previous events: {:#}", feed_no, e),
        }
    }
}

async fn fetch_feed(client: &reqwest::Client, url: &str, feed_index: usize) -> Result<Vec<CalendarEvent>> {
    let response = client
        .get(url)
        .send()
        .await
        // reqwest errors include the URL; strip it so the secret stays out of logs.
        .map_err(|e| anyhow!("request failed: {}", e.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("server returned {}", status));
    }
    let body = response
        .text()
        .await
        .map_err(|e| anyhow!("reading body failed: {}", e.without_url()))?;
    let now = Utc::now();
    parse_and_expand(
        &body,
        feed_index,
        get_timezone(),
        now - Duration::days(WINDOW_PAST_DAYS),
        now + Duration::days(WINDOW_FUTURE_DAYS),
    )
}

// ============================================================================
// iCalendar parsing
// ============================================================================

/// A DTSTART/DTEND/etc. value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum IcsTime {
    Date(NaiveDate),
    DateTime(DateTime<Tz>),
}

impl IcsTime {
    /// Resolve to an instant; dates become local midnight in `tz`.
    fn to_utc(self, tz: Tz) -> Option<DateTime<Utc>> {
        match self {
            IcsTime::Date(d) => local_midnight(d, tz),
            IcsTime::DateTime(dt) => Some(dt.with_timezone(&Utc)),
        }
    }
}

#[derive(Debug, Default)]
struct RawEvent {
    uid: String,
    summary: String,
    description: String,
    location: String,
    cancelled: bool,
    start: Option<IcsTime>,
    end: Option<IcsTime>,
    duration: Option<Duration>,
    rrule: Option<String>,
    exdates: Vec<IcsTime>,
    recurrence_id: Option<IcsTime>,
}

struct Property<'a> {
    name: String,
    params: HashMap<String, String>,
    value: &'a str,
}

/// Undo RFC 5545 line folding (a CRLF followed by a space or tab).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(cont) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(cont);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines
}

/// Split `NAME;P1=a;P2="b:c":value`, honoring quoted parameter values.
fn parse_property(line: &str) -> Option<Property<'_>> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    Some(Property { name, params, value })
}

fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse a date or date-time. Floating times and unknown TZIDs use `default_tz`.
fn parse_time(value: &str, params: &HashMap<String, String>, default_tz: Tz) -> Option<IcsTime> {
    let value = value.trim();
    let is_date = params.get("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) || value.len() == 8;
    if is_date {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(IcsTime::Date);
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(IcsTime::DateTime(Utc.from_utc_datetime(&naive).with_timezone(&chrono_tz::UTC)));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let tz = params
        .get("TZID")
        .and_then(|id| id.parse::<Tz>().ok())
        .unwrap_or(default_tz);
    // `earliest` resolves DST gaps/overlaps instead of dropping the event.
    tz.from_local_datetime(&naive).earliest().map(IcsTime::DateTime)
}

/// Parse an RFC 5545 duration such as `P1D`, `PT1H30M` or `-P1W`.
fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (negative, rest) = match value.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            _ => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (c, in_time) {
                    ('W', false) => Duration::weeks(n),
                    ('D', false) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if negative { -total } else { total })
}

fn parse_events(text: &str, default_tz: Tz) -> Vec<RawEvent> {
    let mut events = Vec::new();
    let mut current: Option<RawEvent> = None;
    // Depth of nested components (e.g. VALARM) inside the current VEVENT.
    let mut nested = 0usize;

    for line in unfold(text) {
        let Some(prop) = parse_property(&line) else { continue };
        let value = prop.value;
        match prop.name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => {
                current = Some(RawEvent::default());
                nested = 0;
            }
            "BEGIN" if current.is_some() => nested += 1,
            "END" if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(ev) = current.take() {
                    events.push(ev);
                }
            }
            "END" if current.is_some() => nested = nested.saturating_sub(1),
            _ => {
                let Some(ev) = current.as_mut() else { continue };
                if nested > 0 {
                    continue;
                }
                match prop.name.as_str() {
                    "UID" => ev.uid = value.to_string(),
                    "SUMMARY" => ev.summary = unescape_text(value),
                    "DESCRIPTION" => ev.description = unescape_text(value),
                    "LOCATION" => ev.location = unescape_text(value),
                    "STATUS" => ev.cancelled = value.eq_ignore_ascii_case("CANCELLED"),
                    "DTSTART" => ev.start = parse_time(value, &prop.params, default_tz),
                    "DTEND" => ev.end = parse_time(value, &prop.params, default_tz),
                    "DURATION" => ev.duration = parse_duration(value),
                    "RRULE" => ev.rrule = Some(value.to_string()),
                    "EXDATE" => ev.exdates.extend(
                        value.split(',').filter_map(|v| parse_time(v, &prop.params, default_tz)),
                    ),
                    "RECURRENCE-ID" => ev.recurrence_id = parse_time(value, &prop.params, default_tz),
                    _ => {}
                }
            }
        }
    }
    events
}

// ============================================================================
// Expansion
// ============================================================================

fn local_midnight(date: NaiveDate, tz: Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&date.and_time(NaiveTime::MIN))
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

fn event_id(feed_index: usize, uid: &str, start: DateTime<Utc>) -> String {
    let mut hasher = DefaultHasher::new();
    (feed_index, uid, start.timestamp()).hash(&mut hasher);
    format!("{}{:016x}", ID_PREFIX, hasher.finish())
}

/// The event's length, from DTEND, DURATION, or the RFC 5545 defaults.
fn event_length(ev: &RawEvent, start: IcsTime, tz: Tz) -> Duration {
    let start_utc = start.to_utc(tz);
    if let (Some(end), Some(start_utc)) = (ev.end.and_then(|e| e.to_utc(tz)), start_utc) {
        return (end - start_utc).max(Duration::zero());
    }
    if let Some(d) = ev.duration {
        return d.max(Duration::zero());
    }
    match start {
        IcsTime::Date(_) => Duration::days(1),
        IcsTime::DateTime(_) => Duration::zero(),
    }
}

/// Rewrite any `UNTIL=` in an RRULE to a UTC timestamp, which the `rrule`
/// crate requires when DTSTART carries a timezone. A date-only UNTIL means the
/// end of that day.
fn normalize_rrule(rule: &str, tz: Tz) -> String {
    rule.split(';')
        .map(|part| {
            let Some((key, value)) = part.split_once('=') else { return part.to_string() };
            if !key.eq_ignore_ascii_case("UNTIL") || value.ends_with('Z') {
                return part.to_string();
            }
            let until = match parse_time(value, &HashMap::new(), tz) {
                Some(IcsTime::Date(d)) => {
                    local_midnight(d + Duration::days(1), tz).map(|t| t - Duration::seconds(1))
                }
                Some(t @ IcsTime::DateTime(_)) => t.to_utc(tz),
                None => None,
            };
            match until {
                Some(u) => format!("UNTIL={}", u.format("%Y%m%dT%H%M%SZ")),
                None => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Start instants of a recurring event within the window.
fn expand_rrule(
    rule: &str,
    start: IcsTime,
    tz: Tz,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>> {
    let dt_start = match start {
        IcsTime::Date(d) => tz
            .from_local_datetime(&d.and_time(NaiveTime::MIN))
            .earliest()
            .context("all-day start falls in a DST gap")?,
        IcsTime::DateTime(dt) => dt,
    };
    let rrule_tz: rrule::Tz = dt_start.timezone().into();
    let rule: rrule::RRule<rrule::Unvalidated> = normalize_rrule(rule, dt_start.timezone())
        .parse()
        .map_err(|e| anyhow!("invalid RRULE: {}", e))?;
    let set = rule
        .build(dt_start.with_timezone(&rrule_tz))
        .map_err(|e| anyhow!("invalid RRULE: {}", e))?
        .after(window_start.with_timezone(&rrule_tz))
        .before(window_end.with_timezone(&rrule_tz));
    Ok(set
        .all(MAX_OCCURRENCES)
        .dates
        .into_iter()
        .map(|d| d.with_timezone(&Utc))
        .collect())
}

/// Parse an .ics document and return every event occurrence overlapping
/// `[window_start, window_end]`. All-day and floating times use `tz`.
fn parse_and_expand(
    text: &str,
    feed_index: usize,
    tz: Tz,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>> {
    if !text.trim_start().starts_with("BEGIN:VCALENDAR") {
        return Err(anyhow!("response is not an iCalendar document"));
    }
    let raw = parse_events(text, tz);

    // Modified single occurrences of a recurring event, keyed by (UID, original start).
    let overridden: HashSet<(String, DateTime<Utc>)> = raw
        .iter()
        .filter_map(|ev| Some((ev.uid.clone(), ev.recurrence_id?.to_utc(tz)?)))
        .collect();

    let mut out = Vec::new();
    for ev in &raw {
        let Some(start) = ev.start else { continue };
        let all_day = matches!(start, IcsTime::Date(_));
        let length = event_length(ev, start, tz);

        let starts: Vec<DateTime<Utc>> = match (&ev.rrule, ev.recurrence_id) {
            (Some(rule), None) => {
                let exdates: HashSet<DateTime<Utc>> =
                    ev.exdates.iter().filter_map(|t| t.to_utc(tz)).collect();
                match expand_rrule(rule, start, tz, window_start - length, window_end) {
                    Ok(starts) => starts
                        .into_iter()
                        .filter(|s| !exdates.contains(s))
                        .filter(|s| !overridden.contains(&(ev.uid.clone(), *s)))
                        .collect(),
                    Err(e) => {
                        warn!(uid = %ev.uid, "Calendar: skipping recurring event: {:#}", e);
                        continue;
                    }
                }
            }
            _ => start.to_utc(tz).into_iter().collect(),
        };

        if ev.cancelled {
            continue;
        }
        for s in starts {
            let end = s + length;
            if end < window_start || s > window_end {
                continue;
            }
            out.push(CalendarEvent {
                id: event_id(feed_index, &ev.uid, s),
                summary: if ev.summary.is_empty() { "(No title)".to_string() } else { ev.summary.clone() },
                description: ev.description.clone(),
                location: ev.location.clone(),
                start: s,
                end,
                all_day,
            });
        }
    }
    out.sort_by_key(|e| e.start);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TZ: Tz = chrono_tz::America::Chicago;

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
        )
    }

    fn expand(body: &str) -> Vec<CalendarEvent> {
        let (a, b) = window();
        let doc = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n", body);
        parse_and_expand(&doc, 0, TZ, a, b).unwrap()
    }

    #[test]
    fn single_timed_event() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Dentist\\, downtown\r\n\
             DTSTART;TZID=America/New_York:20260115T090000\r\n\
             DTEND;TZID=America/New_York:20260115T100000\r\nEND:VEVENT\r\n",
        );
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.summary, "Dentist, downtown");
        assert!(!e.all_day);
        assert_eq!(e.start, Utc.with_ymd_and_hms(2026, 1, 15, 14, 0, 0).unwrap());
        assert_eq!(e.end - e.start, Duration::hours(1));
    }

    #[test]
    fn all_day_event_uses_local_midnight() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Trip\r\n\
             DTSTART;VALUE=DATE:20260120\r\nDTEND;VALUE=DATE:20260122\r\nEND:VEVENT\r\n",
        );
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert!(e.all_day);
        assert_eq!(e.start, Utc.with_ymd_and_hms(2026, 1, 20, 6, 0, 0).unwrap());
        assert_eq!(e.end - e.start, Duration::days(2));
    }

    #[test]
    fn folded_lines_and_alarms() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Long\r\n  title\r\n\
             DTSTART:20260110T150000Z\r\n\
             BEGIN:VALARM\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\n\
             END:VEVENT\r\n",
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Long title");
        assert_eq!(events[0].description, "");
    }

    #[test]
    fn weekly_recurrence_with_exdate_and_override() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:r\r\nSUMMARY:Practice\r\n\
             DTSTART;TZID=America/Chicago:20260105T180000\r\n\
             DTEND;TZID=America/Chicago:20260105T190000\r\n\
             RRULE:FREQ=WEEKLY;UNTIL=20260203T055959Z\r\n\
             EXDATE;TZID=America/Chicago:20260112T180000\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:r\r\nSUMMARY:Practice (moved)\r\n\
             RECURRENCE-ID;TZID=America/Chicago:20260119T180000\r\n\
             DTSTART;TZID=America/Chicago:20260120T180000\r\n\
             DTEND;TZID=America/Chicago:20260120T190000\r\nEND:VEVENT\r\n",
        );
        let summaries: Vec<(String, u32)> = events
            .iter()
            .map(|e| (e.summary.clone(), e.start.with_timezone(&TZ).format("%d").to_string().parse().unwrap()))
            .collect();
        assert_eq!(
            summaries,
            vec![
                ("Practice".to_string(), 5),
                ("Practice (moved)".to_string(), 20),
                ("Practice".to_string(), 26),
                ("Practice".to_string(), 2),
            ]
        );
    }

    #[test]
    fn all_day_recurrence_with_date_until() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:d\r\nSUMMARY:Trash day\r\n\
             DTSTART;VALUE=DATE:20260101\r\nDTEND;VALUE=DATE:20260102\r\n\
             RRULE:FREQ=WEEKLY;UNTIL=20260115\r\nEND:VEVENT\r\n",
        );
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| e.all_day));
    }

    #[test]
    fn cancelled_events_are_dropped() {
        let events = expand(
            "BEGIN:VEVENT\r\nUID:x\r\nSTATUS:CANCELLED\r\nDTSTART:20260110T150000Z\r\nEND:VEVENT\r\n",
        );
        assert!(events.is_empty());
    }

    #[test]
    fn rejects_non_calendar_body() {
        let (a, b) = window();
        assert!(parse_and_expand("<html>Sign in</html>", 0, TZ, a, b).is_err());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(parse_duration("-P1D"), Some(Duration::days(-1)));
        assert_eq!(parse_duration("garbage"), None);
    }
}
