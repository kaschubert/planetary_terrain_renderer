#!/usr/bin/env bash
# Build a colour matched virtual raster over a directory of tiles.
#
#   ./colour_match.sh <dir> <red gain> <green gain> <blue gain> [reference dataset]
#
# Writes <dir>.vrt next to the directory. The gains multiply each band, alpha is left
# alone, and nothing is resampled or copied: a vrt is a description, so this costs no disk
# and takes no time. Point a preprocess example at the .vrt instead of the directory to
# use it.
#
# Separate aerial surveys are balanced differently, and two of them next to each other in
# one terrain show it as a seam. The gains that fix it are not guessable - derive them by
# comparing the two surveys against imagery that covers both, so that differences in land
# cover cancel and only the calibration is left. For LINZ auckland_2024_0.25m matched to
# auckland_2024_0.075m, using the 2010-2012 mosaic as the common reference, that is:
#
#   ./colour_match.sh source_data/auckland_west_south/albedo/0.25m 1.012 0.942 0.823
#
# which is mostly a blue correction: that survey runs blue heavy enough to read as a
# different season.
#
# It also writes <dir>.vrt.manifest.ron, by extending the manifest the download left in
# <dir>. The gains are the one part of this that came from nowhere - no bucket publishes
# them, and nothing recovers them from the vrt short of reading its XML - so without that
# file a terrain built from the vrt could say which survey it is showing but not that its
# colour had been altered. The optional fifth argument names the dataset the gains were
# measured against.
#
# Requires gdal (gdalbuildvrt).
set -euo pipefail

if (($# < 4 || $# > 5)); then
    sed -n '2,4p' "$0" >&2
    exit 1
fi

DIR=$(realpath "$1")
GAINS=("$2" "$3" "$4")
REFERENCE=${5-}

command -v gdalbuildvrt >/dev/null || {
    echo "gdalbuildvrt not found. Install it with: sudo apt install gdal-bin" >&2
    exit 1
}

[[ -d $DIR ]] || {
    echo "not a directory: $DIR" >&2
    exit 1
}

mapfile -t tiles < <(find "$DIR" -name '*.tiff' | sort)
((${#tiles[@]})) || {
    echo "no .tiff files in $DIR" >&2
    exit 1
}

vrt="$DIR.vrt"
list=$(mktemp)
trap 'rm -f "$list"' EXIT
printf '%s\n' "${tiles[@]}" >"$list"

gdalbuildvrt -q -overwrite -input_file_list "$list" "$vrt"

# The gain goes into the vrt itself, as a ScaleRatio on every source of the bands that get
# one. gdal_translate could apply the scaling instead, but only by writing a second vrt
# that reads the first, leaving two files that have to travel together.
python3 - "$vrt" "${GAINS[@]}" <<'PYTHON_EOF'
import re
import sys

path, gains = sys.argv[1], [float(gain) for gain in sys.argv[2:]]


def scale_band(band):
    """Give every source of this band its gain. A complex source computes
    offset + raw * ratio, and gdalbuildvrt writes complex sources throughout."""
    index = int(re.search(r'band="(\d+)"', band.group()).group(1))

    # Bands past the gains given - the alpha band here - are left alone.
    if index > len(gains):
        return band.group()

    ratio = f"<ScaleRatio>{gains[index - 1]}</ScaleRatio>"

    return band.group().replace("</ComplexSource>", ratio + "</ComplexSource>")


vrt = open(path).read()
bands = re.compile(r"<VRTRasterBand.*?</VRTRasterBand>", re.S)

open(path, "w").write(bands.sub(scale_band, vrt))
PYTHON_EOF

# Record the gains beside the vrt, on top of what the download recorded for the tiles.
python3 - "$vrt" "$DIR/manifest.ron" "$REFERENCE" "${#tiles[@]}" "${GAINS[@]}" <<'PYTHON_EOF'
import os
import re
import sys

vrt, source, reference, tiles = sys.argv[1:5]
gains = [float(gain) for gain in sys.argv[5:]]

if not os.path.exists(source):
    sys.exit(
        f"no manifest.ron in {os.path.dirname(source)}, so there is nothing to extend.\n"
        "Run the download script that fetched these tiles with --manifest-only first."
    )

# One `key: value,` per line, which is how write_manifest emits it and how ron pretty
# prints it back. Keep the order, override in place, append what is new.
fields, order = {}, []

for line in open(source):
    field = re.fullmatch(r"\s*(\w+): (.*),\s*", line)

    if field:
        fields[field.group(1)] = field.group(2)
        order.append(field.group(1))

# The vrt lists the tiles that existed when it was built, which is the count that matters
# for anything preprocessed through it.
fields["tiles"] = str(tiles)
fields["gains"] = "Some((" + ", ".join(str(gain) for gain in gains) + "))"
fields["script"] = '"colour_match.sh"'

if reference:
    fields["matched_to"] = f'Some("{reference}")'

order += [name for name in ("gains", "matched_to") if name in fields and name not in order]
body = "".join(f"    {name}: {fields[name]},\n" for name in order)

with open(vrt + ".manifest.ron", "w") as out:
    out.write("// Written by colour_match.sh. Regenerated on every run; edits are lost.\n")
    out.write(f"(\n{body})\n")
PYTHON_EOF

echo "$vrt: ${#tiles[@]} tiles, gains R=$2 G=$3 B=$4, manifest written"
