#!/usr/bin/env bash
#
# ios-sim-check.sh - run every battleground once in the iOS simulator on a given Data folder, as
# the census autopilot plays it, and report what the unified log shows: files not found, panics,
# errors, and a screenshot per battleground to see that loading cleared.
#
#   scripts/ios-sim-check.sh [<sim udid>] [<data dir>]   # default: first booted, wow-data/Trimmed
#
# The app must be built for the simulator first (Xcode, or xcodebuild into target/xcode). Results
# land in target/ios-sim-check/: map-N.log (the app's log lines) and map-N.png.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

SIM="${1:-$(xcrun simctl list devices booted | grep -oE '[0-9A-F-]{36}' | head -1)}"
DATA="$(cd "${2:-wow-data/Trimmed}" && pwd)"
APP=target/xcode/Build/Products/Release-iphonesimulator/AzerothSurvivors.app
OUT=target/ios-sim-check
SECS="${SIM_CHECK_SECS:-60}"
CLASSES=(Warrior Paladin Hunter Rogue Priest Shaman Mage Warlock Druid)
[ -n "${SIM}" ] || { echo "no booted simulator" >&2; exit 1; }
[ -d "${APP}" ] || { echo "no simulator build at ${APP}" >&2; exit 1; }

rm -rf "${OUT}"
mkdir -p "${OUT}"
xcrun simctl install "${SIM}" "${APP}"
for i in $(seq 0 15); do
    class="${CLASSES[(i + 2) % ${#CLASSES[@]}]}"
    xcrun simctl spawn "${SIM}" log stream --level debug --style compact \
        --predicate 'process == "AzerothSurvivors"' >"${OUT}/map-${i}.log" 2>&1 &
    logger=$!
    SIMCTL_CHILD_WOW_DATA="${DATA}" SIMCTL_CHILD_WOW_SURVIVORS_AUTO="${class}:census-run" \
        SIMCTL_CHILD_WOW_SURVIVORS_MAP="${i}" \
        xcrun simctl launch --terminate-running-process "${SIM}" local.azeroth-survivors >/dev/null
    sleep "${SECS}"
    xcrun simctl io "${SIM}" screenshot "${OUT}/map-${i}.png" >/dev/null 2>&1 || true
    kill "${logger}" 2>/dev/null || true
    wait "${logger}" 2>/dev/null || true
    alive=$(xcrun simctl spawn "${SIM}" launchctl list | grep -c azeroth || true)
    notfound=$(grep -ciE 'not found|not in patch chain' "${OUT}/map-${i}.log" || true)
    errors=$(grep -cE ' (E|F[a-z]*) ' "${OUT}/map-${i}.log" || true)
    begun=$(grep -c 'run begins' "${OUT}/map-${i}.log" || true)
    echo "map ${i} (${class}): running=${alive} run-begun=${begun} not-found=${notfound} errors=${errors}"
done
xcrun simctl terminate "${SIM}" local.azeroth-survivors 2>/dev/null || true
