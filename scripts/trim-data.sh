#!/usr/bin/env bash
#
# trim-data.sh - build a Data folder holding only what Warcraft Survivors reads, for the iOS app
# (the full 1.12.1 client is 5 GB; the mode reads a few hundred MB of it).
#
#   scripts/trim-data.sh                  # census + pack  -> wow-data/Trimmed
#   scripts/trim-data.sh census           # only the census runs (target/datatrace/*.txt)
#   scripts/trim-data.sh pack             # only the pack, from the traces already there
#
# Census: the native build runs `WOW_SURVIVORS_AUTO=<class>:census` with `WOW_DATA_TRACE`, which
# lists every file the chain reads. One run tours every class and look at the menu; then one run per
# battleground (classes in turn) casts the whole pool, spawns every enemy and boss of the map and
# presses the mobility spell. Two run at a time, each with its own state folder.
#
# Pack: the small archives (interface, dbc, fonts, misc, base, backup, patch-2) are kept whole; the
# big ones (patch, terrain, model, texture, wmo, sound, speech) are replaced by one patch.MPQ of
# the traced files, each as the full chain resolves it (`mpqpack`). On top of the traces it packs
# every terrain tile each battleground can stream, its leash plus the residency window at the
# widest view distance (`farclip` 777: 26 chunks, 867 yd), and every texture, model and WMO those
# tiles name: a trace holds only where one run went, and a missing tile holds loading up for good.
#
# Prerequisites: Rust, the full game data (scripts/fetch-data.sh). Env: WOW_DATA (default
# wow-data/Data), TRIM_RUN_SECS (per battleground, default 100), TRIM_JOBS (default 2),
# TRIM_TRACES (default target/datatrace), TRIM_CLASS_SHIFT (which class starts at map 0, default 0).
#
#   scripts/trim-data.sh check            # a census on wow-data/Trimmed, classes shifted, then the
#                                         # names it read that the trimmed set lacks
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
[ -f "${HOME}/.cargo/env" ] && . "${HOME}/.cargo/env"

DATA="${WOW_DATA:-wow-data/Data}"
OUT=wow-data/Trimmed
TRACES="${TRIM_TRACES:-target/datatrace}"
RUN_SECS="${TRIM_RUN_SECS:-100}"
JOBS="${TRIM_JOBS:-2}"
CLASSES=(Warrior Paladin Hunter Rogue Priest Shaman Mage Warlock Druid)
MAPS=16
KEEP=(interface.MPQ dbc.MPQ fonts.MPQ misc.MPQ base.MPQ backup.MPQ patch-2.MPQ)

census() {
    cargo build --release -p warcraft-survivors
    rm -rf "${TRACES}"
    mkdir -p "${TRACES}"
    TRACES="$(cd "${TRACES}" && pwd)"
    local bin
    bin="$(pwd)/target/release/warcraft-survivors"
    # One traced run: the auto mode, the battleground, how long, the trace's name.
    run() {
        local mode="$1" map="$2" secs="$3" name="$4" home
        home="$(mktemp -d)"
        echo "census: ${name} (${mode}, map ${map}, ${secs}s)"
        WOW_DATA="${DATA}" BENILLA_HOME="${home}" WOW_SURVIVORS_AUTO="${mode}" \
            WOW_SURVIVORS_MAP="${map}" WOW_DATA_TRACE="${TRACES}/${name}.txt" \
            "${bin}" >"${TRACES}/${name}.log" 2>&1 &
        local pid=$!
        sleep "${secs}"
        kill "${pid}" 2>/dev/null || true
        wait "${pid}" 2>/dev/null || true
        rm -rf "${home}"
        # A run that died early reads little; the pack would be short without a word.
        [ -s "${TRACES}/${name}.txt" ] || echo "census: WARNING ${name} left no trace, see ${TRACES}/${name}.log" >&2
    }
    run "Warrior:census" 0 240 menu-tour &
    local i
    for ((i = 0; i < MAPS; i++)); do
        while [ "$(jobs -rp | wc -l)" -ge "${JOBS}" ]; do sleep 2; done
        local class="${CLASSES[(i + ${TRIM_CLASS_SHIFT:-0}) % ${#CLASSES[@]}]}"
        run "${class}:census-run" "${i}" "${RUN_SECS}" "map-${i}" &
    done
    wait
    cat "${TRACES}"/*.txt | sort -u >"${TRACES}/all.txt"
    echo "census: $(wc -l <"${TRACES}/all.txt") distinct files read"
}

pack() {
    [ -f "${TRACES}/all.txt" ] || { echo "no traces — run: scripts/trim-data.sh census" >&2; exit 1; }
    cargo build --release -p benilla-formats --bin mpqpack
    rm -rf "${OUT}"
    mkdir -p "${OUT}"
    for a in "${KEEP[@]}"; do
        cp -c "${DATA}/${a}" "${OUT}/${a}" 2>/dev/null || cp "${DATA}/${a}" "${OUT}/${a}"
    done
    # `<map id> <wow x> <wow y> <reach yd>` per battleground, from survivors/data.rs.
    sed -n '/pub(crate) static MAPS/,/^];/p' crates/benilla-app/src/survivors/data.rs |
        grep -E 'map:|center:|leash:' | paste - - - |
        sed -E 's/.*map: ([0-9]+),.*center: \[([-0-9.]+), ([-0-9.]+),.*leash: ([0-9.]+),.*/\1 \2 \3 \4/' |
        awk '{ printf "%s %s %s %.0f\n", $1, $2, $3, $4 + 867 }' >"${TRACES}/seeds.txt"
    [ "$(wc -l <"${TRACES}/seeds.txt")" -eq "${MAPS}" ] || { echo "seeds: not ${MAPS} maps" >&2; exit 1; }
    ./target/release/mpqpack "${DATA}" "${TRACES}/all.txt" "${OUT}/patch.MPQ" "${TRACES}/seeds.txt"
    du -sh "${OUT}"
}

# A census on the trimmed set with other classes on each map, then every name it read that the
# full chain holds and the trimmed one does not: what the trim missed.
check() {
    WOW_DATA="${OUT}" TRIM_TRACES=target/datatrace-check TRIM_CLASS_SHIFT=4 scripts/trim-data.sh census
    cargo build --release -p benilla-formats --bin mpqx
    local missed=0 name
    while IFS= read -r name; do
        if ./target/release/mpqx "${OUT}" "${name}" /dev/null >/dev/null 2>&1; then continue; fi
        if ./target/release/mpqx "${DATA}" "${name}" /dev/null >/dev/null 2>&1; then
            echo "missed: ${name}"
            missed=$((missed + 1))
        fi
    done <target/datatrace-check/all.txt
    echo "check: ${missed} files the game read are missing from ${OUT}"
}

case "${1:-all}" in
    census) census ;;
    pack) pack ;;
    check) check ;;
    all) census; pack ;;
    *) echo "usage: $0 [census|pack|check]" >&2; exit 1 ;;
esac
