//! RSS 2.0 + iTunes podcast feed rendering for the library.
//!
//! Turns a set of archived videos into a podcast/media feed any podcast app
//! can subscribe to. Pure string-building — the web layer ([`crate::web`])
//! gathers the items from the live library and serves the result; everything
//! here is deterministic and unit-tested.
//!
//! Enclosure URLs are absolute (`scheme://host/files/...`) because podcast
//! clients fetch media out-of-band, with no notion of the page they came from.

/// One episode in the feed.
pub struct FeedItem {
    pub title: String,
    pub description: String,
    /// Absolute URL to the media file (the `/files/...` mount).
    pub enclosure_url: String,
    pub enclosure_len: u64,
    pub enclosure_type: String,
    /// Stable GUID — the yt-dlp video id.
    pub guid: String,
    /// RFC-2822 publication date.
    pub pub_date: String,
    /// `HH:MM:SS`, or empty when unknown.
    pub duration: String,
    /// Absolute thumbnail URL, or empty.
    pub image_url: String,
}

/// Feed-level metadata wrapping the items.
pub struct Feed {
    pub title: String,
    pub description: String,
    /// Absolute link back to the web UI.
    pub link: String,
    pub items: Vec<FeedItem>,
}

/// MIME type for a media file, by extension. Falls back to a generic
/// `application/octet-stream` so an unknown container still produces a valid
/// enclosure.
pub fn mime_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("ts") => "video/mp2t",
        Some("m4a") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        Some("opus") | Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Escape the five XML special characters for safe inclusion in element text
/// or attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Strip control chars that are illegal in XML 1.0 (e.g. stray
            // bytes from a mangled title), keeping tab/newline/return.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Format a duration in seconds as `H:MM:SS` (or `M:SS`). Empty for `None`.
pub fn fmt_duration(secs: Option<f64>) -> String {
    let Some(s) = secs else { return String::new() };
    if !s.is_finite() || s < 0.0 { return String::new(); }
    let total = s as u64;
    let (h, m, sec) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// Convert yt-dlp's `YYYYMMDD` upload date (or a UNIX mtime fallback) into an
/// RFC-2822 date string for `<pubDate>`. Defaults to the Unix epoch when both
/// are missing so every item still has a valid, stable date.
pub fn pub_date(upload_date: Option<&str>, mtime_unix: Option<u64>) -> String {
    if let Some(d) = upload_date {
        if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
            // Treat as midnight UTC on that calendar day.
            if let Ok(days) = ymd_to_unix_days(&d[0..4], &d[4..6], &d[6..8]) {
                return rfc2822(days * 86_400);
            }
        }
    }
    rfc2822(mtime_unix.unwrap_or(0) as i64)
}

/// Days since the Unix epoch for a Y/M/D, via a standard civil-date algorithm
/// (Howard Hinnant's `days_from_civil`). Avoids a chrono dependency.
fn ymd_to_unix_days(y: &str, m: &str, d: &str) -> Result<i64, ()> {
    let y: i64 = y.parse().map_err(|_| ())?;
    let m: i64 = m.parse().map_err(|_| ())?;
    let d: i64 = d.parse().map_err(|_| ())?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return Err(()); }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
}

/// Format a UNIX timestamp as an RFC-2822 date in GMT (e.g.
/// `Sun, 07 Jun 2026 00:00:00 GMT`). Self-contained, no chrono.
fn rfc2822(ts: i64) -> String {
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]; // epoch = Thursday
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                             "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let wd = WD[(days.rem_euclid(7)) as usize];
    let (y, m, d) = civil_from_days(days);
    format!("{wd}, {d:02} {mon} {y:04} {h:02}:{mi:02}:{s:02} GMT", mon = MON[(m - 1) as usize])
}

/// Inverse of [`ymd_to_unix_days`] — civil date from days-since-epoch.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Render the feed as an RSS 2.0 document with the iTunes podcast namespace.
pub fn render(feed: &Feed) -> String {
    let mut s = String::with_capacity(1024 + feed.items.len() * 512);
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    s.push('\n');
    s.push_str(r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">"#);
    s.push_str("\n<channel>\n");
    s.push_str(&format!("<title>{}</title>\n", xml_escape(&feed.title)));
    s.push_str(&format!("<link>{}</link>\n", xml_escape(&feed.link)));
    s.push_str(&format!("<description>{}</description>\n", xml_escape(&feed.description)));
    s.push_str("<language>en</language>\n");
    s.push_str("<itunes:explicit>no</itunes:explicit>\n");
    for it in &feed.items {
        s.push_str("<item>\n");
        s.push_str(&format!("<title>{}</title>\n", xml_escape(&it.title)));
        s.push_str(&format!("<description>{}</description>\n", xml_escape(&it.description)));
        s.push_str(&format!(
            "<enclosure url=\"{}\" length=\"{}\" type=\"{}\"/>\n",
            xml_escape(&it.enclosure_url), it.enclosure_len, xml_escape(&it.enclosure_type),
        ));
        s.push_str(&format!("<guid isPermaLink=\"false\">{}</guid>\n", xml_escape(&it.guid)));
        s.push_str(&format!("<pubDate>{}</pubDate>\n", xml_escape(&it.pub_date)));
        if !it.duration.is_empty() {
            s.push_str(&format!("<itunes:duration>{}</itunes:duration>\n", xml_escape(&it.duration)));
        }
        if !it.image_url.is_empty() {
            s.push_str(&format!("<itunes:image href=\"{}\"/>\n", xml_escape(&it.image_url)));
        }
        s.push_str("</item>\n");
    }
    s.push_str("</channel>\n</rss>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn mime_by_extension() {
        assert_eq!(mime_for(Path::new("a.mp4")), "video/mp4");
        assert_eq!(mime_for(Path::new("a.MKV")), "video/x-matroska");
        assert_eq!(mime_for(Path::new("a.opus")), "audio/ogg");
        assert_eq!(mime_for(Path::new("a.weird")), "application/octet-stream");
    }

    #[test]
    fn escapes_xml() {
        assert_eq!(xml_escape("a & b <c> \"d\" 'e'"), "a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;");
        assert_eq!(xml_escape("tab\tkeep\u{0}drop"), "tab\tkeepdrop");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(fmt_duration(Some(65.0)), "1:05");
        assert_eq!(fmt_duration(Some(3661.0)), "1:01:01");
        assert_eq!(fmt_duration(None), "");
    }

    #[test]
    fn upload_date_to_rfc2822() {
        // 2024-01-02 → a Tuesday.
        assert_eq!(pub_date(Some("20240102"), None), "Tue, 02 Jan 2024 00:00:00 GMT");
        // The Unix epoch is a Thursday.
        assert_eq!(pub_date(None, Some(0)), "Thu, 01 Jan 1970 00:00:00 GMT");
        // Garbage upload date falls back to the mtime.
        assert_eq!(pub_date(Some("notadate"), Some(0)), "Thu, 01 Jan 1970 00:00:00 GMT");
    }

    #[test]
    fn renders_valid_rss() {
        let feed = Feed {
            title: "catacomb — Demo & co".into(),
            description: "archive".into(),
            link: "http://host/".into(),
            items: vec![FeedItem {
                title: "Episode <1>".into(),
                description: "desc".into(),
                enclosure_url: "http://host/files/channels/Demo/a.mkv".into(),
                enclosure_len: 12345,
                enclosure_type: "video/x-matroska".into(),
                guid: "vidABC".into(),
                pub_date: "Tue, 02 Jan 2024 00:00:00 GMT".into(),
                duration: "1:05".into(),
                image_url: "http://host/files/channels/Demo/a.webp".into(),
            }],
        };
        let xml = render(&feed);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<title>catacomb — Demo &amp; co</title>"));
        assert!(xml.contains("<title>Episode &lt;1&gt;</title>"));
        assert!(xml.contains(r#"<enclosure url="http://host/files/channels/Demo/a.mkv" length="12345" type="video/x-matroska"/>"#));
        assert!(xml.contains("<itunes:duration>1:05</itunes:duration>"));
        assert!(xml.contains(r#"<guid isPermaLink="false">vidABC</guid>"#));
        assert!(xml.trim_end().ends_with("</rss>"));
    }
}
