//! The ISO week a report covers.
//!
//! A report is filed for exactly one ISO 8601 week (Monday–Sunday), and a trainee
//! may hold at most one report per week. [`IsoWeek`] is the value object that
//! carries that period: an `(ISO year, week number)` pair, validated against the
//! real calendar so an impossible week (e.g. week 53 of a short year) cannot be
//! constructed.
//!
//! Its canonical textual form is `YYYY-Www` (e.g. `2026-W24`, week zero-padded) —
//! the shape used on the wire and in storage. [`Display`](std::fmt::Display) and
//! [`FromStr`] are inverses over that form, and both funnel through
//! [`IsoWeek::new`] so the calendar validation has a single home.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::{ISOWeekDate, Weekday};
use jiff::tz::TimeZone;

use crate::DomainError;

/// An ISO 8601 week, identified by its ISO year and week number.
///
/// The ISO year can differ from the calendar year around the December/January
/// boundary, which is exactly why the week is stored as its own pair rather than
/// derived ad hoc from a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsoWeek {
    year: i16,
    week: i8,
}

impl IsoWeek {
    /// Construct a week, validating that `(year, week)` is a real ISO week.
    ///
    /// Validation is delegated to jiff's [`ISOWeekDate`]: weeks outside `1..=52`
    /// (or `1..=53` only in long ISO years) are rejected with
    /// [`DomainError::Invalid`].
    pub fn new(year: i16, week: i8) -> Result<Self, DomainError> {
        ISOWeekDate::new(year, week, Weekday::Monday)
            .map(|_| Self { year, week })
            .map_err(|_| {
                DomainError::Invalid(format!("not a valid ISO week: {year:04}-W{week:02}"))
            })
    }

    /// The ISO week containing `ts`, evaluated in UTC.
    ///
    /// Not used on the create path (the week is supplied by the trainee), but
    /// available for callers that want the current period.
    pub fn from_timestamp_utc(ts: Timestamp) -> Self {
        let wd = ts.to_zoned(TimeZone::UTC).iso_week_date();
        Self {
            year: wd.year(),
            week: wd.week(),
        }
    }

    /// The current ISO week in UTC.
    pub fn current_utc() -> Self {
        Self::from_timestamp_utc(Timestamp::now())
    }

    /// The ISO year.
    pub fn year(&self) -> i16 {
        self.year
    }

    /// The ISO week number (`1..=53`).
    pub fn week(&self) -> i8 {
        self.week
    }

    /// Whether `self` falls strictly after `other` in chronological ISO order.
    ///
    /// Both weeks store an ISO year (not a calendar year), so comparing the
    /// `(year, week)` pairs is the true chronological order, including across the
    /// December/January boundary (e.g. `2026-W01` is after `2025-W52`).
    pub fn is_after(&self, other: &IsoWeek) -> bool {
        (self.year, self.week) > (other.year, other.week)
    }
}

impl fmt::Display for IsoWeek {
    /// Render as `YYYY-Www`, e.g. `2026-W04` or `2026-W24`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-W{:02}", self.year, self.week)
    }
}

impl FromStr for IsoWeek {
    type Err = DomainError;

    /// Parse the canonical `YYYY-Www` form, reusing [`IsoWeek::new`]'s calendar
    /// validation. Anything else is [`DomainError::Invalid`].
    fn from_str(s: &str) -> Result<Self, DomainError> {
        let invalid =
            || DomainError::Invalid(format!("malformed ISO week {s:?}, expected YYYY-Www"));
        let (year, week) = s.split_once("-W").ok_or_else(invalid)?;
        let year: i16 = year.parse().map_err(|_| invalid())?;
        let week: i8 = week.parse().map_err(|_| invalid())?;
        Self::new(year, week)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_an_ordinary_week() {
        let w = IsoWeek::new(2026, 24).unwrap();
        assert_eq!(w.year(), 2026);
        assert_eq!(w.week(), 24);
    }

    #[test]
    fn display_zero_pads_the_week() {
        assert_eq!(IsoWeek::new(2026, 4).unwrap().to_string(), "2026-W04");
        assert_eq!(IsoWeek::new(2026, 24).unwrap().to_string(), "2026-W24");
    }

    #[test]
    fn from_str_round_trips_display() {
        for (y, w) in [(2026, 1), (2026, 4), (2026, 24)] {
            let week = IsoWeek::new(y, w).unwrap();
            assert_eq!(week.to_string().parse::<IsoWeek>().unwrap(), week);
        }
    }

    #[test]
    fn accepts_week_53_in_a_long_year() {
        // 2026 is an ISO long year (53 weeks).
        assert!(IsoWeek::new(2026, 53).is_ok());
        assert_eq!("2026-W53".parse::<IsoWeek>().unwrap().week(), 53);
    }

    #[test]
    fn rejects_week_53_in_a_short_year() {
        // 2025 has 52 ISO weeks.
        assert!(matches!(
            IsoWeek::new(2025, 53),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_out_of_range_and_malformed_strings() {
        for bad in [
            "2026-W00", "2026-W54", "2026-24", "garbage", "2026-W", "-W12",
        ] {
            assert!(
                matches!(bad.parse::<IsoWeek>(), Err(DomainError::Invalid(_))),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn is_after_orders_chronologically() {
        let w24 = IsoWeek::new(2026, 24).unwrap();
        let w25 = IsoWeek::new(2026, 25).unwrap();
        assert!(w25.is_after(&w24));
        assert!(!w24.is_after(&w25));
        // A week is not after itself (strict).
        assert!(!w24.is_after(&w24));

        // Across the ISO year boundary: 2026-W01 follows 2025-W52.
        let y2026_w1 = IsoWeek::new(2026, 1).unwrap();
        let y2025_w52 = IsoWeek::new(2025, 52).unwrap();
        assert!(y2026_w1.is_after(&y2025_w52));
        assert!(!y2025_w52.is_after(&y2026_w1));
    }
}
