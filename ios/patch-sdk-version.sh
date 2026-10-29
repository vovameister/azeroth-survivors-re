#!/usr/bin/env bash
#
# patch-sdk-version.sh - Xcode's last phase, before code signing: stamp the binary as linked
# against the iOS 18 SDK. winit 0.30 has no UIScene lifecycle, and an app linked against the
# iOS 27 SDK without one traps in UIApplicationMain; an older SDK stamp keeps UIKit's legacy
# (window-only) path. Drop this phase once Bevy ships a winit with scene support.
set -euo pipefail

case "${PLATFORM_NAME}" in
  iphonesimulator) PLATFORM=iossim ;;
  iphoneos)        PLATFORM=ios ;;
  *) exit 0 ;;
esac
BIN="${TARGET_BUILD_DIR}/${EXECUTABLE_PATH}"
STAMP_SDK="${LEGACY_SDK_STAMP:-18.0}"

xcrun vtool -set-build-version "${PLATFORM}" "${IPHONEOS_DEPLOYMENT_TARGET}" "${STAMP_SDK}" \
    -replace -output "${BIN}.stamped" "${BIN}" 2>/dev/null
mv "${BIN}.stamped" "${BIN}"
chmod +x "${BIN}"
