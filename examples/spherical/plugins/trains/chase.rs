//! A third-person camera that rides behind a train, from a click on the camera icon in the
//! trains table, that the mouse orbits round the train and zooms, and that lets go when the
//! user asks for the camera back.
//!
//! The pose comes from the same frame the carriage stands in, see carriage_frame, so the
//! camera and the carriage agree: the eye sits on an orbit round the carriage, by default
//! behind and above it along the frame's own axes, and looks at a point a little past it,
//! so the carriage sits in the lower half of the view and the line beyond is what the
//! viewer watches. A drag turns the orbit round the carriage's up and tilts it, the wheel
//! or a drag with the middle button draws it in or out, see Orbit. The heading the orbit is
//! built on is smoothed towards the frame's with an exponential lag, so that the swing round
//! when a train reverses at the end of its line carries the eye round the carriage over
//! about a second rather than a frame; the position is not smoothed, see chase_heading, so
//! the eye keeps its distance while the train runs. On the frame of attaching the heading
//! is taken as it is, since there is nothing to lag from.
//!
//! The pose is written straight into the view entity's Transform and CellCoord through
//! move_to, as the editor's fly-to is. The orbital camera is switched off while the chase
//! has the view: it reads its pose back from those two components every frame and writes
//! new ones from whatever it finds there, which is harmless while it has no input, but a
//! drag would have it pan or zoom from the pose the chase left while the chase rides the
//! train from where it is, and the view would be fought over frame by frame. It is switched
//! back on, as it was, when the chase lets go. The fly camera, when on, rewrites the
//! rotation every frame with its roll zeroed about the planet's axis, which at Auckland is
//! nowhere near level, so attaching turns it off too; T turns it on again, which lets go.
//!
//! The camera is released when the user asks for it back: T; the fly camera's keys; R,
//! which turns the orbital camera on; Escape; F7 turning the trains off, or F4 the lines
//! they run on; the followed train's icon clicked again; the train disappearing; and the
//! camera found somewhere other than where the chase left it, which is how a fly-to from
//! the editor's panel lets go without either module knowing of the other.

use super::table::FollowButton;
use super::{Trains, carriage_frame};
use crate::plugins::auckland_rail::AucklandRail;
use crate::plugins::provenance::ascii;
use crate::plugins::track_frames::{TrackFrame, TrackSplines, move_to};
use bevy::{
    input::mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel},
    math::{DMat3, DQuat, DVec3},
    prelude::*,
};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};

/// How far behind the carriage the eye starts, along the heading's forward, in metres. With
/// the height below, the eye is about 65 m from the carriage, and a 24 m carriage fills about
/// a third of the view from there: near enough to read, far enough to see where it is going.
/// The two together are the default orbit's distance and pitch, see Orbit.
pub const CHASE_BACK: f64 = 60.0;

/// How far above the rail the eye starts, along the heading's up, in metres: high enough to
/// see over the carriage to the track ahead, low enough that the view is of a train and not
/// of a map.
pub const CHASE_UP: f64 = 25.0;

/// How far past the carriage the camera looks, in metres, along the view's bearing, so that
/// the carriage sits in the lower half of the frame and the line beyond is what the viewer
/// watches, from whichever side the orbit has come round to.
pub const CHASE_AHEAD: f64 = 20.0;

/// The time constant of the heading's lag behind the carriage's, in seconds: each second the
/// heading closes all but e to the minus three of the turn left, so a train reversing at the
/// end of its line swings the camera round over about a second, which the eye follows, where
/// a frame would be a cut.
pub const CHASE_SMOOTHING: f64 = 0.3;

/// How far the orbit turns per pixel of drag, in radians: about three tenths of a degree, so
/// a drag across a third of a 1920 px window goes half way round the carriage, which is the
/// feel of the orbital camera's rotate.
pub const TURN_PER_PIXEL: f64 = 0.005;

/// How flat the orbit may lie and how steeply it may look down, in degrees above the
/// carriage's level. On the rail itself the ground hides the train; straight down leaves the
/// view no bearing to keep the carriage in its lower half by.
pub const PITCH_DEGREES: (f64, f64) = (5.0, 85.0);

/// How near the eye may be, in metres: nearer than 20 m a 24 m carriage no longer fits the
/// view. There is no far limit; drawn out far enough the carriage is a dot on its line and
/// the line a thread across the city, which is a view of its own.
pub const NEAREST_METRES: f64 = 20.0;

/// What a pixel of drag with the middle button, and a notch of the wheel, do to the distance,
/// as exponents of e: 140 px of drag or five notches double it or halve it, a touch finer
/// than the orbital camera's zoom so the carriage can be framed.
pub const ZOOM_PER_PIXEL: f64 = 0.005;
pub const ZOOM_PER_NOTCH: f64 = 0.14;

/// How far the camera may be found from where the chase left it before another hand is
/// taken to have moved it, in metres and in degrees. The orbital camera, when it is on,
/// writes the pose back through the grid's f32 split each frame, which at a cell's width
/// moves it by a tenth of a millimetre at most; a fly-to moves it hundreds of metres. Half a
/// metre and half a degree sit between the two with room on either side.
const MOVED_METRES: f64 = 0.5;
const MOVED_DEGREES: f64 = 0.5;

/// The chase camera's systems, added by TrainsPlugin.
pub(super) struct ChaseCameraPlugin;

impl Plugin for ChaseCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChaseCamera>()
            .add_systems(Update, press_follow_buttons)
            // Before the transforms propagate, beside place_trains: the library's camera
            // controllers are not exported to order after, and PostUpdate is after
            // everything in Update without naming any of it, the controllers and the
            // panel's fly-to among them. Before propagation, so the camera's global
            // transform this frame is the chase pose, and the labels, which project through
            // it after propagation, land on the carriages as they are this frame.
            .add_systems(PostUpdate, follow_train.before(TransformSystems::Propagate));
    }
}

/// Where the eye sits round the carriage: turned round the carriage's up from straight
/// behind, tilted above its level, and at a distance. The default is straight behind at the
/// pitch and distance CHASE_BACK and CHASE_UP make, and an attach from a free camera starts
/// there; a switch from one train to another keeps the distance, see orbit_on_attach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    /// Radians round the carriage's up from straight behind it, positive towards its right.
    pub yaw: f64,
    /// Radians above the carriage's level, within PITCH_DEGREES.
    pub pitch: f64,
    /// Metres from the carriage to the eye, NEAREST_METRES at least.
    pub distance: f64,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: CHASE_UP.atan2(CHASE_BACK),
            distance: CHASE_BACK.hypot(CHASE_UP),
        }
    }
}

impl Orbit {
    /// A drag of so many pixels right and down turns the orbit that way: right goes round
    /// towards the carriage's right, and down brings the eye down to look at it flatter, as
    /// the orbital camera's rotate does.
    pub fn turn(&mut self, pixels: Vec2) {
        self.yaw += pixels.x as f64 * TURN_PER_PIXEL;
        self.pitch = (self.pitch - pixels.y as f64 * TURN_PER_PIXEL)
            .clamp(PITCH_DEGREES.0.to_radians(), PITCH_DEGREES.1.to_radians());
    }

    /// Scales the distance by e to the exponent, no nearer than NEAREST_METRES: positive
    /// draws out.
    pub fn zoom(&mut self, exponent: f64) {
        self.distance = (self.distance * exponent.exp()).max(NEAREST_METRES);
    }
}

/// Which train the camera rides behind, where on its orbit, the heading it rides at and the
/// pose it was last given. Default is the camera free, and attaching and letting go both
/// reset to it but for the train.
#[derive(Resource, Default)]
pub struct ChaseCamera {
    /// The carriage entity of the train followed, None when the camera is free. The entity
    /// and not the index into Trains::trains: the live feed adds and removes trains, and the
    /// indices shift under a train that is still there.
    pub following: Option<Entity>,
    /// Where the eye sits round the carriage, which the mouse moves.
    pub orbit: Orbit,
    /// The heading the pose is built on, smoothed towards the carriage frame's rotation, see
    /// chase_heading. None on the frame of attaching, when the frame's is taken as it is.
    pub heading: Option<DQuat>,
    /// The position and rotation last written, in absolute WGS84 metres, which the camera is
    /// found against next frame to tell whether another hand has moved it.
    pub written: Option<(DVec3, DQuat)>,
    /// The button a drag began with over the terrain, until it is released; a drag that
    /// wanders over the table keeps turning, as the orbital camera's would.
    pub dragging: Option<MouseButton>,
    /// Whether the orbital camera was on when the chase took the view, to give it back as it
    /// was.
    pub orbital_was_on: bool,
}

/// What the user did to ask for the camera back this frame, each a reason to let go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Touch {
    /// T, which turns the fly camera on.
    pub fly_toggled: bool,
    /// An arrow key, PageUp, PageDown, Home or End: the fly camera's keys.
    pub fly_key: bool,
    /// R, which turns the orbital camera back on.
    pub orbital_toggled: bool,
    pub escape: bool,
    /// F7 has hidden the trains, or F4 the network and the carriages with it; the table has
    /// gone with them either way.
    pub trains_hidden: bool,
    /// No train has the followed carriage any more.
    pub train_gone: bool,
    /// The camera is not where the chase left it, so something else has moved it.
    pub moved_elsewhere: bool,
}

/// Why the camera is let go of, in the words the log prints, or None while nothing has been
/// touched. Any one touch is enough; the first in this order is the one named when several
/// land on one frame.
pub fn reason_to_let_go(touch: Touch) -> Option<&'static str> {
    if touch.fly_toggled {
        Some("T was pressed")
    } else if touch.fly_key {
        Some("a fly key was pressed")
    } else if touch.orbital_toggled {
        Some("R was pressed")
    } else if touch.escape {
        Some("Escape was pressed")
    } else if touch.trains_hidden {
        Some("the trains were hidden")
    } else if touch.train_gone {
        Some("the train is gone")
    } else if touch.moved_elsewhere {
        Some("the camera was moved")
    } else {
        None
    }
}

/// What a click on a train's icon does: follows that train's carriage, or lets go when it is
/// the one already followed, so the one icon is both the switch on and the switch off.
pub fn follow_or_let_go(following: Option<Entity>, clicked: Entity) -> Option<Entity> {
    if following == Some(clicked) {
        None
    } else {
        Some(clicked)
    }
}

/// The orbit a train is attached at: from a free camera, the default, straight behind; from
/// another train, the default at the distance the orbit had, so a viewer who has drawn in
/// to read one carriage reads the next from as close, and one who has drawn out to see a
/// line sees the next train's line too. The turn and the tilt start over, since they were
/// about the other train.
pub fn orbit_on_attach(previous: Option<&Orbit>) -> Orbit {
    match previous {
        Some(orbit) => Orbit {
            distance: orbit.distance,
            ..Orbit::default()
        },
        None => Orbit::default(),
    }
}

/// The rotation that looks from one position at another with the up given, as a Transform's
/// looking_at does, in f64: local -Z towards the target, local X level across the up, and
/// local Y what is left, which leans back from the up by the pitch.
fn looking_at(eye: DVec3, target: DVec3, up: DVec3) -> DQuat {
    let forward = (target - eye).normalize();
    let right = forward.cross(up).normalize();
    let up = right.cross(forward);

    DQuat::from_mat3(&DMat3::from_cols(right, up, -forward))
}

/// The heading the pose is built on: the frame's rotation, approached from the previous
/// heading by the share of the turn an exponential lag of CHASE_SMOOTHING closes in the time
/// given, so a frame that turns round is swung to over about a second and one that holds its
/// course is matched. With no previous heading the frame's is taken, which is the snap on
/// attaching. A pause long against the time constant closes the turn altogether, which is a
/// snap, and right.
///
/// Only the rotation is smoothed. A position smoothed the same way trails a moving carriage
/// by its speed times the time constant, 6 m at line speed, and the eye would ride that much
/// further back than the orbit says the whole run; a heading lags only while the carriage
/// turns, by a fraction of a degree on the sharpest curve, so the eye keeps its distance. At
/// the half turn of a reversal the slerp goes whichever way round is the shorter, which the
/// curve at the end of the line decides.
pub fn chase_heading(frame: &TrackFrame, dt: f64, previous: Option<DQuat>) -> DQuat {
    match previous {
        None => frame.rotation,
        Some(previous) => {
            let share = 1.0 - (-dt / CHASE_SMOOTHING).exp();
            previous.slerp(frame.rotation, share)
        }
    }
}

/// The camera's pose on its orbit round a carriage at a position, on a heading. The view's
/// bearing is the heading's forward turned round its up by the orbit's yaw, so from behind
/// it is the train's way and from the side it is across the train; the eye sits the orbit's
/// distance back along that bearing and up by its pitch, and looks at the point CHASE_AHEAD
/// past the carriage along it, with the heading's up as up. While the heading swings round
/// at the end of a line the whole orbit goes round with it, so the carriage stays in the
/// view the whole way.
pub fn chase_pose(position: DVec3, heading: DQuat, orbit: &Orbit) -> (DVec3, DQuat) {
    let up = heading * DVec3::Y;
    let bearing = DQuat::from_axis_angle(up, orbit.yaw) * (heading * DVec3::NEG_Z);
    let eye = position - bearing * (orbit.distance * orbit.pitch.cos())
        + up * (orbit.distance * orbit.pitch.sin());

    (eye, looking_at(eye, position + bearing * CHASE_AHEAD, up))
}

/// Gives the view back: the camera free, and the orbital camera on again if it was when the
/// chase took it. Not when R has just turned it on by hand, which is the one state the user
/// has asked for outright.
fn let_go(chase: &mut ChaseCamera, orbital: &mut OrbitalCameraController, hand_back: bool) {
    if hand_back {
        orbital.enabled = chase.orbital_was_on;
    }
    *chase = ChaseCamera::default();
}

/// A press on a train's icon follows that train, or lets go of it when it is the one
/// followed. Attaching takes the view from both controllers, see the module doc, and starts
/// the orbit straight behind with no heading and no pose, so the first frame snaps. A press
/// on the row of a train whose carriage is not spawned yet does nothing.
fn press_follow_buttons(
    buttons: Query<(&FollowButton, &Interaction), Changed<Interaction>>,
    mut chase: ResMut<ChaseCamera>,
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    mut orbital: Query<&mut OrbitalCameraController>,
    mut fly: Query<&mut DebugCameraController>,
) {
    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(train) = trains.trains.get(button.0) else {
            continue;
        };
        let Some(carriage) = train.entity else {
            continue;
        };
        let Ok(mut orbital) = orbital.single_mut() else {
            return;
        };

        match follow_or_let_go(chase.following, carriage) {
            Some(carriage) => {
                // Switching trains keeps what was found at the first attach, since the
                // controller is off now by the chase's own hand, and keeps the zoom, see
                // orbit_on_attach.
                let (orbital_was_on, orbit) = if chase.following.is_some() {
                    (chase.orbital_was_on, orbit_on_attach(Some(&chase.orbit)))
                } else {
                    (orbital.enabled, orbit_on_attach(None))
                };
                *chase = ChaseCamera {
                    following: Some(carriage),
                    orbital_was_on,
                    orbit,
                    ..default()
                };
                orbital.enabled = false;
                for mut fly in &mut fly {
                    fly.enabled = false;
                }
                let name = ascii(train.line_name(&rail));
                let unit = ascii(&train.id);
                info!("trains: chase camera on the {name} train {unit}");
            }
            None => {
                let_go(&mut chase, &mut orbital, true);
                info!("trains: chase camera let go");
            }
        }
    }
}

/// Rides the camera round the followed train, turning and zooming the orbit as the mouse
/// says, or lets go of it when asked. The frame is the carriage's, see carriage_frame, so
/// the two agree; a train whose line has no spline this frame, as on the frame after an
/// insert, is kept but not followed this frame.
#[allow(clippy::too_many_arguments)]
fn follow_train(
    mut chase: ResMut<ChaseCamera>,
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    splines: Res<TrackSplines>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    capture: Res<PointerCapture>,
    grids: Grids,
    mut camera: Query<(
        Entity,
        &mut Transform,
        &mut CellCoord,
        &mut OrbitalCameraController,
    )>,
) {
    let Some(following) = chase.following else {
        return;
    };
    let Ok((entity, mut transform, mut cell, mut orbital)) = camera.single_mut() else {
        return;
    };
    let Some(grid) = grids.parent_grid(entity) else {
        return;
    };
    let train = trains
        .trains
        .iter()
        .find(|train| train.entity == Some(following));

    // Where the camera is now against where the chase left it, in f64 as the orbital
    // camera reads its own; nothing to compare on the frame of attaching.
    let moved_elsewhere = chase.written.is_some_and(|(position, rotation)| {
        let found = grid.grid_position_double(&cell, &transform);
        found.distance(position) > MOVED_METRES
            || transform.rotation.as_dquat().angle_between(rotation) > MOVED_DEGREES.to_radians()
    });
    let touch = Touch {
        fly_toggled: keys.just_pressed(KeyCode::KeyT),
        fly_key: [
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ]
        .into_iter()
        .any(|key| keys.just_pressed(key)),
        orbital_toggled: keys.just_pressed(KeyCode::KeyR),
        escape: keys.just_pressed(KeyCode::Escape),
        trains_hidden: !trains.drawn(&rail),
        train_gone: train.is_none(),
        moved_elsewhere,
    };
    if let Some(reason) = reason_to_let_go(touch) {
        let_go(&mut chase, &mut orbital, !touch.orbital_toggled);
        info!("trains: chase camera let go, {reason}");
        return;
    }
    let Some(train) = train else {
        return;
    };

    // A drag begun over the terrain turns or zooms the orbit until its button is let go,
    // wherever the pointer wanders meanwhile; one begun over the UI is a click on something
    // there. The wheel zooms whether or not a drag is on.
    match chase.dragging {
        Some(button) if !mouse.pressed(button) => chase.dragging = None,
        None if !capture.blocks_pointer() => {
            chase.dragging = [MouseButton::Left, MouseButton::Middle, MouseButton::Right]
                .into_iter()
                .find(|&button| mouse.just_pressed(button));
        }
        _ => {}
    }
    match chase.dragging {
        Some(MouseButton::Middle) => chase.orbit.zoom(motion.delta.y as f64 * ZOOM_PER_PIXEL),
        Some(_) => chase.orbit.turn(motion.delta),
        None => {}
    }
    let notches: f32 = wheel
        .read()
        .map(|scroll| match scroll.unit {
            MouseScrollUnit::Line => scroll.y,
            // A touchpad reports pixels; a line is about twenty of them.
            MouseScrollUnit::Pixel => scroll.y / 20.0,
        })
        .sum();
    if notches != 0.0 {
        chase.orbit.zoom(-notches as f64 * ZOOM_PER_NOTCH);
    }

    let Some(spline) = splines.splines.get(train.line).and_then(Option::as_ref) else {
        return;
    };

    let frame = carriage_frame(spline, train);
    let heading = chase_heading(&frame, time.delta_secs_f64(), chase.heading);
    let (position, rotation) = chase_pose(frame.position, heading, &chase.orbit);
    chase.heading = Some(heading);
    chase.written = Some((position, rotation));
    move_to(
        grid,
        &mut cell,
        &mut transform,
        position,
        rotation.as_quat(),
    );
}

#[cfg(test)]
mod tests;
