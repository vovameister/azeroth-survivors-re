#!/usr/bin/env bash
#
# build-rust.sh - Xcode's "Build Rust" phase: the game as a static library for the SDK being built
# (device or simulator), in a fixed environment (see `cargo` below), always the release profile (debug is too slow to play). On a device build
# it also clones the game data into the bundle; the simulator reads the Mac's copy through the
# scheme's WOW_DATA instead.
set -euo pipefail
cd "${SRCROOT}/.."

[ -f "${HOME}/.cargo/env" ] && . "${HOME}/.cargo/env"
command -v cargo >/dev/null || { echo "error: cargo not found — install Rust: https://rustup.rs" >&2; exit 1; }

case "${PLATFORM_NAME}" in
  iphonesimulator) TARGET=aarch64-apple-ios-sim; SDK=iphonesimulator ;;
  iphoneos)        TARGET=aarch64-apple-ios;     SDK=iphoneos ;;
  *) echo "error: unsupported platform ${PLATFORM_NAME}" >&2; exit 1 ;;
esac
rustup target list --installed | grep -qx "${TARGET}" || rustup target add "${TARGET}"

# Cargo runs in a fixed environment, not Xcode's ~600 build settings. Build scripts declare the
# variables they read, and a change in any reruns that crate and rebuilds everything above it:
# `tracing-oslog` (under bevy_log) reads SDKROOT, which Xcode sets on some runs and not others,
# and that alone rebuilt all of Bevy. Xcode's compiler flags are for the app's sources anyway.
CARGO="$(command -v cargo)"
cargo() {
    env -i \
        HOME="${HOME}" USER="${USER:-}" TMPDIR="${TMPDIR:-/tmp}" \
        PATH="$(dirname "${CARGO}"):/usr/bin:/bin:/usr/sbin:/sbin" \
        ${CARGO_HOME:+"CARGO_HOME=${CARGO_HOME}"} \
        ${RUSTUP_HOME:+"RUSTUP_HOME=${RUSTUP_HOME}"} \
        ${RUSTUP_TOOLCHAIN:+"RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN}"} \
        DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}" \
        SDKROOT="$(xcrun --sdk "${SDK}" --show-sdk-path)" \
        IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET}" \
        "${CARGO}" "$@"
}

# The C that crates build (Lua, blake3, ...) takes its minimum iOS from IPHONEOS_DEPLOYMENT_TARGET,
# but cargo does not rerun a build script when that changes, so objects for another minimum would
# link with "built for newer iOS version" warnings and could call what an older iOS lacks. A
# changed minimum cleans this target's artifacts once.
STAMP="target/ios/${TARGET}.deployment-target"
if [ "$(cat "${STAMP}" 2>/dev/null)" != "${IPHONEOS_DEPLOYMENT_TARGET}" ]; then
    cargo clean --release --target "${TARGET}" --target-dir target/ios
    mkdir -p target/ios
    echo "${IPHONEOS_DEPLOYMENT_TARGET}" > "${STAMP}"
fi
# No future-incompatibility nag in Xcode's issue list: its one source, `block` 0.1.6, comes in
# through wgpu's Metal backend and is Bevy's to bump, not this project's.
cargo build --release -p azeroth-survivors-ios --no-default-features --target "${TARGET}" \
    --target-dir target/ios --config 'future-incompat-report.frequency="never"'

# lua-src links Lua as `static:-bundle`, so the staticlib leaves it out: put the newest build's
# liblua5.1.a beside it for the app's `-llua5.1`.
OUT="target/ios/${TARGET}/release"
LUA="$(ls -t "${OUT}"/build/mlua-sys-*/out/lib/liblua5.1.a 2>/dev/null | head -1)"
[ -n "${LUA}" ] || { echo "error: liblua5.1.a not found under ${OUT}/build" >&2; exit 1; }
cp "${LUA}" "${OUT}/liblua5.1.a"

if [ "${PLATFORM_NAME}" = iphoneos ]; then
    # The trimmed data (scripts/trim-data.sh, a few hundred MB) where it has been built, else the
    # whole 5 GB client.
    if [ -n "${WOW_DATA_SRC:-}" ]; then
        DATA="${WOW_DATA_SRC}"
    elif [ -f wow-data/Trimmed/patch.MPQ ]; then
        DATA=wow-data/Trimmed
    else
        DATA=wow-data/Data
    fi
    DEST="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}/Data"
    [ -f "${DATA}/patch.MPQ" ] || { echo "error: no game data at ${DATA} — run scripts/fetch-data.sh" >&2; exit 1; }
    echo "game data: ${DATA}"
    mkdir -p "${DEST}"
    # A mirror: an archive the source no longer has (the full client's, after trimming) goes.
    for f in "${DEST}"/*.MPQ; do
        [ -e "$f" ] && [ ! -e "${DATA}/$(basename "$f")" ] && rm -f "$f"
    done
    # Copied unless the bundle's is as large and newer: the copy's time is the copy's, so after a
    # switch of source (trimmed back to the full client) only the size tells the two apart.
    for f in "${DATA}"/*.MPQ; do
        to="${DEST}/$(basename "$f")"
        [ -e "${to}" ] && [ "$(stat -f %z "$f")" = "$(stat -f %z "${to}")" ] && [ "${to}" -nt "$f" ] && continue
        cp -c "$f" "${DEST}/" 2>/dev/null || cp "$f" "${DEST}/"
    done
fi
