#!/usr/bin/env bash
# Extract Auckland's passenger rail lines from Auckland Transport's GTFS feed, for the
# spherical example to draw over the city.
#
#   ./download_auckland_rail.sh
#
# Writes examples/spherical/auckland_rail.csv: one row per point, grouped by line and in
# order along it. Unlike the terrain data this is small enough to commit, so the example
# needs no download of its own and this only has to be rerun when the network changes.
#
# The feed (https://gtfs.at.govt.nz/gtfs.zip, about 30 MB, no key) carries every bus,
# train and ferry service. The trains are the routes with route_type 2 run by agency AM,
# Auckland Metro, which leaves out Te Huia to Hamilton. A route has many shapes - short
# workings, depot runs, the odd diversion - so each line is drawn with the shape the most
# trips use, which is the full run end to end. Since the City Rail Link opened in September
# 2026 that is three lines: E-W Swanson to Manukau, S-C Pukekohe round the city loop, and
# O-W Onehunga to Henderson.
#
# The shapes are dense, a point every few metres, which is far more than a line on the
# terrain can show. They are simplified to within a metre, which leaves a few hundred
# points per line.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$DIR/../examples/spherical/auckland_rail.csv"
FEED="https://gtfs.at.govt.nz/gtfs.zip"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== fetching $FEED"
curl -sSL --fail -o "$WORK/gtfs.zip" "$FEED"
unzip -q "$WORK/gtfs.zip" feed_info.txt routes.txt trips.txt shapes.txt -d "$WORK"

python3 - "$WORK" "$OUT" "$FEED" "$(basename "$0")" <<'PYTHON_EOF'
import csv
import math
import sys
from collections import Counter
from datetime import date

work, out, feed, script = sys.argv[1:5]

def rows(name):
    with open(f"{work}/{name}", encoding="utf-8-sig", newline="") as file:
        yield from csv.DictReader(file)

version = next(rows("feed_info.txt"))["feed_version"]

# route_type 2 is rail. AM is Auckland Metro; the other rail agency is Waikato Regional
# Council's Te Huia.
routes = {
    r["route_id"]: r
    for r in rows("routes.txt")
    if r["route_type"] == "2" and r["agency_id"] == "AM"
}

# The shape the most trips use, per route.
usage = Counter()
headsign = {}
for trip in rows("trips.txt"):
    if trip["route_id"] in routes and trip["shape_id"]:
        key = (trip["route_id"], trip["shape_id"])
        usage[key] += 1
        headsign[key] = trip["trip_headsign"]

chosen = {}
for route in routes:
    (_, shape), _ = max(
        ((key, n) for key, n in usage.items() if key[0] == route), key=lambda item: item[1]
    )
    chosen[shape] = route

points = {shape: [] for shape in chosen}
for point in rows("shapes.txt"):
    if point["shape_id"] in points:
        points[point["shape_id"]].append(
            (int(point["shape_pt_sequence"]), float(point["shape_pt_lat"]), float(point["shape_pt_lon"]))
        )

def simplify(line, tolerance):
    """Douglas-Peucker, in metres on a local flat approximation. Over a 50 km line the
    approximation is off by well under the tolerance."""
    scale = math.cos(math.radians(line[0][0])) * 111_320.0
    def metres(p):
        return (p[1] * scale, p[0] * 111_320.0)

    def distance(p, a, b):
        (px, py), (ax, ay), (bx, by) = metres(p), metres(a), metres(b)
        dx, dy = bx - ax, by - ay
        length2 = dx * dx + dy * dy
        if length2 == 0.0:
            return math.hypot(px - ax, py - ay)
        t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / length2))
        return math.hypot(px - ax - t * dx, py - ay - t * dy)

    keep = [False] * len(line)
    keep[0] = keep[-1] = True
    stack = [(0, len(line) - 1)]
    while stack:
        first, last = stack.pop()
        if last <= first + 1:
            continue
        index, worst = max(
            ((i, distance(line[i], line[first], line[last])) for i in range(first + 1, last)),
            key=lambda item: item[1],
        )
        if worst > tolerance:
            keep[index] = True
            stack.append((first, index))
            stack.append((index, last))
    return [p for p, k in zip(line, keep) if k]

lines = []
for shape, route in sorted(chosen.items(), key=lambda item: routes[item[1]]["route_short_name"]):
    raw = [(lat, lon) for _, lat, lon in sorted(points[shape])]
    # The feed repeats points where a shape pauses at a stop.
    deduplicated = [p for i, p in enumerate(raw) if i == 0 or p != raw[i - 1]]
    kept = simplify(deduplicated, 1.0)
    lines.append((routes[route], shape, headsign[(route, shape)], len(raw), kept))

with open(out, "w", encoding="utf-8", newline="\n") as file:
    file.write(f"# Auckland passenger rail lines, from Auckland Transport's GTFS feed {feed}\n")
    file.write(f"# Written by preprocess/{script} on {date.today().isoformat()}, feed version {version}\n")
    file.write("# One shape per line, the one the most trips use, simplified to within a metre:\n")
    for route, shape, sign, count, kept in lines:
        file.write(
            f"#   {route['route_short_name']}  colour {route['route_color']}  "
            f"shape {shape}  {count} points to {len(kept)}  \"{sign}\"\n"
        )
    file.write("line,latitude,longitude\n")
    for route, _, _, _, kept in lines:
        for lat, lon in kept:
            file.write(f"{route['route_short_name']},{lat:.5f},{lon:.5f}\n")

for route, shape, sign, count, kept in lines:
    print(f"{route['route_short_name']}: {count} points to {len(kept)}, {sign}")
print(f"wrote {out}")
PYTHON_EOF
