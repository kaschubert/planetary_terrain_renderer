# Shared by the download scripts: record what a level directory holds.
#
# Source this, then call write_manifest after the tiles are in place:
#
#   write_manifest <dest> <bucket path> <attachment> [level]
#
# It writes <dest>/manifest.ron, which the preprocessor reads and carries into the terrain
# it builds. None of this survives preprocessing otherwise: the output tiles carry no
# metadata at all, so a built terrain cannot say which survey it is showing.
#
# The manifest describes the directory as it stands, not what one run fetched. The scripts
# copy rather than sync on purpose, so a later run for another sheet adds to the same
# directory, and every count here is read back off the disk afterwards - the sheet list
# included, from the filenames rather than from the sheets a run was invoked with. Anything
# else would go stale the moment a second run touched the directory.

write_manifest() { # <dest> <src> <attachment> [level]
    python3 - "$1" "$2" "$3" "${4-}" "$(basename "$0")" <<'PYTHON_EOF'
import os
import re
import sys
from datetime import datetime, timezone

dest, src, attachment, level, script = sys.argv[1:6]

# The two buckets disagree on depth but agree on their tail, <dataset>/<product>/<crs>, so
# index from the right.
parts = src.split(":", 1)[-1].split("/")
dataset, product, crs = parts[-3], parts[-2], parts[-1]

# Imagery names itself <region>_<date>_<resolution>, and the date is a year or a range of
# them. Elevation does not: the national 1 m mosaic is stitched from surveys spanning years
# and publishes no single date, so its manifests leave the field out rather than guess.
captured = None
survey = re.fullmatch(r"(.+)_(\d{4}(?:-\d{4})?)_([\d.]+m)", dataset)

if survey:
    captured, resolution = survey.group(2), survey.group(3)
else:
    depth = re.fullmatch(r"dem_([\d.]+m)", product)
    resolution = depth.group(1) if depth else level

sheets, tiles, size = set(), 0, 0

for entry in os.scandir(dest):
    if not entry.name.endswith(".tiff") or entry.name.startswith("._"):
        continue

    # BA31.tiff is a whole sheet, BA31_5000_0101.tiff a piece of one.
    sheets.add(re.split(r"[._]", entry.name, 1)[0])
    tiles += 1
    size += entry.stat().st_size

fields = [
    ("source", f'"{src}"'),
    ("dataset", f'"{dataset}"'),
    ("product", f'"{product}"'),
    ("crs", f'"EPSG:{crs}"'),
    ("attachment", f'"{attachment}"'),
    ("level", f'Some("{level}")' if level else None),
    ("resolution", f'"{resolution}"' if resolution else None),
    ("captured", f'Some("{captured}")' if captured else None),
    ("sheets", "[" + ", ".join(f'"{sheet}"' for sheet in sorted(sheets)) + "]"),
    ("tiles", str(tiles)),
    ("bytes", str(size)),
    ("script", f'"{script}"'),
    ("updated", datetime.now(timezone.utc).strftime('"%Y-%m-%dT%H:%M:%SZ"')),
]

body = "".join(f"    {name}: {value},\n" for name, value in fields if value is not None)

with open(os.path.join(dest, "manifest.ron"), "w") as out:
    out.write(f"// Written by {script}. Regenerated on every run; edits are lost.\n")
    out.write(f"(\n{body})\n")

print(f"{dest}: {tiles} tiles, {len(sheets)} sheets, manifest.ron written")
PYTHON_EOF
}
