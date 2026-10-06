//! The overlays and tools the spherical example lays over the terrain, one plugin per file
//! with its own modules in a folder of the same name. What more than one plugin uses lives
//! in shared; what only one uses lives with it.

pub mod auckland_rail;
pub mod provenance;
pub mod shared;
pub mod sheet_grid;
pub mod vram_usage;
