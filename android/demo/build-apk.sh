#!/usr/bin/env bash
# Build an installable, debug-signed APK for the Catacomb Rust-core JNI demo,
# driving the SDK build-tools directly (aapt2 → javac → d8 → zip → zipalign →
# apksigner). No Gradle / AGP / AndroidX — the app uses only android.jar, so
# this sidesteps AGP's toolchain constraints (e.g. very new JDKs).
#
# Prereqs (auto-detected): an Android SDK with platforms;android-34 and
# build-tools;34.0.0, plus the native libs built by
# ../rust/catacomb_core/build.sh (which populates ../app/src/main/jniLibs/).
#
# Usage:  ./build-apk.sh          → out/catacomb-spike-debug.apk
set -euo pipefail
cd "$(dirname "$0")"

PKG_PATH="com/catacomb/spike"
BUILD_TOOLS_VER="${BUILD_TOOLS_VER:-34.0.0}"
API="${API:-34}"

# ── Locate the SDK ────────────────────────────────────────────────────────
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
[[ -d "$SDK" ]] || { echo "ERROR: Android SDK not found (set ANDROID_HOME)."; exit 1; }
BT="$SDK/build-tools/$BUILD_TOOLS_VER"
ANDROID_JAR="$SDK/platforms/android-$API/android.jar"
for f in "$BT/aapt2" "$BT/d8" "$BT/zipalign" "$BT/apksigner" "$ANDROID_JAR"; do
    [[ -e "$f" ]] || { echo "ERROR: missing $f"; echo "Install: sdkmanager 'build-tools;$BUILD_TOOLS_VER' 'platforms;android-$API'"; exit 1; }
done

# ── Pick a JDK the SDK tools tolerate ─────────────────────────────────────
# R8/d8 (and apksigner) choke on very new JDKs (e.g. 26 → internal NPE), so
# prefer an LTS ≤ 21. The build-tools wrapper scripts honour JAVA_HOME; javac
# is invoked explicitly with the same JDK. Override with TOOL_JAVA_HOME.
find_tool_jdk() {
    if [[ -n "${TOOL_JAVA_HOME:-}" && -x "$TOOL_JAVA_HOME/bin/java" ]]; then
        echo "$TOOL_JAVA_HOME"; return
    fi
    local c
    for c in java-17-openjdk java-21-openjdk java-11-openjdk; do
        [[ -x "/usr/lib/jvm/$c/bin/java" ]] && { echo "/usr/lib/jvm/$c"; return; }
    done
    # Fall back to whatever `java` is on PATH (may fail on bleeding-edge JDKs).
    local p; p="$(command -v java || true)"
    [[ -n "$p" ]] && echo "$(dirname "$(dirname "$(readlink -f "$p")")")"
}
export JAVA_HOME="$(find_tool_jdk)"
JAVAC="$JAVA_HOME/bin/javac"
KEYTOOL="$JAVA_HOME/bin/keytool"
[[ -x "$JAVA_HOME/bin/java" ]] || { echo "ERROR: no usable JDK (set TOOL_JAVA_HOME)."; exit 1; }
echo "Using JDK for SDK tools: $JAVA_HOME"

# Native libs produced by the Rust cross-compile step.
JNILIBS="../app/src/main/jniLibs"
if ! ls "$JNILIBS"/*/libcatacomb_core.so >/dev/null 2>&1; then
    echo "ERROR: no native libs in $JNILIBS. Run ../rust/catacomb_core/build.sh first."; exit 1
fi

OUT="out"; WORK="$OUT/work"
rm -rf "$OUT"; mkdir -p "$WORK/classes" "$WORK/dex" "$WORK/apk"

echo "── 1/6 aapt2: link resources + manifest ──────────────────────"
"$BT/aapt2" link \
    -I "$ANDROID_JAR" \
    --manifest AndroidManifest.xml \
    --min-sdk-version 24 --target-sdk-version "$API" \
    -o "$WORK/base.apk"

echo "── 2/6 javac: compile Java against android.jar ───────────────"
mapfile -t SRCS < <(find src -name '*.java')
"$JAVAC" --release 11 -Xlint:-options \
    -classpath "$ANDROID_JAR" \
    -d "$WORK/classes" "${SRCS[@]}"

echo "── 3/6 d8: dex the classes ───────────────────────────────────"
mapfile -t CLASSES < <(find "$WORK/classes" -name '*.class')
"$BT/d8" --release --min-api 24 --lib "$ANDROID_JAR" \
    --output "$WORK/dex" "${CLASSES[@]}"

echo "── 4/6 assemble: dex + native libs into the APK ──────────────"
# aapt2 already produced base.apk (manifest + resources.arsc). Add classes.dex
# at the archive root and the .so files under lib/<abi>/, then this is a
# complete (unaligned, unsigned) APK.
cp "$WORK/base.apk" "$WORK/unsigned.apk"
( cd "$WORK/dex" && zip -q "$OLDPWD/$WORK/unsigned.apk" classes.dex )
# Stage lib/<abi>/libcatacomb_core.so from every built ABI.
LIBROOT="$WORK/apk"
for so in "$JNILIBS"/*/libcatacomb_core.so; do
    abi="$(basename "$(dirname "$so")")"
    mkdir -p "$LIBROOT/lib/$abi"
    cp "$so" "$LIBROOT/lib/$abi/"
done
( cd "$LIBROOT" && zip -q -r "$OLDPWD/$WORK/unsigned.apk" lib )

echo "── 5/6 zipalign ──────────────────────────────────────────────"
"$BT/zipalign" -p -f 4 "$WORK/unsigned.apk" "$WORK/aligned.apk"

echo "── 6/6 apksigner: debug-sign ─────────────────────────────────"
KS="$OUT/debug.keystore"
if [[ ! -f "$KS" ]]; then
    "$KEYTOOL" -genkeypair -keystore "$KS" -alias androiddebugkey \
        -storepass android -keypass android \
        -keyalg RSA -keysize 2048 -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
fi
APK="$OUT/catacomb-spike-debug.apk"
"$BT/apksigner" sign \
    --ks "$KS" --ks-pass pass:android --key-pass pass:android \
    --min-sdk-version 24 \
    --out "$APK" "$WORK/aligned.apk"
"$BT/apksigner" verify --min-sdk-version 24 "$APK" && echo "signature OK"

rm -rf "$WORK"
echo ""
echo "✓ APK: $(cd "$(dirname "$APK")" && pwd)/$(basename "$APK")  ($(du -h "$APK" | cut -f1))"
echo "  Install: adb install -r $APK"
