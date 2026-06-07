//! Perceptual-hash fingerprinting for content-aware duplicate detection.
//!
//! The library's `maintenance` scan finds duplicates only by yt-dlp video ID
//! — it catches "you downloaded this twice", but not the *same content* under
//! a different ID (a reupload, a cross-platform mirror, a re-encode). This
//! module fingerprints the actual pixels so those can be grouped.
//!
//! # How it stays fast
//!
//! - **Sampled frames, fast seek.** Per video we grab [`FRAMES`] frames at
//!   percentage offsets using ffmpeg *keyframe seek* (`-ss` before `-i`) and
//!   downscale to a 9×8 grayscale grid during decode — so we never full-decode
//!   a video, just touch a few keyframes.
//! - **dHash.** Each frame becomes a 64-bit difference hash (compare each pixel
//!   to its right neighbour). Robust to re-encoding, scaling, and minor quality
//!   changes; sensitive to actually-different content.
//! - **Compute once, cache.** Fingerprints are stored in SQLite keyed by
//!   `(path, mtime)` (see [`crate::database`]), so a video is hashed once and
//!   skipped forever after — new downloads are hashed as they arrive.
//! - **Duration bucketing.** Re-uploads share ~the same runtime, so
//!   [`group_similar`] only compares videos within a duration tolerance,
//!   turning an O(n²) all-pairs compare into a near-linear sliding window.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Frames sampled per video. Six spread-out frames is enough to distinguish
/// content while keeping the per-video cost to a few fast ffmpeg seeks.
pub const FRAMES: usize = 6;

/// dHash grid width/height. A `(W+1)×H` grayscale image yields `W*H` bits;
/// 9×8 → 64 bits, one frame hash per `u64`.
const GW: usize = 9;
const GH: usize = 8;

/// Fraction-of-duration offsets at which frames are sampled. Avoids the very
/// start/end (intros, outros, black frames) where unrelated videos look alike.
const OFFSETS: [f64; FRAMES] = [0.10, 0.25, 0.40, 0.55, 0.70, 0.85];

/// Compute a difference hash from a `GW*GH` grayscale buffer (row-major).
/// Bit *k* is set when a pixel is brighter than its right-hand neighbour.
pub fn dhash(gray: &[u8]) -> u64 {
    debug_assert_eq!(gray.len(), GW * GH);
    let mut hash = 0u64;
    let mut bit = 0u32;
    for y in 0..GH {
        for x in 0..GW - 1 {
            if gray[y * GW + x] > gray[y * GW + x + 1] {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

/// Hamming distance between two frame hashes (number of differing bits).
#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Extract one frame at `at_secs` as a `GW×GH` grayscale buffer via ffmpeg.
/// Returns `None` if ffmpeg is missing, the seek failed, or the output was the
/// wrong size. Uses keyframe seek (`-ss` before `-i`) for speed.
fn extract_frame_gray(path: &Path, at_secs: f64) -> Option<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-loglevel").arg("error")
        .arg("-ss").arg(format!("{at_secs:.3}"))
        .arg("-i").arg(path)
        .arg("-frames:v").arg("1")
        .arg("-an").arg("-sn")
        .arg("-vf").arg(format!("scale={GW}:{GH}:flags=area,format=gray"))
        .arg("-f").arg("rawvideo")
        .arg("-")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.status.success() && out.stdout.len() >= GW * GH {
        Some(out.stdout[..GW * GH].to_vec())
    } else {
        None
    }
}

/// Fingerprint a single video: sample [`FRAMES`] frames and hash each. Returns
/// the per-frame hashes (possibly fewer than `FRAMES` if some seeks failed, or
/// empty when the duration is unusable / ffmpeg can't read the file).
pub fn fingerprint(path: &Path, duration_secs: f64) -> Vec<u64> {
    if !(duration_secs.is_finite() && duration_secs > 1.0) {
        return Vec::new();
    }
    OFFSETS
        .iter()
        .filter_map(|f| extract_frame_gray(path, duration_secs * f).map(|g| dhash(&g)))
        .collect()
}

/// One video to fingerprint.
#[derive(Clone, Debug)]
pub struct FpInput {
    pub path: PathBuf,
    pub mtime_unix: i64,
    pub video_id: String,
    pub duration_secs: f64,
}

/// Result of fingerprinting one [`FpInput`].
#[derive(Clone, Debug)]
pub struct FpComputed {
    pub input: FpInput,
    pub hashes: Vec<u64>,
}

/// Fingerprint many videos in parallel, bumping `progress` after each one so a
/// UI can show "N / total". Mirrors the library scanner's hand-rolled worker
/// pool (no rayon dependency).
pub fn compute_batch(
    inputs: Vec<FpInput>,
    n_workers: usize,
    progress: &std::sync::atomic::AtomicUsize,
) -> Vec<FpComputed> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    let len = inputs.len();
    if len == 0 {
        return Vec::new();
    }
    let workers = n_workers.max(1).min(len);
    let inputs: Arc<Vec<Mutex<Option<FpInput>>>> =
        Arc::new(inputs.into_iter().map(|v| Mutex::new(Some(v))).collect());
    let results: Arc<Vec<Mutex<Option<FpComputed>>>> =
        Arc::new((0..len).map(|_| Mutex::new(None)).collect());
    let next = Arc::new(AtomicUsize::new(0));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let inputs = inputs.clone();
            let results = results.clone();
            let next = next.clone();
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= len {
                    break;
                }
                let input = inputs[i].lock().unwrap().take().unwrap();
                let hashes = fingerprint(&input.path, input.duration_secs);
                *results[i].lock().unwrap() = Some(FpComputed { input, hashes });
                progress.fetch_add(1, Ordering::Relaxed);
            });
        }
    });

    Arc::try_unwrap(results)
        .unwrap_or_else(|_| unreachable!("scope joined all workers"))
        .into_iter()
        .map(|m| m.into_inner().unwrap().unwrap())
        .collect()
}

/// A stored fingerprint, ready for grouping.
#[derive(Clone, Debug)]
pub struct FpRecord {
    pub video_id: String,
    pub duration_secs: f64,
    pub hashes: Vec<u64>,
}

/// Two videos are "similar" when at least `min_match` of their frame hashes
/// each find a partner in the other within `max_ham` bits.
fn similar(a: &FpRecord, b: &FpRecord, max_ham: u32, min_match: usize) -> bool {
    if a.hashes.is_empty() || b.hashes.is_empty() {
        return false;
    }
    let need = min_match.min(a.hashes.len()).min(b.hashes.len());
    let mut matches = 0usize;
    for &ha in &a.hashes {
        if b.hashes.iter().any(|&hb| hamming(ha, hb) <= max_ham) {
            matches += 1;
            if matches >= need {
                return true;
            }
        }
    }
    false
}

/// Disjoint-set (union-find) for clustering similar videos transitively.
struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// Group videos that share visual content. Only videos whose durations are
/// within `dur_tol` seconds are ever compared (sorted + sliding window), so
/// this is near-linear unless a library is full of identical-length clips.
/// Returns groups (indices into `records`) of size ≥ 2, largest first.
pub fn group_similar(
    records: &[FpRecord],
    dur_tol: f64,
    max_ham: u32,
    min_match: usize,
) -> Vec<Vec<usize>> {
    let n = records.len();
    if n < 2 {
        return Vec::new();
    }
    // Index order sorted by duration so we can stop comparing once the next
    // candidate is outside the tolerance window.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        records[a]
            .duration_secs
            .partial_cmp(&records[b].duration_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut uf = UnionFind::new(n);
    for oi in 0..order.len() {
        let i = order[oi];
        for &j in &order[oi + 1..] {
            if records[j].duration_secs - records[i].duration_secs > dur_tol {
                break; // sorted: no later candidate can be in range either
            }
            if similar(&records[i], &records[j], max_ham, min_match) {
                uf.union(i, j);
            }
        }
    }

    // Collect clusters of size ≥ 2.
    let mut by_root: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for idx in 0..n {
        let r = uf.find(idx);
        by_root.entry(r).or_default().push(idx);
    }
    let mut groups: Vec<Vec<usize>> = by_root.into_values().filter(|g| g.len() >= 2).collect();
    groups.sort_by(|a, b| b.len().cmp(&a.len()));
    groups
}

/// Default tolerances for [`group_similar`]: durations within 3 s, frame
/// hashes within 8/64 bits, and at least 3 of 6 frames matching.
pub const DEFAULT_DUR_TOL: f64 = 3.0;
pub const DEFAULT_MAX_HAM: u32 = 8;
pub const DEFAULT_MIN_MATCH: usize = 3;

/// The whole dedup pipeline, shared by both front-ends: mtime-gate `inputs`
/// against what's stored, fingerprint the new/changed ones in parallel
/// (bumping `progress`, after first storing the count in `total`), upsert,
/// prune anything not in `valid_paths`, then group by visual similarity.
/// Returns groups (≥2) of **file paths**; each UI maps those to its own
/// display rows. Errors are DB errors stringified.
pub fn rebuild_and_group(
    db: &crate::database::Database,
    inputs: Vec<FpInput>,
    valid_paths: &std::collections::HashSet<String>,
    workers: usize,
    progress: &std::sync::atomic::AtomicUsize,
    total: &std::sync::atomic::AtomicUsize,
) -> Result<Vec<Vec<String>>, String> {
    use std::sync::atomic::Ordering;
    let known = db.fingerprint_mtimes().map_err(|e| e.to_string())?;
    let todo: Vec<FpInput> = inputs
        .into_iter()
        .filter(|i| known.get(&i.path.display().to_string()) != Some(&i.mtime_unix))
        .collect();
    total.store(todo.len(), Ordering::Relaxed);

    let computed = compute_batch(todo, workers, progress);
    for c in &computed {
        let _ = db.upsert_fingerprint(
            &c.input.path.display().to_string(), c.input.mtime_unix,
            &c.input.video_id, c.input.duration_secs, &c.hashes,
        );
    }
    let _ = db.prune_fingerprints(valid_paths);

    let stored = db.load_fingerprints().map_err(|e| e.to_string())?;
    let records: Vec<FpRecord> = stored.iter().map(|s| FpRecord {
        video_id: s.video_id.clone(), duration_secs: s.duration_secs, hashes: s.hashes.clone(),
    }).collect();
    let groups_idx =
        group_similar(&records, DEFAULT_DUR_TOL, DEFAULT_MAX_HAM, DEFAULT_MIN_MATCH);
    Ok(groups_idx
        .into_iter()
        .map(|g| g.into_iter().map(|i| stored[i].path.clone()).collect())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dhash_gradients() {
        // Strictly increasing rows → every pixel < its right neighbour → no
        // bits set. Strictly decreasing → all 64 bits set.
        let mut inc = [0u8; GW * GH];
        let mut dec = [0u8; GW * GH];
        for y in 0..GH {
            for x in 0..GW {
                inc[y * GW + x] = (x * 20) as u8;
                dec[y * GW + x] = ((GW - x) * 20) as u8;
            }
        }
        assert_eq!(dhash(&inc), 0);
        assert_eq!(dhash(&dec), u64::MAX >> (64 - (GW - 1) * GH));
    }

    #[test]
    fn hamming_basic() {
        assert_eq!(hamming(0b1010, 0b1010), 0);
        assert_eq!(hamming(0b1010, 0b0000), 2);
    }

    fn rec(id: &str, dur: f64, hashes: &[u64]) -> FpRecord {
        FpRecord { video_id: id.into(), duration_secs: dur, hashes: hashes.to_vec() }
    }

    #[test]
    fn groups_identical_content() {
        let h = [1u64, 2, 3, 4, 5, 6];
        let recs = vec![
            rec("a", 100.0, &h),
            rec("b", 101.0, &h),               // same hashes, near duration
            rec("c", 100.5, &[!1, !2, !3, !4]),// very different hashes
        ];
        let groups = group_similar(&recs, DEFAULT_DUR_TOL, DEFAULT_MAX_HAM, DEFAULT_MIN_MATCH);
        assert_eq!(groups.len(), 1);
        let g: Vec<&str> = groups[0].iter().map(|&i| recs[i].video_id.as_str()).collect();
        assert!(g.contains(&"a") && g.contains(&"b") && !g.contains(&"c"));
    }

    #[test]
    fn duration_tolerance_excludes_far_matches() {
        let h = [10u64, 20, 30, 40, 50, 60];
        // Identical hashes but durations 100s apart → must not group.
        let recs = vec![rec("a", 60.0, &h), rec("b", 600.0, &h)];
        assert!(group_similar(&recs, DEFAULT_DUR_TOL, DEFAULT_MAX_HAM, DEFAULT_MIN_MATCH).is_empty());
    }

    /// End-to-end check against real ffmpeg: a video and a re-encoded,
    /// downscaled, quality-degraded copy must group together, while an
    /// unrelated video must not. Opt-in (needs ffmpeg) — run with
    /// `cargo test --release real_ffmpeg -- --ignored --nocapture`.
    #[test]
    #[ignore = "requires ffmpeg; generates test videos"]
    fn real_ffmpeg_groups_reencodes() {
        let dir = std::env::temp_dir().join(format!("ytoff-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gen = |args: &[&str]| {
            let ok = Command::new("ffmpeg").arg("-nostdin").arg("-y").arg("-loglevel").arg("error")
                .args(args).status().map(|s| s.success()).unwrap_or(false);
            assert!(ok, "ffmpeg gen failed for {args:?}");
        };
        let orig = dir.join("orig.mp4");
        let reenc = dir.join("reenc.mp4");
        let diff = dir.join("diff.mp4");
        gen(&["-f","lavfi","-i","testsrc=duration=20:size=640x480:rate=10", orig.to_str().unwrap()]);
        // Re-encode: downscale + heavy compression + different container framerate.
        gen(&["-i", orig.to_str().unwrap(), "-vf","scale=320:240","-r","15","-c:v","libx264","-crf","38", reenc.to_str().unwrap()]);
        gen(&["-f","lavfi","-i","testsrc2=duration=20:size=640x480:rate=10", diff.to_str().unwrap()]);

        let t0 = std::time::Instant::now();
        let recs = vec![
            FpRecord { video_id: "orig".into(),  duration_secs: 20.0, hashes: fingerprint(&orig, 20.0) },
            FpRecord { video_id: "reenc".into(), duration_secs: 20.0, hashes: fingerprint(&reenc, 20.0) },
            FpRecord { video_id: "diff".into(),  duration_secs: 20.0, hashes: fingerprint(&diff, 20.0) },
        ];
        eprintln!("fingerprinted 3 videos in {:?} ({:?}/video)", t0.elapsed(), t0.elapsed() / 3);
        for r in &recs { assert_eq!(r.hashes.len(), FRAMES, "{} got {} frames", r.video_id, r.hashes.len()); }

        let groups = group_similar(&recs, DEFAULT_DUR_TOL, DEFAULT_MAX_HAM, DEFAULT_MIN_MATCH);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(groups.len(), 1, "exactly one duplicate group expected, got {groups:?}");
        let ids: Vec<&str> = groups[0].iter().map(|&i| recs[i].video_id.as_str()).collect();
        assert!(ids.contains(&"orig") && ids.contains(&"reenc"), "orig+reenc should group: {ids:?}");
        assert!(!ids.contains(&"diff"), "unrelated video must not be in the group: {ids:?}");
    }

    #[test]
    fn transitive_clustering() {
        // a~b and b~c (each pair shares ≥3 frames) → one group {a,b,c}.
        let a = rec("a", 100.0, &[1, 2, 3, 4, 5, 6]);
        let b = rec("b", 100.0, &[1, 2, 3, 99, 98, 97]);
        let c = rec("c", 100.0, &[99, 98, 97, 4, 5, 6]);
        let groups = group_similar(&[a, b, c], DEFAULT_DUR_TOL, 0, 3);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }
}
