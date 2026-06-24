//! Clock formatting helpers.

use chrono::{DateTime, Local, TimeZone};

pub struct FormattedTime {
    pub formatted: String,
    pub date: String,
    pub weekday: String,
}

pub fn format_time(ts_ms: i64) -> FormattedTime {
    let dt: DateTime<Local> = Local.timestamp_millis_opt(ts_ms).single().unwrap_or_else(Local::now);
    FormattedTime {
        formatted: dt.format("%H:%M:%S").to_string(),
        date: dt.format("%Y-%m-%d").to_string(),
        weekday: dt.format("%A").to_string(),
    }
}
