//! Catacomb Android JNI core (Stage-1 prototype, Phase 4).
//!
//! This crate proves the "shared Rust core" leg of the Android feasibility
//! spike: the *same* pure modules the desktop/web binary uses
//! (`vtt`, `error_class`, `platform`) cross-compile to an Android `.so` and are
//! callable from Kotlin over JNI — no logic fork, no rewrite.
//!
//! ## What's exposed
//!
//! Every entry point is String-in / String-out (JNI's easiest, most portable
//! shape) and mirrors a desktop function:
//!
//! | Kotlin (`RustCore`)      | Rust module          | Purpose                                   |
//! |--------------------------|----------------------|-------------------------------------------|
//! | `vttParse(String)`       | [`vtt::parse`]       | VTT/SRT → JSON `[{start,text}, …]`         |
//! | `classifyError(String)`  | [`error_class`]      | yt-dlp log → `{class,label,hint}` JSON     |
//! | `platformFromUrl(String)`| [`platform`]         | URL → `{dir_name,display_name,icon}` JSON  |
//! | `platformDirName(String)`| [`platform`]         | URL → backup-folder name (plain string)    |
//!
//! ## Panic safety
//!
//! A Rust panic unwinding across the FFI boundary into the JVM is undefined
//! behaviour. The crate is built with `panic = "abort"`, and every entry point
//! additionally wraps its body in [`std::panic::catch_unwind`] so a bug
//! degrades to an empty/typed result instead of tearing down the process.

// ── Shared, unmodified Catacomb modules ─────────────────────────────────────
// Included by path (not copied) so this stays in lockstep with the desktop
// source. These are pure std + serde and carry no platform-specific code.
#[path = "../../../../src/vtt.rs"]
pub mod vtt;
#[path = "../../../../src/error_class.rs"]
pub mod error_class;
#[path = "../../../../src/platform.rs"]
pub mod platform;

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use serde::Serialize;

/// Serializable view of a parsed subtitle cue (JNI can't hand back the native
/// [`vtt::Cue`] directly, so we JSON-encode a flat list of these).
#[derive(Serialize)]
struct CueJson {
    start: f64,
    text: String,
}

/// Serializable classification result: the kebab-case class id plus its
/// human-facing label and hint.
#[derive(Serialize)]
struct ClassificationJson {
    class: String,
    label: String,
    hint: String,
}

/// Serializable platform descriptor for a URL.
#[derive(Serialize)]
struct PlatformJson {
    dir_name: String,
    display_name: String,
    icon: String,
}

/// Read a `JString` argument into an owned Rust `String`, tolerating a null /
/// undecodable value by returning an empty string (never panics).
fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s)
        .map(|js| js.into())
        .unwrap_or_default()
}

/// Build a Java `String` from a Rust `&str`. On the (near-impossible) failure
/// to allocate a Java string, returns a null jstring so the JVM sees `null`
/// rather than a dangling pointer.
fn to_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    match env.new_string(s) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Run `f` (which produces a String) under `catch_unwind`, returning a Java
/// string of the result or `fallback` if the closure panicked.
fn guarded_string<F: FnOnce() -> String + std::panic::UnwindSafe>(
    env: &mut JNIEnv,
    fallback: &str,
    f: F,
) -> jstring {
    match std::panic::catch_unwind(f) {
        Ok(out) => to_jstring(env, &out),
        Err(_) => to_jstring(env, fallback),
    }
}

/// `RustCore.vttParse(vtt: String): String` — parse WebVTT/SRT text into a JSON
/// array of `{start, text}` cues (empty array on any error).
#[no_mangle]
pub extern "system" fn Java_com_catacomb_spike_RustCore_vttParse<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    input: JString<'local>,
) -> jstring {
    let text = jstring_to_string(&mut env, &input);
    guarded_string(&mut env, "[]", move || {
        let cues: Vec<CueJson> = vtt::parse(&text)
            .into_iter()
            .map(|c| CueJson { start: c.start, text: c.text })
            .collect();
        serde_json::to_string(&cues).unwrap_or_else(|_| "[]".to_string())
    })
}

/// `RustCore.classifyError(log: String): String` — classify a yt-dlp failure
/// log (one entry per line) into `{class, label, hint}` JSON.
#[no_mangle]
pub extern "system" fn Java_com_catacomb_spike_RustCore_classifyError<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    input: JString<'local>,
) -> jstring {
    let log = jstring_to_string(&mut env, &input);
    guarded_string(&mut env, "{}", move || {
        let class = error_class::classify(log.lines());
        // The enum serializes to its kebab-case id (e.g. "rate-limited").
        let class_id = serde_json::to_value(class)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "other".to_string());
        let out = ClassificationJson {
            class: class_id,
            label: class.label().to_string(),
            hint: class.hint().to_string(),
        };
        serde_json::to_string(&out).unwrap_or_else(|_| "{}".to_string())
    })
}

/// `RustCore.platformFromUrl(url: String): String` — resolve a URL to its
/// platform descriptor `{dir_name, display_name, icon}` JSON.
#[no_mangle]
pub extern "system" fn Java_com_catacomb_spike_RustCore_platformFromUrl<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    input: JString<'local>,
) -> jstring {
    let url = jstring_to_string(&mut env, &input);
    guarded_string(&mut env, "{}", move || {
        let p = platform::Platform::from_url(&url);
        let out = PlatformJson {
            dir_name: p.dir_name().to_string(),
            display_name: p.display_name().to_string(),
            icon: p.icon().to_string(),
        };
        serde_json::to_string(&out).unwrap_or_else(|_| "{}".to_string())
    })
}

/// `RustCore.platformDirName(url: String): String` — the backup-folder name for
/// a URL's platform (e.g. `"channels"` for YouTube). Plain string, no JSON.
#[no_mangle]
pub extern "system" fn Java_com_catacomb_spike_RustCore_platformDirName<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    input: JString<'local>,
) -> jstring {
    let url = jstring_to_string(&mut env, &input);
    guarded_string(&mut env, "other", move || {
        platform::Platform::from_url(&url).dir_name().to_string()
    })
}

// ── Host-side tests ─────────────────────────────────────────────────────────
// These exercise the wrapped logic through this crate (not JNI, which needs a
// JVM) so a plain `cargo test` on the dev host proves the shared modules are
// reachable and produce the JSON shapes the Kotlin layer expects.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtt_parse_produces_cue_json() {
        let sample = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello world\n";
        let cues: Vec<CueJson> = vtt::parse(sample)
            .into_iter()
            .map(|c| CueJson { start: c.start, text: c.text })
            .collect();
        let json = serde_json::to_string(&cues).unwrap();
        assert!(json.contains("\"start\":1.0"), "json: {json}");
        assert!(json.contains("Hello world"), "json: {json}");
    }

    #[test]
    fn classify_rate_limit_serializes_kebab_case() {
        let class = error_class::classify(
            ["ERROR: HTTP Error 429: Too Many Requests"].into_iter(),
        );
        let id = serde_json::to_value(class).unwrap();
        assert_eq!(id.as_str(), Some("rate-limited"));
        assert_eq!(class.label(), "rate-limited");
        assert!(!class.hint().is_empty());
    }

    #[test]
    fn platform_from_url_youtube_dir_is_channels() {
        let p = platform::Platform::from_url("https://youtu.be/dQw4w9WgXcQ");
        assert_eq!(p.dir_name(), "channels");
        assert_eq!(p.display_name(), "YouTube");
    }

    #[test]
    fn platform_from_url_unknown_is_other() {
        let p = platform::Platform::from_url("https://example.com/x");
        assert_eq!(p.dir_name(), "other");
    }
}
