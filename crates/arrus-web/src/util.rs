//! Small formatting helpers.

/// `1.2 MB`, `811 KB`, `12 B`: binary (1024-based) units labelled KB/MB/GB,
/// matching the server, so a 5 GiB limit reads `5.0 GB`.
pub fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[u]) } else { format!("{v:.0} {}", UNITS[u]) }
}

/// Playback time `m:ss.d`.
pub fn fmt_time(secs: f64) -> String {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let tenths = (secs * 10.0).round() as u64;
    let (m, rest) = (tenths / 600, tenths % 600);
    format!("{m}:{:02}.{}", rest / 10, rest % 10)
}

/// Duration for lists: `10 s`, `2.5 s`.
pub fn fmt_secs(secs: f64) -> String {
    if (secs - secs.round()).abs() < 0.05 { format!("{:.0} s", secs.round()) } else { format!("{secs:.1} s") }
}

/// `just now`, `5 min ago`, `3 h ago`, `2 days ago`; `None` past a week (show a date).
pub fn fmt_ago(now: i64, then: i64) -> Option<String> {
    let d = (now - then).max(0);
    Some(match d {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", d / 60),
        3600..86400 => format!("{} h ago", d / 3600),
        86400..172800 => "yesterday".to_owned(),
        172800..604800 => format!("{} days ago", d / 86400),
        _ => return None,
    })
}

/// (year, month 1–12, day 1–31) of a UTC day number (Unix seconds / 86 400).
pub fn civil(day: i64) -> (i64, u32, u32) {
    // Howard Hinnant's days-to-civil.
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `12 Nov`.
pub fn fmt_day(day: i64) -> String {
    let (_, m, d) = civil(day);
    format!("{d} {}", MONTHS[m as usize - 1])
}

/// `12 Nov 2026`.
pub fn fmt_date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{d} {} {y}", MONTHS[m as usize - 1])
}

/// Monday = 0 … Sunday = 6 (1 Jan 1970 was a Thursday).
pub fn weekday(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

/// `45 min`, `3 h 05`, `2 h`, `120 h`.
pub fn fmt_minutes(m: u64) -> String {
    match m {
        0..60 => format!("{m} min"),
        60..6000 if !m.is_multiple_of(60) => format!("{} h {:02}", m / 60, m % 60),
        _ => format!("{} h", m / 60),
    }
}

/// `12.3k`, `999`.
pub fn fmt_count(n: f64) -> String {
    if n >= 10_000.0 {
        format!("{:.0}k", n / 1000.0)
    } else if n >= 1000.0 {
        format!("{:.1}k", n / 1000.0)
    } else if n.fract().abs() > 0.05 && n < 10.0 {
        format!("{n:.1}")
    } else {
        format!("{n:.0}")
    }
}

// ---- charts -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum Agg {
    Sum,
    /// Per-day average (counts of people: summing days double-counts).
    Mean,
    Last,
}

/// Days grouped so the chart has at most ~90 columns.
pub fn bucket_size(n: usize) -> usize {
    n.div_ceil(90).max(1)
}

pub fn bucketed(values: &[f64], size: usize, agg: Agg) -> Vec<f64> {
    values
        .chunks(size)
        .map(|c| match agg {
            Agg::Sum => c.iter().sum(),
            Agg::Mean => c.iter().sum::<f64>() / c.len() as f64,
            Agg::Last => *c.last().unwrap_or(&0.0),
        })
        .collect()
}

/// A round axis top at or above `max`: 1, 2, 5 × 10ⁿ (at least 1).
pub fn nice_max(max: f64) -> f64 {
    if max <= 1.0 {
        return 1.0;
    }
    let mag = 10f64.powf(max.log10().floor());
    for f in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if f * mag >= max {
            return f * mag;
        }
    }
    10.0 * mag
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Critical,
}

/// Storage meter colour: amber from 80 %, red from 95 %.
pub fn storage_level(used: u64, limit: u64) -> Level {
    if limit == 0 {
        return Level::Critical;
    }
    let f = used as f64 / limit as f64;
    if f >= 0.95 {
        Level::Critical
    } else if f >= 0.80 {
        Level::Warn
    } else {
        Level::Ok
    }
}

/// Header-safe file name: non-ASCII or control characters are percent-encoded.
pub fn header_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii() && !ch.is_ascii_control() && ch != '%' {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// File-name extension, lower-case.
pub fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(fmt_bytes(12), "12 B");
        assert_eq!(fmt_bytes(1_500), "1.5 KB");
        assert_eq!(fmt_bytes(2_250), "2.2 KB");
        assert_eq!(fmt_bytes(830_000), "811 KB");
        assert_eq!(fmt_bytes(100 * 1024 * 1024), "100 MB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn times() {
        assert_eq!(fmt_time(0.0), "0:00.0");
        assert_eq!(fmt_time(65.34), "1:05.3");
        assert_eq!(fmt_time(59.96), "1:00.0");
        assert_eq!(fmt_time(f64::NAN), "0:00.0");
        assert_eq!(fmt_secs(10.0), "10 s");
        assert_eq!(fmt_secs(2.5), "2.5 s");
    }

    #[test]
    fn ago() {
        assert_eq!(fmt_ago(100, 90).unwrap(), "just now");
        assert_eq!(fmt_ago(1000, 400).unwrap(), "10 min ago");
        assert_eq!(fmt_ago(10_000, 2_000).unwrap(), "2 h ago");
        assert_eq!(fmt_ago(100_000, 0).unwrap(), "yesterday");
        assert_eq!(fmt_ago(300_000, 0).unwrap(), "3 days ago");
        assert_eq!(fmt_ago(10_000_000, 0), None);
        assert_eq!(fmt_ago(0, 100).unwrap(), "just now");
    }

    #[test]
    fn days() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(20_404), (2025, 11, 12));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(fmt_day(20_769), "12 Nov");
        assert_eq!(fmt_date(20_769), "12 Nov 2026");
        assert_eq!(weekday(0), 3);
        assert_eq!(weekday(20_769), 3, "12 Nov 2026 is a Thursday");
        assert_eq!(fmt_minutes(45), "45 min");
        assert_eq!(fmt_minutes(185), "3 h 05");
        assert_eq!(fmt_minutes(120), "2 h");
        assert_eq!(fmt_minutes(7200), "120 h");
        assert_eq!(fmt_count(12_345.0), "12k");
        assert_eq!(fmt_count(1_234.0), "1.2k");
        assert_eq!(fmt_count(2.5), "2.5");
        assert_eq!(fmt_count(42.0), "42");
    }

    #[test]
    fn chart_axes_and_buckets() {
        assert_eq!(nice_max(0.0), 1.0);
        assert_eq!(nice_max(3.0), 5.0);
        assert_eq!(nice_max(10.0), 10.0);
        assert_eq!(nice_max(11.0), 20.0);
        assert_eq!(nice_max(240.0), 250.0);
        assert_eq!(nice_max(999.0), 1000.0);
        assert_eq!(bucket_size(30), 1);
        assert_eq!(bucket_size(90), 1);
        assert_eq!(bucket_size(365), 5);
        assert_eq!(bucket_size(730), 9);
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(bucketed(&v, 2, Agg::Sum), vec![3.0, 7.0, 5.0]);
        assert_eq!(bucketed(&v, 2, Agg::Mean), vec![1.5, 3.5, 5.0]);
        assert_eq!(bucketed(&v, 2, Agg::Last), vec![2.0, 4.0, 5.0]);
    }

    #[test]
    fn storage() {
        assert_eq!(storage_level(79, 100), Level::Ok);
        assert_eq!(storage_level(80, 100), Level::Warn);
        assert_eq!(storage_level(95, 100), Level::Critical);
        assert_eq!(storage_level(0, 0), Level::Critical);
    }

    #[test]
    fn filenames() {
        assert_eq!(header_filename("Show 1.arrus"), "Show 1.arrus");
        assert_eq!(header_filename("Chorégraphie.arrus"), "Chor%C3%A9graphie.arrus");
        assert_eq!(header_filename("100%.mp3"), "100%25.mp3");
        assert_eq!(extension("a.B.ARRUS"), "arrus");
        assert_eq!(extension("noext"), "");
    }
}
