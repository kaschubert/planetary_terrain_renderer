#!/usr/bin/env bash
# Download the LINZ open data (https://github.com/linz/imagery,
# https://github.com/linz/elevation) for West and South Auckland, the sheets either side
# of the one download_auckland.sh covers.
#
#   ./download_auckland_west_south.sh         # both sheets (~27 GiB)
#   ./download_auckland_west_south.sh BA31    # only the given Topo50 sheets
#
# BA31 is West Auckland, from the Waitakere coast across Henderson to the harbour. BB32 is
# South Auckland, Manukau through Papakura. BB31, the southwest corner between them, is not
# included by default but works as an argument.
#
#   source_data/auckland_west_south/height/1m      LiDAR DEM    2 tiles   2.4 GiB
#   source_data/auckland_west_south/albedo/0.25m   aerial 2024 80 tiles  14.3 GiB
#   source_data/auckland_west_south/albedo/0.5m    aerial 2010 50 tiles   9.9 GiB
#
# There is no 0.075 m level here, which is the whole reason this is a separate script.
# The survey covers these sheets - 198 GiB for BA31 and 172 for BB32 - but preprocessing
# resamples onto a grid of around 0.6 m either way, so those 370 GiB would buy nothing
# the 0.5 m mosaic does not already give, at more than twice the disk this machine has
# free. The city centre sheet keeps its 0.075 m because it is one sheet, not three.
#
# The two albedo levels are complementary and preprocess_auckland_west_south feeds both to
# one attachment, coarser first so the finer wins where they overlap. The 2010 mosaic
# covers both sheets completely; the 2024 survey adds current colour over 639 km2 of
# BA31's 864 but only 52 km2 of BB32, so most of South Auckland renders from the older
# imagery. Expect a visible seam where the two eras meet.
#
# The 0.25 m survey needs colour matching against the 0.075 m one the centre sheet uses,
# which is a separate step because it is a property of those two surveys rather than of the
# download. Run it once after this, before preprocessing:
#
#   ./colour_match.sh source_data/auckland_west_south/albedo/0.25m 1.012 0.942 0.823
#
# Both buckets are public (AWS Open Data) and tiles share Topo50 sheet names, either whole
# (BA31.tiff, elevation) or subdivided (BA31_5000_0101.tiff, imagery). Downloads are
# resumable and safe to interrupt: rclone compares size and ETag, so truncated tiles are
# re-fetched instead of being mistaken for complete ones.
#
# Requires rclone (sudo apt install rclone). The remote is defined in the rclone.conf next
# to this script, so no AWS credentials and no configuration in your home directory are
# needed.
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

if ((${#SHEETS[@]} == 0)); then
    SHEETS=(BA31 BB32) # west, south
fi

CONFIG="$DIR/rclone.conf"

# <level> <bucket path>, finest first.
HEIGHT_LEVELS=(
    "1m      nz:nz-elevation/new-zealand/new-zealand/dem_1m/2193"
)
ALBEDO_LEVELS=(
    "0.25m   nz:nz-imagery/auckland/auckland_2024_0.25m/rgb/2193"
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

fetch_levels() { # <attachment> <level spec>...
    local attachment=$1 spec level src
    shift
    for spec in "$@"; do
        read -r level src <<<"$spec"
        echo "== $attachment $level"
        fetch "$src" "$DIR/source_data/auckland_west_south/$attachment/$level" \
            "$attachment" "$level"
    done
}

fetch_levels height "${HEIGHT_LEVELS[@]}"
fetch_levels albedo "${ALBEDO_LEVELS[@]}"
