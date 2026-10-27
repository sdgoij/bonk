//! Dates as business dates, not instants.
//!
//! An invoice date is a calendar date. Converting it through a timezone is how
//! documents end up filed in the wrong VAT period, so `LocalDate` deliberately
//! carries no time and no offset.

use time::Date;
use time::format_description::well_known::Iso8601;

use crate::error::DomainError;

/// A calendar date in ISO 8601 form.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate(Date);

impl LocalDate {
    /// Today's date in UTC.
    ///
    /// Only for defaults in the UI; documents always store the user's chosen
    /// business date.
    pub fn today_utc() -> Self {
        Self(time::OffsetDateTime::now_utc().date())
    }

    /// Parses `YYYY-MM-DD`.
    ///
    /// The shape is checked before parsing so that a full datetime
    /// (`2026-10-27T00:00:00Z`) is rejected rather than silently truncated to
    /// its date prefix.
    pub fn parse_iso(value: &str) -> Result<Self, DomainError> {
        let bytes = value.as_bytes();
        let shape_ok = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
        if !shape_ok {
            return Err(DomainError::InvalidDate(value.to_owned()));
        }
        Date::parse(value, &Iso8601::DATE)
            .map(Self)
            .map_err(|_| DomainError::InvalidDate(value.to_owned()))
    }

    /// Builds a date from its parts.
    pub fn from_ymd(year: i32, month: u8, day: u8) -> Result<Self, DomainError> {
        let month = time::Month::try_from(month)
            .map_err(|_| DomainError::InvalidDate(format!("{year:04}-{month:02}-{day:02}")))?;
        Date::from_calendar_date(year, month, day)
            .map(Self)
            .map_err(|_| DomainError::InvalidDate(format!("{year:04}-{month:02}-{day:02}")))
    }

    /// The underlying `time::Date`.
    pub const fn as_date(self) -> Date {
        self.0
    }

    /// Adds a number of days, rejecting overflow.
    pub fn plus_days(self, days: i64) -> Result<Self, DomainError> {
        self.0
            .checked_add(time::Duration::days(days))
            .map(Self)
            .ok_or(DomainError::DateOutOfRange)
    }

    /// Formats as `YYYY-MM-DD`.
    pub fn to_iso_string(self) -> String {
        self.0.format(&Iso8601::DATE).unwrap_or_default()
    }
}

impl std::fmt::Display for LocalDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_iso_string())
    }
}

impl std::fmt::Debug for LocalDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalDate({})", self.to_iso_string())
    }
}

impl std::str::FromStr for LocalDate {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_iso(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_iso() {
        let date = LocalDate::parse_iso("2026-10-27").unwrap();
        assert_eq!(date.to_iso_string(), "2026-10-27");
        assert_eq!(date.to_string(), "2026-10-27");
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        assert!(LocalDate::parse_iso("2026-02-30").is_err());
        assert!(LocalDate::parse_iso("2026-13-01").is_err());
        assert!(LocalDate::parse_iso("27-10-2026").is_err());
        assert!(LocalDate::parse_iso("2026-10-27T00:00:00Z").is_err());
        assert!(LocalDate::parse_iso("").is_err());
    }

    #[test]
    fn builds_from_parts_and_handles_leap_days() {
        assert!(LocalDate::from_ymd(2024, 2, 29).is_ok());
        assert!(LocalDate::from_ymd(2025, 2, 29).is_err());
        assert!(LocalDate::from_ymd(2026, 0, 10).is_err());
    }

    #[test]
    fn date_arithmetic_never_crosses_a_timezone() {
        let issued = LocalDate::parse_iso("2026-03-31").unwrap();
        assert_eq!(issued.plus_days(1).unwrap().to_iso_string(), "2026-04-01");
        assert_eq!(issued.plus_days(-31).unwrap().to_iso_string(), "2026-02-28");
    }

    #[test]
    fn ordering_is_chronological() {
        let earlier = LocalDate::parse_iso("2026-01-31").unwrap();
        let later = LocalDate::parse_iso("2026-02-01").unwrap();
        assert!(earlier < later);
    }
}
