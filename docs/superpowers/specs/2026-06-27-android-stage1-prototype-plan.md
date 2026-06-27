# Stage-1 Android Engine Prototype — Plan

**Status:** Planning — based on completed feasibility research (2026-06-27)

## Goal

Prove that a standalone on-device download engine works on Android by:
- Testing WebView-BotGuard POT generation (à la YTDLnis) against real YouTube
- Verifying anti-bot access *without* curl_cffi (which is broken on Android)
- Demonstrating Rust core → Android .so via JNI

**Critical test:** Does WebView-BotGuard + web PO token recover full-format YouTube access without curl_cffi?

## Scope

**In scope:**
- Minimal Android app shell (Kotlin + Jetpack Compose)
- HLahwani/yt-dlp-android integration (May 2026+)
- WebView-based BotGuard/POT token generation
- Passing web GVS PO token to yt-dlp
- Single download test against real YouTube on a real device
- Rust .so compilation for pure modules (vtt, error_class, platform)

**Out of scope:**
- Full Catacomb UI
- Database/library/fingerprint modules (deferred to Stage-2)
- Persistent download queue
- Settings/configuration

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Android App (Kotlin)                     │
│  ┌────────────────┐          ┌─────────────────────────┐  │
│  │  UI (Compose)  │          │   Rust .so (JNI bridge)  │  │
│  │  - URL input   │          │   - vtt parsing          │  │
│  │  - Download    │          │   - error classification │  │
│  │  - Log viewer  │          │   - platform utilities   │  │
│  └────────────────┘          └─────────────────────────┘  │
│           │                                  │             │
│           ▼                                  ▼             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │               WebView (BotGuard)                      │  │
│  │  - Loads YouTube video page                          │  │
│  │  - Executes BotGuard JS challenge                    │  │
│  │  - Extracts web GVS PO token                         │  │
│  └──────────────────────────────────────────────────────┘  │
│           │                                                  │
│           ▼                                                  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │         HLahwani/yt-dlp-android                       │  │
│  │  - QuickJS embedded (Q2 solved)                      │  │
│  │  - Accepts PO token via extractor arg                │  │
│  │  - Downloads video to device storage                 │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Implementation Steps

### Phase 1: App Shell + yt-dlp-android Integration
1. Create Android Studio project (Kotlin, Compose BOM)
2. Add HLahwani/yt-dlp-android dependency
3. Implement basic Compose UI (URL input, Download button, Log view)
4. Wire up yt-dlp-android download call
5. Test basic download (without POT) to verify integration

### Phase 2: WebView-BotGuard Token Generation
1. Study YTDLnis WebView implementation (open source reference)
2. Implement WebView that loads YouTube video page
3. Extract and execute BotGuard JS challenge
4. Capture web GVS PO token from response
5. Verify token format and expiration

### Phase 3: Token Passing + Anti-Bot Test
1. Pass PO token to yt-dlp-android as extractor argument
2. Test download with various YouTube URLs (short, long, 4K)
3. **Critical test:** Verify full-format access without curl_cffi
4. Document which formats/speeds work vs fail

### Phase 4: Rust .so Compilation
1. Create minimal Rust cdylib crate
2. Add pure modules: vtt, error_class, platform
3. Configure cargo-ndk for cross-compilation
4. Expose functions via uniffi
5. Build .so for arm64-v8a
6. Call from Kotlin via JNI

### Phase 5: Integration Test on Real Device
1. Install on physical Android device
2. Run full test suite (various YouTube URLs)
3. Collect metrics: success rate, formats, errors
4. **Go/No-Go decision point**

## Success Criteria

**Primary (anti-bot):**
- ✓ WebView-BotGuard generates valid web GVS PO tokens
- ✓ yt-dlp-android accepts PO token and downloads successfully
- ✓ Full-format access (≥1080p) achieved *without* curl_cffi
- ✓ Test passes on ≥3 distinct YouTube video types

**Secondary (Rust JNI):**
- ✓ Rust .so compiles for arm64-v8a without errors
- ✓ Kotlin can call exported functions
- ✓ vtt parsing works on a real .vtt file

## Fallback Plan

**If anti-bot test fails:**
- Pivot to client-to-server app using Catacomb's existing web API
- Implement browse/download against `/api/library` + `/api/download`
- Revisit standalone engine when anti-bot landscape improves

## Timeline

- Phase 1: 3-4 days (app shell + yt-dlp integration)
- Phase 2: 4-5 days (WebView-BotGuard implementation)
- Phase 3: 2-3 days (token passing + critical test)
- Phase 4: 2-3 days (Rust .so compilation)
- Phase 5: 2-3 days (device testing + decision)

**Total:** ~2-3 weeks

## Dependencies

**External libraries:**
- HLahwani/yt-dlp-android (May 2026+)
- Jetpack Compose BOM
- AndroidX WebView
- cargo-ndk (Rust cross-compile)

**Reference implementations:**
- YTDLnis (WebView-BotGuard approach)
- Seal (yt-dlp-android usage patterns)

## Notes

- The WebView approach eliminates the need for Deno/Node sidecar (Q2 solved)
- QuickJS embedded in yt-dlp-android handles JS interp (Q2 solved)
- Rust .so is low-risk (build already verified locally)
- The **only** open question is anti-bot without curl_cffi — this prototype answers it
