use std::sync::OnceLock;
use chrono_tz::Tz;

/// Global timezone setting for the application
static APP_TIMEZONE: OnceLock<Tz> = OnceLock::new();

/// Global touch mode setting (use buttons instead of links)
static TOUCH_MODE: OnceLock<bool> = OnceLock::new();

/// Initialize the timezone from the given string
pub fn init_timezone(tz_str: &str) {
    let timezone: Tz = tz_str.parse().unwrap_or_else(|_| {
        eprintln!("Warning: Invalid timezone '{}', falling back to UTC", tz_str);
        chrono_tz::UTC
    });

    if APP_TIMEZONE.set(timezone).is_err() {
        eprintln!("Warning: Timezone already initialized");
    }
}

/// Get the configured timezone
pub fn get_timezone() -> Tz {
    *APP_TIMEZONE.get().unwrap_or(&chrono_tz::UTC)
}

/// Initialize touch mode
pub fn init_touch_mode(enabled: bool) {
    if TOUCH_MODE.set(enabled).is_err() {
        eprintln!("Warning: Touch mode already initialized");
    }
}

/// Check if touch mode is enabled (buttons instead of links)
pub fn is_touch_mode() -> bool {
    *TOUCH_MODE.get().unwrap_or(&false)
}

