// PhotoCraft for Android: the browser build (packaging/web/package.sh) bundled in an app that
// shows it full screen in a WebView, offline. Build with packaging/android/package.sh.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "PhotoCraftAndroid"
include(":app")
