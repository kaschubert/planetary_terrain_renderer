//! The discs drawn on the points while editing, one per point near the camera, sized in
//! pixels so they read the same at any zoom, coloured and sized by the point's mode, and
//! white where selected.

use super::{DISC_REACH, EditorCamera, RailEditor, RailEditorGizmos};
use crate::plugins::auckland_rail::{AucklandRail, line_colour};
use crate::plugins::shared::rail_network::PointMode;
use bevy::prelude::*;
use bevy_terrain::prelude::OrbitalCameraController;
use big_space::prelude::Grids;

/// Disc diameters on screen, in pixels, by mode. A ground point's is enough to see and to
/// hit, and small enough not to hide the line between neighbours. A fixed point's is a third
/// larger, so an anchor can be told from across the valley it holds the line over. A between
/// point's is smaller still: it has no height of its own, and is no more than where the
/// chord passes. Selection adds the same to each, so a selected fixed point is still the
/// larger and the kind still reads under the white.
const DISC_PIXELS: f32 = 5.0;
const FIXED_DISC_PIXELS: f32 = 7.0;
const BETWEEN_DISC_PIXELS: f32 = 4.0;
const SELECTION_PIXELS: f32 = 4.0;

/// A fixed point's disc: the amber the plan draws its anchors in, which no line is coloured
/// and which the warning red of a steep stretch is kept away from.
const FIXED_COLOUR: Color = Color::srgb(0.85, 0.51, 0.17);

/// A between point's disc: a mid grey, for a point that holds nothing of its own.
const BETWEEN_COLOUR: Color = Color::srgb(0.55, 0.55, 0.55);

/// Line segments per disc. The default 32 is for circles that fill the screen; at five
/// pixels across twelve shows no corners, and there can be a thousand discs a frame.
const DISC_RESOLUTION: u32 = 12;

/// The diameter in pixels and the colour of a point's disc, by its mode and whether it is
/// selected. A ground point draws in its line's colour, a fixed point in amber and a between
/// point in grey; selected, any of them is white and larger.
pub(super) fn disc_style(mode: PointMode, selected: bool, line: Color) -> (f32, Color) {
    let (pixels, colour) = match mode {
        PointMode::Ground => (DISC_PIXELS, line),
        PointMode::Fixed => (FIXED_DISC_PIXELS, FIXED_COLOUR),
        PointMode::Between => (BETWEEN_DISC_PIXELS, BETWEEN_COLOUR),
    };

    if selected {
        (pixels + SELECTION_PIXELS, Color::WHITE)
    } else {
        (pixels, colour)
    }
}

/// A disc on every point near the camera, facing it, sized and coloured by disc_style.
pub(super) fn draw_point_discs(
    mut gizmos: Gizmos<RailEditorGizmos>,
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    grids: Grids,
    camera: Query<EditorCamera, With<OrbitalCameraController>>,
) {
    // Dirty, the positions are a mutation behind the points and the selection indexes the
    // points, so a disc could land on the wrong point for this one frame. Skip it instead.
    if !editor.editing || !rail.visible || rail.dirty {
        return;
    }

    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };
    // None for a frame or two at startup, before the window has told the camera its size.
    let Some(viewport) = camera.camera.logical_viewport_size() else {
        return;
    };
    let Projection::Perspective(perspective) = camera.projection else {
        return;
    };

    // The focal length in pixels: a metre at a metre's distance covers this many pixels, so
    // a disc of so many pixels at a distance is that many over this, in metres.
    let focal_pixels = viewport.y / (2.0 * (perspective.fov / 2.0).tan());

    // As the lines: positions are absolute and large, gizmos want them relative to the
    // camera's cell, and the subtraction has to happen in f64.
    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    for (line_index, line) in rail.network.lines.iter().enumerate() {
        let colour = line_colour(&line.name);

        for (index, (point, &position)) in line.points.iter().zip(&line.positions).enumerate() {
            let distance = position.distance(camera_position);
            if distance > DISC_REACH {
                continue;
            }
            let render_position = (position - cell_origin).as_vec3();
            let Ok(on_screen) = camera
                .camera
                .world_to_viewport(camera.global, render_position)
            else {
                continue;
            };
            if !(on_screen.cmpge(Vec2::ZERO).all() && on_screen.cmple(viewport).all()) {
                continue;
            }

            let selected = editor.selection.contains(&(line_index, index));
            let (pixels, disc_colour) = disc_style(point.mode, selected, colour);
            let radius = distance as f32 * pixels / (2.0 * focal_pixels);
            let facing = (camera_position - position).normalize().as_vec3();

            gizmos
                .circle(
                    Isometry3d::new(render_position, Quat::from_rotation_arc(Vec3::Z, facing)),
                    radius,
                    disc_colour,
                )
                .resolution(DISC_RESOLUTION);
        }
    }
}
