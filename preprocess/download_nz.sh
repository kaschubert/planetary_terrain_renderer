#!/usr/bin/env bash
# Download LINZ open data (https://github.com/linz/imagery, https://github.com/linz/elevation)
# for preprocessing with the preprocess_nz example.
#
#   ./download_nz.sh              # all of New Zealand (~14 GiB)
#   ./download_nz.sh BQ31 BQ32    # only the given Topo50 sheets (e.g. Wellington)
#
# Tiles are stored in preprocess/source_data/nz/{height,albedo}.
#
# Imagery:   10 m national satellite mosaic, RGBA COGs, EPSG:2193
# Elevation: 8 m contour-derived national DEM, Float32 COGs, EPSG:2193
#
# Both buckets are public (AWS Open Data) and tiles share Topo50 sheet names.
# Downloads are resumable and safe to interrupt: rclone compares size and ETag,
# so truncated tiles are re-fetched instead of being mistaken for complete ones.
#
# Requires rclone (sudo apt install rclone). The remote is defined in the
# rclone.conf next to this script, so no AWS credentials and no configuration
# in your home directory are needed.
#
# Each level directory gets a manifest.ron recording the bucket path its tiles came from,
# the survey and its capture date, the sheets on disk and the tile count. The preprocessor
# carries it into the terrain, which otherwise has no way to say what it is showing. Pass
# --manifest-only to write manifests for tiles already downloaded, without fetching.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"

source "$DIR/manifest.sh"

MANIFEST_ONLY=0

if [[ ${1-} == --manifest-only ]]; then
    MANIFEST_ONLY=1
    shift
fi

SHEETS=("$@")

CONFIG="$DIR/rclone.conf"
IMAGERY="nz:nz-imagery/new-zealand/new-zealand_2024-2025_10m/rgb/2193"
ELEVATION="nz:nz-elevation/new-zealand/new-zealand-contour/dem_8m/2193"

command -v rclone >/dev/null || {
    echo "rclone not found. Install it with: sudo apt install rclone" >&2
    exit 1
}

filters() { # restrict to .tiff, and to the requested Topo50 sheets if any
    if ((${#SHEETS[@]} == 0)); then
        printf '%s\n' --include '*.tiff'
    else
        local sheet
        for sheet in "${SHEETS[@]}"; do
            printf '%s\n' --include "$sheet.tiff" --include "${sheet}_*.tiff"
        done
    fi
}

fetch() { # <src> <dest> <attachment> [level]
    local src=$1 dest=$2 attachment=$3 level=${4-} filter
    mkdir -p "$dest"

    if ((MANIFEST_ONLY)); then
        # Backfilling a directory that is already downloaded. Every figure in a manifest is
        # read off the disk, so there is nothing to fetch to write one.
        write_manifest "$dest" "$src" "$attachment" "$level"
        return
    fi

    mapfile -t filter < <(filters)
    # copy, never sync: a filtered run must not delete sheets fetched earlier.
    rclone --config "$CONFIG" copy "$src" "$dest" "${filter[@]}" \
        --transfers 8 --checkers 16 --retries 3 --progress
    write_manifest "$dest" "$src" "$attachment" "$level"
}

fetch "$ELEVATION" "$DIR/source_data/nz/height" height
fetch "$IMAGERY" "$DIR/source_data/nz/albedo" albedo
