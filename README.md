# Planetary Terrain Renderer

![Screenshot 2025-04-26 at 15 13 40](https://github.com/user-attachments/assets/b3101705-20fc-4b6a-abfd-98e2a120b6f3)

A large-scale planetary terrain renderer written in Rust using the Bevy game engine.

This project is developed by [Kurt Kühnert](https://github.com/kurtkuehnert) and contains the reference implementation of my [Master Thesis](https://doi.org/10.60687/2025-0147).
This terrain renderer focuses on visualizing large-scale terrains in a seamless, continuous, and efficient manner.
The source code was developed as the open-source plugin **[bevy_terrain](https://github.com/kurtkuehnert/bevy_terrain)** for the Bevy game engine.

This [Video](https://youtu.be/zdz3-77-3EI) showcases the capabilities and features of this terrain renderer.

## Abstract

Realtime rendering of virtual globes represents the pinnacle of largescale terrain
rendering. Modeling the surface of an entire planet has vast applications, ranging
from Geographic Information Systems (GIS) to educational software. However, the
immense scale of planetary terrain introduces significant challenges, including
levelofdetail (LOD) management and numerical precision limitations. This thesis
provides an overview of the fundamental challenges in planetary terrain rendering
and examines existing solutions. Building on this foundation, a comprehensive
framework for planetary terrain rendering is presented, supporting terrains on an
ellipsoidal base shape, which accurately represents the true spheroid form of planets,
such as the WGS84 reference ellipsoid. The framework covers key aspects such
as viewdependent terrain geometry management, terrain data streaming, and an
accurate spatial reference system (SRS) that integrates seamlessly with the quadtree
based subdivision of terrain geometry and data. A novel approach to maintaining high
precision despite the limitations of floatingpoint accuracy on the Graphics Process
ing Unit (GPU) is introduced. This method leverages a Taylor series approximation
to compute positions on the ellipsoidal surface relative to the viewer. Additionally, a
hierarchical system of coordinate transformations is proposed to accurately represent
terrain positions at various scales. A crucial feature of any virtual globe framework is
its ability to render multiple localized datasets on top of the planetary surface. This
thesis presents a method for achieving this, supported by a preprocessing pipeline
that converts arbitrary georeferenced raster files into datasets compatible with the
rendering system. An extensive opensource reference implementation is provided,
and the framework is evaluated using multiple datasets.

## Screenshots

![10](https://github.com/user-attachments/assets/da19c3b7-dad4-40f1-a94c-f4d987017ca2)
![Screenshot 2025-04-26 at 15 19 18](https://github.com/user-attachments/assets/5b92bb08-ecce-4194-beca-ff7a5b35ade4)
![11](https://github.com/user-attachments/assets/cc82078b-677c-4c2a-8ddf-5f1d2f444882)

## Examples

To try out the terrain renderer, you first have to preprocess your dataset (GeoTIFF).
Some example datasets are available [here](https://drive.proton.me/urls/ZRDAC9SWTM#IxwKkKWSBgnV).
Use the preprocess CLI or a prepared configuration in the `preprocess/examples` directory.
Then run the `examples/spherical/spherical.rs` demo with the preprocessed dataset selected.
The default path for the datasets is `source_data`.

## Run exclusively on Vulkan

```sh
WGPU_BACKEND=vulkan cargo run --example spherical
```

## Debug Controls

These are the debug controls of the plugin.
Use them to navigate the terrain, experiment with the quality settings, and enter the different debug views.
There are two camera controller options available: a fly camera for navigating using the keyboard and an orbital camera
using only the mouse.

### Fly Camera

- `T` - toggle fly camera movement
- Move the mouse to look around
- Press the arrow keys to move the camera horizontally
- Use `PageUp` and `PageDown` to move the camera vertically
- Use `Home` and `End` to increase/decrease the camera's movement speed

### Orbital Camera

- `R` - toggle orbital camera movement
- Hold the left mouse button to pan the camera
- Hold the right mouse button to rotate the camera: it goes the way you drag, right round to the right of the point under the cursor and up to look down on it
- Hold the middle mouse button and drag down to zoom out or up to zoom in, or turn the mouse wheel; both zoom towards the point under the cursor

### Visualization Toggles

- `F1` - toggle this list of controls inside the app
- `F2` - toggle the table of where each terrain's data came from
- `F3` - toggle the Topo50 sheet grid over the terrain, coloured by the finest imagery downloaded, as does `show grid` in the F2 panel
- Mouse wheel - raise or lower the sheet grid, once `wheel sets height` is ticked in the F2 panel, which takes the wheel from the camera's zoom while ticked
- `F4` - toggle Auckland's rail lines over the city, one colour per line
- `F9` - toggle the station names over the rail lines, each anchored to the track at its station; F4 hides them with the lines
- `W` - toggle wireframe view
- `L` - toggle terrain data LOD view
- `Y` - toggle terrain geometry LOD view
- `Q` - toggle tile tree view
- `P` - toggle pixel view
- `U` - toggle UV view
- `B` - toggle normals view
- `M` - toggle morphing
- `K` - toggle blending
- `Z` - toggle tile tree LOD
- `S` - toggle lighting
- `G` - toggle texture sampling using gradients
- `H` - toggle high precision coordinates
- `F` - toggle freeze view frustum
- Hold `Ctrl` - detach the culling camera: the terrain keeps loading and culling for the pose you had, while you fly off to look at it from outside; the letter toggles stand aside while it is held, so Ctrl+S, Ctrl+Z and Ctrl+Y reach the rail editor, at the cost of two lines on the console per chord
- `D` - toggle surface approximation debug

### Rail Editor

While editing, every point draws as a disc on its line: the line's colour for a ground point, amber for a fixed point and grey for a between point, and white when selected. A stretch between two anchors, a tunnel or a bridge, draws dashed whether editing or not, and while editing a stretch of ground steeper than 3.5 per cent, which rail does not climb, draws red to say a tunnel or a bridge is wanted there. The panel in the bottom right corner reads out the point last clicked: its line and index, latitude and longitude, the terrain under it, its height and what that means for its mode, and the grade of the two segments it joins, marked where steeper than rail climbs. Under the readout are the panel's buttons; one that cannot apply is dimmed, and while F4 hides the lines every button but `save` is, as the mouse and the keys are inert then. The gizmo stands on the point last clicked: a drag along the ground resamples the terrain under a ground point as it goes, and a lift changes a ground point's offset or a fixed point's height. The frames F6 shows are what the track models will be placed on: a transform every 25 m along a spline through the points, facing along the line and standing up from the ground, so a kink or a lean in them is a point to move.

- `F5` - toggle the rail editor panel and editing: the discs draw, the mouse works on them, and the panel shows
- `F6` - toggle the track frames: an arrow every 25 m along the line, a white tick up, a grey sleeper across, and the two running lines either side
- `ground` / `fixed` / `between` - give the selection that mode: ground and fixed keep the point where it is, as an offset from the terrain under it or as a height of its own; between drops it onto the chord between its anchors
- `drop to ground` - put the selection back on the terrain with no offset
- `span selection` - make a tunnel or a bridge of three or more selected points on a line, its ends fixed where they are and the inside between
- `select steep run` - grow the selection over the ground steeper than rail climbs either side of it, stopping at a stretch already spanned
- `undo` / `redo` / `save` - what the keys below do
- `prev` / `next` - step through the points the startup sampling found the ground moved under by more than a metre since the file was saved, selecting each and flying the camera to it
- `Click` a disc - select that point; a click on the terrain leaves the selection as it is
- `Shift`+click a disc on the same line - select the run of points from the last one clicked to it, which is how a tunnel is selected: click one portal, `Shift`+click the other; on another line it adds the point
- `Double-click` the terrain beside a line - add a point to it there, on the ground
- `Drag` the gizmo - move the selection: red arrow east, green up, blue south; green square along the ground, red and blue squares in a vertical wall; any lift turns a between point fixed
- `Delete` - remove the selected points, keeping at least two on a line
- `Escape` - clear the selection
- `Ctrl+Z` / `Ctrl+Y` - undo and redo, up to 200 edits back
- `Ctrl+S` - save the rail lines, edits and all, back to examples/spherical/plugins/auckland_rail/auckland_rail.csv, editor on or off, as the panel's `save` button does; a toast beside the panel says how many points were saved, or why not

### Trains

One carriage per line drives itself along the drawn track, end to end and back at 72 km/h on the left-hand running line, with its line, its unit and its speed above it, as a stand-in while there is no live feed. With `AT_API_KEY` set in the environment, a free key from a subscription on dev-portal.at.govt.nz, the carriages instead stand where Auckland Transport's realtime feed last reported each train on a trip, fetched every 10 seconds and run on between fetches at the speed each reported, and with each train's next stop and how late it runs from the trip updates feed, fetched every 30 seconds; a caption above the table says how many trains the feed has and how long ago it was fetched, or why there is no feed. The two feeds together are 480 calls an hour against the key's 35,000 a week, about 70 hours of running. A table in the bottom left corner lists the trains, each with its unit where the feed names one, how far along its line it is in kilometres, which way it is running, `>` in the file's point order and `<` back, its speed, and its next stop and how late it runs where the feed has said, and a camera icon on every row that puts a chase camera behind that train, 60 m back and 25 m up, looking a little ahead of it. While it rides, the mouse moves the camera round the train rather than the world.

- `F7` - toggle the trains, and the table of them
- `F8` - toggle the live feed: on, one carriage per train Auckland Transport reports, where it reports it; off, the stand-ins; nothing without `AT_API_KEY`
- Camera icon in the trains table - follow that train with a chase camera, from behind, or at the zoom you had when switching from another train; click it again, press `Escape`, `T`, `R` or a fly key, or hide the trains to let go
- Drag with the left or right button while following - orbit the camera round the train, right to go round its right and up to look down on it
- Mouse wheel, or drag with the middle button, while following - zoom in and out, no nearer than 20 m and as far as you like

### Quality Adjustments

- `N` - decrease blend distance
- `E` - increase blend distance
- `I` - decrease morph distance
- `O` - increase morph distance
- `X` - decrease grid size
- `J` - increase grid size

## GPU Frame Capture (macOS)

When enabling the `metal_capture` feature, you can trigger a GPU frame capture using the `C` key.
Recorded captures are stored in the `captures` directory of the project.
They can be examined and analyzed using Xcode.

## Attribution

The examples use the following [demo datasets](https://drive.proton.me/urls/ZRDAC9SWTM#IxwKkKWSBgnV):

- GEBCO Compilation Group (2023) - GEBCO 2023 Grid
- Unearthed Outdoors - True Marble Global Image Dataset GeoTIFF - [Creative Commons Attribution 3.0 United States
  License](https://creativecommons.org/licenses/by/3.0/us/legalcode)
- ©swisstopo - swissALTIRegio
- This work utilizes data made available under the Norwegian Licence for Open Government Data (NLOD), distributed by the
  Norwegian Offshore Directorate. The data were originally acquired by various entities. For more information on the
  data,
  please visit the Norwegian Offshore Directorate's open data page:
  https://www.sodir.no/en/about-us/open-data/.

## License

Planetary Terrain Renderer source code is dual-licensed under either:

* MIT License (LICENSE-MIT or http://opensource.org/licenses/MIT)
* Apache License, Version 2.0 (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

The Thesis.pdf is excluded from both of these and is licensed under
the [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) license instead.
