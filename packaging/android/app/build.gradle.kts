plugins {
    id("com.android.application")
}

// package.sh passes the workspace version; a bare `gradle assembleRelease` still builds.
val photocraftVersion: String = (project.findProperty("photocraftVersion") as String?) ?: "0.0.0"
val photocraftVersionCode: Int = (project.findProperty("photocraftVersionCode") as String?)?.toInt() ?: 1

android {
    namespace = "ai.storyteller.photocraft.android"
    compileSdk = 35

    defaultConfig {
        applicationId = "ai.storyteller.photocraft.android"
        // Android 10+: downloads go to the shared Downloads folder through MediaStore without a
        // storage permission, and its WebView runs WebGL2.
        minSdk = 29
        targetSdk = 35
        versionCode = photocraftVersionCode
        versionName = photocraftVersion
    }

    signingConfigs {
        // A key kept in the repository so every build installs over the last one. It only signs
        // this sideloaded app; it is not a Play Store key.
        create("sideload") {
            storeFile = rootProject.file("sideload.keystore")
            storePassword = "photocraft"
            keyAlias = "photocraft"
            keyPassword = "photocraft"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("sideload")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    androidResources {
        // The app's wasm parts are gzip already; the boot wasm is small. Storing them as they are
        // keeps the APK from compressing them twice.
        noCompress += listOf("wasm", "gz")
    }

    lint {
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.12.1")
}
