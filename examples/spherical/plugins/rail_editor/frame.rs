//! The east, north and up directions at a point on the spheroid: the frame the move gizmo's
//! arrows stand in, and the one the track frames are built from.
//!
//! Up is the direction heights move along in TerrainShape::position_unit_to_local, the
//! direction of the point on the spheroid from the centre, so that a distance along it
//! changes a height by exactly that distance. That is within a fifth of a degree of the
//! geodetic normal, which nothing here can tell. Local X is east, local Y is up and local Z
//! is south, which is the right-handed frame (east across up is south) that a rotation can
//! hold, with Y up as Bevy has it.

use bevy::math::{DMat3, DQuat, DVec3};
use bevy_terrain::prelude::TerrainShape;

/// The east, north and up directions at a point on the spheroid, unit length and mutually
/// perpendicular.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub east: DVec3,
    pub north: DVec3,
    pub up: DVec3,
}

impl Frame {
    /// The frame at a direction on the unit sphere. Up is where position_unit_to_local
    /// puts a height, north the part of the planet's axis that is level there, and east
    /// across them towards increasing longitude. At a pole north has no meaning and is
    /// taken along x, which nothing on the network is near.
    pub fn at_unit(unit: DVec3) -> Self {
        let up = (TerrainShape::WGS84.scale() * unit).normalize();
        let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or(DVec3::X);
        let east = north.cross(up);

        Self { east, north, up }
    }

    /// The rotation the gizmo's handle carries: local x to east, local y to up, local z to
    /// south.
    pub fn rotation(&self) -> DQuat {
        DQuat::from_mat3(&DMat3::from_cols(self.east, self.up, -self.north))
    }

    /// A translation the gizmo reports along the handle's local axes, as a displacement in
    /// space: what the rotation does to it.
    pub fn displacement(&self, local: DVec3) -> DVec3 {
        self.east * local.x + self.up * local.y - self.north * local.z
    }
}

/// The direction on the unit sphere a position is above: the inverse of
/// position_unit_to_local, which puts a height along the direction of the point on the
/// spheroid from the centre, so that dividing by the axes and normalising undoes it exactly
/// whatever the height. position_local_to_unit instead drops a perpendicular onto the
/// spheroid, the geodetic answer, which differs in direction by up to a fifth of a degree:
/// enough that a point lifted 50 m would wander 16 cm sideways. The two agree on the
/// surface, which is where a double-click's terrain hit is.
pub fn unit_under(position: DVec3) -> DVec3 {
    (position / TerrainShape::WGS84.scale()).normalize()
}

/// The geodetic normal at a direction on the unit sphere, the true vertical. The unit sphere
/// maps to the spheroid by scaling, so a direction's latitude is the parametric one, and the
/// normal is the gradient of the spheroid there: the direction scaled by the inverse axes.
/// It leans off Frame::at_unit's up, the direction heights run along, by up to a fifth of a
/// degree in the mid latitudes.
pub fn geodetic_normal(unit: DVec3) -> DVec3 {
    (unit / TerrainShape::WGS84.scale()).normalize()
}

// Visible to the other test modules, which borrow its places and its angle helper.
#[cfg(test)]
pub(crate) mod tests;
