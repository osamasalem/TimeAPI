use chrono::{DateTime, Datelike, Timelike};
use serde_derive::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct Time {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minutes: u8,
    seconds: u8,
}

impl<T> From<DateTime<T>> for Time
where
    T: chrono::TimeZone,
{
    fn from(value: DateTime<T>) -> Self {
        Self {
            year: value.year() as u16,
            month: value.month() as u8,
            day: value.day() as u8,
            hour: value.hour() as u8,
            minutes: value.minute() as u8,
            seconds: value.second() as u8,
        }
    }
}

#[derive(Deserialize)]
pub struct TimeZone {
    pub continent: String,
    pub region: String,
}
