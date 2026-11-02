#!/usr/bin/env bash
# Installs the app on the running emulator or device, opens it, and checks it's still running and
# in front a few seconds later with nothing in the crash log. Saves a screenshot either way.
#
#   android-smoke-test.sh <apk> <screenshot.png>
set -euo pipefail

apk=${1:?usage: android-smoke-test.sh <apk> <screenshot.png>}
screenshot=${2:?usage: android-smoke-test.sh <apk> <screenshot.png>}
package=com.speakinginbits.financetracker

adb install -r "$apk"
adb logcat -c
adb shell am start -W -n "$package/android.app.NativeActivity"
# Long enough for a crash on start-up or on the first frames to happen.
sleep 10

adb exec-out screencap -p > "$screenshot"
pid=$(adb shell pidof "$package" | tr -d '\r' || true)
front=$(adb shell dumpsys activity activities | grep -m 1 topResumedActivity | tr -d '\r' || true)
crashes=$(adb logcat -d -b crash | tr -d '\r')

if [ -z "$pid" ] || [[ $front != *"$package"* ]] || [ -n "$crashes" ]; then
  echo "Finance Tracker didn't stay open." >&2
  echo "Running as process: ${pid:-none}" >&2
  echo "In front: ${front:-unknown}" >&2
  echo "$crashes" >&2
  adb logcat -d -t 200 '*:E' >&2 || true
  exit 1
fi
echo "Finance Tracker is open and running (process $pid)."
