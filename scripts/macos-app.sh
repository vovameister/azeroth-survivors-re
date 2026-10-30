#!/usr/bin/env bash
#
# macos-app.sh - package the native player build as a self-contained macOS app bundle.
#
#   scripts/macos-app.sh                  # -> dist/Azeroth Survivors.app (Data cloned inside)
#   WOW_DATA=/path/to/Data scripts/macos-app.sh
#   APP_NO_DATA=1 scripts/macos-app.sh    # no Data inside; the app then reads $WOW_DATA or
#                                         # ~/Library/Application Support/AzerothSurvivors/Data
#
# The bundle runs the player build (no `dev` feature: no source-tree paths in the binary). Its
# launcher points WOW_DATA at Contents/Resources/Data and BENILLA_HOME at
# ~/Library/Application Support/AzerothSurvivors, so nothing is ever written inside the bundle.
# Data is copied with `cp -c` (an APFS clone): no extra disk space on the same volume.
#
# Prerequisites: Rust (rust-toolchain.toml), the game data (scripts/fetch-data.sh).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

NAME="Azeroth Survivors"
BUNDLE_ID="local.azeroth-survivors"
APP="dist/${NAME}.app"
DATA="${WOW_DATA:-wow-data/Data}"
TARGET_DIR=target/player
BIN="${TARGET_DIR}/release/warcraft-survivors"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"

if [ -z "${APP_NO_DATA:-}" ] && [ ! -f "${DATA}/terrain.MPQ" ]; then
    echo "no game data at ${DATA} — run scripts/fetch-data.sh (or set WOW_DATA, or APP_NO_DATA=1)" >&2
    exit 1
fi

command -v cargo >/dev/null || { [ -f "${HOME}/.cargo/env" ] && . "${HOME}/.cargo/env"; }
command -v cargo >/dev/null || { echo "cargo not found — install Rust: https://rustup.rs" >&2; exit 1; }
cargo build --release -p warcraft-survivors --no-default-features --target-dir "${TARGET_DIR}"

rm -rf "${APP}"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "${BIN}" "${APP}/Contents/MacOS/warcraft-survivors"

# The launcher: the bundle's executable, a Mach-O (Gatekeeper refuses a script as the main
# executable). It sets WOW_DATA and BENILLA_HOME, sends output to last-run.log, and execs the game.
launcher_src="$(mktemp -d)/launcher.c"
cat > "${launcher_src}" <<'EOF'
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
    char exe[PATH_MAX], here[PATH_MAX], path[PATH_MAX], support[PATH_MAX];
    uint32_t size = sizeof exe;
    if (_NSGetExecutablePath(exe, &size) != 0 || !realpath(exe, here)) return 1;
    *strrchr(here, '/') = '\0';

    const char *home = getenv("HOME");
    snprintf(support, sizeof support, "%s/Library/Application Support/AzerothSurvivors", home ? home : "/tmp");
    mkdir(support, 0755);

    if (!getenv("WOW_DATA")) {
        struct stat st;
        snprintf(path, sizeof path, "%s/../Resources/Data", here);
        if (stat(path, &st) != 0) snprintf(path, sizeof path, "%s/Data", support);
        setenv("WOW_DATA", path, 1);
    }
    if (!getenv("BENILLA_HOME")) {
        snprintf(path, sizeof path, "%s/benilla-config", support);
        setenv("BENILLA_HOME", path, 1);
    }

    snprintf(path, sizeof path, "%s/last-run.log", support);
    int log = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (log >= 0) { dup2(log, 1); dup2(log, 2); close(log); }

    snprintf(path, sizeof path, "%s/warcraft-survivors", here);
    argv[0] = path;
    execv(path, argv);
    perror("execv");
    return 1;
}
EOF
clang -O2 -mmacosx-version-min=13.0 -o "${APP}/Contents/MacOS/launcher" "${launcher_src}"
rm -rf "$(dirname "${launcher_src}")"

if [ -z "${APP_NO_DATA:-}" ]; then
    echo "cloning game data into the bundle…"
    mkdir -p "${APP}/Contents/Resources/Data"
    cp -c "${DATA}"/*.MPQ "${APP}/Contents/Resources/Data/" 2>/dev/null \
        || cp "${DATA}"/*.MPQ "${APP}/Contents/Resources/Data/"
fi

# The icon, the iOS app's (rendered from ios/App/AppIcon.svg): sips cuts the sizes, iconutil packs
# them.
png=ios/App/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png
if [ -f "${png}" ]; then
    tmp="$(mktemp -d)"
    set_dir="${tmp}/AppIcon.iconset"
    mkdir -p "${set_dir}"
    for s in 16 32 128 256 512; do
        sips -z $s $s "${png}" --out "${set_dir}/icon_${s}x${s}.png" >/dev/null
        sips -z $((s * 2)) $((s * 2)) "${png}" --out "${set_dir}/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "${set_dir}" -o "${APP}/Contents/Resources/AppIcon.icns" || true
    rm -rf "${tmp}"
fi

cat > "${APP}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>               <string>${NAME}</string>
    <key>CFBundleDisplayName</key>        <string>${NAME}</string>
    <key>CFBundleIdentifier</key>         <string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key>            <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
    <key>CFBundleExecutable</key>         <string>launcher</string>
    <key>CFBundleIconFile</key>           <string>AppIcon</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>LSMinimumSystemVersion</key>     <string>13.0</string>
    <key>LSApplicationCategoryType</key>  <string>public.app-category.games</string>
    <key>NSHighResolutionCapable</key>    <true/>
    <key>LSSupportsGameMode</key>         <true/>
</dict>
</plist>
EOF

# Ad-hoc signature: enough for this Mac to launch it (not for sharing; that needs a Developer ID).
echo "signing (ad-hoc)…"
codesign --force --deep --sign - "${APP}"

echo "built ${APP}"
du -sh "${APP}"
