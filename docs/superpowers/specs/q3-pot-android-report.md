# Q3 Research Report: POT / Proof-of-Origin Tokens on Android

**Research date:** 2026-06-27
**Question:** Is YouTube's Proof-of-Origin (POT) token requirement also enforced on the mobile/TV player client surfaces that yt-dlp can use — or can an on-device Android Catacomb avoid the entire POT machinery?

---

## 1. Which YouTube Player Clients Require PO Tokens?

### The Definitive Client Table (from yt-dlp PO Token Guide, June 2026)

| Client | GVS PO Token Required | Player PO Token Required | Notes |
|---|---|---|---|
| `web` | YES (also Subs) | NO | SABR-only formats; needs JS runtime |
| `web_safari` | YES | NO | HLS (m3u8) formats; same as web |
| `mweb` | YES | NO | Mobile web |
| `web_music` | YES | NO | — |
| `web_creator` | YES | NO | Requires account cookies |
| `tv_simply` | YES | NO | Account cookies not supported |
| `web_embedded` | **NO** | **NO** | Only embeddable videos available |
| `tv` | **NO** | **NO** | All formats DRM'd without valid cookies; requires login |
| `android_vr` | **NO** | **NO** | Made-for-kids videos unavailable |
| `android` | YES | YES | Account cookies not supported; no way to get DroidGuard token |
| `ios` | YES | YES | Account cookies not supported; no way to get iOSGuard token |
| `android_sdkless` | **Possibly NO** | **Possibly NO** | Used as JS-less default; 403 issues reported (Feb 2026) |

**Sources:** [yt-dlp PO Token Guide (GitHub Wiki)](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide) · [yt-dlp/yt-dlp-wiki raw source](https://github.com/yt-dlp/yt-dlp-wiki/blob/master/PO%20Token%20Guide.md) · [Issue #12596 android/ios client question](https://github.com/yt-dlp/yt-dlp/issues/12596)

### Key finding on two token types

YouTube enforces two distinct PO token types:

- **GVS (Google Video Server):** Required for the video streaming CDN requests themselves. Most "web" surface clients need this.
- **Player:** Required for Innertube player requests (fetching format URLs). Only `android` and `ios` clients currently need this.

A PO token from one platform **cannot be used on another** — web BotGuard tokens don't work for Android attestation and vice versa. Source: [yt-dlp PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide)

### The attestation hierarchy

- **Web clients** (`web`, `mweb`, `web_music`, etc.) → BotGuard (JavaScript, runs in browser)
- **Android client** → DroidGuard (Google Play Services, native Android)
- **iOS client** → iOSGuard (native iOS)

The key constraint: **there is currently no known way to generate a DroidGuard or iOSGuard token outside a real device with Google Play Services.** The bgutil/BgUtils toolchain only reverse-engineers BotGuard (web). Source: [BgUtils repo](https://github.com/LuanRT/BgUtils) · [Issue #12596](https://github.com/yt-dlp/yt-dlp/issues/12596)

---

## 2. POT Generation on Android — Current State

### 2a. WebView-based POT generation (BotGuard / Web tokens)

**This is proven viable and already shipped in YTDLnis.**

YTDLnis (a production Android yt-dlp frontend, v1.8.9.1 as of June 2026) implements POT generation directly inside the app using Android's `WebView` to execute YouTube's BotGuard JavaScript attestation. Key evidence:

- v1.8.0 (Oct 2024): First `po_token` setting for YouTube extractor arguments
- v1.8.3 (Mar 2025): "Automatically generate WEB PO Tokens & Visitor Data inside the app" using WebView; two modes (Auth / Non-Auth); `safeBrowsingEnabled` set for API 26+
- v1.8.4 (Apr 2025): Added "Get Data Sync ID" in the PO token WebView screen; custom YouTube URL support
- v1.8.2 (Feb 2025): GVS and Player PO Token configuration per client

**The mechanism:** The WebView loads the YouTube page and executes the BotGuard JS challenge. Because the WebView runs a real Chromium engine on Android, BotGuard's browser fingerprinting succeeds. The resulting token is passed to yt-dlp as an extractor argument (`--extractor-args "youtube:po_token=web+…"`). **No Deno/Node sidecar is needed** — the WebView IS the JS runtime for this purpose.

Sources: [YTDLnis CHANGELOG.md](https://github.com/deniscerri/ytdlnis/blob/main/CHANGELOG.md) · [YTDLnis v1.8.3 release](https://github.com/deniscerri/ytdlnis/releases/tag/v1.8.3) · [YTDLnis v1.8.4 release](https://github.com/deniscerri/ytdlnis/releases/tag/v1.8.4)

**Important caveats:**
- These are **Web (BotGuard) tokens**, used with the `web` or `web_music` yt-dlp client — they attest a web browser session
- The token is **bound to the video ID** and has a short lifespan (~12 hours per yt-dlp docs), so a new token is needed per video — but the WebView can be reused and called per-download
- The token is also session-bound (Visitor ID or account Session ID)
- For silent background downloads (no UI open), this approach requires a foreground Android Service with a hidden WebView, which is architecturally possible but adds complexity

### 2b. DroidGuard / Android-native POT

**Status: Unsolved / flagged-for-device.**

The `android` yt-dlp player client would require a DroidGuard-issued token. DroidGuard is Google's proprietary integrity attestation running in Google Play Services. There is no known reverse-engineering or open reimplementation of DroidGuard comparable to BgUtils/BotGuard. The yt-dlp community's current position (as expressed in issue #12596) is that the `android` client **should be removed from the default client list** because there is "no way to get droidguard and iosguard po token" from outside a genuine device/Play Services context.

**Flagged-for-device:** Whether Google Play Integrity / DroidGuard tokens could be obtained from within an Android app (which does have access to Play Services) is a meaningful open question that would require on-device testing and is not documented in the open-source yt-dlp ecosystem.

### 2c. bgutil-ytdlp-pot-provider — Android story

**No Android port exists.** bgutil-ytdlp-pot-provider is a loopback HTTP server backed by Node.js/Deno/Bun running BgUtils JavaScript. It only handles BotGuard (web tokens). There is no documented Android embedding, Android AAR, or JNI wrapper. The Docker Hub image and pip package are for server environments only. Sources: [bgutil-ytdlp-pot-provider GitHub](https://github.com/Brainicism/bgutil-ytdlp-pot-provider) · [bgutil-ytdlp-pot-provider PyPI](https://pypi.org/project/bgutil-ytdlp-pot-provider/)

---

## 3. POT-Exempt Client Approaches — Durability Assessment

### Current exempt options and their problems

**`android_vr` (no POT required as of early 2026):**
- **Documented as exempt** in the yt-dlp PO Token Guide
- **Actively degrading:** As of March 5, 2026, issue #16150 reports it has become "erratic," intermittently returning only format 18 (360p pre-muxed MP4) rather than the full format range
- YouTube appears to be running an A/B experiment enabling "SABR-only streaming" for some sessions using this client
- Made-for-kids content is unavailable
- **Assessment: fragile.** The exemption is being actively eroded by an A/B test as of 2026; it is documented behavior, not a stable policy commitment

**`tv` client (no POT required, but requires cookies):**
- Removed from yt-dlp's default client list in commit 23b846506378 (early 2026) due to issue #15583
- Returns `LOGIN_REQUIRED` playability status without valid logged-in session cookies
- Formats are DRM-protected even with cookies
- **Assessment: effectively unusable** for an anonymous on-device downloader. Requires a logged-in YouTube account AND provides DRM-locked output

**`web_embedded` (no POT required):**
- Limited to embeddable videos only — excludes a large fraction of YouTube content (age-gated, channel-monetized, most music, etc.)
- **Assessment: too limited for a general-purpose archiver**

**`android_sdkless` (new default for JS-less scenario as of commit 23b8465):**
- Made the JS-less default client replacing `tv` in early 2026
- Issue #15712 documents HTTP 403 errors with android_sdkless formats (Feb 2026)
- The fix was applied but the client's exemption status from POT is not definitively documented
- **Assessment: possible** but not yet proven stable; more investigation needed

Sources: [Issue #16150 android_vr erratic](https://github.com/yt-dlp/yt-dlp/issues/16150) · [Issue #15583 tv LOGIN_REQUIRED](https://github.com/yt-dlp/yt-dlp/issues/15583) · [Commit 23b8465 client defaults](https://github.com/yt-dlp/yt-dlp/commit/23b846506378a6a9c9a0958382d37f943f7cfa51) · [Issue #15712 android_sdkless 403](https://github.com/yt-dlp/yt-dlp/issues/15712)

### Pattern: YouTube actively closes mobile/TV loopholes

- The `tv` client was removed from defaults after YouTube enforced LOGIN_REQUIRED (Jan 2026)
- The `android_vr` client is being restricted via SABR-only experiment (March 2026)
- This is a clear, ongoing pattern of YouTube progressively closing the "unauthenticated non-web-client" loopholes
- NewPipe has multiple open issues (2025–2026) for "Sign in to confirm you're not a bot" that remain unresolved, indicating the mobile/TV alternative client surfaces provide no durable escape

Sources: [NewPipe issue #11139](https://github.com/TeamNewPipe/NewPipe/issues/11139) · [NewPipe issue #13114](https://github.com/TeamNewPipe/NewPipe/issues/13114) · [NewPipe issue #13356](https://github.com/TeamNewPipe/NewPipe/issues/13356)

---

## 4. Recommended On-Device Anti-Bot Architecture

Based on the evidence above, the recommended approach for Catacomb on Android is:

### Primary path: WebView-based BotGuard token generation

Use Android's built-in `WebView` (Chromium) to run BotGuard attestation, extracting a web GVS PO token for use with the `web` or `web_safari` yt-dlp client. This is the approach already proven in YTDLnis (production, 4+ million users). The token is short-lived and video-bound, requiring per-download token generation — acceptable if done in the background before each download starts.

**Why not use a non-POT client instead:** The `android_vr` exemption is being actively eroded (A/B test, March 2026), `tv` requires login and yields DRM output, `web_embedded` is too limited, and YouTube's track record shows these loopholes close within months.

**Why not use bgutil sidecar on Android:** No Android port; would require running a Node/Deno process which is the very problem we're trying to avoid.

### Secondary path: Per-download `web_safari` / `android_sdkless` fallback

Use `android_sdkless` as a fallback when no POT is available (e.g., first run before WebView generates a token). Monitor for continued 403 behavior.

### Fragility assessment: RISKY

The overall approach is **risky in the cat-and-mouse sense** — not unsolved, but requiring active maintenance:
- The WebView/BotGuard approach works today and is production-proven in YTDLnis
- YouTube has historically closed client loopholes within 3–12 months of widespread adoption
- BotGuard itself evolves; BgUtils maintainers have historically kept up, but there is no SLA
- Each yt-dlp release may shift the client default list, requiring tracking
- A YTDLnis-style implementation requires an Android Service with a persistent WebView for headless operation, which adds architectural complexity

---

## 5. Summary of Flagged-for-Device Items

The following claims cannot be verified by web research alone and require on-device testing:

1. **Whether DroidGuard tokens are accessible to an Android app via Play Services APIs** — this could hypothetically allow the `android` client to work natively without POT fakery, but no documentation exists
2. **Whether android_sdkless 403 issue is fully resolved** — the fix was merged but long-term stability is unclear
3. **Whether the android_vr A/B SABR experiment has rolled out fully or is still partial** — the issue #16150 reports it as intermittent (retry usually works), so extent of rollout is unknown
4. **Whether YTDLnis's WebView approach requires the user to be logged into YouTube in the WebView** — the "Auth" vs "Non-Auth" distinction in their changelog suggests two modes, but the scope of the Non-Auth token's validity is not documented

---

## Verdict

**RISKY**

**Is POT required on the mobile client surface?** YES — for any reliable, general-purpose downloader targeting the full YouTube catalog. The seemingly-exempt clients (`android_vr`, `tv`) are either actively being restricted (android_vr, March 2026) or require a logged-in account and yield DRM output (tv). The cleaner path is to embrace POT via WebView BotGuard — not avoid it.

**Recommended approach:** Implement WebView-based BotGuard POT generation (as YTDLnis already does), use it with the `web` or `web_safari` yt-dlp player client, and treat the exemption-based clients as unstable fallbacks to be dropped when YouTube closes them. This avoids any external Deno/Node sidecar. The approach is production-proven but requires ongoing maintenance as BotGuard and yt-dlp evolve.

---

## Sources

- [yt-dlp PO Token Guide (GitHub Wiki)](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide)
- [yt-dlp PO Token Guide (raw wiki source)](https://github.com/yt-dlp/yt-dlp-wiki/blob/master/PO%20Token%20Guide.md)
- [yt-dlp Upcoming Requirements Announcement (Issue #14404)](https://github.com/yt-dlp/yt-dlp/issues/14404)
- [yt-dlp android/ios client question (Issue #12596)](https://github.com/yt-dlp/yt-dlp/issues/12596)
- [yt-dlp android_vr erratic formats (Issue #16150)](https://github.com/yt-dlp/yt-dlp/issues/16150)
- [yt-dlp tv client LOGIN_REQUIRED (Issue #15583)](https://github.com/yt-dlp/yt-dlp/issues/15583)
- [yt-dlp android_sdkless 403 error (Issue #15712)](https://github.com/yt-dlp/yt-dlp/issues/15712)
- [yt-dlp commit: adjust default clients (#15601)](https://github.com/yt-dlp/yt-dlp/commit/23b846506378a6a9c9a0958382d37f943f7cfa51)
- [bgutil-ytdlp-pot-provider GitHub](https://github.com/Brainicism/bgutil-ytdlp-pot-provider)
- [bgutil-ytdlp-pot-provider PyPI](https://pypi.org/project/bgutil-ytdlp-pot-provider/)
- [BgUtils (BotGuard reverse engineering library)](https://github.com/LuanRT/BgUtils)
- [YTDLnis CHANGELOG.md](https://github.com/deniscerri/ytdlnis/blob/main/CHANGELOG.md)
- [YTDLnis Release v1.8.3 (WebView POT)](https://github.com/deniscerri/ytdlnis/releases/tag/v1.8.3)
- [YTDLnis Release v1.8.4 (Data Sync ID)](https://github.com/deniscerri/ytdlnis/releases/tag/v1.8.4)
- [YTDLnis Release v1.8.8 (June 2026)](https://github.com/deniscerri/ytdlnis/releases/tag/v1.8.8)
- [NewPipe "Sign in to confirm" Issue #11139](https://github.com/TeamNewPipe/NewPipe/issues/11139)
- [NewPipe "Sign in to confirm" Issue #13114](https://github.com/TeamNewPipe/NewPipe/issues/13114)
- [NewPipe "Sign in to confirm" Issue #13356](https://github.com/TeamNewPipe/NewPipe/issues/13356)
- [PO Token System — DeepWiki (yt-dlp)](https://deepwiki.com/yt-dlp/yt-dlp/3.4.1-po-token-system)
- [YouTube Authentication — DeepWiki (yt-dlp wiki)](https://deepwiki.com/yt-dlp/yt-dlp-wiki/3.2-youtube-authentication)
