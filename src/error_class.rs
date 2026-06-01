//! Classification of yt-dlp / network failures into actionable buckets.
//!
//! When a job fails, the user sees the last line of stderr — usually a
//! cryptic yt-dlp error like `ERROR: [youtube] dQw4w9WgXcQ: Sign in to
//! confirm you're not a bot. ...`. That doesn't tell a non-expert what to
//! actually *do*. This module pattern-matches the well-known fingerprints
//! and returns both:
//!
//! - an [`ErrorClass`] for the UI to render as a colored badge,
//! - and a short human-readable `hint` string with the suggested fix.
//!
//! The classifier is intentionally conservative: when no pattern matches,
//! it returns [`ErrorClass::Other`] and lets the existing raw log do the
//! talking. False positives would be worse than no classification.

use serde::Serialize;

/// One of a handful of well-known yt-dlp failure modes, or `Other` when
/// the log doesn't match any pattern. The string serialisation is what
/// the JSON API + JS UI consumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorClass {
    /// HTTP 429 / "Too Many Requests", or YouTube's "Sign in to confirm
    /// you're not a bot" rate-limit cookie wall. Suggest cookies or wait.
    RateLimited,
    /// Video is members-only / private / behind a paywall. Needs cookies
    /// from a logged-in browser session with access.
    MembersOnly,
    /// Geo-blocked in the user's region. Needs a proxy or VPN.
    Geoblocked,
    /// Video unavailable / removed / deleted / copyright-strike.
    NotFound,
    /// Required codec (ffmpeg / decoder) missing on the system.
    CodecMissing,
    /// Out of disk space mid-download.
    DiskFull,
    /// Network-level failure (timeout, DNS, connection refused) not
    /// otherwise classified.
    NetworkError,
    /// Cookies file exists but yt-dlp rejected it (expired session etc.).
    BadCookies,
    /// Catchall. The existing log is the user's only hint.
    Other,
}

impl ErrorClass {
    /// Short label for badges and JSON. Human-facing but kept terse.
    pub fn label(self) -> &'static str {
        match self {
            ErrorClass::RateLimited => "rate-limited",
            ErrorClass::MembersOnly => "members-only",
            ErrorClass::Geoblocked => "geo-blocked",
            ErrorClass::NotFound => "not found",
            ErrorClass::CodecMissing => "codec missing",
            ErrorClass::DiskFull => "disk full",
            ErrorClass::NetworkError => "network error",
            ErrorClass::BadCookies => "bad cookies",
            ErrorClass::Other => "error",
        }
    }

    /// One-sentence suggested action. Reads as "do X to fix this."
    pub fn hint(self) -> &'static str {
        match self {
            ErrorClass::RateLimited =>
                "YouTube is rate-limiting you or demanding a captcha (bot detection). Refresh your cookies.txt from a logged-in browser session (Settings → Cookies), enable the POT token provider, and/or wait 10–60 minutes before retrying.",
            ErrorClass::MembersOnly =>
                "This video requires a logged-in session with access (members-only, private, or paid). Update cookies.txt from an account that can view it.",
            ErrorClass::Geoblocked =>
                "This video is not available in your region. Try a different cookies.txt from an unblocked region, or use a VPN.",
            ErrorClass::NotFound =>
                "The video appears to have been removed by the uploader or platform. Nothing to download.",
            ErrorClass::CodecMissing =>
                "A required codec or tool is missing. Make sure ffmpeg is installed and on your PATH.",
            ErrorClass::DiskFull =>
                "The destination disk is full. Free up space and retry — yt-dlp resumes from where it stopped.",
            ErrorClass::NetworkError =>
                "Network issue reaching the platform. Check connectivity / firewall and retry.",
            ErrorClass::BadCookies =>
                "yt-dlp rejected the cookies file. The session probably expired — export a fresh cookies.txt from your browser.",
            ErrorClass::Other => "",
        }
    }
}

/// Classify a failed yt-dlp job by scanning its log buffer.
///
/// Walks the most recent lines first because yt-dlp's terminal error is
/// usually the final or near-final line; earlier output may contain
/// unrelated noise. Returns [`ErrorClass::Other`] when no fingerprint
/// matches.
pub fn classify<'a, I>(log_lines: I) -> ErrorClass
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: DoubleEndedIterator,
{
    // Collect into a Vec because we want to walk from the end. Logs are
    // capped at ~800 lines elsewhere so this is fine.
    let lines: Vec<&str> = log_lines.into_iter().collect();
    for line in lines.iter().rev() {
        let l = line.to_ascii_lowercase();

        // ── Rate limits / bot challenges ─────────────────────────────
        // Checked first so a bot-challenge phrasing that also contains
        // "video unavailable" (e.g. YouTube's captcha wall) is classified
        // as rate-limited rather than NotFound below.
        if l.contains("http error 429")
            || l.contains("too many requests")
            || l.contains("sign in to confirm you")
            || l.contains("sign in to confirm your age")
            || l.contains("ratelimit")
            || l.contains("rate limit")
            || l.contains("captcha")
            || l.contains("requiring a captcha challenge")
            || l.contains("not a bot")
        {
            return ErrorClass::RateLimited;
        }

        // ── Access-controlled content ─────────────────────────────────
        if l.contains("members-only")
            || l.contains("join this channel")
            || l.contains("private video")
            || l.contains("requires payment")
            || l.contains("requires purchase")
        {
            return ErrorClass::MembersOnly;
        }

        // ── Geo blocking ──────────────────────────────────────────────
        // Many fingerprints share the suffix "in your country"; that's the
        // strongest signal so we check it directly. "in your region" covers
        // the corporate-region variant some platforms use.
        if l.contains("in your country")
            || l.contains("in your region")
            || l.contains("geo restricted")
            || l.contains("video unavailable in your")
        {
            return ErrorClass::Geoblocked;
        }

        // ── Removed / unavailable ─────────────────────────────────────
        // Distinguish from geo above: the geo check matched "in your
        // country" already. A bare "video unavailable" with no region
        // qualifier means the upload is gone.
        if l.contains("this video has been removed")
            || l.contains("video has been deleted")
            || l.contains("account has been terminated")
            || (l.contains("video unavailable") && !l.contains("in your"))
            || (l.contains("http error 404") && !l.contains("playlist"))
        {
            return ErrorClass::NotFound;
        }

        // ── Local toolchain ───────────────────────────────────────────
        if l.contains("ffmpeg not found")
            || l.contains("ffprobe not found")
            || l.contains("you have requested merging of multiple formats")
            || l.contains("postprocessing: ffmpeg")
        {
            return ErrorClass::CodecMissing;
        }

        // ── Disk full ────────────────────────────────────────────────
        if l.contains("no space left on device")
            || l.contains("disk full")
            || l.contains("write error")
                && (l.contains("space") || l.contains("enospc"))
        {
            return ErrorClass::DiskFull;
        }

        // ── Cookies rejected ─────────────────────────────────────────
        if l.contains("invalid cookies")
            || l.contains("cookies file")
                && (l.contains("expired") || l.contains("invalid") || l.contains("malformed"))
        {
            return ErrorClass::BadCookies;
        }

        // ── Network ──────────────────────────────────────────────────
        if l.contains("name or service not known")
            || l.contains("temporary failure in name resolution")
            || l.contains("connection refused")
            || l.contains("connection reset")
            || l.contains("connection timed out")
            || l.contains("network is unreachable")
            || l.contains("ssl: certificate")
            || l.contains("read timed out")
        {
            return ErrorClass::NetworkError;
        }
    }
    ErrorClass::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_str(s: &str) -> ErrorClass {
        classify(s.lines())
    }

    #[test]
    fn detects_rate_limit_429() {
        assert_eq!(
            classify_str("ERROR: HTTP Error 429: Too Many Requests"),
            ErrorClass::RateLimited
        );
    }

    #[test]
    fn detects_sign_in_to_confirm() {
        assert_eq!(
            classify_str("ERROR: [youtube] dQw4w9WgXcQ: Sign in to confirm you're not a bot. Use --cookies-from-browser..."),
            ErrorClass::RateLimited
        );
    }

    #[test]
    fn detects_members_only() {
        assert_eq!(
            classify_str("ERROR: [youtube] abc: Join this channel to get access to members-only content."),
            ErrorClass::MembersOnly
        );
    }

    #[test]
    fn detects_video_removed() {
        assert_eq!(
            classify_str("ERROR: [youtube] xyz: Video unavailable. This video has been removed by the uploader."),
            ErrorClass::NotFound
        );
    }

    #[test]
    fn captcha_wall_is_rate_limited_not_not_found() {
        // The captcha line also contains "Video unavailable", which the
        // NotFound rule would otherwise grab. The RateLimited rule must
        // win because the fix is "refresh cookies / wait", not "the
        // upload is gone".
        assert_eq!(
            classify_str("ERROR: [youtube] tfoprUBw0H0: Video unavailable. YouTube is requiring a captcha challenge before playback"),
            ErrorClass::RateLimited
        );
    }

    #[test]
    fn detects_geo_block() {
        assert_eq!(
            classify_str("ERROR: The uploader has not made this video available in your country."),
            ErrorClass::Geoblocked
        );
    }

    #[test]
    fn detects_disk_full() {
        assert_eq!(
            classify_str("ERROR: unable to write data: [Errno 28] No space left on device"),
            ErrorClass::DiskFull
        );
    }

    #[test]
    fn detects_network() {
        assert_eq!(
            classify_str("ERROR: Unable to download webpage: <urlopen error [Errno -3] Temporary failure in name resolution>"),
            ErrorClass::NetworkError
        );
    }

    #[test]
    fn detects_ffmpeg_missing() {
        assert_eq!(
            classify_str("ERROR: ffmpeg not found. The downloaded file cannot be merged."),
            ErrorClass::CodecMissing
        );
    }

    #[test]
    fn returns_other_when_no_match() {
        assert_eq!(
            classify_str("ERROR: something weird happened that we have no fingerprint for"),
            ErrorClass::Other
        );
    }

    #[test]
    fn walks_from_end_first() {
        // Earlier noise about ffmpeg shouldn't override the actual terminal
        // failure on the last line.
        let log = "[info] some ffmpeg postprocessing thing\n\
                   [download] 50% done\n\
                   ERROR: HTTP Error 429: Too Many Requests";
        assert_eq!(classify_str(log), ErrorClass::RateLimited);
    }
}
