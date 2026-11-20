#!/usr/bin/env bash
# Start the APK on a running emulator (adb), let it run, and collect what happened:
#   $OUT/logcat.txt, $OUT/error-log.txt (the app's own log), $OUT/screen.png, $OUT/summary.txt
# Usage: packaging/android/emulator-check.sh <apk> <out dir>
set -uo pipefail
APK="$1"
OUT="$2"
PKG=ai.storyteller.photocraft.android
mkdir -p "$OUT"

adb wait-for-device
adb logcat -c || true
adb install -r -g "$APK" || { echo "install failed" >"$OUT/summary.txt"; exit 1; }
adb shell am start -W -n "$PKG/.MainActivity"

# The editor's wasm takes a while to compile on an emulator; check it is still up as it goes.
elapsed=0
for wait in 15 15 30 30 30; do
  sleep "$wait"
  elapsed=$((elapsed + wait))
  pid=$(adb shell pidof "$PKG" | tr -d '\r')
  echo "after ${elapsed}s: pid=${pid:-none (not running)}" | tee -a "$OUT/progress.txt"
done

adb exec-out screencap -p >"$OUT/screen.png" || true
adb logcat -d -v time >"$OUT/logcat.txt" || true
adb shell cat "/sdcard/Download/PhotoCraft/PhotoCraft-error-log.txt" >"$OUT/error-log.txt" 2>/dev/null || true

{
  echo "== progress"
  cat "$OUT/progress.txt"
  echo "== crash (AndroidRuntime / native)"
  grep -E "FATAL EXCEPTION|AndroidRuntime|F DEBUG|Fatal signal|ANR in|am_crash" "$OUT/logcat.txt" | head -60
  echo "== app error log (end)"
  tail -n 60 "$OUT/error-log.txt"
  echo "== WebView/chromium errors"
  grep -E "chromium|cr_" "$OUT/logcat.txt" | grep -iE "error|fatal|crash|fail" | head -30
} >"$OUT/summary.txt"
cat "$OUT/summary.txt"
