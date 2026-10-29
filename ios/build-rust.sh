#!/usr/bin/env bash
#
# build-rust.sh - Xcode's "Build Rust" phase: the game as a static library for the SDK being built
# (device or simulator), always the release profile (debug is too slow to play). On a device build
# it also clones the game data into the bundle; the simulator reads the Mac's copy through the
# scheme's WOW_DATA instead.
set -euo pipefail
cd "${SRCROOT}/.."

[ -f "${HOME}/.cargo/env" ] && . "${HOME}/.cargo/env"
command -v cargo >/dev/null || { echo "error: cargo not found — install Rust: https://rustup.rs" >&2; exit 1; }

case "${PLATFORM_NAME}" in
  iphonesimulator) TARGET=aarch64-apple-ios-sim ;;
  iphoneos)        TARGET=aarch64-apple-ios ;;
  *) echo "error: unsupported platform ${PLATFORM_NAME}" >&2; exit 1 ;;
esac
rustup target list --installed | grep -qx "${TARGET}" || rustup target add "${TARGET}"

# Xcode's own compiler flags are for the app's sources, not for the C that crates build.
unset CFLAGS CXXFLAGS LDFLAGS OTHER_CFLAGS OTHER_LDFLAGS
cargo build --release -p azeroth-survivors-ios --no-default-features --target "${TARGET}" --target-dir target/ios

# lua-src links Lua as `static:-bundle`, so the staticlib leaves it out: put the newest build's
# liblua5.1.a beside it for the app's `-llua5.1`.
OUT="target/ios/${TARGET}/release"
LUA="$(ls -t "${OUT}"/build/mlua-sys-*/out/lib/liblua5.1.a 2>/dev/null | head -1)"
[ -n "${LUA}" ] || { echo "error: liblua5.1.a not found under ${OUT}/build" >&2; exit 1; }
cp "${LUA}" "${OUT}/liblua5.1.a"

if [ "${PLATFORM_NAME}" = iphoneos ]; then
    DATA="${WOW_DATA_SRC:-wow-data/Data}"
    DEST="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}/Data"
    [ -f "${DATA}/terrain.MPQ" ] || { echo "error: no game data at ${DATA} — run scripts/fetch-data.sh" >&2; exit 1; }
    mkdir -p "${DEST}"
    for f in "${DATA}"/*.MPQ; do
        [ "${DEST}/$(basename "$f")" -nt "$f" ] || cp -c "$f" "${DEST}/" 2>/dev/null || cp "$f" "${DEST}/"
    done
fi
