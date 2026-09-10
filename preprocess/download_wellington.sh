#!/usr/bin/env bash
# Download the LINZ open data (https://github.com/linz/imagery,
# https://github.com/linz/elevation) that beats what download_nz.sh gets for
# Wellington, one directory per resolution level.
#
#   ./download_wellington.sh              # all levels, Wellington city (~81 GiB)
#   ./download_wellington.sh BQ31         # only the given Topo50 sheets
#
# download_nz.sh covers the whole country at 10 m imagery and an 8 m
# contour-derived DEM. Everything below is finer than that, so each level goes
# in its own directory and the renderer can be pointed at whichever one fits:
#
#   source_data/wellington/height/1m       LiDAR DEM      3 tiles    1.6 GiB
#   source_data/wellington/albedo/0.075m   aerial 2025  505 tiles   37.0 GiB
#   source_data/wellington/albedo/0.2m     aerial 2025  147 tiles   42.7 GiB
#
# Tile counts and sizes are for the default sheets. 1 m is the finest elevation
# LINZ publishes anywhere in New Zealand, so height has a single level.
#
# The two albedo levels trade detail against reach, and are both flown in 2025:
# 0.075m is Wellington city only, while 0.2m spans twenty Topo50 sheets across
# the wider region. Passing sheets beyond the default three leaves 0.075m empty
# for them, and gives elevation and 0.2m colour but no fine detail.
#
# Unlike download_nz.sh, no arguments does not mean everything: the elevation
# source here is the national mosaic, so an unfiltered run would pull all
# 350 GiB of New Zealand. The default is the sheets the 0.075 m survey covers.
#
# Both buckets are public (AWS Open Data) and tiles share Topo50 sheet names,
# either whole (BQ31.tiff, elevation) or subdivided (BQ31_1000_4946.tiff,
# imagery). Downloads are resumable and safe to interrupt: rclone compares size
# and ETag, so truncated tiles are re-fetched instead of being mistaken for
# complete ones.
#
# Requires rclone (sudo apt install rclone). The remote is defined in the
# rclone.conf next to this script, so no AWS credentials and no configuration
# in your home directory are needed.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SHEETS=("$@")

if ((${#SHEETS[@]} == 0)); then
    SHEETS=(BP31 BQ31 BQ32) # everything the 0.075 m survey reaches
fi

CONFIG="$DIR/rclone.conf"

# <level> <bucket path>, finest first.
HEIGHT_LEVELS=(
    "1m      nz:nz-elevation/new-zealand/new-zealand/dem_1m/2193"
)
ALBEDO_LEVELS=(
    "0.075m  nz:nz-imagery/wellington/wellington_2025_0.075m/rgb/2193"
    "0.2m    nz:nz-imagery/wellington/wellington_2025_0.2m/rgb/2193"
)

command -v rclone >/dev/null || {
    echo "rclone not found. Install it with: sudo apt install rclone" >&2
    exit 1
}

filters() { # restrict to .tiff, and to the requested Topo50 sheets
    local sheet
    for sheet in "${SHEETS[@]}"; do
        printf '%s\n' --include "$sheet.tiff" --include "${sheet}_*.tiff"
    done
}

fetch() { # <src> <dest>
    local src=$1 dest=$2 filter
    mkdir -p "$dest"
    mapfile -t filter < <(filters)
    # copy, never sync: a filtered run must not delete sheets fetched earlier.
    rclone --config "$CONFIG" copy "$src" "$dest" "${filter[@]}" \
        --transfers 8 --checkers 16 --retries 3 --progress
    echo "$dest: $(find "$dest" -name '*.tiff' | wc -l) tiles"
}

fetch_levels() { # <attachment> <level spec>...
    local attachment=$1 spec level src
    shift
    for spec in "$@"; do
        read -r level src <<<"$spec"
        echo "== $attachment $level"
        fetch "$src" "$DIR/source_data/wellington/$attachment/$level"
    done
}

fetch_levels height "${HEIGHT_LEVELS[@]}"
fetch_levels albedo "${ALBEDO_LEVELS[@]}"
