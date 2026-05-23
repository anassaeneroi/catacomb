//! Aggregate statistics over the scanned library + watched/resume state.
//!
//! All numbers are computed on demand from the in-memory [`Channel`] tree
//! and the SQLite-backed watched/positions data — no separate stats table.
//! The cost is one library traversal per request, which is cheap relative to
//! the initial scan.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::library::Channel;

/// Top-N row in the channel breakdown tables.
#[derive(Serialize, Clone)]
pub struct ChannelStat {
    pub name: String,
    pub count: usize,
    pub size_bytes: u64,
    pub duration_secs: f64,
}

/// One column of the per-year upload histogram.
#[derive(Serialize, Clone)]
pub struct YearStat {
    pub year: u32,
    pub count: usize,
}

/// One column of the per-week download-activity histogram.
///
/// `week_start_unix` is the UNIX epoch of the Monday 00:00 UTC that begins
/// the bucket, so clients can format the label in any locale.
#[derive(Serialize, Clone)]
pub struct WeekStat {
    pub week_start_unix: u64,
    pub count: usize,
    pub size_bytes: u64,
}

/// Top-level stats payload returned by `GET /api/stats`.
#[derive(Serialize, Clone)]
pub struct StatsReport {
    pub total_channels: usize,
    pub total_videos: usize,
    pub total_playlists: usize,
    pub total_size_bytes: u64,
    pub total_duration_secs: f64,
    /// Count of videos the user has explicitly marked as watched.
    pub watched_count: usize,
    /// Sum of durations across watched videos. May be 0 if info.json is missing.
    pub watched_duration_secs: f64,
    /// Number of videos with a non-trivial resume position.
    pub continue_watching_count: usize,
    pub top_channels_by_size: Vec<ChannelStat>,
    pub top_channels_by_count: Vec<ChannelStat>,
    pub videos_per_year: Vec<YearStat>,
    pub downloads_per_week: Vec<WeekStat>,
}

/// How many rows to include in each Top-N channel table.
const TOP_N: usize = 10;
/// How many recent weeks to surface in the downloads histogram.
const RECENT_WEEKS: usize = 12;
/// Seconds in a week (7 × 24 × 3600).
const WEEK_SECS: u64 = 604_800;

/// Build a full [`StatsReport`] from the scanned library plus watched/resume
/// state. `now_unix` is supplied for testability; in production this is just
/// `SystemTime::now()`.
pub fn build(
    channels: &[Channel],
    watched: &std::collections::HashSet<String>,
    resume_positions: &std::collections::HashMap<String, f64>,
    now_unix: u64,
) -> StatsReport {
    let total_channels = channels.len();
    let total_playlists: usize = channels.iter().map(|c| c.playlists.len()).sum();

    let mut total_videos = 0usize;
    let mut total_size_bytes = 0u64;
    let mut total_duration_secs = 0f64;
    let mut watched_count = 0usize;
    let mut watched_duration_secs = 0f64;
    let mut by_size: Vec<ChannelStat> = Vec::with_capacity(total_channels);
    let mut by_count: Vec<ChannelStat> = Vec::with_capacity(total_channels);
    let mut years: BTreeMap<u32, usize> = BTreeMap::new();
    // Weekly buckets keyed by week-start unix.
    let mut weeks: BTreeMap<u64, (usize, u64)> = BTreeMap::new();
    let week_start_now = monday_start(now_unix);
    let oldest_week = week_start_now.saturating_sub(WEEK_SECS * (RECENT_WEEKS as u64 - 1));

    for ch in channels {
        let mut ch_count = 0usize;
        let mut ch_size = 0u64;
        let mut ch_duration = 0f64;
        for v in ch.all_videos() {
            total_videos += 1;
            ch_count += 1;
            if let Some(s) = v.file_size {
                total_size_bytes += s;
                ch_size += s;
            }
            if let Some(d) = v.duration_secs {
                total_duration_secs += d;
                ch_duration += d;
            }
            if watched.contains(&v.id) {
                watched_count += 1;
                if let Some(d) = v.duration_secs {
                    watched_duration_secs += d;
                }
            }
            // Year column from upload_date.
            if let Some(d) = v.upload_date.as_deref() {
                if d.len() >= 4 {
                    if let Ok(y) = d[..4].parse::<u32>() {
                        *years.entry(y).or_insert(0) += 1;
                    }
                }
            }
            // Weekly bucket from the video file's mtime.
            if let Some(path) = v.video_path.as_ref() {
                if let Some(mtime) = file_mtime_unix(path) {
                    if mtime >= oldest_week {
                        let bucket = monday_start(mtime);
                        let entry = weeks.entry(bucket).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += v.file_size.unwrap_or(0);
                    }
                }
            }
        }
        by_size.push(ChannelStat {
            name: ch.name.clone(),
            count: ch_count,
            size_bytes: ch_size,
            duration_secs: ch_duration,
        });
        by_count.push(ChannelStat {
            name: ch.name.clone(),
            count: ch_count,
            size_bytes: ch_size,
            duration_secs: ch_duration,
        });
    }
    by_size.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    by_size.truncate(TOP_N);
    by_count.sort_by(|a, b| b.count.cmp(&a.count));
    by_count.truncate(TOP_N);

    let videos_per_year = years
        .into_iter()
        .map(|(year, count)| YearStat { year, count })
        .collect();

    // Fill in zero buckets for weeks with no downloads so the chart has a
    // continuous x-axis.
    let mut downloads_per_week = Vec::with_capacity(RECENT_WEEKS);
    for i in 0..RECENT_WEEKS as u64 {
        let week_start = oldest_week + i * WEEK_SECS;
        let (count, size_bytes) = weeks.get(&week_start).copied().unwrap_or((0, 0));
        downloads_per_week.push(WeekStat {
            week_start_unix: week_start,
            count,
            size_bytes,
        });
    }

    let continue_watching_count = resume_positions.iter()
        .filter(|(_, pos)| **pos > 3.0)
        .count();

    StatsReport {
        total_channels,
        total_videos,
        total_playlists,
        total_size_bytes,
        total_duration_secs,
        watched_count,
        watched_duration_secs,
        continue_watching_count,
        top_channels_by_size: by_size,
        top_channels_by_count: by_count,
        videos_per_year,
        downloads_per_week,
    }
}

/// Read mtime from a path and return it as a UNIX timestamp in seconds.
fn file_mtime_unix(p: &std::path::Path) -> Option<u64> {
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta.modified().ok()?;
    mtime.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Return the UNIX timestamp of the Monday 00:00 UTC that begins the ISO week
/// containing `t_unix`. We approximate via 1970-01-05 (a Monday) as the epoch
/// anchor and snap to the previous 7-day boundary.
fn monday_start(t_unix: u64) -> u64 {
    // 1970-01-05 00:00 UTC is a Monday; its unix timestamp is 4 * 86400.
    const FIRST_MONDAY: u64 = 4 * 86_400;
    if t_unix < FIRST_MONDAY {
        return 0;
    }
    let offset = (t_unix - FIRST_MONDAY) / WEEK_SECS;
    FIRST_MONDAY + offset * WEEK_SECS
}

/// Convenience: today's UTC unix timestamp.
pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monday_start_aligns_to_monday() {
        // 1970-01-05 00:00 UTC = 345600 (Monday).
        assert_eq!(monday_start(345_600), 345_600);
        // 1970-01-12 00:00 UTC = 950400 (next Monday).
        assert_eq!(monday_start(950_400), 950_400);
        // A Wednesday rolls back to the Monday two days earlier.
        assert_eq!(monday_start(345_600 + 2 * 86_400), 345_600);
        // Pre-1970-01-05 returns 0.
        assert_eq!(monday_start(86_400), 0);
    }
}
