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
# Both buckets are public (AWS Open Data), tiles share Topo50 sheet names,
# and downloads are resumable: existing non-empty files are skipped.
set -euo pipefail

IMAGERY_BASE="https://nz-imagery.s3.ap-southeast-2.amazonaws.com"
IMAGERY_PREFIX="new-zealand/new-zealand_2024-2025_10m/rgb/2193/"
ELEVATION_BASE="https://nz-elevation.s3.ap-southeast-2.amazonaws.com"
ELEVATION_PREFIX="new-zealand/new-zealand-contour/dem_8m/2193/"

DIR="$(cd "$(dirname "$0")" && pwd)"
SHEETS=("$@")

list_keys() { # <base> <prefix> — paginated listing of all .tiff keys
    local base=$1 prefix=$2 after="" page
    while :; do
        page=$(curl -sf "$base/?list-type=2&prefix=$prefix${after:+&start-after=$after}")
        grep -oE '<Key>[^<]+\.tiff</Key>' <<<"$page" | sed -E 's|</?Key>||g' || true
        [[ $page == *'<IsTruncated>true</IsTruncated>'* ]] || break
        after=$(grep -oE '<Key>[^<]+</Key>' <<<"$page" | tail -1 | sed -E 's|</?Key>||g')
    done
}

filter_sheets() {
    if ((${#SHEETS[@]} == 0)); then
        cat
    else
        local pattern
        pattern=$(printf '/%s|' "${SHEETS[@]}")
        grep -E "(${pattern%|})[._]" || true
    fi
}

fetch() { # <base> <prefix> <dest>
    local base=$1 prefix=$2 dest=$3
    mkdir -p "$dest"
    list_keys "$base" "$prefix" | filter_sheets |
        xargs -P 8 -I{} bash -c '
            file="$2/$(basename "$1")"
            if [[ ! -s $file ]]; then
                curl -sf --retry 3 -o "$file" "$0/$1" && echo "$(basename "$file")"
            fi' "$base" {} "$dest"
    echo "$dest: $(ls "$dest" | wc -l) tiles"
}

fetch "$ELEVATION_BASE" "$ELEVATION_PREFIX" "$DIR/source_data/nz/height"
fetch "$IMAGERY_BASE" "$IMAGERY_PREFIX" "$DIR/source_data/nz/albedo"
