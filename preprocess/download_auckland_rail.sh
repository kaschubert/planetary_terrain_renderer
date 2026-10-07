#!/usr/bin/env bash
# Extract Auckland's passenger rail lines from Auckland Transport's GTFS feed, for the
# spherical example to draw over the city.
#
#   ./download_auckland_rail.sh
#
# Writes examples/spherical/plugins/auckland_rail/auckland_rail.csv: one row per point, grouped by line and in
# order along it. Unlike the terrain data this is small enough to commit, so the example
# needs no download of its own and this only has to be rerun when the network changes.
#
# The example edits that file in place and marks it with an "Edited by hand" comment when it
# does. Once the file carries that mark this writes auckland_rail.gtfs.csv beside it instead,
# so a fresh feed can be diffed against the edits rather than destroying them.
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
#
# Also writes auckland_stations.csv beside it: the train stations and their platforms, for
# the labels on the map and for turning the realtime feed's stop ids into names. A station
# is a parent stop named "... Train Station" and its platforms are the stops under it; the
# name goes last in the row so it may hold a comma. That file is never edited by hand, so it
# is written every time.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$DIR/../examples/spherical/plugins/auckland_rail/auckland_rail.csv"
STATIONS_OUT="$DIR/../examples/spherical/plugins/auckland_rail/auckland_stations.csv"
FEED="https://gtfs.at.govt.nz/gtfs.zip"

# The edits are the valuable part; the feed can always be fetched again.
if [ -f "$OUT" ] && grep -q '^# Edited by hand' "$OUT"; then
    GTFS_OUT="${OUT%.csv}.gtfs.csv"
    echo "== $OUT has been edited by hand in the spherical example"
    echo "== writing $GTFS_OUT beside it instead, to diff against the edits"
    OUT="$GTFS_OUT"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== fetching $FEED"
curl -sSL --fail -o "$WORK/gtfs.zip" "$FEED"
unzip -q "$WORK/gtfs.zip" feed_info.txt routes.txt trips.txt shapes.txt stops.txt -d "$WORK"

python3 - "$WORK" "$OUT" "$FEED" "$(basename "$0")" "$STATIONS_OUT" <<'PYTHON_EOF'
import csv
import math
import sys
from collections import Counter
from datetime import date

work, out, feed, script, stations_out = sys.argv[1:6]

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
    # Every point starts on the ground with no offset, and nothing is known about the terrain
    # under it until the example has sampled and saved it. Blank means not known, not zero.
    file.write("line,latitude,longitude,mode,height,terrain\n")
    for route, _, _, _, kept in lines:
        for lat, lon in kept:
            file.write(f"{route['route_short_name']},{lat:.5f},{lon:.5f},ground,0.0,\n")

for route, shape, sign, count, kept in lines:
    print(f"{route['route_short_name']}: {count} points to {len(kept)}, {sign}")
print(f"wrote {out}")

# The stations: the parent stops named as stations, which leaves out Te Huia's Waikato
# stations and the bus stops beside stations, and the platforms under them, which are what
# a trip update names. Each station is followed by its platforms.
stops = list(rows("stops.txt"))
stations = {
    s["stop_id"]: s
    for s in stops
    if s["location_type"] == "1" and s["stop_name"].endswith(" Train Station")
}
platforms = [s for s in stops if s["location_type"] == "0" and s.get("parent_station") in stations]

def stop_row(stop, parent):
    return (
        f"{stop['stop_id']},{parent},{stop['stop_code']},"
        f"{float(stop['stop_lat']):.5f},{float(stop['stop_lon']):.5f},{stop['stop_name']}\n"
    )

with open(stations_out, "w", encoding="utf-8", newline="\n") as file:
    file.write(f"# Auckland's train stations and their platforms, from Auckland Transport's GTFS feed {feed}\n")
    file.write(f"# Written by preprocess/{script} on {date.today().isoformat()}, feed version {version}\n")
    file.write(
        f"# {len(stations)} stations, each followed by its platforms, {len(platforms)} platforms in all.\n"
        "# A platform's parent is its station's stop_id; a station's is blank. The name is last so it may hold a comma.\n"
    )
    file.write("stop_id,parent,code,latitude,longitude,name\n")
    for station in sorted(stations.values(), key=lambda s: s["stop_name"]):
        file.write(stop_row(station, ""))
        under = [p for p in platforms if p["parent_station"] == station["stop_id"]]
        for platform in sorted(under, key=lambda p: p["stop_id"]):
            file.write(stop_row(platform, station["stop_id"]))
print(f"{len(stations)} stations with {len(platforms)} platforms, wrote {stations_out}")
PYTHON_EOF
