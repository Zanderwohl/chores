use std::sync::OnceLock;
use chrono_tz::Tz;

/// Global timezone setting for the application
static APP_TIMEZONE: OnceLock<Tz> = OnceLock::new();

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

