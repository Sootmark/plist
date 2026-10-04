//! Property list dates: seconds since 2001-01-01 UTC in binary files,
//! ISO 8601 text in XML.

use common::time::{days_from_civil, Precision, Semantic, Ts, TICKS_PER_SECOND};

/// Seconds from 1970-01-01 to 2001-01-01, the reference date.
const REFERENCE_DATE_UNIX_SECONDS: i64 = 978_307_200;

/// A binary date: seconds since the reference date, as a double.
///
/// Zero is the reference date itself, not "never set": a property list
/// stores a date only when there is one. Values too far from it to be
/// represented (or not finite) give an invalid timestamp.
pub(crate) fn from_reference_seconds(seconds: f64) -> Ts {
    if seconds == 0.0 {
        return Ts::from_ticks(
            REFERENCE_DATE_UNIX_SECONDS * TICKS_PER_SECOND,
            Precision::Microsecond,
        );
    }
    Ts::from_cocoa_seconds(seconds)
}

/// An XML date, `YYYY[-MM[-DD[THH[:MM[:SS]]]]]Z` as Apple's readers (and
/// Python's) accept it: omitted fields are the start of the period.
/// `None` when the text isn't of that form or names a time that doesn't
/// exist (February 30, hour 24).
pub(crate) fn from_iso8601(text: &str) -> Option<Ts> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T').unwrap_or((text, ""));
    let mut date = date.split('-');
    let year = field(date.next()?, 4)?;
    let month = optional_field(date.next(), 1)?;
    let day = optional_field(date.next(), 1)?;
    let mut time = time.split(':').filter(|part| !part.is_empty());
    let hour = optional_field(time.next(), 0)?;
    let minute = optional_field(time.next(), 0)?;
    let second = optional_field(time.next(), 0)?;
    if date.next().is_some() || time.next().is_some() {
        return None;
    }
    let seconds = clock_seconds(hour, minute, second)?;
    let days = calendar_days(year, month, day)?;
    let ticks = (days * 86_400 + seconds) * TICKS_PER_SECOND;
    let ts = Ts::from_ticks(ticks, Precision::Second);
    (ts.semantic() == Semantic::Utc).then_some(ts)
}

/// A field of exactly `digits` ASCII digits.
fn field(text: &str, digits: usize) -> Option<i64> {
    let digits_only = text.len() == digits && text.bytes().all(|b| b.is_ascii_digit());
    digits_only.then(|| text.parse().ok())?
}

/// A two-digit field, or `default` when the text stops before it.
fn optional_field(text: Option<&str>, default: i64) -> Option<i64> {
    text.map_or(Some(default), |text| field(text, 2))
}

fn clock_seconds(hour: i64, minute: i64, second: i64) -> Option<i64> {
    (hour < 24 && minute < 60 && second < 60).then_some(hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 of a date that exists in the proleptic Gregorian
/// calendar.
fn calendar_days(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month as u32, day as u32);
    let round_trip = common::time::civil_from_days(days);
    (round_trip == (year, month as u32, day as u32)).then_some(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(ts: Ts) -> String {
        ts.to_iso8601().unwrap()
    }

    #[test]
    fn binary_dates_count_from_2001() {
        assert_eq!(
            iso(from_reference_seconds(0.0)),
            "2001-01-01T00:00:00.0000000Z"
        );
        assert_eq!(
            iso(from_reference_seconds(-31_622_400.0)),
            "2000-01-01T00:00:00.0000000Z"
        );
        assert_eq!(
            from_reference_seconds(f64::NAN).semantic(),
            Semantic::Invalid
        );
    }

    #[test]
    fn xml_dates_may_stop_early() {
        assert_eq!(
            iso(from_iso8601("2019-09-16T18:42:44Z").unwrap()),
            "2019-09-16T18:42:44.0000000Z"
        );
        assert_eq!(
            iso(from_iso8601("2019-09Z").unwrap()),
            "2019-09-01T00:00:00.0000000Z"
        );
        assert_eq!(
            iso(from_iso8601("2019-09-16T18Z").unwrap()),
            "2019-09-16T18:00:00.0000000Z"
        );
        assert_eq!(
            iso(from_iso8601("0000-12-30T00:00:00Z").unwrap()),
            "0000-12-30T00:00:00.0000000Z"
        );
    }

    #[test]
    fn impossible_xml_dates_are_refused() {
        for text in [
            "2019-02-30T00:00:00Z",
            "2019-09-16T24:00:00Z",
            "2019-09-16T18:42:44",
            "19-09-16Z",
            "2019-09-16T18:42:44:01Z",
            "2019-09-16-01Z",
            "",
        ] {
            assert_eq!(from_iso8601(text), None, "{text}");
        }
    }
}
