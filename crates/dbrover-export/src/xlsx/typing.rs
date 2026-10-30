//! What each value becomes in a spreadsheet cell: a number, a boolean, a date or text.
//!
//! Values arrive as text (ADR 0004). A value becomes a number or a date only when Excel holds
//! it exactly and shows it as the database printed it, or as that same value in a date format;
//! anything else stays text, so an id or a code is never mangled. The rules, in
//! `docs/exports.md`:
//!
//! - A column's declared kind says what to try: numbers, booleans, dates or timestamps. A
//!   declared text column, or one of a kind Excel has nothing for, stays text.
//! - A column with no declared type (PostgreSQL) is typed value by value: a plain decimal
//!   number becomes a number, an ISO date or timestamp a date, everything else text.
//! - Never a number: a leading zero (`007`, a code rather than a quantity), more than 15
//!   significant digits (beyond what Excel's numbers hold exactly, as with long ids), `-0`,
//!   `NaN` and infinities.
//! - Never a date: one before 1900 (Excel's dates start there), a time zone or offset (Excel
//!   has none, and converting would change what the value says), or a fraction of a second
//!   finer than milliseconds (Excel's precision).

use dbrover_core::ValueKind;

/// Excel's numbers are IEEE doubles, exact to 15 significant decimal digits.
const MAX_EXACT_DIGITS: usize = 15;

/// The first year of Excel's 1900 date system, and its last.
const FIRST_YEAR: u16 = 1900;
const LAST_YEAR: u16 = 9999;

/// A value as a spreadsheet cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CellValue<'a> {
    Number(f64),
    Boolean(bool),
    Date(Date),
    DateTime(Date, Time),
    Text(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Date {
    pub(crate) year: u16,
    pub(crate) month: u8,
    pub(crate) day: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Time {
    pub(crate) hour: u16,
    pub(crate) minute: u8,
    pub(crate) second: u8,
    pub(crate) millisecond: u16,
    /// Whether the value showed a fraction of a second, which its format then shows too.
    pub(crate) fraction: bool,
}

/// `text`, from a column of `kind`, as a cell.
pub(crate) fn cell_value(kind: ValueKind, text: &str) -> CellValue<'_> {
    let typed = match kind {
        ValueKind::Integer | ValueKind::Decimal => exact_decimal(text).map(CellValue::Number),
        ValueKind::Float => float(text).map(CellValue::Number),
        ValueKind::Boolean => boolean(text).map(CellValue::Boolean),
        ValueKind::Date => date(text).map(CellValue::Date),
        ValueKind::Timestamp => date_time(text),
        ValueKind::Undeclared => exact_decimal(text)
            .map(CellValue::Number)
            .or_else(|| date(text).map(CellValue::Date))
            .or_else(|| date_time(text)),
        ValueKind::TimestampWithZone | ValueKind::Text => None,
    };
    typed.unwrap_or(CellValue::Text(text))
}

/// A plain decimal number Excel holds exactly: digits with an optional minus sign and
/// fraction, no exponent.
fn exact_decimal(text: &str) -> Option<f64> {
    let (whole, fraction) = split_number(text)?;
    let significant = format!("{whole}{fraction}");
    if significant.trim_start_matches('0').len() > MAX_EXACT_DIGITS {
        return None;
    }
    text.parse().ok()
}

/// A floating-point value as the database printed it: a decimal with an optional exponent.
/// A double printed in full is exact as a double, which Excel's numbers are.
fn float(text: &str) -> Option<f64> {
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (text, None),
    };
    if let Some(exponent) = exponent {
        let unsigned = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        digits(unsigned)?;
    }
    split_number(mantissa)?;
    text.parse().ok().filter(|number: &f64| number.is_finite())
}

/// A number's whole and fractional digits, if `text` is digits with an optional minus sign
/// and fraction, without a leading zero (`0` and `0.5` excepted) and other than zero
/// negated.
fn split_number(text: &str) -> Option<(&str, &str)> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let well_formed = !whole.is_empty()
        && all_digits(whole)
        && all_digits(fraction)
        && (unsigned.contains('.') == !fraction.is_empty());
    let leading_zero = whole.len() > 1 && whole.starts_with('0');
    let negative_zero = text.starts_with('-')
        && whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte == b'0');
    (well_formed && !leading_zero && !negative_zero).then_some((whole, fraction))
}

/// What databases print booleans as: `t` and `f` (PostgreSQL), `true` and `false`, `1` and
/// `0` (SQL Server's `bit`).
fn boolean(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "t" | "true" | "1" => Some(true),
        "f" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// `YYYY-MM-DD`, a real day from 1900 to 9999.
fn date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let date = Date {
        year: digits(&text[0..4])?,
        month: u8::try_from(digits(&text[5..7])?).ok()?,
        day: u8::try_from(digits(&text[8..10])?).ok()?,
    };
    let in_range = (FIRST_YEAR..=LAST_YEAR).contains(&date.year)
        && (1..=12).contains(&date.month)
        && (1..=days_in_month(date.year, date.month)).contains(&date.day);
    in_range.then_some(date)
}

/// `YYYY-MM-DD HH:MM:SS` (or with a `T`), with up to three digits of a second's fraction and
/// no time zone.
fn date_time(text: &str) -> Option<CellValue<'_>> {
    let (day, time) = text.split_at_checked(10)?;
    let date = date(day)?;
    let time = time.strip_prefix([' ', 'T'])?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let parts: Vec<&str> = clock.split(':').collect();
    let [hour, minute, second] = parts.as_slice() else {
        return None;
    };
    if [hour, minute, second].iter().any(|part| part.len() != 2) {
        return None;
    }
    let fraction_digits = fraction.trim_end_matches('0');
    if time.contains('.') && (fraction.is_empty() || fraction_digits.len() > 3) {
        return None;
    }
    let millisecond = if fraction_digits.is_empty() {
        0
    } else {
        digits(&format!("{fraction_digits:0<3}"))?
    };
    let time = Time {
        hour: digits(hour).filter(|&hour| hour < 24)?,
        minute: u8::try_from(digits(minute)?)
            .ok()
            .filter(|&minute| minute < 60)?,
        second: u8::try_from(digits(second)?)
            .ok()
            .filter(|&second| second < 60)?,
        millisecond,
        fraction: !fraction.is_empty(),
    };
    Some(CellValue::DateTime(date, time))
}

/// ASCII digits as a number; `None` for anything else.
fn digits(text: &str) -> Option<u16> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: u16, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    fn time(hour: u16, minute: u8, second: u8, millisecond: u16, fraction: bool) -> Time {
        Time {
            hour,
            minute,
            second,
            millisecond,
            fraction,
        }
    }

    /// The values known to go wrong in spreadsheets, and what each kind of column makes of
    /// them.
    #[test]
    fn types_values_by_the_rules() {
        use CellValue as V;
        use ValueKind as K;
        let cases: &[(ValueKind, &str, CellValue<'_>)] = &[
            (K::Integer, "42", V::Number(42.0)),
            (K::Integer, "-7", V::Number(-7.0)),
            (K::Integer, "007", V::Text("007")),
            (
                K::Integer,
                "123456789012345",
                V::Number(123_456_789_012_345.0),
            ),
            (K::Integer, "1234567890123456", V::Text("1234567890123456")),
            (K::Integer, "9007199254740993", V::Text("9007199254740993")),
            (K::Integer, "-0", V::Text("-0")),
            (K::Decimal, "9.50", V::Number(9.5)),
            (K::Decimal, "0.000001", V::Number(0.000_001)),
            (
                K::Decimal,
                "12345678901234.56",
                V::Text("12345678901234.56"),
            ),
            (K::Decimal, "1e5", V::Text("1e5")),
            (K::Decimal, "1,5", V::Text("1,5")),
            (K::Decimal, ".5", V::Text(".5")),
            (K::Float, "1e5", V::Number(100_000.0)),
            (K::Float, "-2.5E-3", V::Number(-0.0025)),
            (
                K::Float,
                "0.30000000000000004",
                V::Number(0.300_000_000_000_000_04),
            ),
            (K::Float, "NaN", V::Text("NaN")),
            (K::Float, "Infinity", V::Text("Infinity")),
            (K::Float, "inf", V::Text("inf")),
            (K::Float, "-0", V::Text("-0")),
            (K::Boolean, "t", V::Boolean(true)),
            (K::Boolean, "FALSE", V::Boolean(false)),
            (K::Boolean, "1", V::Boolean(true)),
            (K::Boolean, "maybe", V::Text("maybe")),
            (K::Date, "2024-02-29", V::Date(day(2024, 2, 29))),
            (K::Date, "2023-02-29", V::Text("2023-02-29")),
            (K::Date, "1899-12-31", V::Text("1899-12-31")),
            (K::Date, "20240101", V::Text("20240101")),
            (
                K::Timestamp,
                "2024-01-31 23:59:58",
                V::DateTime(day(2024, 1, 31), time(23, 59, 58, 0, false)),
            ),
            (
                K::Timestamp,
                "2024-01-31T08:00:00.5",
                V::DateTime(day(2024, 1, 31), time(8, 0, 0, 500, true)),
            ),
            (
                K::Timestamp,
                "2024-01-31 08:00:00.120000",
                V::DateTime(day(2024, 1, 31), time(8, 0, 0, 120, true)),
            ),
            (
                K::Timestamp,
                "2024-01-31 08:00:00.123456",
                V::Text("2024-01-31 08:00:00.123456"),
            ),
            (
                K::Timestamp,
                "2024-01-31 24:00:00",
                V::Text("2024-01-31 24:00:00"),
            ),
            (
                K::Timestamp,
                "2024-01-31 08:00:00+02",
                V::Text("2024-01-31 08:00:00+02"),
            ),
            (
                K::Timestamp,
                "2024-01-31T08:00:00Z",
                V::Text("2024-01-31T08:00:00Z"),
            ),
            (
                K::TimestampWithZone,
                "2024-01-31 08:00:00",
                V::Text("2024-01-31 08:00:00"),
            ),
            (K::Text, "42", V::Text("42")),
            (K::Undeclared, "42", V::Number(42.0)),
            (K::Undeclared, "00501", V::Text("00501")),
            (K::Undeclared, "1e5", V::Text("1e5")),
            (K::Undeclared, "t", V::Text("t")),
            (K::Undeclared, "2024-01-31", V::Date(day(2024, 1, 31))),
            (
                K::Undeclared,
                "2024-01-31 08:00:00",
                V::DateTime(day(2024, 1, 31), time(8, 0, 0, 0, false)),
            ),
            (
                K::Undeclared,
                "2024-01-31 08:00:00+00",
                V::Text("2024-01-31 08:00:00+00"),
            ),
            (K::Undeclared, "", V::Text("")),
        ];
        for (kind, text, expected) in cases {
            assert_eq!(cell_value(*kind, text), *expected, "{kind:?} {text:?}");
        }
    }
}
