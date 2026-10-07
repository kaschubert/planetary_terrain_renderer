//! The overlays and tools the spherical example lays over the terrain, one plugin per file
//! with its own modules in a folder of the same name. What more than one plugin uses lives
//! in shared; what only one uses lives with it, as the rail editor's frame does.

pub mod auckland_rail;
pub mod live_trains;
pub mod provenance;
pub mod rail_editor;
pub mod shared;
pub mod sheet_grid;
pub mod stations;
pub mod track_frames;
pub mod trains;
pub mod vram_usage;
