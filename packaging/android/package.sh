#!/usr/bin/env bash
# Build PhotoCraft for Android:  $DIST/PhotoCraft-<version>.apk
#
# Usage: packaging/android/package.sh [--skip-web]
#
# The app is the browser build (packaging/web/package.sh) bundled in a WebView app that runs it
# full screen and offline. Needs what the web build needs (trunk, the wasm32-unknown-unknown
# target), plus JDK 17, Gradle 8.9+ and the Android SDK (ANDROID_HOME) with platform 35.
# --skip-web reuses the site the last web build left in target/web-package.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/android"

if [[ "${1:-}" != "--skip-web" ]]; then
  "$ROOT/packaging/web/package.sh"
fi

SITE="$CARGO_TARGET_DIR/web-package/photocraft-web-$VERSION"
[[ -f "$SITE/index.html" ]] || { echo "error: $SITE/index.html missing; run without --skip-web" >&2; exit 1; }

# The site, served by the app from assets/www.
ASSETS="$HERE/app/src/main/assets"
rm -rf "$ASSETS/www"
mkdir -p "$ASSETS/www"
cp -R "$SITE/." "$ASSETS/www/"
# Host-only files the app has no use for.
rm -f "$ASSETS/www/_headers" "$ASSETS/www/.htaccess" "$ASSETS/www/HOSTING.md"

# android-bridge.js must run before the app starts: first thing in <head>.
python3 - "$ASSETS/www/index.html" <<'PY'
import re, sys
path = sys.argv[1]
html = open(path, encoding="utf-8").read()
tag = '<script src="../android-bridge.js"></script>'
new, n = re.subn(r"(<head[^>]*>)", lambda m: m.group(1) + tag, html, count=1, flags=re.I)
if n != 1:
    sys.exit("error: no <head> in " + path)
open(path, "w", encoding="utf-8").write(new)
PY

# Launcher icon from the app's own icon set.
RES="$HERE/app/src/main/res"
icon() { mkdir -p "$RES/mipmap-$1" && cp "$ROOT/assets/app-icon/hicolor/$2/apps/ai.storyteller.photocraft.png" "$RES/mipmap-$1/ic_launcher.png"; }
icon mdpi 48x48
icon hdpi 128x128
icon xhdpi 128x128
icon xxhdpi 256x256
icon xxxhdpi 512x512

# versionCode must grow with each version: 0.17.0 -> 1700, 1.2.3 -> 10203.
IFS=. read -r major minor patch <<<"${VERSION%%-*}"
CODE=$((10#${major:-0} * 10000 + 10#${minor:-0} * 100 + 10#${patch:-0}))
# A build number (CI's run number) keeps later builds of the same version installable as updates.
if [[ -n "${PHOTOCRAFT_ANDROID_BUILD:-}" ]]; then
  CODE=$((CODE * 10000 + 10#$PHOTOCRAFT_ANDROID_BUILD % 10000))
fi

(cd "$HERE" && ./gradlew --no-daemon assembleRelease \
  -PphotocraftVersion="$VERSION" -PphotocraftVersionCode="$CODE")

mkdir -p "$DIST"
APK="$DIST/PhotoCraft-$VERSION.apk"
cp "$HERE/app/build/outputs/apk/release/app-release.apk" "$APK"
echo "Built $APK ($(($(wc -c <"$APK") / 1048576)) MiB, versionCode $CODE)"
