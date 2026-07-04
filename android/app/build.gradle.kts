import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.catacomb.spike"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.catacomb.spike"
        minSdk = 24
        targetSdk = 34
        versionCode = 2
        versionName = "0.2-spike"
        // youtubedl-android ships native Python for these ABIs; the Rust core
        // (jniLibs/) ships arm64-v8a + x86_64. Restrict to the intersection so
        // every ABI in the APK has both libraries.
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        // Reproducible debug key so `assembleDebug` produces an installable,
        // stably-signed APK without Android Studio. Generated on first build
        // by build-apk.sh if absent.
        create("debugks") {
            val ks = rootProject.file("debug.keystore")
            if (ks.exists()) {
                storeFile = ks
                storePassword = "android"
                keyAlias = "androiddebugkey"
                keyPassword = "android"
            }
        }
    }

    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
            if (rootProject.file("debug.keystore").exists()) {
                signingConfig = signingConfigs.getByName("debugks")
            }
        }
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
    composeOptions {
        // Matches Kotlin 1.9.24.
        kotlinCompilerExtensionVersion = "1.5.14"
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
        // youtubedl-android extracts its bundled Python/ffmpeg/aria2c payloads
        // (libpython.zip.so, …) from the native lib dir at first launch, so the
        // .so files MUST be extracted to disk — i.e. legacy packaging on
        // (extractNativeLibs=true). With them left compressed-in-APK the engine
        // init fails.
        jniLibs {
            useLegacyPackaging = true
        }
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.06.00")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.activity:activity-compose:1.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.2")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")

    // On-device yt-dlp engine (bundled Python + yt-dlp) + ffmpeg + aria2c.
    implementation("io.github.junkfood02.youtubedl-android:library:0.18.1")
    implementation("io.github.junkfood02.youtubedl-android:ffmpeg:0.18.1")
    implementation("io.github.junkfood02.youtubedl-android:aria2c:0.18.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}
