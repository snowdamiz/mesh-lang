//! DateTime stdlib module for Mesh (Phase 136).
//!
//! All DateTime values are i64 Unix milliseconds at the ABI level.
//! The supported calendar range and RFC 3339 behavior retain the original
//! DateTime contract, independently of the underlying calendar library.

use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::io::{err_result, ok_int, MeshResult};
use crate::string::{mesh_str, MeshString};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

// Original DateTime bounds: -262143-01-01 through +262142-12-31 (UTC).
const MIN_MS: i64 = -8_334_601_228_800_000;
const MAX_MS: i64 = 8_210_266_876_799_999;

fn supported_ms(ms: i64) -> Option<i64> {
    (MIN_MS..=MAX_MS).contains(&ms).then_some(ms)
}

fn parse_iso8601(text: &str) -> Option<i64> {
    // time accepts any one-byte separator; the original parser accepted only these three.
    let prefix = text.as_bytes().get(..19)?;
    if !prefix.is_ascii() || !matches!(prefix[10], b'T' | b't' | b' ') {
        return None;
    }

    let mut normalized = Cow::Borrowed(text);
    // Chrono accepted leap seconds at any minute and represented them as
    // the following Unix second, retaining the fractional part. time instead
    // validates leap-second dates and clamps the fraction. Parse :59 and add
    // the extra second after UTC normalization.
    let leap = &prefix[17..19] == b"60";
    if leap {
        normalized.to_mut().replace_range(17..19, "59");
    }
    // Chrono also accepted U+2212 as the timezone minus sign. Only normalize
    // that sign in the numeric offset, not punctuation elsewhere in the input.
    if let Some((prefix, offset)) = text.rsplit_once('−') {
        if prefix.len() >= 19 && offset.len() == 5 {
            normalized
                .to_mut()
                .replace_range(prefix.len()..prefix.len() + '−'.len_utf8(), "-");
        }
    }
    let dt = OffsetDateTime::parse(&normalized, &Rfc3339).ok()?;
    let ms =
        dt.unix_timestamp() * 1_000 + i64::from(dt.millisecond()) + if leap { 1_000 } else { 0 };
    supported_ms(ms)
}

// ── utc_now ──────────────────────────────────────────────────────────────────

/// DateTime.utc_now() -> DateTime (i64 ms)
///
/// Uses std::time::SystemTime without a local timezone dependency.
#[no_mangle]
pub extern "C" fn mesh_datetime_utc_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_millis() as i64
}

// ── ISO 8601 parse / format ───────────────────────────────────────────────────

/// DateTime.from_iso8601(s) -> Result<DateTime, String>
///
/// Accepts RFC 3339 with T, t, or space separator. Normalizes timezone offsets to UTC.
/// Returns Err for naive strings (no timezone) or malformed input.
#[no_mangle]
pub extern "C" fn mesh_datetime_from_iso8601(s: *const MeshString) -> *mut MeshResult {
    unsafe {
        let text = (*s).as_str();
        match parse_iso8601(text) {
            None => {
                let e = "invalid ISO 8601 datetime";
                err_result(e)
            }
            Some(ms) => {
                // The codegen that unpacks Result<DateTime, String> reads the
                // Ok payload as a boxed i64 (same as SqliteConn).
                ok_int(ms)
            }
        }
    }
}

/// DateTime.to_iso8601(dt) -> String
///
/// Always emits Z suffix and 3 decimal digits: "2024-01-15T10:30:00.000Z"
#[no_mangle]
pub extern "C" fn mesh_datetime_to_iso8601(ms: i64) -> *mut MeshString {
    let dt = supported_ms(ms)
        .and_then(|ms| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok())
        .expect("mesh_datetime_to_iso8601: ms out of range");
    let year = dt.year();
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else {
        format!("{year:+05}")
    };
    mesh_str(&format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        dt.millisecond()
    ))
}

// ── Unix timestamp interop ────────────────────────────────────────────────────

/// DateTime.from_unix_ms(ms) -> Result<DateTime, String>
#[no_mangle]
pub extern "C" fn mesh_datetime_from_unix_ms(ms: i64) -> *mut MeshResult {
    match supported_ms(ms) {
        None => {
            let e = "unix timestamp out of range";
            err_result(e)
        }
        Some(ms) => ok_int(ms),
    }
}

/// DateTime.to_unix_ms(dt) -> Int
#[no_mangle]
pub extern "C" fn mesh_datetime_to_unix_ms(ms: i64) -> i64 {
    ms // Already stored as ms — identity conversion
}

/// DateTime.from_unix_secs(s) -> Result<DateTime, String>
#[no_mangle]
pub extern "C" fn mesh_datetime_from_unix_secs(secs: i64) -> *mut MeshResult {
    match secs.checked_mul(1_000).and_then(supported_ms) {
        None => {
            let e = "unix timestamp out of range";
            err_result(e)
        }
        Some(ms) => ok_int(ms),
    }
}

/// DateTime.to_unix_secs(dt) -> Int
#[no_mangle]
pub extern "C" fn mesh_datetime_to_unix_secs(ms: i64) -> i64 {
    ms / 1_000
}

// ── Duration arithmetic ───────────────────────────────────────────────────────

/// DateTime.add(dt, n, unit) -> DateTime
///
/// Supported units (atom literals without leading colon): ms, second, minute, hour, day, week
/// An unknown unit, or a result outside the representable range, is a Mesh
/// panic. n can be negative (equivalent to subtraction).
#[no_mangle]
pub extern "C-unwind" fn mesh_datetime_add(ms: i64, n: i64, unit: *const MeshString) -> i64 {
    // Atom literals are lowered without the leading ':' (e.g. :day -> "day").
    let unit = unsafe { (*unit).as_str() };
    let factor = match unit {
        "ms" => {
            // The original milliseconds constructor panicked for i64::MIN,
            // unlike the fallible constructors for the other units.
            assert!(n != i64::MIN, "TimeDelta::milliseconds out of bounds");
            1
        }
        "second" => 1_000,
        "minute" => 60_000,
        "hour" => 3_600_000,
        "day" => 86_400_000,
        "week" => 604_800_000,
        other => crate::panic::raise(format_args!(
            "DateTime.add: unknown unit {other:?}; valid units are :ms, :second, \
             :minute, :hour, :day, :week"
        )),
    };
    supported_ms(ms)
        .zip(n.checked_mul(factor))
        .and_then(|(ms, delta)| ms.checked_add(delta))
        .and_then(supported_ms)
        .unwrap_or_else(|| {
            crate::panic::raise(format_args!(
                "DateTime.add: adding {n} {unit} to {ms} ms is out of the supported range"
            ))
        })
}

/// DateTime.diff(dt1, dt2, unit) -> Float
///
/// Returns (dt1 - dt2) in the given unit as f64.
/// Positive if dt1 is after dt2, negative if dt1 is before dt2.
/// Supported units (atom literals without leading colon): ms, second, minute, hour, day, week
#[no_mangle]
pub extern "C-unwind" fn mesh_datetime_diff(
    dt1_ms: i64,
    dt2_ms: i64,
    unit: *const MeshString,
) -> f64 {
    // Atom literals are lowered without the leading ':' (e.g. :day -> "day").
    let unit = unsafe { (*unit).as_str() };
    let delta_ms = (dt1_ms as i128 - dt2_ms as i128) as f64;
    match unit {
        "ms" => delta_ms,
        "second" => delta_ms / 1_000.0,
        "minute" => delta_ms / 60_000.0,
        "hour" => delta_ms / 3_600_000.0,
        "day" => delta_ms / 86_400_000.0,
        "week" => delta_ms / 604_800_000.0,
        other => crate::panic::raise(format_args!(
            "DateTime.diff: unknown unit {other:?}; valid units are :ms, :second, \
             :minute, :hour, :day, :week"
        )),
    }
}

// ── Comparison ────────────────────────────────────────────────────────────────

/// DateTime.is_before(dt1, dt2) -> Bool (i8: 1=true, 0=false)
#[no_mangle]
pub extern "C" fn mesh_datetime_before(dt1_ms: i64, dt2_ms: i64) -> i8 {
    if dt1_ms < dt2_ms {
        1
    } else {
        0
    }
}

/// DateTime.is_after(dt1, dt2) -> Bool (i8: 1=true, 0=false)
#[no_mangle]
pub extern "C" fn mesh_datetime_after(dt1_ms: i64, dt2_ms: i64) -> i8 {
    if dt1_ms > dt2_ms {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_result(result: *mut MeshResult, expected: Option<i64>, error: &str) {
        unsafe {
            let result = &*result;
            match expected {
                Some(ms) => {
                    assert_eq!(result.tag, 0);
                    assert_eq!(*result.value.cast::<i64>(), ms);
                }
                None => {
                    assert_eq!(result.tag, 1);
                    assert_eq!((*result.value.cast::<MeshString>()).as_str(), error);
                }
            }
        }
    }

    // Golden values captured from the original locked Chrono 0.4.44 parser.
    #[test]
    fn rfc3339_compatibility() {
        crate::gc::mesh_rt_init();
        for (input, expected) in [
            ("1970-01-01T00:00:00Z", Some(0)),
            ("1970-01-01t00:00:00.001z", Some(1)),
            ("1969-12-31 23:59:59.99999999999999999999Z", Some(-1)),
            ("1969-12-31T23:59:59.00000000001Z", Some(-1000)),
            (
                "2024-01-15T10:30:00.123456789987654321+05:30",
                Some(1705294800123),
            ),
            ("2024-01-15T10:30:00.12-00:30", Some(1705316400120)),
            ("2024-01-15T10:30:00.12−00:30", Some(1705316400120)),
            ("2024-01-15T10:30:00-00:00", Some(1705314600000)),
            ("2024-01-15T10:30:00+23:59", Some(1705228260000)),
            ("2024-01-15T10:30:00-23:59", Some(1705400940000)),
            ("2016-12-31T23:59:60.123456789Z", Some(1483228800123)),
            (
                "2017-01-01T00:59:60.999999999999999+01:00",
                Some(1483228800999),
            ),
            ("1970-01-01T00:00:60.5Z", Some(60500)),
            ("1969-12-31T23:59:60.5Z", Some(500)),
            ("2024-02-29T12:34:60Z", Some(1709210100000)),
            ("0000-01-01T00:00:00+00:01", Some(-62167219260000)),
            ("0000-02-29T00:00:00Z", Some(-62162121600000)),
            ("9999-12-31T23:59:59.999-00:01", Some(253402300859999)),
            ("9999-12-31T23:59:60.999Z", Some(253402300800999)),
            ("-0001-12-31T23:59:00Z", None),
            ("+10000-01-01T00:00:59.999Z", None),
            ("10000-01-01T00:00:00Z", None),
            ("+0000-01-01T00:00:00Z", None),
            ("2024-01-15X10:30:00Z", None),
            ("2024-01-15\t10:30:00Z", None),
            ("2024-01-15T10:30:00", None),
            ("2024-01-15T10:30:00.Z", None),
            ("2024-01-15T10:30:00,123Z", None),
            ("2024-01-15T10:30:00.123Z ", None),
            ("2024-01-15T10:30:00 Z", None),
            ("2024-01-15T10:30:00+0530", None),
            ("2024-01-15T10:30:00+05:60", None),
            ("2024-01-15T10:30:00+24:00", None),
            ("2024-01-15T10:30:00+00:00:00", None),
            ("2023-02-29T00:00:00Z", None),
            ("2024-04-31T00:00:00Z", None),
            ("2024-01-15T24:00:00Z", None),
            ("2024-01-15T10:60:00Z", None),
            ("2024-01-15T10:30:61Z", None),
            ("2024-1-15T10:30:00Z", None),
            ("2024-01-15T1:30:00Z", None),
            ("2024-01-15T10:30:00.１２３Z", None),
            ("", None),
        ] {
            assert_eq!(parse_iso8601(input), expected, "{input:?}");
            assert_result(
                mesh_datetime_from_iso8601(mesh_str(input)),
                expected,
                "invalid ISO 8601 datetime",
            );
        }
    }

    // Golden values captured from DateTime::from_timestamp_millis and
    // to_rfc3339_opts(SecondsFormat::Millis, true) on Chrono 0.4.44.
    #[test]
    fn unix_milliseconds_and_format_compatibility() {
        crate::gc::mesh_rt_init();
        for (ms, expected) in [
            (-8334601228800001, None),
            (-8334601228800000, Some("-262143-01-01T00:00:00.000Z")),
            (-8334601228799999, Some("-262143-01-01T00:00:00.001Z")),
            (-62167219200001, Some("-0001-12-31T23:59:59.999Z")),
            (-62167219200000, Some("0000-01-01T00:00:00.000Z")),
            (-1, Some("1969-12-31T23:59:59.999Z")),
            (0, Some("1970-01-01T00:00:00.000Z")),
            (1, Some("1970-01-01T00:00:00.001Z")),
            (253402300799999, Some("9999-12-31T23:59:59.999Z")),
            (253402300800000, Some("+10000-01-01T00:00:00.000Z")),
            (8210266876799998, Some("+262142-12-31T23:59:59.998Z")),
            (8210266876799999, Some("+262142-12-31T23:59:59.999Z")),
            (8210266876800000, None),
            (-9223372036854775808, None),
            (9223372036854775807, None),
        ] {
            assert_result(
                mesh_datetime_from_unix_ms(ms),
                expected.map(|_| ms),
                "unix timestamp out of range",
            );
            if let Some(text) = expected {
                let formatted = mesh_datetime_to_iso8601(ms);
                assert_eq!(unsafe { (*formatted).as_str() }, text);
            }
        }
    }

    #[test]
    fn unix_seconds_and_arithmetic_compatibility() {
        crate::gc::mesh_rt_init();
        for (secs, expected) in [
            (-8334601228801, None),
            (-8334601228800, Some(-8334601228800000)),
            (-1, Some(-1000)),
            (0, Some(0)),
            (8210266876799, Some(8210266876799000)),
            (8210266876800, None),
            (i64::MIN, None),
            (i64::MAX, None),
        ] {
            assert_result(
                mesh_datetime_from_unix_secs(secs),
                expected,
                "unix timestamp out of range",
            );
        }

        assert_eq!(mesh_datetime_to_unix_secs(-999), 0);
        assert_eq!(mesh_datetime_to_unix_secs(-1001), -1);
        assert_eq!(mesh_datetime_to_unix_ms(i64::MIN), i64::MIN);
        for (unit, millis) in [
            ("ms", 1),
            ("second", 1000),
            ("minute", 60000),
            ("hour", 3600000),
            ("day", 86400000),
            ("week", 604800000),
        ] {
            let unit = mesh_str(unit);
            assert_eq!(mesh_datetime_add(-1, 2, unit), 2 * millis - 1);
            assert_eq!(mesh_datetime_add(1, -2, unit), 1 - 2 * millis);
            assert_eq!(mesh_datetime_diff(0, millis, unit), -1.0);
        }

        assert_eq!(mesh_datetime_add(MIN_MS, 1, mesh_str("ms")), MIN_MS + 1);
        assert_eq!(mesh_datetime_add(MAX_MS, -1, mesh_str("ms")), MAX_MS - 1);
        for (ms, n, unit) in [
            (MIN_MS, -1, "ms"),
            (MAX_MS, 1, "ms"),
            (MAX_MS + 1, -1, "ms"),
            (MIN_MS - 1, 1, "ms"),
            (0, i64::MAX, "ms"),
            (0, i64::MAX, "week"),
            (0, i64::MIN, "second"),
        ] {
            let panic =
                std::panic::catch_unwind(|| mesh_datetime_add(ms, n, mesh_str(unit))).unwrap_err();
            assert_eq!(
                crate::panic::MeshPanic::of(&*panic).unwrap().reason(),
                format!("Mesh panic: DateTime.add: adding {n} {unit} to {ms} ms is out of the supported range"),
            );
        }
        let panic = std::panic::catch_unwind(|| mesh_datetime_add(0, i64::MIN, mesh_str("ms")))
            .unwrap_err();
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
        assert_eq!(message, Some("TimeDelta::milliseconds out of bounds"));
        let panic = std::panic::catch_unwind(|| mesh_datetime_add(0, 1, mesh_str("fortnight")))
            .unwrap_err();
        assert_eq!(
            crate::panic::MeshPanic::of(&*panic).unwrap().reason(),
            "Mesh panic: DateTime.add: unknown unit \"fortnight\"; valid units are :ms, :second, :minute, :hour, :day, :week",
        );
    }

    /// A timestamp outside the supported range is an error; a difference is in the
    /// unit asked for, and an unknown unit is a Mesh panic.
    #[test]
    fn out_of_range_timestamps_and_units() {
        crate::gc::mesh_rt_init();
        let tag = |result: *mut MeshResult| unsafe { (*result).tag };
        assert_eq!(tag(mesh_datetime_from_unix_ms(i64::MAX)), 1);
        assert_eq!(tag(mesh_datetime_from_unix_secs(i64::MAX)), 1);
        let week = 604_800_000;
        assert_eq!(mesh_datetime_diff(week, 0, mesh_str("week")), 1.0);
        let unknown = std::panic::catch_unwind(|| mesh_datetime_diff(0, 0, mesh_str("fortnight")));
        let panic = unknown.unwrap_err();
        let message = crate::panic::MeshPanic::of(&*panic).unwrap().reason();
        assert!(
            message.contains("DateTime.diff: unknown unit \"fortnight\""),
            "{message}"
        );
    }
}
