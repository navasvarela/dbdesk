//! Values as text, the way SQL Server's own tools show them, and result columns' types.
//!
//! `tiberius` hands over dates and times as the wire's day and tick counts; they are turned
//! into calendar dates with `jiff`, which DbRover already depends on, rather than with
//! `tiberius`'s `chrono` or `time` features.

use dbrover_core::{Cell, ColumnInfo, Row};
use jiff::ToSpan;
use jiff::civil::{Date as CivilDate, DateTime as CivilDateTime};
use tiberius::ColumnType;
use tiberius::time::{Date, DateTime, DateTime2, DateTimeOffset, SmallDateTime, Time};
use tiberius::{Column, ColumnData};

/// Binary values longer than this are shown as their size, not their bytes: a result grid is
/// no place for a megabyte of hex.
const MAX_BINARY_HEX_BYTES: usize = 64;

/// Where `date`, `datetime2` and `datetimeoffset` count days from.
const DAY_ONE: CivilDate = CivilDate::constant(1, 1, 1);
/// Where `datetime` and `smalldatetime` count days from.
const DAY_1900: CivilDate = CivilDate::constant(1900, 1, 1);

/// `datetime` counts time in three-hundredths of a second.
const DATETIME_TICKS_PER_SECOND: i64 = 300;

pub(crate) fn column_info(column: &Column) -> ColumnInfo {
    ColumnInfo {
        name: column.name().to_owned(),
        type_name: type_name(column.column_type()).map(str::to_owned),
    }
}

/// The column's type as SQL Server names it, where the result's metadata says it exactly.
/// `Intn`, `Floatn` and the other nullable variants only say the type's family; their size
/// isn't kept.
fn type_name(column_type: ColumnType) -> Option<&'static str> {
    Some(match column_type {
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Float4 => "real",
        ColumnType::Float8 => "float",
        ColumnType::Money => "money",
        ColumnType::Money4 => "smallmoney",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Datetime => "datetime",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Decimaln => "decimal",
        ColumnType::Numericn => "numeric",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::BigVarBin => "varbinary",
        ColumnType::BigVarChar => "varchar",
        ColumnType::BigBinary => "binary",
        ColumnType::BigChar => "char",
        ColumnType::NVarchar => "nvarchar",
        ColumnType::NChar => "nchar",
        ColumnType::Xml => "xml",
        ColumnType::Text => "text",
        ColumnType::NText => "ntext",
        ColumnType::Image => "image",
        ColumnType::SSVariant => "sql_variant",
        ColumnType::Null
        | ColumnType::Intn
        | ColumnType::Floatn
        | ColumnType::Datetimen
        | ColumnType::Udt => return None,
    })
}

pub(crate) fn cells(row: tiberius::Row) -> Row {
    let types: Vec<ColumnType> = row.columns().iter().map(Column::column_type).collect();
    row.into_iter()
        .zip(types)
        .map(|(value, column_type)| cell(value, column_type))
        .collect()
}

fn cell(value: ColumnData<'static>, column_type: ColumnType) -> Cell {
    match value {
        ColumnData::U8(number) => number.map(|n| n.to_string()),
        ColumnData::I16(number) => number.map(|n| n.to_string()),
        ColumnData::I32(number) => number.map(|n| n.to_string()),
        ColumnData::I64(number) => number.map(|n| n.to_string()),
        ColumnData::F32(number) => number.map(|n| n.to_string()),
        // Money arrives as a float, already divided by 10,000; its four decimals are shown
        // as SQL Server shows them. Beyond about 10^11 a float can't hold all four exactly.
        ColumnData::F64(number)
            if matches!(column_type, ColumnType::Money | ColumnType::Money4) =>
        {
            number.map(|n| format!("{n:.4}"))
        }
        ColumnData::F64(number) => number.map(|n| n.to_string()),
        ColumnData::Bit(bit) => bit.map(|bit| if bit { "1" } else { "0" }.to_owned()),
        ColumnData::String(text) => text.map(|text| text.into_owned()),
        ColumnData::Guid(guid) => guid.map(|guid| guid.to_string().to_ascii_uppercase()),
        ColumnData::Binary(bytes) => bytes.map(|bytes| binary(&bytes)),
        ColumnData::Numeric(number) => number.map(|n| n.to_string()),
        ColumnData::Xml(xml) => xml.map(|xml| xml.to_string()),
        ColumnData::DateTime(value) => value.map(datetime),
        ColumnData::SmallDateTime(value) => value.map(small_datetime),
        ColumnData::Time(value) => value.map(time),
        ColumnData::Date(value) => value.map(date),
        ColumnData::DateTime2(value) => value.map(datetime2),
        ColumnData::DateTimeOffset(value) => value.map(datetime_offset),
    }
}

fn binary(bytes: &[u8]) -> String {
    if bytes.len() > MAX_BINARY_HEX_BYTES {
        return format!("<binary, {} bytes>", bytes.len());
    }
    format!(
        "0x{}",
        bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()
    )
}

/// `days` after `from`, or `None` past the calendar's end, which no value SQL Server sends
/// reaches.
fn day(from: CivilDate, days: i64) -> Option<CivilDate> {
    from.checked_add(days.days()).ok()
}

/// `2026-10-24`.
fn date(value: Date) -> String {
    day(DAY_ONE, i64::from(value.days())).map_or_else(
        || format!("<{} days after 0001-01-01>", value.days()),
        |date| date.to_string(),
    )
}

/// `07:28:23.9300000`, with as many fractional digits as the type's scale.
fn time(value: Time) -> String {
    let scale = u32::from(value.scale());
    let per_second = 10_u64.pow(scale);
    let seconds = value.increments() / per_second;
    let fraction = value.increments() % per_second;
    let clock = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    if scale == 0 {
        clock
    } else {
        format!("{clock}.{fraction:0width$}", width = scale as usize)
    }
}

/// `2026-10-24 07:28:23.9300000`.
fn datetime2(value: DateTime2) -> String {
    format!("{} {}", date(value.date()), time(value.time()))
}

/// `2026-10-24 09:28:23.9300000 +02:00`: the server sends the time in UTC and the offset
/// separately, and shows the local time.
fn datetime_offset(value: DateTimeOffset) -> String {
    let offset = i64::from(value.offset());
    let utc = value.datetime2();
    let local = day(DAY_ONE, i64::from(utc.date().days()))
        .map(|date| date.to_datetime(jiff::civil::Time::midnight()))
        .and_then(|midnight| {
            let increments = utc.time().increments();
            let per_second = 10_u64.pow(u32::from(utc.time().scale()));
            let seconds = i64::try_from(increments / per_second).ok()?;
            // Added one at a time: a span's units must share a sign.
            midnight
                .checked_add(seconds.seconds())
                .and_then(|utc| utc.checked_add(offset.minutes()))
                .ok()
                .map(|local| (local, increments % per_second))
        });
    let sign = if offset < 0 { '-' } else { '+' };
    let zone = format!("{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60);
    match local {
        Some((local, fraction)) => {
            let scale = usize::from(utc.time().scale());
            let fraction = if scale == 0 {
                String::new()
            } else {
                format!(".{fraction:0scale$}")
            };
            format!("{}{fraction} {zone}", local.strftime("%Y-%m-%d %H:%M:%S"))
        }
        None => format!("{} {zone} (UTC)", datetime2(utc)),
    }
}

/// `2026-10-24 07:28:23.930`, to the millisecond, as `datetime`'s three-hundredths of a
/// second round to.
fn datetime(value: DateTime) -> String {
    let ticks = i64::from(value.seconds_fragments());
    let milliseconds = (ticks * 1000 + DATETIME_TICKS_PER_SECOND / 2) / DATETIME_TICKS_PER_SECOND;
    clock_on(DAY_1900, i64::from(value.days()), milliseconds).map_or_else(
        || format!("<datetime {} days, {ticks} ticks>", value.days()),
        |at| at.strftime("%Y-%m-%d %H:%M:%S%.3f").to_string(),
    )
}

/// `2026-10-24 07:28:00`. `smalldatetime` keeps whole minutes, which is what
/// `tiberius` calls its `seconds_fragments`.
fn small_datetime(value: SmallDateTime) -> String {
    let milliseconds = i64::from(value.seconds_fragments()) * 60_000;
    clock_on(DAY_1900, i64::from(value.days()), milliseconds).map_or_else(
        || format!("<smalldatetime {} days>", value.days()),
        |at| at.strftime("%Y-%m-%d %H:%M:%S").to_string(),
    )
}

fn clock_on(from: CivilDate, days: i64, milliseconds: i64) -> Option<CivilDateTime> {
    day(from, days)?
        .to_datetime(jiff::civil::Time::midnight())
        .checked_add(milliseconds.milliseconds())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_dates_and_times_as_sql_server_does() {
        // 2026-10-24 is day 739,912 after 0001-01-01 and day 46,317 after 1900-01-01.
        let date_2026 = Date::new(739_912);
        let cases = [
            (date(date_2026), "2026-10-24"),
            (time(Time::new(269_039_300_000, 7)), "07:28:23.9300000"),
            (time(Time::new(26_903, 0)), "07:28:23"),
            (
                datetime2(DateTime2::new(date_2026, Time::new(269_039_300_000, 7))),
                "2026-10-24 07:28:23.9300000",
            ),
            (
                datetime_offset(DateTimeOffset::new(
                    DateTime2::new(date_2026, Time::new(26_903, 0)),
                    120,
                )),
                "2026-10-24 09:28:23 +02:00",
            ),
            (
                datetime_offset(DateTimeOffset::new(
                    DateTime2::new(date_2026, Time::new(10, 1)),
                    -330,
                )),
                "2026-10-23 18:30:01.0 -05:30",
            ),
            (
                datetime(DateTime::new(46_317, 26_903 * 300 + 279)),
                "2026-10-24 07:28:23.930",
            ),
            (
                small_datetime(SmallDateTime::new(46_317, 448)),
                "2026-10-24 07:28:00",
            ),
        ];
        for (written, expected) in cases {
            assert_eq!(written, expected);
        }
    }

    #[test]
    fn shows_short_binary_as_hex_and_long_binary_as_its_size() {
        assert_eq!(binary(&[0x1f, 0xa0]), "0x1FA0");
        assert_eq!(binary(&[0; 65]), "<binary, 65 bytes>");
    }
}
