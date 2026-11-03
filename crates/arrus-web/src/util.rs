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
