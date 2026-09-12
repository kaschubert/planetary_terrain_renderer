#!/usr/bin/env bash
# Download the LINZ open data (https://github.com/linz/imagery,
# https://github.com/linz/elevation) that beats what download_nz.sh gets for Auckland,
# one directory per resolution level.
#
#   ./download_auckland.sh              # the central Auckland sheet (~84 GiB)
#   ./download_auckland.sh BA31 BA32    # only the given Topo50 sheets
#
# download_nz.sh covers the whole country at 10 m imagery and an 8 m contour-derived DEM.
# Everything below is finer than that, and each level goes in its own directory:
#
#   source_data/auckland/height/1m       LiDAR DEM       1 tile    0.4 GiB
#   source_data/auckland/albedo/0.075m   aerial 2024  1141 tiles  80.5 GiB
#   source_data/auckland/albedo/0.5m     aerial 2010    20 tiles   3.1 GiB
#
# Tile counts and sizes are for the default sheet. As with Wellington, the two albedo
# levels are complementary and preprocess_auckland feeds both to one attachment: the
# 0.075 m survey is the finest published but only reaches 394 km2 of BA32's 864, while the
# 0.5 m mosaic reaches 691. Without the coarse level the rest of the sheet renders with
# elevation and no colour. The 0.5 m imagery is from 2010-2012, so it will not match the
# 2024 colour where they meet; it is filling gaps, not competing.
#
# Auckland is much larger than Wellington: all 22 sheets the 2024 survey covers come to
# 1.3 TiB at 0.075 m, so the default is the single sheet holding the city centre. The
# neighbours are big too - BA31 is 185 GiB, AZ31 181, BB32 160 - so add them one at a
# time and watch the disk.
#
# Note that auckland_2024_0.25m, the obvious region-wide filler, does NOT cover BA32. It
# spans AZ30, AZ31, BA30, BA31, BB30, BB31 and BB32 only, which is why the 0.5 m mosaic is
# used here instead.
#
# Both buckets are public (AWS Open Data) and tiles share Topo50 sheet names, either whole
# (BA32.tiff, elevation) or subdivided (BA32_1000_0101.tiff, imagery). Downloads are
# resumable and safe to interrupt: rclone compares size and ETag, so truncated tiles are
# re-fetched instead of being mistaken for complete ones.
#
# Requires rclone (sudo apt install rclone). The remote is defined in the rclone.conf next
# to this script, so no AWS credentials and no configuration in your home directory are
# needed.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SHEETS=("$@")

if ((${#SHEETS[@]} == 0)); then
    SHEETS=(BA32) # the sheet holding the Auckland city centre
fi

CONFIG="$DIR/rclone.conf"

# <level> <bucket path>, finest first.
HEIGHT_LEVELS=(
    "1m      nz:nz-elevation/new-zealand/new-zealand/dem_1m/2193"
)
ALBEDO_LEVELS=(
    "0.075m  nz:nz-imagery/auckland/auckland_2024_0.075m/rgb/2193"
    "0.5m    nz:nz-imagery/auckland/auckland_2010-2012_0.5m/rgb/2193"
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
        fetch "$src" "$DIR/source_data/auckland/$attachment/$level"
    done
}

fetch_levels height "${HEIGHT_LEVELS[@]}"
fetch_levels albedo "${ALBEDO_LEVELS[@]}"
