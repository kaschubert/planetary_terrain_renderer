use crate::debug::PointerCapture;
use crate::picking::PickingData;
use bevy::{
    color::palettes::basic,
    input::{
        ButtonInput,
        mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel},
    },
    math::{DQuat, DVec2, DVec3, Mat4, Vec2},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use big_space::prelude::*;

/// Radians of orbit per pixel of a middle-button drag: about three tenths of a degree, so a
/// drag across a third of a 1920 px window goes half way round the point under the cursor.
const ROTATION_PER_PIXEL: f64 = 0.005;

/// What a pixel of right-button drag and a notch of the wheel do to the distance, in
/// doublings: a 140 px drag or five notches double it or halve it, the feel the spherical
/// example's chase camera was tuned to by hand.
const ZOOM_PER_PIXEL: f64 = 0.0072;
const ZOOM_PER_NOTCH: f64 = 0.2;

/// A wheel zoom is over once the eased distance is within this many doublings of its
/// target: well under a percent, which is under a metre at any distance the wheel is used
/// from.
const ZOOM_SETTLED: f64 = 0.001;

fn ray_sphere_intersection(
    ray_origin: DVec3,
    ray_direction: DVec3,
    sphere_origin: DVec3,
    radius: f64,
) -> Option<DVec3> {
    let oc = ray_origin - sphere_origin;
    let b = 2.0 * oc.dot(ray_direction);
    let c = oc.dot(oc) - radius * radius;

    let sqrt_discriminant = (b * b - 4.0 * c).sqrt();

    if sqrt_discriminant.is_nan() {
        return None; // No intersection
    }

    // Compute the roots of the quadratic equation
    let t1 = (-b - sqrt_discriminant) / 2.0;
    let t2 = (-b + sqrt_discriminant) / 2.0;

    [t1, t2]
        .into_iter()
        .filter(|&t| t >= 0.0)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .map(|t| ray_origin + ray_direction * t)
}

#[derive(Clone, Copy, Debug)]
pub struct PanData {
    pan_coords: Vec2,
    world_from_clip: Mat4,
}

#[derive(Clone, Copy, Debug)]
pub struct ZoomData {
    target_zoom: f64,
    zoom: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct RotationData {
    target_rotation: DVec2,
    rotation: DVec2,
    initial_tilt: f64,
}

#[derive(Clone, Debug, Component)]
#[require(PickingData, Camera3d, FloatingOrigin = FloatingOrigin)]
pub struct OrbitalCameraController {
    /// Whether the controller moves the camera. R toggles it; the spherical example's chase
    /// camera turns it off while it has the view, and on again when it lets go.
    pub enabled: bool,
    /// Whether the mouse wheel zooms. The spherical example's sheet grid takes the wheel for
    /// its height while its box is ticked, and turns this off meanwhile.
    pub wheel_zooms: bool,
    cursor_coords: Vec2,
    anchor_position: DVec3,
    anchor_cell: CellCoord,
    camera_position: DVec3,
    camera_rotation: DQuat,
    pan_data: Option<PanData>,
    zoom_data: Option<ZoomData>,
    rotation_data: Option<RotationData>,
    time_to_reach_target: f64,
}

impl Default for OrbitalCameraController {
    fn default() -> Self {
        Self {
            enabled: true,
            wheel_zooms: true,
            zoom_data: None,
            pan_data: None,
            rotation_data: None,
            time_to_reach_target: 0.1,
            cursor_coords: Vec2::ZERO,
            anchor_position: Default::default(),
            anchor_cell: Default::default(),
            camera_position: Default::default(),
            camera_rotation: Default::default(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn orbital_camera_controller(
    mut gizmos: Option<Gizmos>,
    grids: Grids,
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse_move: Res<AccumulatedMouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    capture: Res<PointerCapture>,
    mut camera: Query<(
        Entity,
        &mut Transform,
        &mut CellCoord,
        &PickingData,
        &mut OrbitalCameraController,
    )>,
    mut window: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let Ok((camera, mut camera_transform, mut camera_cell, picking_data, mut controller)) =
        camera.single_mut()
    else {
        return;
    };

    keyboard
        .just_pressed(KeyCode::KeyR)
        .then(|| controller.enabled = !controller.enabled);

    if !controller.enabled {
        return;
    }

    let smoothing = (time.delta_secs_f64() / controller.time_to_reach_target).min(1.0);
    let grid = grids.parent_grid(camera).unwrap();
    let (mut window, mut cursor_options) = window.single_mut().unwrap();

    let terrain_origin = DVec3::ZERO;
    let camera_position = grid.grid_position_double(&camera_cell, &camera_transform);
    let camera_rotation = camera_transform.rotation.as_dquat();
    let mut new_camera_position = camera_position;
    let mut new_camera_rotation = camera_rotation;

    let cursor_cell = picking_data.cell;
    let cursor_position = picking_data.translation.map(|translation| {
        grid.grid_position_double(&cursor_cell, &Transform::from_translation(translation))
    });
    let cursor_coords = picking_data.cursor_coords;

    let mut update_cursor_coords = true;

    if mouse_buttons.pressed(MouseButton::Left) {
        if controller.pan_data.is_none() && cursor_position.is_some() && !capture.blocks_pointer() {
            controller.anchor_position = cursor_position.unwrap();
            controller.anchor_cell = cursor_cell;
            controller.camera_position = camera_position;
            controller.camera_rotation = camera_rotation;
            controller.pan_data = Some(PanData {
                world_from_clip: picking_data.world_from_clip,
                pan_coords: cursor_coords,
            });
        }

        if let Some(data) = &mut controller.pan_data {
            data.pan_coords = data.pan_coords.lerp(cursor_coords, smoothing as f32);
        }
    } else {
        controller.pan_data = None;
    }

    if mouse_buttons.pressed(MouseButton::Middle) {
        if controller.rotation_data.is_none()
            && cursor_position.is_some()
            && !capture.blocks_pointer()
        {
            controller.anchor_position = cursor_position.unwrap();
            controller.anchor_cell = cursor_cell;
            controller.camera_position = camera_position;
            controller.camera_rotation = camera_rotation;
            controller.rotation_data = Some(RotationData {
                target_rotation: DVec2::ZERO,
                rotation: DVec2::ZERO,
                initial_tilt: (controller.anchor_position - terrain_origin)
                    .angle_between(controller.camera_position - controller.anchor_position),
            });
        } else {
            update_cursor_coords = false;
        }

        if let Some(data) = controller.rotation_data.as_mut() {
            // The camera goes the way the mouse drags: right takes it round to the right of
            // the anchor and up lifts it to look down more, as the spherical example's chase
            // camera moves. The clamp keeps the tilt between straight above the anchor and
            // the ground whichever way the mouse maps to it.
            // Todo: fix tilt clamping
            data.target_rotation += mouse_move.delta.as_dvec2() * ROTATION_PER_PIXEL;
            data.target_rotation.y = data.target_rotation.y.clamp(
                -data.initial_tilt,
                std::f64::consts::FRAC_PI_2 - data.initial_tilt,
            );

            data.rotation = data.rotation.lerp(data.target_rotation, smoothing);
        }
    } else {
        controller.rotation_data = None;
    }

    // The wheel zooms as the right button does, towards the point under the cursor. It is
    // read every frame, so notches do not pile up while something else has the wheel.
    let notches: f64 = wheel
        .read()
        .map(|scroll| match scroll.unit {
            MouseScrollUnit::Line => scroll.y as f64,
            // A touchpad reports pixels; a line is about twenty of them.
            MouseScrollUnit::Pixel => scroll.y as f64 / 20.0,
        })
        .sum();
    let dragging_zoom = mouse_buttons.pressed(MouseButton::Right);
    let wheeling = controller.wheel_zooms && notches != 0.0 && !capture.blocks_pointer();

    if dragging_zoom || wheeling {
        if controller.zoom_data.is_none() && cursor_position.is_some() && !capture.blocks_pointer()
        {
            controller.anchor_position = cursor_position.unwrap();
            controller.anchor_cell = cursor_cell;
            controller.camera_position = camera_position;
            controller.camera_rotation = camera_rotation;

            let zoom = (cursor_position.unwrap() - camera_position).length().log2();

            controller.zoom_data = Some(ZoomData {
                target_zoom: zoom,
                zoom,
            });
        } else if dragging_zoom {
            update_cursor_coords = false;
        }

        if let Some(data) = controller.zoom_data.as_mut() {
            // Dragging down draws the camera out and up brings it in, and a notch of the
            // wheel up brings it in, in doublings of the distance so a step feels the same
            // near and far.
            if dragging_zoom {
                data.target_zoom += mouse_move.delta.y as f64 * ZOOM_PER_PIXEL;
            }
            data.target_zoom -= notches * ZOOM_PER_NOTCH;
            data.zoom = data.zoom.lerp(data.target_zoom, smoothing);
        }
    } else if mouse_buttons.pressed(MouseButton::Left) || mouse_buttons.pressed(MouseButton::Middle)
    {
        // A pan or a rotate starting takes over from a wheel zoom still easing in.
        controller.zoom_data = None;
    } else if let Some(data) = controller
        .zoom_data
        .as_mut()
        .filter(|data| (data.zoom - data.target_zoom).abs() > ZOOM_SETTLED)
    {
        // A wheel zoom eases on to its target after the notch, as a drag's does while the
        // button is held.
        data.zoom = data.zoom.lerp(data.target_zoom, smoothing);
    } else {
        controller.zoom_data = None;
    }

    if update_cursor_coords {
        if cursor_options.grab_mode == CursorGrabMode::Locked {
            cursor_options.grab_mode = CursorGrabMode::None;
            let window_size = window.size();
            window.set_cursor_position(Some(
                Vec2::new(controller.cursor_coords.x, 1.0 - controller.cursor_coords.y)
                    * window_size,
            ));
        }

        controller.cursor_coords = cursor_coords;
    } else {
        cursor_options.grab_mode = CursorGrabMode::Locked;
    }

    if controller.pan_data.is_none()
        && controller.rotation_data.is_none()
        && controller.zoom_data.is_none()
    {
        controller.anchor_position = cursor_position.unwrap_or(DVec3::NAN);
    }

    if let Some(pan_data) = controller.pan_data {
        // Invariants:
        // The anchor world position remains at the screen space position of the cursor.
        // The terrain is just rotated, but not translated relative to the camera.

        // Todo: calculate this without world_from_clip using the rule of three
        // this should be possible to compute using

        let ndc_coords = (pan_data.pan_coords * 2.0 - 1.0).extend(0.0001); // Todo: using f64 we should be able to set this to 1.0 for the near plane
        let translation = pan_data.world_from_clip.project_point3(ndc_coords);
        let new_cursor_position = grid.grid_position_double(
            &controller.anchor_cell,
            &Transform::from_translation(translation),
        );

        let camera_cursor_direction =
            (new_cursor_position - controller.camera_position).normalize();

        let radius = (controller.anchor_position - terrain_origin).length();

        // compute ray sphere intersection, where the sphere has a radius of the length of the anchor position
        // this way the anchor point should line up correctly with the cursor
        let Some(new_cursor_position) = ray_sphere_intersection(
            controller.camera_position,
            camera_cursor_direction,
            terrain_origin,
            radius,
        ) else {
            controller.pan_data = None;
            return;
        };

        // based of the anchor position and the cursor hit position compute the new camera transform
        // the world origin should stay at the center of the screen
        let initial_direction = (controller.anchor_position - terrain_origin).normalize();
        let new_direction = (new_cursor_position - terrain_origin).normalize();

        // the camera should be rotated by this amount, so that the panning anchor ends up under the cursor
        let rotation = DQuat::from_rotation_arc(new_direction, initial_direction);

        new_camera_position =
            terrain_origin + rotation * (controller.camera_position - terrain_origin);
        new_camera_rotation = rotation * controller.camera_rotation;
    }

    if let Some(rotation_data) = controller.rotation_data {
        // Invariants:
        // The cursor world position stays at the same screen-space location.
        // The distance between anchor and camera remains constant.

        let heading_axis = (controller.anchor_position - terrain_origin).normalize(); // terrain normal
        let tilt_axis = controller.camera_rotation * DVec3::X; // camera right direction

        let rotation_heading = DQuat::from_axis_angle(heading_axis, rotation_data.rotation.x);
        let rotation_tilt = DQuat::from_axis_angle(tilt_axis, rotation_data.rotation.y);
        let rotation = rotation_heading * rotation_tilt;

        new_camera_position = controller.anchor_position
            + rotation * (controller.camera_position - controller.anchor_position);
        new_camera_rotation = rotation * controller.camera_rotation;
    }

    if let Some(zoom_data) = controller.zoom_data {
        // Invariants:
        // The terrain origin stays at the screen center.
        // The cursor world position stays at the same screen-space location.

        let anchor_terrain = controller.anchor_position - terrain_origin;
        let camera_terrain = terrain_origin - controller.camera_position;
        let camera_anchor = controller.anchor_position - controller.camera_position;

        // compute the side lengths and the angles of the triangle anchor - terrain origin - new camera
        let a = anchor_terrain.length();
        let b = 2.0_f64.powf(zoom_data.zoom);

        let alpha = camera_terrain.angle_between(camera_anchor);
        let beta = (b / a * alpha.sin()).asin();
        let gamma = std::f64::consts::PI - alpha - beta;

        let c = f64::sqrt(a * a + b * b - 2.0 * a * b * gamma.cos());

        if beta.is_nan() {
            controller.zoom_data = None;
            return;
        }

        // rotation from the anchor direction towards the initial camera direction
        let rotation =
            DQuat::from_axis_angle(camera_terrain.cross(camera_anchor).normalize(), beta);

        let camera_position = terrain_origin + rotation * (c * anchor_terrain.normalize());

        let initial_direction = camera_terrain.normalize();
        let new_direction = (terrain_origin - camera_position).normalize();

        new_camera_position = camera_position;
        new_camera_rotation =
            DQuat::from_rotation_arc(initial_direction, new_direction) * controller.camera_rotation;
    }

    let (new_cell, new_translation) = grid.translation_to_grid(new_camera_position);

    *camera_cell = new_cell;
    camera_transform.translation = new_translation;
    camera_transform.rotation = new_camera_rotation.as_quat();

    let anchor_size = 200.0;

    // Gizmos come from bevy_gizmos, which an app may run the debug plugin without; then the
    // anchor is simply not drawn.
    if let Some(gizmos) = gizmos.as_mut() {
        gizmos.sphere(
            (controller.anchor_position - grid.cell_to_float(&new_cell)).as_vec3(),
            new_camera_position.distance(controller.anchor_position) as f32 / anchor_size,
            basic::GREEN,
        );
    }
}
