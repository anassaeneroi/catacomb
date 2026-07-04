#!/usr/bin/env bash
# Build the Catacomb Android app (Compose + bundled yt-dlp + Rust JNI core).
#
# Runs Gradle on a JDK it tolerates (<= 21; the system JDK may be newer and
# breaks R8/d8) and ensures the Rust native libs exist first. Produces a
# debug-signed, installable APK.
#
# Usage: ./build-apk.sh
# Output: app/build/outputs/apk/debug/app-debug.apk
set -euo pipefail
cd "$(dirname "$0")"

# ── JDK for Gradle/AGP (<= 21) ────────────────────────────────────────────
find_jdk() {
    if [[ -n "${TOOL_JAVA_HOME:-}" && -x "$TOOL_JAVA_HOME/bin/java" ]]; then
        echo "$TOOL_JAVA_HOME"; return
    fi
    local c
    for c in java-17-openjdk java-21-openjdk; do
        [[ -x "/usr/lib/jvm/$c/bin/java" ]] && { echo "/usr/lib/jvm/$c"; return; }
    done
    echo "${JAVA_HOME:-}"
}
export JAVA_HOME="$(find_jdk)"
[[ -x "$JAVA_HOME/bin/java" ]] || { echo "ERROR: need a JDK <= 21 (set TOOL_JAVA_HOME)."; exit 1; }
echo "Using JDK: $JAVA_HOME"

# ── Android SDK location for Gradle ───────────────────────────────────────
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
[[ -d "$SDK" ]] || { echo "ERROR: Android SDK not found (set ANDROID_HOME)."; exit 1; }
if [[ ! -f local.properties ]]; then
    echo "sdk.dir=$SDK" > local.properties
fi

# ── Native libs (Rust core) ───────────────────────────────────────────────
if ! ls app/src/main/jniLibs/*/libcatacomb_core.so >/dev/null 2>&1; then
    echo "Building Rust core native libs…"
    ( cd rust/catacomb_core && ./build.sh )
fi

# ── Debug keystore (stable, reproducible signing) ─────────────────────────
if [[ ! -f debug.keystore ]]; then
    "$JAVA_HOME/bin/keytool" -genkeypair -keystore debug.keystore \
        -alias androiddebugkey -storepass android -keypass android \
        -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi

echo "── gradlew assembleDebug ─────────────────────────────────────"
./gradlew --no-daemon assembleDebug "$@"

APK="app/build/outputs/apk/debug/app-debug.apk"
if [[ -f "$APK" ]]; then
    echo ""
    echo "✓ APK: $(pwd)/$APK  ($(du -h "$APK" | cut -f1))"
    echo "  Install: adb install -r $APK"
else
    echo "✗ build finished but $APK not found" >&2; exit 1
fi
